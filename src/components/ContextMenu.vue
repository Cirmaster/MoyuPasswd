<!--
  ContextMenu.vue - 通用右键菜单

  用途：在目标元素触发 contextmenu 后，由父组件记录坐标并打开本菜单。
  功能：
  1. Teleport 到 body，fixed 定位，自动做视口边界钳制（右下越界时回弹）
  2. 点击菜单外部 / 按 Esc / 滚动 / 窗口缩放时关闭
  3. 键盘 ↑↓ 导航、Enter 执行、鼠标悬停高亮
  4. 支持图标、危险项、禁用项、分隔线

  @example
  ```vue
  <ContextMenu
    v-model:open="menu.open"
    :x="menu.x"
    :y="menu.y"
    :items="menuItems"
    @select="handleSelect"
  />
  ```
-->
<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, watch, type Component } from 'vue'

/** 菜单项结构 */
interface ContextMenuItem {
  /** 唯一标识，select 事件回传 */
  key: string
  /** 展示文字 */
  label: string
  /** 左侧图标（从 @/components/icons 导入的组件） */
  icon?: Component
  /** 危险操作（红色文字） */
  danger?: boolean
  /** 禁用 */
  disabled?: boolean
  /** 是否在本项上方显示分隔线 */
  divided?: boolean
}

/** 组件 Props */
interface Props {
  /** 是否打开 */
  open: boolean
  /** 打开位置（视口坐标，通常是鼠标事件 clientX/clientY） */
  x: number
  /** 打开位置（视口坐标） */
  y: number
  /** 菜单项 */
  items: ContextMenuItem[]
}

const props = defineProps<Props>()

/** 组件事件 */
const emit = defineEmits<{
  /** 更新打开状态（配合 v-model:open） */
  'update:open': [value: boolean]
  /** 选中某一项（key） */
  select: [key: string]
}>()

/** 菜单根元素（用于测量尺寸与判断点击是否在菜单内） */
const menuRef = ref<HTMLElement | null>(null)

/** 菜单最终定位（经视口钳制） */
const position = ref({ left: 0, top: 0 })

/** 当前键盘高亮的菜单项 key */
const activeKey = ref<string | null>(null)

/** 菜单与视口边缘的最小间距 */
const VIEWPORT_MARGIN = 8

/** 可执行（非禁用）的菜单项列表，用于键盘导航 */
const enabledItems = computed(() => props.items.filter((item) => !item.disabled))

/**
 * 计算并更新菜单位置：打开位置尽量贴近鼠标，超出视口时向内回弹
 */
const updatePosition = async () => {
  await nextTick()
  const el = menuRef.value
  const width = el?.offsetWidth ?? 180
  const height = el?.offsetHeight ?? 0
  const maxLeft = Math.max(VIEWPORT_MARGIN, window.innerWidth - width - VIEWPORT_MARGIN)
  const maxTop = Math.max(VIEWPORT_MARGIN, window.innerHeight - height - VIEWPORT_MARGIN)
  position.value = {
    left: Math.min(Math.max(VIEWPORT_MARGIN, props.x), maxLeft),
    top: Math.min(Math.max(VIEWPORT_MARGIN, props.y), maxTop),
  }
}

/** 关闭菜单 */
const close = () => {
  emit('update:open', false)
}

/**
 * 执行菜单项
 * @param item - 菜单项
 */
const selectItem = (item: ContextMenuItem) => {
  if (item.disabled) return
  close()
  emit('select', item.key)
}

/**
 * 键盘导航：↑↓ 切换高亮、Enter 执行、Esc 关闭
 * @param event - 键盘事件
 */
const handleKeydown = (event: KeyboardEvent) => {
  if (!props.open) return
  const list = enabledItems.value
  if (list.length === 0) return

  if (event.key === 'Escape') {
    event.preventDefault()
    close()
    return
  }

  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault()
    const currentIndex = list.findIndex((item) => item.key === activeKey.value)
    const offset = event.key === 'ArrowDown' ? 1 : -1
    const nextIndex =
      currentIndex === -1
        ? 0
        : (currentIndex + offset + list.length) % list.length
    const next = list[nextIndex]
    if (next) activeKey.value = next.key
    return
  }

  if (event.key === 'Enter') {
    event.preventDefault()
    const active = list.find((item) => item.key === activeKey.value) ?? list[0]
    if (active) selectItem(active)
  }
}

/**
 * 点击菜单外部关闭（捕获阶段监听，避免被内部点击抢先）
 * @param event - 鼠标事件
 */
const handleOutsidePointerDown = (event: MouseEvent) => {
  if (!props.open) return
  const target = event.target
  if (target instanceof Node && menuRef.value?.contains(target)) return
  close()
}

/** 滚动或窗口尺寸变化时关闭（避免菜单与触发位置错位） */
const handleViewportChange = () => {
  if (props.open) close()
}

// 打开状态/坐标变化时重置高亮并重新计算位置
watch(
  () => [props.open, props.x, props.y] as const,
  ([open]) => {
    if (!open) return
    activeKey.value = enabledItems.value[0]?.key ?? null
    // 先用原始坐标占位，避免菜单在测量前短暂出现在左上角
    position.value = { left: props.x, top: props.y }
    void updatePosition()
  },
)

// 菜单项变化时保证高亮项仍然有效
watch(
  () => props.items,
  () => {
    if (!props.open) return
    if (!enabledItems.value.some((item) => item.key === activeKey.value)) {
      activeKey.value = enabledItems.value[0]?.key ?? null
    }
  },
  { deep: true },
)

document.addEventListener('mousedown', handleOutsidePointerDown, true)
document.addEventListener('keydown', handleKeydown, true)
window.addEventListener('scroll', handleViewportChange, true)
window.addEventListener('resize', handleViewportChange)

onUnmounted(() => {
  document.removeEventListener('mousedown', handleOutsidePointerDown, true)
  document.removeEventListener('keydown', handleKeydown, true)
  window.removeEventListener('scroll', handleViewportChange, true)
  window.removeEventListener('resize', handleViewportChange)
})
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      ref="menuRef"
      role="menu"
      class="fixed z-[100] min-w-[172px] max-w-[260px] overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
      :style="{ left: `${position.left}px`, top: `${position.top}px` }"
      @contextmenu.prevent
    >
      <template v-for="item in items" :key="item.key">
        <div v-if="item.divided" class="my-1 h-px bg-border" />
        <button
          type="button"
          role="menuitem"
          :disabled="item.disabled"
          class="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm transition-colors disabled:pointer-events-none disabled:opacity-50"
          :class="[
            item.danger ? 'text-destructive' : '',
            activeKey === item.key && !item.disabled
              ? item.danger
                ? 'bg-destructive/10'
                : 'bg-accent text-accent-foreground'
              : !item.disabled
                ? item.danger
                  ? 'hover:bg-destructive/10'
                  : 'hover:bg-accent hover:text-accent-foreground'
                : '',
          ]"
          @mouseenter="activeKey = item.key"
          @click="selectItem(item)"
        >
          <component :is="item.icon" v-if="item.icon" class="h-4 w-4 shrink-0" />
          <span class="flex-1 truncate">{{ item.label }}</span>
        </button>
      </template>
    </div>
  </Teleport>
</template>
