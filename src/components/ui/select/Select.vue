<!--
  Select.vue - 主题化下拉选择（替代原生 <select>）

  原生 <select> 的弹出列表由 WebKit/系统渲染，不跟 CSS 主题走（深色主题下
  弹出白色列表），且 WebView2 / WebKitGTK 各平台渲染不一致，CSS 基本管不到。
  这里按 ContextMenu 的自绘弹层惯例（Teleport 到 body + 主题令牌配色）实现，
  任意色系、明暗模式下观感统一。

  API 按调用点的实际用法收敛为「选项数组 + v-model」：
    <Select v-model="x" :options="[{ value, label }]" class="w-full" />
  - value 支持空字符串（「无（顶级分类）」这类真实选项）与数字（每页条数）；
    不用 reka-ui 的 Select 正是因为它禁止空字符串 value
  - class 合并进触发器（默认 h-10 w-full 输入框样式，可覆盖尺寸）
  - 其余透传属性（title / id 等）落在触发器上
  键盘：↑↓ 移动、Enter/Space 选中、Esc 关闭、Home/End 首尾、首字母跳转；
  带基本 ARIA（combobox + listbox）。焦点始终留在触发器上（activedescendant 模式）。
-->

<script lang="ts">
/** 下拉选项（value 允许空串与数字，label 为展示文本） */
export interface SelectOption {
  value: string | number
  label: string
  disabled?: boolean
}
</script>

<script setup lang="ts" generic="T extends string | number">
import { computed, onBeforeUnmount, ref, useAttrs, useId } from 'vue'
import type { HTMLAttributes } from 'vue'
import { cn } from '@/lib/utils'

interface Props {
  options: SelectOption[]
  placeholder?: string
  disabled?: boolean
  class?: HTMLAttributes['class']
}

const props = withDefaults(defineProps<Props>(), {
  placeholder: '请选择',
  disabled: false,
})

/** 泛型 T 由各调用点的 v-model 类型推断（string / 数字 / 字符串字面量联合） */
const model = defineModel<T>()

/** 透传属性（title / id 等）手动落在触发器上（多根组件不会自动继承） */
const attrs = useAttrs()

const triggerRef = ref<HTMLElement | null>(null)
const listRef = ref<HTMLElement | null>(null)
const open = ref(false)

/** 当前高亮项下标（-1 表示无） */
const activeIndex = ref(-1)

const listboxId = useId()

/** 弹层定位（fixed 坐标 + 翻转/高度钳制） */
const position = ref({ top: 0, left: 0, width: 0, maxHeight: 240, flip: false })

/** 选中项展示文本；找不到（未选择）时显示占位符 */
const selectedLabel = computed(
  () => props.options.find((o) => o.value === model.value)?.label,
)

const popupStyle = computed(() => ({
  top: `${position.value.top}px`,
  left: `${position.value.left}px`,
  width: `${position.value.width}px`,
  'max-height': `${position.value.maxHeight}px`,
  // 空间不足向上翻转时从触发器上沿向上展开
  transform: position.value.flip ? 'translateY(-100%)' : undefined,
}))

/** 按触发器位置计算弹层坐标：优先向下，下方空间不足则翻转向上，横向钳入视口 */
function updatePosition() {
  const el = triggerRef.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  const spaceBelow = window.innerHeight - rect.bottom - 8
  const spaceAbove = rect.top - 8
  const flip = spaceBelow < 180 && spaceAbove > spaceBelow
  position.value = {
    top: flip ? rect.top - 4 : rect.bottom + 4,
    left: Math.max(8, Math.min(rect.left, window.innerWidth - rect.width - 8)),
    width: rect.width,
    maxHeight: Math.min(320, Math.max(120, flip ? spaceAbove : spaceBelow)),
    flip,
  }
}

function openMenu() {
  if (props.disabled || open.value) return
  updatePosition()
  open.value = true
  // 初始高亮当前选中项（无选中/选中项禁用则高亮第一个可用项）
  const selectedIdx = props.options.findIndex((o) => o.value === model.value && !o.disabled)
  activeIndex.value =
    selectedIdx >= 0 ? selectedIdx : props.options.findIndex((o) => !o.disabled)
  window.addEventListener('resize', updatePosition)
  // 捕获阶段监听：弹层可能位于可滚动容器（对话框/设置页）内，滚动时跟随触发器
  window.addEventListener('scroll', updatePosition, true)
  document.addEventListener('mousedown', onOutsideDown, true)
}

function closeMenu(refocus = false) {
  if (!open.value) return
  open.value = false
  window.removeEventListener('resize', updatePosition)
  window.removeEventListener('scroll', updatePosition, true)
  document.removeEventListener('mousedown', onOutsideDown, true)
  if (refocus) triggerRef.value?.focus()
}

function toggle() {
  if (open.value) closeMenu(true)
  else openMenu()
}

/** 点击触发器/弹层之外的区域关闭（mousedown 判定，避免与选项点击竞态） */
function onOutsideDown(e: MouseEvent) {
  const target = e.target as Node
  if (triggerRef.value?.contains(target) || listRef.value?.contains(target)) return
  closeMenu()
}

function selectAt(index: number) {
  const option = props.options[index]
  if (!option || option.disabled) return
  // options 按 SelectOption 声明（value 为 string | number），运行时与 model 同型，收窄回 T
  model.value = option.value as T
  closeMenu(true)
}

/** 把高亮项滚进可视区 */
function scrollActiveIntoView() {
  requestAnimationFrame(() => {
    listRef.value
      ?.querySelector('[data-active="true"]')
      ?.scrollIntoView({ block: 'nearest' })
  })
}

/** 上下移动高亮（跳过禁用项，环形绕回） */
function moveActive(step: 1 | -1) {
  const opts = props.options
  if (opts.length === 0) return
  let idx = activeIndex.value
  for (let i = 0; i < opts.length; i++) {
    idx = (idx + step + opts.length) % opts.length
    if (!opts[idx]?.disabled) break
  }
  activeIndex.value = idx
  scrollActiveIntoView()
}

function moveActiveToEdge(edge: 'first' | 'last') {
  const opts = props.options
  const seq = edge === 'first' ? opts : [...opts].reverse()
  const found = seq.find((o) => !o.disabled)
  if (!found) return
  activeIndex.value = opts.indexOf(found)
  scrollActiveIntoView()
}

/** 首字母跳转：连续输入按前缀匹配，500ms 内视为连续输入 */
let typeBuffer = ''
let typeTimer: ReturnType<typeof setTimeout> | undefined

function onTypeahead(char: string) {
  typeBuffer += char.toLowerCase()
  clearTimeout(typeTimer)
  typeTimer = setTimeout(() => (typeBuffer = ''), 500)
  const opts = props.options
  // 单字符从下一项开始找（连续同一字母在候选项间循环），多字符前缀从头找
  const start = typeBuffer.length > 1 ? 0 : activeIndex.value + 1
  for (let i = 0; i < opts.length; i++) {
    const idx = (start + i) % opts.length
    const opt = opts[idx]
    if (opt && !opt.disabled && opt.label.toLowerCase().startsWith(typeBuffer)) {
      activeIndex.value = idx
      scrollActiveIntoView()
      return
    }
  }
}

function onKeydown(e: KeyboardEvent) {
  if (props.disabled) return

  if (!open.value) {
    if (['Enter', ' ', 'ArrowDown', 'ArrowUp'].includes(e.key)) {
      e.preventDefault()
      openMenu()
    }
    return
  }

  switch (e.key) {
    case 'ArrowDown':
      e.preventDefault()
      moveActive(1)
      break
    case 'ArrowUp':
      e.preventDefault()
      moveActive(-1)
      break
    case 'Home':
      e.preventDefault()
      moveActiveToEdge('first')
      break
    case 'End':
      e.preventDefault()
      moveActiveToEdge('last')
      break
    case 'Enter':
    case ' ':
      e.preventDefault()
      if (activeIndex.value >= 0) selectAt(activeIndex.value)
      break
    case 'Escape':
      e.preventDefault()
      closeMenu(true)
      break
    case 'Tab':
      closeMenu()
      break
    default:
      // 可打印字符 → 首字母/前缀跳转
      if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) onTypeahead(e.key)
  }
}

onBeforeUnmount(() => {
  clearTimeout(typeTimer)
  closeMenu()
})
</script>

<template>
  <button
    ref="triggerRef"
    type="button"
    role="combobox"
    :aria-expanded="open"
    aria-haspopup="listbox"
    :aria-controls="listboxId"
    :aria-activedescendant="open && activeIndex >= 0 ? `${listboxId}-item-${activeIndex}` : undefined"
    :disabled="disabled"
    v-bind="attrs"
    :class="
      cn(
        'flex h-10 w-full items-center justify-between gap-2 rounded-md border border-input bg-card px-3 py-2 text-sm text-foreground',
        'ring-offset-background outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2',
        'disabled:cursor-not-allowed disabled:opacity-50',
        props.class,
      )
    "
    @click="toggle"
    @keydown="onKeydown"
  >
    <span :class="cn('truncate', selectedLabel === undefined && 'text-muted-foreground')">
      {{ selectedLabel ?? placeholder }}
    </span>
    <!-- 折叠箭头（lucide chevron-down，随文本色半透明） -->
    <svg
      class="h-4 w-4 shrink-0 opacity-50"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="m6 9 6 6 6-6" />
    </svg>
  </button>

  <!-- 自绘弹层：配色全部走主题令牌，深色/任意色系下与整体一致 -->
  <Teleport to="body">
    <div
      v-if="open"
      :id="listboxId"
      ref="listRef"
      role="listbox"
      :style="popupStyle"
      class="fixed z-[100] overflow-y-auto rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-md"
    >
      <div
        v-for="(option, index) in options"
        :id="`${listboxId}-item-${index}`"
        :key="option.value"
        role="option"
        :aria-selected="option.value === model"
        :aria-disabled="option.disabled"
        :data-active="index === activeIndex"
        :class="
          cn(
            'relative flex cursor-pointer select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none',
            index === activeIndex && 'bg-accent text-accent-foreground',
            option.value === model && 'bg-select text-select-foreground',
            option.disabled && 'pointer-events-none opacity-50',
          )
        "
        @mousedown.prevent
        @click="selectAt(index)"
        @mouseenter="activeIndex = index"
      >
        <span class="truncate">{{ option.label }}</span>
        <!-- 选中对勾（lucide check） -->
        <svg
          v-if="option.value === model"
          class="ml-auto h-3.5 w-3.5 shrink-0"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M20 6 9 17l-5-5" />
        </svg>
      </div>
    </div>
  </Teleport>
</template>
