/**
 * useCategoryDragSort.ts - 分类列表拖拽排序 / 拖拽改变层级（自定义 Pointer Events 实现，不用浏览器原生 Drag & Drop API）
 *
 * 交互规则：
 * - 按住分类名称后移动超过阈值（6px）进入拖拽；单击仍然只是选中分类；
 * - 拖拽时在 body 上挂一个自定义拖拽浮层（DOM 元素，跟随指针，显示分类名 + 当前落点提示）；
 * - 目标行按指针位置分三区：上 25% = 插到目标前、中间 50% = 成为目标的子分类、下 25% = 插到目标后；
 * - 允许跨父级拖动（拖到别的分类上会导致父级变化），后端会做防环校验，前端也会提前拦掉拖到自身后代的情况；
 * - 拖到滚动容器上下边缘自动滚动；
 * - 松手后：父级变化先调 moveCategory 改父级，再调 reorderCategories 调整同级顺序；失败 Toast 提示。
 *
 * 视图侧用法：
 * ```ts
 * const { startDrag, isDragging, dropIndicator, shouldIgnoreClick } = useCategoryDragSort({
 *   categoryTree,
 *   reorderCategories,
 *   moveCategory: (id, parentId) => updateCategory(id, undefined, parentId),
 *   onSuccess: (message) => showToast('success', message),
 *   onError: (message) => showToast('error', message),
 * })
 * ```
 * 模板：名称元素加 `@pointerdown="startDrag($event, node)"`（不要加 draggable）；
 * 行元素加 `:data-category-id="node.id"` 和 `relative`（渲染插入指示线/子分类高亮）。
 */
import { onUnmounted, ref, type Ref } from 'vue'
import type { CategoryTreeNode } from '@/stores/password'

/** 落点类型：插到前面 / 成为子分类 / 插到后面 */
export type CategoryDropPosition = 'before' | 'into' | 'after'

/** 组合式函数依赖 */
export interface CategoryDragSortOptions {
  /** 分类树（Store 的 categoryTree） */
  categoryTree: Ref<CategoryTreeNode[]>
  /** 提交同一父分类下的新顺序（Store 的 reorderCategories） */
  reorderCategories: (orderedIds: string[]) => Promise<void>
  /** 把分类移动到指定父分类（null 表示顶级；通常包装 Store 的 updateCategory(id, undefined, parentId)） */
  moveCategory: (id: string, parentId: string | null) => Promise<void>
  /** 成功放入某个分类内部（成为子分类）后的回调，视图可借此展开目标分类 */
  onNested?: (targetId: string) => void
  /** 操作成功回调（参数为提示文案） */
  onSuccess?: (message: string) => void
  /** 操作失败回调（参数为错误消息） */
  onError: (message: string) => void
}

/** 开始拖拽前需要移动的像素阈值（小于阈值视为点击） */
const DRAG_THRESHOLD_PX = 6

/** 自动滚动触发边缘宽度（像素） */
const AUTO_SCROLL_EDGE_PX = 40

/** 自动滚动每次步进（像素） */
const AUTO_SCROLL_STEP_PX = 16

/** 拖拽结束后抑制 click 的时长（毫秒），避免松手误触选中 */
const CLICK_SUPPRESS_MS = 250

/** 目标行上/下区域占比：上 25% = 插到前，下 25% = 插到后，中间 50% = 成为子分类 */
const EDGE_ZONE_RATIO = 0.25

/**
 * 分类拖拽排序组合式函数
 * @param options - 依赖项与回调
 */
export function useCategoryDragSort(options: CategoryDragSortOptions) {
  /** 当前正在拖拽的分类 ID（松手即清空） */
  const draggingId = ref<string | null>(null)

  /** 当前落点（目标分类 ID + 落点类型） */
  const dropTarget = ref<{ id: string; position: CategoryDropPosition } | null>(null)

  /** 进入拖拽前按下的分类 ID */
  let pendingNodeId: string | null = null

  /** 按下时的指针坐标 */
  let startX = 0
  let startY = 0

  /** 是否已越过阈值进入拖拽状态 */
  let dragStarted = false

  /** 拖拽中的自动滚动容器（找不到时为 null，退回窗口滚动） */
  let scrollContainer: HTMLElement | null = null

  /** 抑制 click 的截止时间戳 */
  let suppressClickUntil = 0

  /** 事件监听是否已挂载 */
  let listening = false

  /** 拖拽浮层 DOM（挂在 body 上，避免被布局/层级裁剪） */
  let ghostEl: HTMLDivElement | null = null
  let ghostHintEl: HTMLDivElement | null = null

  /** 按 ID 查找分类树节点 */
  const findNode = (id: string): CategoryTreeNode | null =>
    options.categoryTree.value.find((node) => node.id === id) ?? null

  /** 某父级下的全部兄弟节点（parentId 为 null 时取顶级；按分类树当前展示顺序） */
  const siblingsByParent = (parentId: string | null): CategoryTreeNode[] =>
    options.categoryTree.value.filter((item) => (item.parentId ?? null) === parentId)

  /**
   * 目标是否位于被拖拽分类的子树内（拖到自己的后代上会形成环，必须拦截）
   * @param target - 目标分类
   * @param dragged - 被拖拽分类
   */
  const isInsideDraggedSubtree = (
    target: CategoryTreeNode,
    dragged: CategoryTreeNode,
  ): boolean => {
    const visited = new Set<string>()
    let parentId: string | null = target.parentId ?? null
    while (parentId) {
      if (parentId === dragged.id) return true
      if (visited.has(parentId)) return false
      visited.add(parentId)
      parentId = findNode(parentId)?.parentId ?? null
    }
    return false
  }

  /**
   * 沿 DOM 向上找到可滚动容器（拖拽到边缘时自动滚动）
   * @param start - 起始元素
   */
  const findScrollContainer = (start: HTMLElement | null): HTMLElement | null => {
    let el: HTMLElement | null = start
    while (el && el !== document.body) {
      const style = window.getComputedStyle(el)
      if (
        (style.overflowY === 'auto' || style.overflowY === 'scroll') &&
        el.scrollHeight > el.clientHeight
      ) {
        return el
      }
      el = el.parentElement
    }
    return null
  }

  /**
   * 拖到滚动容器/窗口边缘时自动滚动
   * @param clientY - 指针视口 Y
   */
  const autoScroll = (clientY: number) => {
    if (scrollContainer) {
      const rect = scrollContainer.getBoundingClientRect()
      if (clientY < rect.top + AUTO_SCROLL_EDGE_PX) {
        scrollContainer.scrollTop -= AUTO_SCROLL_STEP_PX
      } else if (clientY > rect.bottom - AUTO_SCROLL_EDGE_PX) {
        scrollContainer.scrollTop += AUTO_SCROLL_STEP_PX
      }
      return
    }
    if (clientY < AUTO_SCROLL_EDGE_PX) {
      window.scrollBy(0, -AUTO_SCROLL_STEP_PX)
    } else if (clientY > window.innerHeight - AUTO_SCROLL_EDGE_PX) {
      window.scrollBy(0, AUTO_SCROLL_STEP_PX)
    }
  }

  /**
   * 创建跟随指针的拖拽浮层（直接操作 DOM，确保任何主题/层级下都可见）
   * @param label - 分类名称
   */
  const createGhost = (label: string) => {
    removeGhost()
    const root = document.createElement('div')
    root.setAttribute('data-category-drag-ghost', '')
    root.style.position = 'fixed'
    root.style.left = '0px'
    root.style.top = '0px'
    root.style.zIndex = '2147483647'
    root.style.pointerEvents = 'none'
    root.style.display = 'flex'
    root.style.flexDirection = 'column'
    root.style.gap = '2px'
    root.style.maxWidth = '240px'
    root.style.padding = '6px 10px'
    root.style.borderRadius = '8px'
    root.style.border = '1px solid var(--border, #d4d4d8)'
    root.style.background = 'var(--popover, #ffffff)'
    root.style.color = 'var(--popover-foreground, #18181b)'
    root.style.boxShadow = '0 8px 20px rgba(0, 0, 0, 0.22)'
    root.style.fontSize = '13px'
    root.style.lineHeight = '18px'
    root.style.opacity = '0.97'
    root.style.willChange = 'left, top'

    const title = document.createElement('div')
    title.textContent = label
    title.style.fontWeight = '500'
    title.style.overflow = 'hidden'
    title.style.textOverflow = 'ellipsis'
    title.style.whiteSpace = 'nowrap'

    const hint = document.createElement('div')
    hint.textContent = '拖拽中…'
    hint.style.fontSize = '11px'
    hint.style.opacity = '0.7'
    hint.style.overflow = 'hidden'
    hint.style.textOverflow = 'ellipsis'
    hint.style.whiteSpace = 'nowrap'

    root.appendChild(title)
    root.appendChild(hint)
    document.body.appendChild(root)
    ghostEl = root
    ghostHintEl = hint
  }

  /** 移除拖拽浮层 */
  const removeGhost = () => {
    ghostEl?.remove()
    ghostEl = null
    ghostHintEl = null
  }

  /**
   * 更新浮层位置（相对指针带偏移）
   * @param clientX - 指针视口 X
   * @param clientY - 指针视口 Y
   */
  const moveGhost = (clientX: number, clientY: number) => {
    if (!ghostEl) return
    ghostEl.style.left = `${clientX + 12}px`
    ghostEl.style.top = `${clientY + 10}px`
  }

  /**
   * 根据落点更新浮层里的操作提示
   * @param target - 当前落点（null 表示无有效落点）
   */
  const updateGhostHint = (target: { id: string; position: CategoryDropPosition } | null) => {
    if (!ghostHintEl) return
    if (!target) {
      ghostHintEl.textContent = '松手取消'
      return
    }
    const targetNode = findNode(target.id)
    if (!targetNode) {
      ghostHintEl.textContent = '松手取消'
      return
    }
    if (target.position === 'into') {
      ghostHintEl.textContent = `放入「${targetNode.name}」作为子分类`
    } else if (target.position === 'before') {
      ghostHintEl.textContent = `移到「${targetNode.name}」前面`
    } else {
      ghostHintEl.textContent = `移到「${targetNode.name}」后面`
    }
  }

  /**
   * 根据指针位置命中分类行并更新落点
   *
   * 通过 `data-category-id` 标记识别行；上/下 25% 为前后插入，中间 50% 为成为子分类；
   * 拖到自身后代上非法（防环）。
   * @param clientX - 指针视口 X
   * @param clientY - 指针视口 Y
   */
  const updateDropTarget = (clientX: number, clientY: number) => {
    if (!pendingNodeId) return
    const dragged = findNode(pendingNodeId)
    if (!dragged) {
      dropTarget.value = null
      return
    }

    const hit = document
      .elementFromPoint(clientX, clientY)
      ?.closest('[data-category-id]') as HTMLElement | null
    const targetId = hit?.dataset.categoryId ?? null
    if (!hit || !targetId) {
      dropTarget.value = null
      return
    }

    const targetNode = findNode(targetId)
    if (
      !targetNode ||
      targetNode.id === dragged.id ||
      isInsideDraggedSubtree(targetNode, dragged)
    ) {
      dropTarget.value = null
      return
    }

    const rect = hit.getBoundingClientRect()
    const ratio = rect.height > 0 ? (clientY - rect.top) / rect.height : 0.5
    let position: CategoryDropPosition
    if (ratio < EDGE_ZONE_RATIO) position = 'before'
    else if (ratio > 1 - EDGE_ZONE_RATIO) position = 'after'
    else position = 'into'
    dropTarget.value = { id: targetNode.id, position }
  }

  /**
   * 计算并提交落点结果（可能先改父级，再调整同级顺序）
   * @param draggedId - 被拖拽分类 ID
   * @param target - 落点
   */
  const commitDrop = async (
    draggedId: string,
    target: { id: string; position: CategoryDropPosition },
  ) => {
    const dragged = findNode(draggedId)
    const targetNode = findNode(target.id)
    if (!dragged || !targetNode || isInsideDraggedSubtree(targetNode, dragged)) return

    // 落点对应的新父级：成为子分类 → 目标本身；前/后插入 → 目标所在父级
    const newParentId =
      target.position === 'into' ? targetNode.id : (targetNode.parentId ?? null)
    const parentChanged = (dragged.parentId ?? null) !== newParentId

    if (parentChanged) {
      try {
        await options.moveCategory(draggedId, newParentId)
      } catch (e) {
        options.onError(String(e))
        return
      }
    }

    // 改父级后 categoryTree 已更新，按新父级计算同级顺序
    const siblings = siblingsByParent(newParentId)
    const ids = siblings.map((item) => item.id).filter((id) => id !== draggedId)
    if (target.position === 'into') {
      // 成为子分类：追加到子分类末尾
      ids.push(draggedId)
    } else {
      const targetIndex = ids.indexOf(targetNode.id)
      if (targetIndex === -1) return
      ids.splice(target.position === 'after' ? targetIndex + 1 : targetIndex, 0, draggedId)
    }

    const originalIds = siblingsByParent(
      parentChanged ? newParentId : (dragged.parentId ?? null),
    ).map((item) => item.id)
    // 父级和顺序都没变化时直接结束
    if (!parentChanged && ids.join('\u0000') === originalIds.join('\u0000')) {
      return
    }

    try {
      await options.reorderCategories(ids)
      if (parentChanged) {
        if (target.position === 'into') {
          // 放入后展开目标分类，避免「子分类看起来消失了」
          options.onNested?.(targetNode.id)
        }
        const suffix =
          target.position === 'into'
            ? `已成为「${targetNode.name}」的子分类`
            : `已移动到「${targetNode.name}」${target.position === 'before' ? '前面' : '后面'}`
        options.onSuccess?.(suffix)
      } else {
        options.onSuccess?.('分类顺序已更新')
      }
    } catch (e) {
      options.onError(String(e))
    }
  }

  /** 清理拖拽状态、全局监听与浮层（不提交排序） */
  const cleanup = () => {
    if (listening) {
      window.removeEventListener('pointermove', handlePointerMove)
      window.removeEventListener('pointerup', handlePointerUp)
      window.removeEventListener('pointercancel', handlePointerCancel)
      window.removeEventListener('blur', handlePointerCancel)
      listening = false
    }
    pendingNodeId = null
    dragStarted = false
    scrollContainer = null
    draggingId.value = null
    dropTarget.value = null
    removeGhost()
  }

  /**
   * 指针移动：未到阈值不算拖拽；进入拖拽后更新浮层、落点并自动滚动
   * @param event - 指针事件
   */
  function handlePointerMove(event: PointerEvent) {
    if (!pendingNodeId) return

    if (!dragStarted) {
      const distance = Math.hypot(event.clientX - startX, event.clientY - startY)
      if (distance < DRAG_THRESHOLD_PX) return

      const node = findNode(pendingNodeId)
      if (!node) {
        cleanup()
        return
      }
      dragStarted = true
      draggingId.value = node.id
      createGhost(node.name)
      moveGhost(event.clientX, event.clientY)
      scrollContainer = findScrollContainer(event.target as HTMLElement | null)
    }

    // 拖拽中阻止文本选择/触屏滚动等默认行为
    event.preventDefault()
    moveGhost(event.clientX, event.clientY)
    updateDropTarget(event.clientX, event.clientY)
    updateGhostHint(dropTarget.value)
    autoScroll(event.clientY)
  }

  /**
   * 指针抬起：若已进入拖拽则按落点提交
   * @param event - 指针事件
   */
  function handlePointerUp(event: PointerEvent) {
    const wasDragging = dragStarted
    const draggedId = pendingNodeId
    if (wasDragging) {
      updateDropTarget(event.clientX, event.clientY)
      updateGhostHint(dropTarget.value)
      const finalTarget = dropTarget.value
      // 拖拽结束后短暂抑制 click，避免松手误触选中分类
      suppressClickUntil = Date.now() + CLICK_SUPPRESS_MS
      cleanup()
      if (draggedId && finalTarget) {
        void commitDrop(draggedId, finalTarget)
      }
      return
    }
    cleanup()
  }

  /** 取消拖拽（pointercancel / 窗口失焦） */
  function handlePointerCancel() {
    if (dragStarted) suppressClickUntil = Date.now() + CLICK_SUPPRESS_MS
    cleanup()
  }

  /**
   * 开始按住分类名称（尚未进入拖拽，移动超过阈值才进入）
   * @param event - 指针事件
   * @param node - 被按住的分类
   */
  const startDrag = (event: PointerEvent, node: CategoryTreeNode) => {
    // 清理上一次可能残留的监听/状态（例如指针在窗口外松开导致 pointerup 丢失）
    if (listening) cleanup()

    // 仅响应鼠标左键 / 触屏/触控笔主指针
    if (event.button !== 0) return
    // 避免与右键菜单、折叠按钮等交互冲突
    if ((event.target as HTMLElement | null)?.closest('button')) return

    pendingNodeId = node.id
    dragStarted = false
    startX = event.clientX
    startY = event.clientY
    scrollContainer = null

    if (!listening) {
      // pointermove 需要 preventDefault（阻止文本选择/触屏滚动），显式声明非 passive
      window.addEventListener('pointermove', handlePointerMove, { passive: false })
      window.addEventListener('pointerup', handlePointerUp)
      window.addEventListener('pointercancel', handlePointerCancel)
      window.addEventListener('blur', handlePointerCancel)
      listening = true
    }
  }

  /**
   * 拖拽刚结束时是否应忽略这次 click（防止松手误选中分类）
   */
  const shouldIgnoreClick = (): boolean => Date.now() < suppressClickUntil

  /**
   * 某分类是否正在被拖拽（用于降低不透明度）
   * @param id - 分类 ID
   */
  const isDragging = (id: string): boolean => draggingId.value === id

  /**
   * 某分类行应显示的落点提示位置
   * @param id - 分类 ID
   * @returns 'before' | 'into' | 'after' | null
   */
  const dropIndicator = (id: string): CategoryDropPosition | null => {
    const target = dropTarget.value
    if (!target || target.id !== id) return null
    return target.position
  }

  // 组件卸载时移除全局监听与浮层，避免泄漏
  onUnmounted(() => {
    cleanup()
  })

  return {
    draggingId,
    dropTarget,
    startDrag,
    isDragging,
    dropIndicator,
    shouldIgnoreClick,
  }
}
