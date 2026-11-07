<!--
  HomeView.vue - 密码列表主页面

  这是应用的主界面，解锁后进入此页面。
  布局结构：
  - 顶部导航栏：Logo、快捷功能按钮、设置、主题切换、锁定
  - 左侧栏（w-56，可滚动）：本地筛选框 + 智能视图 + 分类树 + 标签多选
  - 右侧内容区：搜索/排序工具栏 + 密码列表表格 + 分页

  功能：
  1. 密码列表展示（表格形式，名称下带标签小徽章）
  2. 智能视图：全部 / 收藏 / 最近使用（带数量）
  3. 分类树筛选（可折叠、缩进最多按 8 级封顶、含后代数量）
  4. 标签多选筛选（AND，选中态明显）
  5. 搜索（空格分词 AND，Ctrl/Cmd+F 或 / 聚焦搜索框）
  6. 排序（更新时间/创建时间/名称/最近使用/使用频率）与分页（20/50/100/200）
  7. 收藏/取消收藏、复制用户名/密码、添加/编辑/删除
  8. 密码生成器、快速搜索（Ctrl+K）、快速添加
  9. 锁屏、托盘最小化、主题切换

  路由路径：/home
-->

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, type Component } from 'vue'
import { useRouter } from 'vue-router'
import { storeToRefs } from 'pinia'
import { invoke } from '@/lib/api'
import { setDragGhost } from '@/lib/dragGhost'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Select } from '@/components/ui/select'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import {
  usePasswordStore,
  type CategoryTreeNode,
  type PasswordItem,
  type SortKey,
} from '@/stores/password'
import { useTheme } from '@/composables/useTheme'
import { useCategoryDragSort } from '@/composables/useCategoryDragSort'
import { formatTimestamp } from '@/lib/utils'
import PasswordFormDialog from '@/components/PasswordFormDialog.vue'
import PasswordGenerator from '@/components/PasswordGenerator.vue'
import PasswordStrength from '@/components/PasswordStrength.vue'
import QuickSearch from '@/components/QuickSearch.vue'
import QuickAdd from '@/components/QuickAdd.vue'
import ContextMenu from '@/components/ContextMenu.vue'
import WindowControls from '@/components/WindowControls.vue'
import Toast from '@/components/Toast.vue'
import {
  IconArrowDown,
  IconArrowUp,
  IconChevronDown,
  IconChevronRight,
  IconClock,
  IconClose,
  IconCopy,
  IconEmpty,
  IconGrid,
  IconKey,
  IconLock,
  IconLogoLock,
  IconTray,
  IconMoon,
  IconPencil,
  IconPlus,
  IconSearch,
  IconSettings,
  IconSquarePen,
  IconStar,
  IconSun,
  IconTag,
  IconTrash,
} from '@/components/icons'

// ==================== 依赖注入 ====================

/** 路由实例 */
const router = useRouter()

/** 主题管理 */
const { isDark, toggleTheme } = useTheme()

/** 密码 Store */
const passwordStore = usePasswordStore()

// 从 Store 中解构响应式状态（Web 契约见「密码分类与快速检索优化方案」第 5 节）
const {
  // 筛选 / 排序 / 分页状态
  activeCategory,
  activeTags,
  searchQuery,
  sortBy,
  sortOrder,
  pageSize,
  currentPage,
  // 列表数据
  passwords,
  pagedPasswords,
  totalFiltered,
  pageCount,
  // 分类树与标签
  categoryTree,
  tags,
  categoryNameById,
  // 计数与展示辅助
  allCount,
  favoriteCount,
  recentCount,
  activeCategoryName,
} = storeToRefs(passwordStore)

// 从 Store 中解构方法
const {
  loadPasswords,
  loadStats,
  loadCategories,
  loadTags,
  setActiveCategory,
  setSearchQuery,
  toggleTag,
  clearTags,
  setSort,
  setPage,
  setPageSize,
  toggleFavorite,
  deletePassword,
  addCategory,
  updateCategory,
  deleteCategory,
  reorderCategories,
} = passwordStore

// ==================== 常量 ====================

/** 侧栏标签默认展示数量，超过后提供「展开全部 / 收起」 */
const SIDEBAR_TAG_LIMIT = 12

/** 每页条数可选项 */
const PAGE_SIZE_OPTIONS = [20, 50, 100, 200] as const

/** 排序方式下拉选项（value 与 Store 的 SortKey 一致） */
const SORT_OPTIONS: Array<{ value: SortKey; label: string }> = [
  { value: 'updated_at', label: '更新时间' },
  { value: 'created_at', label: '创建时间' },
  { value: 'title', label: '名称' },
  { value: 'last_used_at', label: '最近使用' },
  { value: 'use_count', label: '使用频率' },
]

// ==================== 本地状态 ====================

/** 侧栏本地筛选关键词（只过滤侧栏分类/标签的展示，不影响列表搜索） */
const sidebarFilter = ref('')

/** 侧栏折叠的分类 ID 集合（默认全部展开） */
const collapsedCategoryIds = ref<Set<string>>(new Set())

/** 侧栏标签是否展开全部 */
const tagsExpanded = ref(false)

/** 搜索框引用（Input 组件实例，$el 为其内部原生 input） */
const searchInputRef = ref<{ $el?: HTMLInputElement } | null>(null)

/** 控制密码表单弹窗是否显示 */
const showFormDialog = ref(false)

/** 当前编辑的密码项，null 表示添加模式 */
const editingItem = ref<PasswordItem | null>(null)

/** 控制密码生成器弹窗是否显示 */
const showGenerator = ref(false)

/** 控制快速搜索弹窗是否显示 */
const showQuickSearch = ref(false)

/** 控制快速添加弹窗是否显示 */
const showQuickAdd = ref(false)

/** 剪贴板清除时间（秒），从设置中读取 */
const clipboardClearTime = ref(30)

/**
 * 当前会话的密码交付方式（由后端能力判定，见 Rust 的 get_capabilities）
 * - `inject`：全局按键注入（Windows / Linux X11），点复制后倒计时内按 Ctrl+V
 * - `dnd`：拖放交付（Linux Wayland），把密码拖到目标输入框
 * - `none`：都不支持
 * 初始按 `inject` 渲染，拿到真实值后按需切换（避免首屏闪一下拖拽把手）
 */
const delivery = ref<'inject' | 'dnd' | 'none'>('inject')


/** Toast 提示状态 */
const toast = ref({
  show: false,
  type: 'success' as 'success' | 'error' | 'info',
  message: '',
})

/**
 * 显示 Toast 提示
 * @param type - 提示类型
 * @param message - 提示消息
 */
const showToast = (type: 'success' | 'error' | 'info', message: string) => {
  toast.value = { show: true, type, message }
}

// ==================== 侧栏：分类树 ====================

/** 分类树节点 id → 节点映射（用于关键词过滤时回溯祖先链） */
const categoryNodeMap = computed(() => {
  const map = new Map<string, CategoryTreeNode>()
  for (const node of categoryTree.value) map.set(node.id, node)
  return map
})

/**
 * 因祖先节点被折叠而需要隐藏的分类 id 集合
 * categoryTree 为深度优先顺序（父节点一定排在子节点之前），一次遍历即可传递隐藏状态
 */
const hiddenCategoryIds = computed(() => {
  const hidden = new Set<string>()
  for (const node of categoryTree.value) {
    if (
      node.parentId != null &&
      (collapsedCategoryIds.value.has(node.parentId) || hidden.has(node.parentId))
    ) {
      hidden.add(node.id)
    }
  }
  return hidden
})

/**
 * 按关键词过滤分类树：命中的节点连同其祖先链一起保留，方便看清层级
 * @param keyword - 已转为小写的筛选关键词
 */
const filterCategoryByKeyword = (keyword: string): CategoryTreeNode[] => {
  if (!keyword) return categoryTree.value

  const keepIds = new Set<string>()
  for (const node of categoryTree.value) {
    if (!node.name.toLowerCase().includes(keyword)) continue

    keepIds.add(node.id)
    // 向上补齐祖先链，保证缩进层级完整
    let parentId = node.parentId
    while (parentId != null) {
      if (keepIds.has(parentId)) break
      keepIds.add(parentId)
      parentId = categoryNodeMap.value.get(parentId)?.parentId ?? null
    }
  }
  return categoryTree.value.filter((node) => keepIds.has(node.id))
}

/** 侧栏实际展示的分类行（本地筛选 + 折叠；筛选时忽略折叠，直接展示命中的层级） */
const visibleCategories = computed(() => {
  const keyword = sidebarFilter.value.trim().toLowerCase()
  if (keyword) return filterCategoryByKeyword(keyword)
  if (hiddenCategoryIds.value.size === 0) return categoryTree.value
  return categoryTree.value.filter((node) => !hiddenCategoryIds.value.has(node.id))
})

/**
 * 分类行缩进样式：每级 12px；层级不设限，但缩进最多按 8 级封顶，避免深层分类把侧栏挤爆
 * （额外 8px 为基础内边距，保持与行首图标对齐）
 * @param depth - 分类树深度
 */
const categoryIndentStyle = (depth: number) => ({
  paddingLeft: `${8 + Math.min(depth, 8) * 12}px`,
})

/**
 * 切换分类折叠状态（本地 Set，默认展开）
 * @param id - 分类 ID
 */
const toggleCategoryCollapse = (id: string) => {
  const next = new Set(collapsedCategoryIds.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  collapsedCategoryIds.value = next
}

/**
 * 分类拖拽排序（按住名称直接拖拽，无手柄；仅同父级排序）
 * 拖拽中用插入指示线提示落点，排序结果先乐观更新、后端失败自动回滚。
 */
const {
  startDrag: startCategoryDrag,
  isDragging: isCategoryDragging,
  dropIndicator: categoryDropIndicator,
  shouldIgnoreClick: shouldIgnoreCategoryClick,
} = useCategoryDragSort({
  categoryTree,
  reorderCategories,
  // 拖到目标行中间时把分类移动到目标下（父级变化）
  moveCategory: async (id, parentId) => {
    await updateCategory(id, undefined, parentId)
  },
  // 放入子分类后展开目标，避免新子分类因折叠而“消失”
  onNested: (targetId) => {
    const next = new Set(collapsedCategoryIds.value)
    next.delete(targetId)
    collapsedCategoryIds.value = next
  },
  onError: (message) => showToast('error', '操作失败: ' + message),
})

/**
 * 点击分类（拖拽刚结束时忽略，避免松手误触选中）
 * @param node - 分类节点
 */
const handleCategoryClick = (node: CategoryTreeNode) => {
  if (shouldIgnoreCategoryClick()) return
  setActiveCategory(node.id)
}

// ==================== 分类右键菜单（快捷管理分类） ====================

/** 分类右键菜单状态（categoryId 为 null 表示在空白处右键：新增顶级分类） */
const categoryMenu = ref({
  open: false,
  x: 0,
  y: 0,
  categoryId: null as string | null,
})

/** 右键目标分类节点（null 表示空白处） */
const categoryMenuNode = computed<CategoryTreeNode | null>(() => {
  const id = categoryMenu.value.categoryId
  if (!id) return null
  return categoryTree.value.find((node) => node.id === id) ?? null
})

/** 右键菜单项（根据右键目标动态生成） */
const categoryMenuItems = computed<
  Array<{
    key: string
    label: string
    icon?: Component
    danger?: boolean
    disabled?: boolean
    divided?: boolean
  }>
>(() => {
  const node = categoryMenuNode.value
  const items: Array<{
    key: string
    label: string
    icon?: Component
    danger?: boolean
    disabled?: boolean
    divided?: boolean
  }> = []

  if (!node) {
    // 空白处右键：快速新增顶级分类
    items.push({ key: 'add-root', label: '新增顶级分类', icon: IconPlus })
  } else {
    items.push({ key: 'add-child', label: `在「${node.name}」下新增子分类`, icon: IconPlus })
    items.push({ key: 'rename', label: '重命名分类', icon: IconPencil })
    if (node.parentId) {
      items.push({ key: 'make-top', label: '置为顶级分类', icon: IconArrowUp })
    }
    items.push({
      key: 'delete',
      label: '删除分类',
      icon: IconTrash,
      danger: true,
      divided: true,
      // 内置「其他」分类是兜底分类，后端禁止删除
      disabled: node.id === 'other',
    })
  }

  items.push({ key: 'manage', label: '打开分类管理', icon: IconSquarePen, divided: true })
  return items
})

/**
 * 打开分类右键菜单
 * @param event - 鼠标事件（用于定位菜单）
 * @param categoryId - 目标分类 ID；null 表示空白处（新增顶级分类）
 */
const openCategoryMenu = (event: MouseEvent, categoryId: string | null) => {
  event.preventDefault()
  categoryMenu.value = { open: true, x: event.clientX, y: event.clientY, categoryId }
}

// ==================== 分类快捷新增 / 重命名弹窗 ====================

/** 是否显示分类快捷弹窗 */
const showCategoryDialog = ref(false)

/** 弹窗模式：新增 / 重命名 */
const categoryDialogMode = ref<'add' | 'rename'>('add')

/** 分类名称输入 */
const categoryDialogName = ref('')

/** 新增时的父分类（空串表示顶级分类） */
const categoryDialogParentId = ref('')

/** 父分类下拉选项（含「无（顶级分类）」空值项；层级按 depth 前缀缩进） */
const categoryParentOptions = computed(() => [
  { value: '', label: '无（顶级分类）' },
  ...categoryTree.value.map((node) => ({
    value: node.id,
    label: `${'— '.repeat(node.depth)}${node.name}`,
  })),
])

/** 正在重命名的分类 ID（新增模式为 null） */
const categoryEditingId = ref<string | null>(null)

/** 分类保存中 */
const categorySaving = ref(false)

/**
 * 打开新增分类弹窗
 * @param parentId - 预设父分类 ID；null 表示顶级分类
 */
const openCategoryAddDialog = (parentId: string | null) => {
  categoryDialogMode.value = 'add'
  categoryDialogName.value = ''
  categoryDialogParentId.value = parentId ?? ''
  categoryEditingId.value = null
  showCategoryDialog.value = true
}

/**
 * 打开重命名分类弹窗（父分类保持不变）
 * @param node - 目标分类节点
 */
const openCategoryRenameDialog = (node: CategoryTreeNode) => {
  categoryDialogMode.value = 'rename'
  categoryDialogName.value = node.name
  categoryDialogParentId.value = node.parentId ?? ''
  categoryEditingId.value = node.id
  showCategoryDialog.value = true
}

/**
 * 保存分类（新增 / 重命名）
 */
const handleSaveCategoryDialog = async () => {
  const name = categoryDialogName.value.trim()
  if (!name || categorySaving.value) return
  categorySaving.value = true
  try {
    if (categoryDialogMode.value === 'rename' && categoryEditingId.value) {
      // 仅改名称：parentId 不传（undefined）= 不修改父分类
      await updateCategory(categoryEditingId.value, name)
    } else {
      // 新增：空串 = 顶级分类
      await addCategory(name, categoryDialogParentId.value || null)
    }
    // 清空输入框并关闭弹窗（Store 内部已刷新分类/当前页/统计）
    categoryDialogName.value = ''
    categoryEditingId.value = null
    categoryDialogParentId.value = ''
    showCategoryDialog.value = false
  } catch (e) {
    showToast('error', '保存失败: ' + String(e))
  } finally {
    categorySaving.value = false
  }
}

/**
 * 删除分类（密码移到「其他」，子分类上移一级）
 * @param node - 目标分类节点
 */
const handleDeleteCategory = async (node: CategoryTreeNode) => {
  if (node.id === 'other') return
  if (!confirm(`确定删除「${node.name}」？该分类下的密码将移到“其他”，子分类将上移一级`)) return
  try {
    await deleteCategory(node.id)
  } catch (e) {
    showToast('error', '删除失败: ' + String(e))
  }
}

/**
 * 删除密码（二次确认 + 成功/失败提示）
 *
 * 直接绑定模板点击事件会把 store 抛出的错误变成 unhandled rejection，
 * 用户对删除是否成功毫无感知；这里确认后再删，并统一提示结果。
 * @param item - 目标密码条目
 */
const handleDeletePassword = async (item: PasswordItem) => {
  if (!confirm(`确定删除「${item.title}」？`)) return
  try {
    await deletePassword(item.id)
    showToast('success', '已删除')
  } catch (e) {
    showToast('error', '删除失败: ' + String(e))
  }
}

/**
 * 处理分类右键菜单动作
 * @param key - 菜单项 key
 */
const handleCategoryMenuSelect = async (key: string) => {
  const node = categoryMenuNode.value
  categoryMenu.value = { ...categoryMenu.value, open: false }

  switch (key) {
    case 'add-root':
      openCategoryAddDialog(null)
      break
    case 'add-child':
      if (node) openCategoryAddDialog(node.id)
      break
    case 'rename':
      if (node) openCategoryRenameDialog(node)
      break
    case 'make-top':
      if (node && node.parentId) {
        if (!confirm(`将「${node.name}」置为顶级分类？`)) return
        try {
          await updateCategory(node.id, undefined, null)
        } catch (e) {
          showToast('error', '操作失败: ' + String(e))
        }
      }
      break
    case 'delete':
      if (node) await handleDeleteCategory(node)
      break
    case 'manage':
      router.push('/categories')
      break
  }
}

// ==================== 侧栏：标签 ====================

/** 侧栏标签（受本地筛选关键词影响） */
const filteredSidebarTags = computed(() => {
  const keyword = sidebarFilter.value.trim().toLowerCase()
  if (!keyword) return tags.value
  return tags.value.filter((tag) => tag.name.toLowerCase().includes(keyword))
})

/** 侧栏当前展示的标签（默认最多 12 个，展开后显示全部） */
const visibleSidebarTags = computed(() =>
  tagsExpanded.value
    ? filteredSidebarTags.value
    : filteredSidebarTags.value.slice(0, SIDEBAR_TAG_LIMIT),
)

/** 侧栏标签是否超过 12 个（决定是否显示「展开全部 / 收起」） */
const hasMoreSidebarTags = computed(() => filteredSidebarTags.value.length > SIDEBAR_TAG_LIMIT)

/**
 * 判断标签是否处于选中态（大小写不敏感，与 Store 的标签筛选语义一致）
 * @param name - 标签名
 */
const isTagActive = (name: string) => {
  const lower = name.toLowerCase()
  return activeTags.value.some((tag) => tag.toLowerCase() === lower)
}

// ==================== 智能视图 / 工具栏 ====================

/**
 * 切换智能视图
 * 契约 6.2：切到「最近使用」时同时按最近使用时间倒序排列
 * @param id - all | favorite | recent
 */
const selectSmartView = (id: 'all' | 'favorite' | 'recent') => {
  setActiveCategory(id)
  if (id === 'recent') setSort('last_used_at', 'desc')
}

/** 排序方式下拉变更（切换字段时由 Store 采用该字段的默认方向，方向仍可再用右侧箭头按钮调整） */
const handleSortKeyChange = (value: string | number | undefined) => {
  if (value === undefined) return
  const key = value as SortKey
  if (!SORT_OPTIONS.some((option) => option.value === key)) return
  // 仅当用户切到不同字段时才会触发，此时不带 order：新字段默认 title 升序、其余降序
  setSort(key)
}

/** 切换排序升降序 */
const toggleSortOrder = () => {
  setSort(sortBy.value, sortOrder.value === 'asc' ? 'desc' : 'asc')
}

/** 每页条数变更（Store 内部会重置到第 1 页） */
const handlePageSizeChange = (value: string | number | undefined) => {
  if (value === undefined) return
  const size = Number(value)
  if (!Number.isFinite(size)) return
  setPageSize(size)
}

/**
 * 清除筛选：清空搜索词与已选标签，并回到「全部」
 * 用于「筛选无结果」空状态
 */
const clearFilters = () => {
  setSearchQuery('')
  clearTags()
  setActiveCategory('all')
}

// ==================== 搜索快捷键 ====================

/** 聚焦搜索框并选中已有内容，方便直接覆盖输入 */
const focusSearch = () => {
  const input = searchInputRef.value?.$el as HTMLInputElement | null | undefined
  input?.focus()
  input?.select()
}

/** 判断事件目标是否是文本输入类元素（用于 / 快捷键不劫持输入） */
const isEditableTarget = (target: EventTarget | null): boolean => {
  if (!(target instanceof HTMLElement)) return false
  const tag = target.tagName.toLowerCase()
  return tag === 'input' || tag === 'textarea' || tag === 'select' || target.isContentEditable
}

/**
 * 全局键盘快捷键：
 * - Ctrl/Cmd+F 聚焦搜索框（替代浏览器查找）
 * - / 聚焦搜索框；输入框内按 / 不劫持
 * @param event - 键盘事件
 */
const handleGlobalKeydown = (event: KeyboardEvent) => {
  if (event.isComposing) return

  if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === 'f') {
    // 弹窗（表单/快速搜索等）打开时不抢焦点，避免把焦点从弹窗内输入框抢走
    if (document.querySelector('[role="dialog"]')) return
    event.preventDefault()
    focusSearch()
    return
  }

  if (
    event.key === '/' &&
    !event.ctrlKey &&
    !event.metaKey &&
    !event.altKey &&
    !isEditableTarget(event.target)
  ) {
    event.preventDefault()
    focusSearch()
  }
}

// ==================== 表格单元辅助 ====================

/** 取密码项标签（兼容旧数据可能缺少 tags 字段） */
const itemTags = (item: PasswordItem): string[] => item.tags ?? []

/** 名称下最多展示的标签小徽章数量 */
const visibleItemTags = (item: PasswordItem): string[] => itemTags(item).slice(0, 3)

/** 名称下被省略的标签数量（超出 3 个时显示 +N） */
const itemExtraTagCount = (item: PasswordItem): number => Math.max(0, itemTags(item).length - 3)

/** 密码项全部标签拼接（用于 title 悬浮展示完整标签） */
const itemTagsTitle = (item: PasswordItem): string => itemTags(item).join('、')

/** 分类 id → 分类名称（分类不存在时回退显示 id） */
const categoryLabel = (id: string): string => categoryNameById.value[id] ?? id

// ==================== 复制操作 ====================

/**
 * 复制文本到剪贴板（带自动清除）
 * @param text - 要复制的文本
 * @param event - 鼠标事件（用于获取位置）
 */
const copyToClipboard = async (
  text: string,
  event?: MouseEvent,
  title?: string,
  subtitle?: string,
) => {
  try {
    // title / subtitle 只用于浮窗里的展示；Wayland 下后端会把文本改放进浮窗
    await invoke('copy_text_to_clipboard', {
      text,
      clearAfter: clipboardClearTime.value,
      title,
      subtitle,
    })
    showToast('success', '已复制')

    // Wayland：后端已改道进浮窗，不写剪贴板，也就没有倒计时窗口和光标跟随
    if (delivery.value === 'dnd') return

    // 获取鼠标位置并显示全局倒计时窗口
    //（x/y 是 CSS 逻辑屏幕坐标，后端乘渲染缩放换算为物理坐标；见 scale.rs）
    const x = event?.screenX ?? window.screen.width / 2
    const y = event?.screenY ?? window.screen.height / 2
    await invoke('show_countdown', { seconds: clipboardClearTime.value, x, y })

    // 启动光标跟随
    await invoke('start_follow_cursor')
  } catch (e) {
    console.error('Copy failed:', e)
    showToast('error', '复制失败')
  }
}

/**
 * 通过后端登记待粘贴密码（不进系统剪贴板）
 * 前端全程不接触明文；倒计时内在目标窗口按 Ctrl+V 由后端解密并注入。
 * Wayland 下没有按键注入，且**不会**回退写剪贴板（那等于把密码交给整个桌面），
 * 改为提示用户走拖放交付（见下方 dnd* 系列函数）。
 * @param id - 密码项 ID
 * @param event - 鼠标事件（用于定位倒计时窗口）
 */
const copyPasswordViaBackend = async (id: string, event?: MouseEvent) => {
  // Wayland：唯一的交付通道是拖放。点「复制」的语义变成「放进拖拽池」——
  // 池子会出现在屏幕上，用户再从池里把这一条拖到目标输入框。
  // 这里不解密、不写剪贴板，只是让后端记下「这条待交付」。
  if (delivery.value === 'dnd') {
    try {
      const { count } = await invoke<{ count: number }>('add_to_dnd_pool', { id })
      showToast(
        'success',
        count > 1
          ? `已复制 ${count} 条，拖到目标输入框即可`
          : '已复制，拖到目标输入框即可',
      )
    } catch (e) {
      console.error('复制失败:', e)
      showToast('error', String(e))
    }
    return
  }
  if (delivery.value === 'none') {
    showToast('error', '当前会话不支持密码交付')
    return
  }
  try {
    // x/y 是 CSS 逻辑屏幕坐标，后端乘渲染缩放换算为物理坐标（见 scale.rs）
    const x = event?.screenX ?? window.screen.width / 2
    const y = event?.screenY ?? window.screen.height / 2
    const { ttl } = await invoke<{ ttl: number; mode: string }>(
      'copy_password_to_clipboard',
      { id },
    )
    showToast('success', '密码就绪，倒计时内按 Ctrl+V 粘贴')
    // 后端已登记使用记录（last_used_at/use_count，契约 4.3）；
    // 刷新当前页与统计，让「最近使用」视图/计数与使用频率排序立即反映本次使用
    void loadPasswords()
    void loadStats()
    // 启动倒计时和光标跟随
    await invoke('show_countdown', { seconds: ttl, x, y })
    await invoke('start_follow_cursor')
  } catch (e) {
    console.error('复制密码失败:', e)
    showToast('error', String(e))
  }
}

// ==================== 拖放交付（Wayland） ====================

// ==================== 整行拖拽交付（Wayland） ====================
//
// 池子是"暂存区"，但很多时候用户只想拖一次——那就直接从列表行上拖走，
// 不必先点复制再跑到池子里找。两个入口语义不同，不冲突：
//   点复制 → 入池（可反复拖、可暂存）
//   直接拖行 → 一次性交付

/** 当前已武装的行（未武装成功时不允许起拖） */
const dndRowPendingId = ref<string | null>(null)

/**
 * 拖拽时的占位串
 *
 * 不能是空串：WebKitGTK 对空内容不会声明任何 MIME，目标就不会来取数据，
 * Rust 侧填密码的逻辑一次都不会触发。真密码随后由 `drag-data-get` 覆盖。
 */
const DND_PLACEHOLDER = 'moyu-passwd-pending'

/**
 * 按下行即武装（只登记条目与解密闭包，解密发生在 Rust 取数据那一刻）
 *
 * 必须在 pointerdown 而不是 dragstart：invoke 是异步的，而 dragstart 里
 * 设置数据必须同步完成，等不到 IPC 返回。
 */
const armRowDnd = async (id: string) => {
  if (delivery.value !== 'dnd') return
  // 同步占位再发 IPC：等 invoke 回来再设标记会让「按下即拖」必然失败
  dndRowPendingId.value = id
  try {
    await invoke('prepare_dnd_payload', { id })
  } catch (e) {
    dndRowPendingId.value = null
    console.error('拖放准备失败:', e)
  }
}

/**
 * 鼠标移入行就预武装
 *
 * `prepare_dnd_payload` 是异步 IPC：等 pointerdown 才发，手快时 drag-begin 里
 * `is_armed()` 仍为 false，Rust 不接管数据，拖出去就是空/占位串。
 */
const prearmRowDnd = (id: string) => {
  if (delivery.value !== 'dnd') return
  void armRowDnd(id)
}

/** 开始拖动整行 */
const handleRowDndStart = (event: DragEvent, id: string, title: string) => {
  if (!event.dataTransfer || dndRowPendingId.value !== id) {
    // 没武装成功就别拖：否则目标会粘出占位串，用户以为成功
    event.preventDefault()
    return
  }
  event.dataTransfer.setData('text/plain', DND_PLACEHOLDER)
  event.dataTransfer.effectAllowed = 'copy'
  // 幽灵固定在光标右下方，别盖住目标输入框的插入点
  setDragGhost(event, title)
}

/** 拖动结束：后端在 drag-end 里收尾，这里只清本地标记 */
const handleRowDndEnd = () => {
  dndRowPendingId.value = null
}

// ==================== 弹窗 / 窗口操作 ====================

/**
 * 打开添加密码弹窗
 * 将 editingItem 设为 null，弹窗进入添加模式
 */
const openAddDialog = () => {
  editingItem.value = null
  showFormDialog.value = true
}

/**
 * 打开编辑密码弹窗
 * @param item - 要编辑的密码项
 */
const openEditDialog = (item: PasswordItem) => {
  editingItem.value = item
  showFormDialog.value = true
}

/**
 * 最小化到托盘
 * 隐藏窗口，显示在系统托盘
 */
const handleMinimizeToTray = async () => {
  try {
    await invoke('minimize_to_tray')
  } catch (e) {
    console.error('最小化到托盘失败:', e)
  }
}

/**
 * 锁定应用
 * 只需调用后端命令，后端负责清除密钥并通知所有窗口
 */
const handleLock = async () => {
  try {
    await invoke('lock_app')
  } catch (e) {
    console.error('锁定失败:', e)
  }
}

// ==================== 生命周期 ====================

/** 事件监听清理函数（组件卸载时统一取消，防止叠加监听） */
const unlisteners: Array<() => void> = []

onUnmounted(() => {
  unlisteners.forEach((unlisten) => unlisten())
  unlisteners.length = 0
  window.removeEventListener('keydown', handleGlobalKeydown)
})

onMounted(async () => {
  // 先注册全局搜索快捷键，避免数据加载异常时快捷键失效
  window.addEventListener('keydown', handleGlobalKeydown)

  // 契约 6.2：并行加载密码 / 分类 / 标签 / 统计（统计用于侧栏计数）
  await Promise.all([loadPasswords(), loadStats(), loadCategories(), loadTags()])

  // 加载设置（剪贴板清除时间）
  try {
    const savedSettings = await invoke<{ clipboard_clear_time: number }>('get_settings')
    // 0 = 明确不自动清除，不能用 || 兜底（0 会被折算成 30）
    clipboardClearTime.value = savedSettings.clipboard_clear_time ?? 30
  } catch (e) {
    console.warn('加载设置失败:', e)
  }

  // 交付能力：Wayland 下没有按键注入，密码只能靠拖放交出去（见 Rust 的 get_capabilities）。
  // 读失败时按默认的注入方式渲染——X11/Windows 用户不受影响，Wayland 用户会在点复制时
  // 收到后端明确的错误提示，而不是看到一个假的「已复制」。
  try {
    const caps = await invoke<{ delivery: 'inject' | 'dnd' | 'none' }>('get_capabilities')
    delivery.value = caps.delivery
  } catch (e) {
    console.warn('读取交付能力失败，按注入方式处理:', e)
  }

  // 监听全局快捷键事件（保存 unlisten，卸载时统一清理，防止反复进出页面叠加监听）。
  // Ctrl+K 的快速搜索走独立小窗（后端全局快捷键直接开窗），不再发 show-quick-search 事件；
  // 内嵌搜索弹窗仅由主界面搜索入口打开。
  const { listen } = await import('@tauri-apps/api/event')

  // app-locked 锁定清理与跳转由 App.vue 全局统一处理（clearSensitive + 跳解锁页），此处不再重复监听

  // 监听快速添加快捷键事件
  unlisteners.push(
    await listen('show-quick-add', () => {
      showQuickAdd.value = true
    }),
  )

  // 监听密码生成器快捷键事件
  unlisteners.push(
    await listen('show-password-generator', () => {
      showGenerator.value = true
    }),
  )

  // 拖放交付结果（后端在 GTK 的 drag-end 里判定目标是否接受后发来）
  unlisteners.push(
    // 池窗口会自己给反馈；主窗口这里保留一份，主窗口在前台时同样能看到结果
    await listen<{ delivered: boolean; served: boolean }>('dnd-result', (event) => {
      const { delivered, served } = event.payload ?? { delivered: false, served: false }
      if (delivered) {
        showToast('success', '密码已交付到目标输入框')
        // 后端已记使用记录，这里只刷新展示
        void loadPasswords()
        void loadStats()
      } else if (!served) {
        // 数据根本没被取走：这不是「目标拒收」，是拖放链路没走通，别让用户以为换个输入框就行
        showToast('error', '拖放未生效（目标没有取走数据），请重试')
      } else {
        showToast('error', '这个窗口不接受拖放，换个输入框试试')
      }
    }),
  )
})
</script>

<template>
  <div class="h-screen flex flex-col bg-background">
    <!-- 顶部导航栏 = 自绘标题栏（h-10 压矮，原生标题栏已关闭）；
         data-tauri-drag-region 区域可拖动窗口，右侧 WindowControls 通栏贴窗边 -->
    <header class="h-10 pl-3 flex items-stretch justify-between select-none" data-tauri-drag-region>
      <div class="flex items-center gap-2" data-tauri-drag-region>
        <IconLogoLock class="h-5 w-5 text-primary-ink" data-tauri-drag-region />
        <h1 class="text-base font-semibold" data-tauri-drag-region>MoyuPasswd</h1>
      </div>

      <div class="flex items-stretch">
        <div class="flex items-center gap-1">
          <Button variant="ghost" size="icon-sm" title="密码生成器" @click="showGenerator = true">
            <IconKey class="h-4 w-4" />
          </Button>
          <Button variant="ghost" size="icon-sm" title="设置" @click="router.push('/settings')">
            <IconSettings class="h-4 w-4" />
          </Button>
          <Button variant="ghost" size="icon-sm" title="切换主题" @click="toggleTheme">
            <IconSun v-if="isDark" class="h-4 w-4" />
            <IconMoon v-else class="h-4 w-4" />
          </Button>
          <!-- 最小化到托盘按钮 -->
          <Button variant="ghost" size="icon-sm" title="最小化到托盘" @click="handleMinimizeToTray">
            <IconTray class="h-4 w-4" />
          </Button>
          <Button variant="ghost" size="icon-sm" title="锁定" @click="handleLock">
            <IconLock class="h-4 w-4" />
          </Button>
        </div>
        <WindowControls />
      </div>
    </header>

    <!-- 岛屿布局：侧栏岛 + 内容岛浮在框架底上，岛间留隙、圆角、无描边分割 -->
    <div class="flex flex-1 gap-2 px-2 pb-2 overflow-hidden">
      <!-- 左侧栏岛：本地筛选 / 智能视图 / 分类树 / 标签 -->
      <aside class="w-56 shrink-0 rounded-[10px] bg-card flex flex-col overflow-hidden">
        <!-- 侧栏本地筛选：只过滤分类/标签的展示，不影响列表搜索 -->
        <div class="p-2.5 border-b shrink-0">
          <div class="relative">
            <IconSearch class="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
            <Input
              v-model="sidebarFilter"
              placeholder="筛选分类/标签"
              class="h-8 pl-8 text-xs"
            />
          </div>
        </div>

        <!-- 可滚动的侧栏内容 -->
        <div class="flex-1 overflow-y-auto p-2 space-y-4">
          <!-- 智能视图 -->
          <div class="space-y-0.5">
            <button
              type="button"
              class="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-sm transition-colors"
              :class="activeCategory === 'all' ? 'bg-select text-select-foreground' : 'hover:bg-muted'"
              @click="selectSmartView('all')"
            >
              <IconGrid class="h-4 w-4 shrink-0" />
              <span class="flex-1 text-left">全部</span>
              <span
                class="text-xs tabular-nums"
                :class="activeCategory === 'all' ? 'text-select-foreground/70' : 'text-muted-foreground'"
              >
                {{ allCount }}
              </span>
            </button>

            <button
              type="button"
              class="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-sm transition-colors"
              :class="activeCategory === 'favorite' ? 'bg-select text-select-foreground' : 'hover:bg-muted'"
              @click="selectSmartView('favorite')"
            >
              <IconStar class="h-4 w-4 shrink-0" />
              <span class="flex-1 text-left">收藏</span>
              <span
                class="text-xs tabular-nums"
                :class="activeCategory === 'favorite' ? 'text-select-foreground/70' : 'text-muted-foreground'"
              >
                {{ favoriteCount }}
              </span>
            </button>

            <button
              type="button"
              class="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-sm transition-colors"
              :class="activeCategory === 'recent' ? 'bg-select text-select-foreground' : 'hover:bg-muted'"
              @click="selectSmartView('recent')"
            >
              <IconClock class="h-4 w-4 shrink-0" />
              <span class="flex-1 text-left">最近使用</span>
              <span
                class="text-xs tabular-nums"
                :class="activeCategory === 'recent' ? 'text-select-foreground/70' : 'text-muted-foreground'"
              >
                {{ recentCount }}
              </span>
            </button>
          </div>

          <!-- 分类分组（右键菜单：空白处新增顶级分类，行上新增子分类/重命名/删除） -->
          <div @contextmenu.prevent="openCategoryMenu($event, null)">
            <div class="flex items-center justify-between px-2 py-1">
              <span class="text-xs font-medium text-muted-foreground">分类</span>
              <div class="flex items-center gap-2">
                <button
                  type="button"
                  class="flex items-center justify-center w-4 h-4 rounded text-muted-foreground hover:text-foreground hover:bg-muted transition-colors"
                  title="新增分类"
                  @click="openCategoryAddDialog(null)"
                >
                  <IconPlus class="h-3 w-3" />
                </button>
                <button
                  type="button"
                  class="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
                  title="管理分类"
                  @click="router.push('/categories')"
                >
                  <IconSquarePen class="h-3 w-3" />
                  管理分类
                </button>
              </div>
            </div>

            <div class="space-y-0.5">
              <div
                v-for="node in visibleCategories"
                :key="node.id"
                class="relative flex items-center gap-1 rounded-md text-sm cursor-pointer select-none transition-colors"
                :class="[
                  activeCategory === node.id ? 'bg-select text-select-foreground' : 'hover:bg-muted',
                  isCategoryDragging(node.id) ? 'opacity-50' : '',
                  categoryDropIndicator(node.id) === 'into' ? 'ring-2 ring-primary-ink ring-inset bg-primary-ink/10' : '',
                ]"
                :style="categoryIndentStyle(node.depth)"
                :data-category-id="node.id"
                @click="handleCategoryClick(node)"
                @contextmenu.prevent.stop="openCategoryMenu($event, node.id)"
              >
                <!-- 拖拽插入指示线（上/下边缘） -->
                <span
                  v-if="categoryDropIndicator(node.id) === 'before'"
                  class="pointer-events-none absolute left-0 right-0 -top-px h-0.5 rounded bg-primary-ink"
                />
                <span
                  v-if="categoryDropIndicator(node.id) === 'after'"
                  class="pointer-events-none absolute left-0 right-0 -bottom-px h-0.5 rounded bg-primary-ink"
                />
                <!-- 有子节点时显示折叠箭头（本地 Set 记录折叠状态，默认展开） -->
                <button
                  v-if="node.hasChildren"
                  type="button"
                  class="shrink-0 flex items-center justify-center w-4 h-4 rounded hover:bg-black/10"
                  :title="collapsedCategoryIds.has(node.id) ? '展开子分类' : '折叠子分类'"
                  @click.stop="toggleCategoryCollapse(node.id)"
                >
                  <IconChevronRight
                    v-if="collapsedCategoryIds.has(node.id)"
                    class="h-3.5 w-3.5"
                  />
                  <IconChevronDown v-else class="h-3.5 w-3.5" />
                </button>
                <span v-else class="shrink-0 w-4" />

                <!-- 名称：按住即可拖拽排序（无拖拽手柄，自定义指针拖拽） -->
                <span
                  class="flex-1 min-w-0 truncate py-1.5 cursor-grab touch-none select-none active:cursor-grabbing"
                  :title="`${node.name}（按住名称可拖拽排序）`"
                  @pointerdown="startCategoryDrag($event, node)"
                >{{ node.name }}</span>
                <span
                  class="shrink-0 pr-2 text-xs tabular-nums"
                  :class="activeCategory === node.id ? 'text-select-foreground/70' : 'text-muted-foreground'"
                >
                  {{ node.totalCount }}
                </span>
              </div>

              <p
                v-if="visibleCategories.length === 0"
                class="px-2 py-1 text-xs text-muted-foreground"
              >
                {{ sidebarFilter.trim() ? '无匹配分类' : '还没有分类' }}
              </p>
            </div>
          </div>

          <!-- 标签分组 -->
          <div>
            <div class="flex items-center justify-between px-2 py-1">
              <span class="flex items-center gap-1 text-xs font-medium text-muted-foreground">
                <IconTag class="h-3 w-3" />
                标签
              </span>
              <button
                v-if="activeTags.length > 0"
                type="button"
                class="text-xs text-primary-ink hover:underline"
                title="清除已选标签"
                @click="clearTags"
              >
                清除
              </button>
            </div>

            <div class="space-y-0.5">
              <div
                v-for="tag in visibleSidebarTags"
                :key="tag.name"
                class="flex items-center gap-2 px-2 py-1 rounded-md text-sm cursor-pointer select-none transition-colors"
                :class="isTagActive(tag.name) ? 'bg-select text-select-foreground' : 'hover:bg-muted'"
                @click="toggleTag(tag.name)"
              >
                <span class="flex-1 min-w-0 truncate">{{ tag.name }}</span>
                <span
                  class="shrink-0 text-xs tabular-nums"
                  :class="isTagActive(tag.name) ? 'text-select-foreground/70' : 'text-muted-foreground'"
                >
                  {{ tag.count }}
                </span>
              </div>

              <p
                v-if="filteredSidebarTags.length === 0"
                class="px-2 py-1 text-xs text-muted-foreground"
              >
                {{ sidebarFilter.trim() ? '无匹配标签' : '还没有标签' }}
              </p>

              <!-- 标签超过 12 个时提供展开/收起 -->
              <button
                v-if="hasMoreSidebarTags"
                type="button"
                class="w-full px-2 py-1 text-left text-xs text-muted-foreground hover:text-foreground transition-colors"
                @click="tagsExpanded = !tagsExpanded"
              >
                {{ tagsExpanded ? '收起' : `展开全部（${filteredSidebarTags.length}）` }}
              </button>
            </div>
          </div>
        </div>
      </aside>

      <!-- 右侧内容区 -->
      <main class="flex-1 rounded-[10px] bg-card flex flex-col overflow-hidden min-w-0">
        <!-- 工具栏：搜索 / 排序 / 添加 -->
        <div class="p-4 border-b space-y-3">
          <div class="flex items-center gap-3">
            <div class="relative flex-1 min-w-0">
              <IconSearch class="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
              <Input
                ref="searchInputRef"
                v-model="searchQuery"
                placeholder="搜索名称 / 用户名 / 网址 / 标签 / 拼音…"
                class="pl-9"
              />
            </div>

            <!-- 排序方式：更新时间 / 创建时间 / 名称 / 最近使用 / 使用频率 -->
            <Select
              :model-value="sortBy"
              :options="SORT_OPTIONS"
              class="h-9 w-36 shrink-0 px-2"
              title="排序方式"
              @update:model-value="handleSortKeyChange"
            />

            <!-- 升降序切换 -->
            <Button
              variant="outline"
              size="icon"
              class="shrink-0"
              :title="sortOrder === 'asc' ? '升序（点击切换为降序）' : '降序（点击切换为升序）'"
              @click="toggleSortOrder"
            >
              <IconArrowUp v-if="sortOrder === 'asc'" class="h-4 w-4" />
              <IconArrowDown v-else class="h-4 w-4" />
            </Button>

            <Button class="shrink-0" @click="openAddDialog">
              <IconPlus class="h-4 w-4 mr-2" />
              添加密码
            </Button>
          </div>

          <!-- 已选标签 chips（单个移除 / 全部清除） -->
          <div
            v-if="activeTags.length > 0"
            class="flex items-center gap-1.5 flex-wrap"
          >
            <span class="text-xs text-muted-foreground">已选标签：</span>
            <span
              v-for="tag in activeTags"
              :key="tag"
              class="inline-flex items-center gap-1 pl-2 pr-1 py-0.5 rounded-full bg-primary-ink/10 text-primary-ink text-xs"
            >
              {{ tag }}
              <button
                type="button"
                class="rounded-full p-0.5 hover:bg-primary-ink/20 transition-colors"
                :title="`移除标签 ${tag}`"
                @click="toggleTag(tag)"
              >
                <IconClose class="h-3 w-3" />
              </button>
            </span>
            <button
              type="button"
              class="text-xs text-muted-foreground hover:text-foreground hover:underline underline-offset-2 transition-colors"
              @click="clearTags"
            >
              清除全部
            </button>
          </div>
        </div>

        <!-- 密码列表表格 -->
        <div class="flex-1 overflow-auto p-4">
          <!-- table-fixed 固定列宽分配：内容超长只截断（title 显全文），不再横向撑破表格 -->
          <Table v-if="totalFiltered > 0" class="table-fixed">
            <colgroup>
              <!-- 名称吃剩余宽度；其余列定宽，总宽适配 900px 窗口下的内容区 -->
              <col />
              <col class="w-[20%]" />
              <!-- 密码列 = •••• + 强度条 + 复制按钮；Linux 字体 • 字形偏宽，列宽须给足，
                   且单元格内容允许收缩（bullet/强度条优先让位），保证复制按钮永不被裁 -->
              <col class="w-[120px]" />
              <col class="w-[80px]" />
              <col class="w-[84px]" />
              <!-- 操作列 = 两个 icon 按钮 72px + 间距 4px + 单元格 p-2 内边距 16px，再留 4px 余量 -->
              <col class="w-[96px]" />
            </colgroup>
            <TableHeader>
              <TableRow>
                <TableHead>名称</TableHead>
                <TableHead>用户名</TableHead>
                <TableHead>密码</TableHead>
                <TableHead>分类</TableHead>
                <TableHead>更新时间</TableHead>
                <!-- sticky 列需要不透明背景遮住滚动内容 -->
                <TableHead class="text-center sticky right-0 bg-card">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="item in pagedPasswords"
                :key="item.id"
                class="group"
                :draggable="delivery === 'dnd'"
                :class="delivery === 'dnd' ? 'cursor-grab active:cursor-grabbing select-none' : ''"
                :title="delivery === 'dnd' ? '按住这一行拖到目标输入框（也可以点右侧按钮复制）' : undefined"
                @pointerenter="prearmRowDnd(item.id)"
                @pointerdown="armRowDnd(item.id)"
                @dragstart="handleRowDndStart($event, item.id, item.title)"
                @dragend="handleRowDndEnd"
              >
                <!-- 名称：收藏星标 + 标题 + 最多 3 个标签小徽章（超出 +N） -->
                <TableCell class="font-medium overflow-hidden">
                  <div class="flex items-start gap-2 min-w-0">
                    <button
                      type="button"
                      class="shrink-0 mt-0.5 text-muted-foreground hover:text-yellow-500 transition-colors"
                      :title="item.is_favorite ? '取消收藏' : '收藏'"
                      @click="toggleFavorite(item.id)"
                    >
                      <IconStar class="h-4 w-4" :fill="item.is_favorite ? 'currentColor' : 'none'" />
                    </button>

                    <div class="min-w-0 flex-1">
                      <span class="block truncate" :title="item.title">{{ item.title }}</span>

                      <div
                        v-if="itemTags(item).length > 0"
                        class="flex items-center gap-1 mt-0.5 min-w-0 overflow-hidden"
                        :title="itemTagsTitle(item)"
                      >
                        <Badge
                          v-for="tag in visibleItemTags(item)"
                          :key="tag"
                          variant="outline"
                          class="min-w-0 shrink max-w-[64px] px-1.5 py-0 text-[10px] font-normal text-muted-foreground"
                        >
                          <span class="min-w-0 truncate">{{ tag }}</span>
                        </Badge>
                        <Badge
                          v-if="itemExtraTagCount(item) > 0"
                          variant="outline"
                          class="shrink-0 px-1.5 py-0 text-[10px] font-normal text-muted-foreground"
                          :title="itemTagsTitle(item)"
                        >
                          +{{ itemExtraTagCount(item) }}
                        </Badge>
                      </div>
                    </div>
                  </div>
                </TableCell>

                <!-- 用户名（可复制） -->
                <TableCell class="overflow-hidden">
                  <div class="flex items-center gap-2 min-w-0">
                    <span class="truncate" :title="item.username">{{ item.username }}</span>
                    <button
                      type="button"
                      class="shrink-0 text-muted-foreground hover:text-foreground transition-colors"
                      title="复制用户名"
                      @click="(e) => copyToClipboard(item.username, e, item.title, '用户名')"
                    >
                      <IconCopy class="h-4 w-4" />
                    </button>
                  </div>
                </TableCell>

                <!-- 密码（强度 + 复制/拖放；明文由后端交付，前端不接触） -->
                <TableCell class="overflow-hidden">
                  <!-- 收缩优先级：•••• 与强度条让位（min-w-0 + shrink），复制/拖拽按钮（shrink-0）永不被裁 -->
                  <div class="flex items-center gap-2">
                    <span class="min-w-0 truncate text-muted-foreground">••••</span>
                    <PasswordStrength
                      v-if="item.password_strength"
                      :level="item.password_strength"
                      compact
                      class="w-10 min-w-0 shrink"
                    />
                    <!-- 一个按钮两种语义：注入会话＝复制，Wayland＝放进拖拽池。
                         不再有行内把手：一个动作两个入口会让用户不知道该拖哪个，
                         而且 16px 的把手挤在表格行里本来就不好点（见拖拽池方案 §11） -->
                    <button
                      type="button"
                      class="shrink-0 text-muted-foreground hover:text-foreground transition-colors"
                      :title="delivery === 'dnd' ? '复制密码（再从浮窗拖到目标输入框）' : '复制密码'"
                      @click="(e) => copyPasswordViaBackend(item.id, e)"
                    >
                      <IconCopy class="h-4 w-4" />
                    </button>
                  </div>
                </TableCell>

                <!-- 分类 -->
                <TableCell class="overflow-hidden">
                  <Badge
                    variant="secondary"
                    class="block max-w-full truncate"
                    :title="categoryLabel(item.category)"
                  >
                    {{ categoryLabel(item.category) }}
                  </Badge>
                </TableCell>

                <!-- 更新时间 -->
                <TableCell class="text-muted-foreground text-sm overflow-hidden">
                  <span class="truncate block" :title="formatTimestamp(item.updated_at)">
                    {{ formatTimestamp(item.updated_at) }}
                  </span>
                </TableCell>

                <!-- 操作列钉在右侧：任何情况下编辑/删除都可见；
                     group-hover 同步行悬停高亮（sticky 的不透明底色会盖住行 hover） -->
                <TableCell class="sticky right-0 bg-card group-hover:bg-muted/50">
                  <div class="flex items-center justify-center gap-1">
                    <Button variant="ghost" size="icon" title="编辑" @click="openEditDialog(item)">
                      <IconPencil class="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      class="text-destructive hover:text-destructive"
                      title="删除"
                      @click="handleDeletePassword(item)"
                    >
                      <IconTrash class="h-4 w-4" />
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>

          <div
            v-else
            class="flex flex-col items-center justify-center py-12 text-muted-foreground"
          >
            <!-- 空状态：区分「暂无密码」与「筛选无结果」 -->
            <IconEmpty class="h-12 w-12 mb-4" stroke-width="1.5" />

            <template v-if="allCount === 0">
              <p class="text-lg font-medium">暂无密码</p>
              <p class="text-sm">点击“添加密码”按钮开始使用</p>
            </template>

            <template v-else>
              <p class="text-lg font-medium">筛选无结果</p>
              <p class="text-sm">“{{ activeCategoryName }}”下没有符合条件的密码</p>
              <Button variant="outline" size="sm" class="mt-4" @click="clearFilters">
                清除筛选
              </Button>
            </template>
          </div>
        </div>

        <!-- 底部分页：共 N 条 · 第 x / y 页 + 每页条数 + 上一页/下一页 -->
        <footer class="border-t px-4 py-2 flex items-center justify-between gap-3 text-sm shrink-0">
          <span class="text-muted-foreground">
            共 {{ totalFiltered }} 条<template v-if="totalFiltered > 0"> · 第 {{ currentPage }} / {{ pageCount }} 页</template>
          </span>

          <div v-if="totalFiltered > 0" class="flex items-center gap-3">
            <label class="flex items-center gap-1.5 text-muted-foreground">
              每页
              <Select
                :model-value="pageSize"
                :options="PAGE_SIZE_OPTIONS.map((size) => ({ value: Number(size), label: String(size) }))"
                class="h-8 w-24 px-2"
                @update:model-value="handlePageSizeChange"
              />
              条
            </label>

            <Button
              variant="outline"
              size="sm"
              :disabled="currentPage <= 1"
              @click="setPage(currentPage - 1)"
            >
              上一页
            </Button>
            <Button
              variant="outline"
              size="sm"
              :disabled="currentPage >= pageCount"
              @click="setPage(currentPage + 1)"
            >
              下一页
            </Button>
          </div>
        </footer>
      </main>
    </div>

    <!-- 密码表单弹窗 -->
    <PasswordFormDialog
      v-model:open="showFormDialog"
      :edit-item="editingItem"
      @saved="() => {}"
    />

    <!-- 密码生成器弹窗 -->
    <PasswordGenerator
      v-model:open="showGenerator"
      @generated="() => {}"
    />

    <!-- 快速搜索弹窗 -->
    <QuickSearch v-model:open="showQuickSearch" />

    <!-- 快速添加弹窗 -->
    <QuickAdd v-model:open="showQuickAdd" />

    <!-- 分类右键菜单（空白处新增顶级分类；分类行上新增子分类/重命名/置顶级/删除） -->
    <ContextMenu
      v-model:open="categoryMenu.open"
      :x="categoryMenu.x"
      :y="categoryMenu.y"
      :items="categoryMenuItems"
      @select="handleCategoryMenuSelect"
    />

    <!-- 分类快捷新增 / 重命名弹窗 -->
    <Dialog v-model:open="showCategoryDialog">
      <DialogContent class="sm:max-w-[360px]">
        <DialogHeader>
          <DialogTitle>{{ categoryDialogMode === 'rename' ? '重命名分类' : '新增分类' }}</DialogTitle>
          <DialogDescription>
            {{ categoryDialogMode === 'rename' ? '修改分类名称，父分类保持不变' : '可选择挂载到哪个父分类下（层级不限）' }}
          </DialogDescription>
        </DialogHeader>
        <div class="py-4 grid gap-4">
          <div class="grid gap-2">
            <Label for="home-category-name">分类名称</Label>
            <Input
              id="home-category-name"
              v-model="categoryDialogName"
              placeholder="输入分类名称"
              @keydown.enter="handleSaveCategoryDialog"
            />
          </div>
          <div v-if="categoryDialogMode === 'add'" class="grid gap-2">
            <Label for="home-category-parent">父分类</Label>
            <Select
              id="home-category-parent"
              v-model="categoryDialogParentId"
              :options="categoryParentOptions"
            />
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" @click="showCategoryDialog = false">取消</Button>
          <Button
            :disabled="!categoryDialogName.trim() || categorySaving"
            @click="handleSaveCategoryDialog"
          >
            {{ categorySaving ? '保存中...' : (categoryDialogMode === 'rename' ? '保存' : '添加') }}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <!-- Toast 提示 -->
    <Toast
      v-model:show="toast.show"
      :type="toast.type"
      :message="toast.message"
    />
  </div>
</template>
