<!--
  TagInput.vue - 标签输入组件

  用于给密码条目编辑标签（标签是分类之外的第二维分类）。
  功能：
  1. 已选标签以徽章展示，可单个删除
  2. 回车 / 逗号 / 分号添加标签，Backspace 删除末尾标签
  3. 输入时展示已有标签建议（来自 props.suggestions），点击即可添加
  4. 支持粘贴「a,b,c」批量添加
  5. 去重（大小写不敏感）、trim、单标签最长 32 字符、最多 maxTags 个

  本组件不依赖 Store，建议列表由父组件传入。

  @example
  ```vue
  <TagInput v-model="form.tags" :suggestions="tagSuggestions" />
  ```
-->
<script setup lang="ts">
import { computed, ref } from 'vue'
import { IconClose, IconTag } from '@/components/icons'

/** 组件 Props */
interface Props {
  /** 已选标签 */
  modelValue: string[]
  /** 可选建议（通常来自 Store 的全局标签） */
  suggestions?: string[]
  /** 输入框占位文字 */
  placeholder?: string
  /** 最多可选标签数 */
  maxTags?: number
  /** 是否禁用 */
  disabled?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  suggestions: () => [],
  placeholder: '输入标签后按回车添加',
  maxTags: 20,
  disabled: false,
})

/** 组件事件 */
const emit = defineEmits<{
  /** 标签列表变化 */
  'update:modelValue': [value: string[]]
}>()

/** 单个标签最大字符数（与后端规则一致） */
const MAX_TAG_CHARS = 32
/** 建议下拉最大条数 */
const MAX_SUGGESTIONS = 8

/** 输入框内容 */
const inputValue = ref('')

/** 输入框是否聚焦（控制建议下拉） */
const focused = ref(false)

/** 建议下拉当前高亮索引 */
const activeSuggestionIndex = ref(0)

/** 超出上限时的临时提示 */
const limitHint = ref('')

/** 根节点（用于点击外部关闭建议下拉） */
const rootRef = ref<HTMLElement | null>(null)

/** 是否已达标签上限 */
const isFull = computed(() => props.modelValue.length >= props.maxTags)

/** 当前是否有可展示的建议 */
const filteredSuggestions = computed(() => {
  const selected = new Set(props.modelValue.map((tag) => tag.toLocaleLowerCase()))
  const keyword = inputValue.value.trim().toLocaleLowerCase()
  return props.suggestions
    .filter((name) => {
      const lower = name.toLocaleLowerCase()
      if (selected.has(lower)) return false
      return keyword ? lower.includes(keyword) : true
    })
    .slice(0, MAX_SUGGESTIONS)
})

/**
 * 规范化标签：trim + 按字符截断到 32
 * @param raw - 原始输入
 */
const normalizeTag = (raw: string): string => {
  const trimmed = raw.trim()
  if (!trimmed) return ''
  return Array.from(trimmed).slice(0, MAX_TAG_CHARS).join('')
}

/**
 * 添加一个标签（自动去重、检查上限）
 * @param raw - 原始标签文本
 */
const addTag = (raw: string) => {
  const tag = normalizeTag(raw)
  if (!tag) return

  // 大小写不敏感去重
  const exists = props.modelValue.some(
    (item) => item.toLocaleLowerCase() === tag.toLocaleLowerCase(),
  )
  if (exists) return

  if (isFull.value) {
    limitHint.value = `最多添加 ${props.maxTags} 个标签`
    window.setTimeout(() => {
      limitHint.value = ''
    }, 1800)
    return
  }

  emit('update:modelValue', [...props.modelValue, tag])
}

/**
 * 从输入内容添加（支持逗号/分号分隔的多个标签）
 */
const commitInput = () => {
  const raw = inputValue.value
  if (!raw.trim()) return
  raw
    .split(/[,;，；]/)
    .map((part) => part.trim())
    .filter(Boolean)
    .forEach(addTag)
  inputValue.value = ''
  activeSuggestionIndex.value = 0
}

/**
 * 删除指定位置的标签
 * @param index - 标签索引
 */
const removeTag = (index: number) => {
  const next = props.modelValue.slice()
  next.splice(index, 1)
  emit('update:modelValue', next)
}

/**
 * 回车/分隔符提交；Backspace 删除末尾；方向键选择建议
 * @param event - 键盘事件
 */
const handleKeydown = (event: KeyboardEvent) => {
  if (
    event.key === 'Enter' ||
    event.key === ',' ||
    event.key === '，' ||
    event.key === ';' ||
    event.key === '；'
  ) {
    event.preventDefault()
    // 有高亮建议时优先使用建议
    if (
      event.key === 'Enter' &&
      focused.value &&
      filteredSuggestions.value.length > 0 &&
      filteredSuggestions.value[activeSuggestionIndex.value]
    ) {
      addTag(filteredSuggestions.value[activeSuggestionIndex.value] as string)
      inputValue.value = ''
      activeSuggestionIndex.value = 0
      return
    }
    commitInput()
    return
  }

  if (event.key === 'Backspace' && !inputValue.value && props.modelValue.length > 0) {
    event.preventDefault()
    removeTag(props.modelValue.length - 1)
    return
  }

  if (event.key === 'ArrowDown' && filteredSuggestions.value.length > 0) {
    event.preventDefault()
    activeSuggestionIndex.value = Math.min(
      activeSuggestionIndex.value + 1,
      filteredSuggestions.value.length - 1,
    )
    return
  }

  if (event.key === 'ArrowUp' && filteredSuggestions.value.length > 0) {
    event.preventDefault()
    activeSuggestionIndex.value = Math.max(activeSuggestionIndex.value - 1, 0)
    return
  }

  if (event.key === 'Escape') {
    focused.value = false
  }
}

/**
 * 粘贴文本：支持「标签1,标签2」批量添加
 * @param event - 粘贴事件
 */
const handlePaste = (event: ClipboardEvent) => {
  const text = event.clipboardData?.getData('text') ?? ''
  if (!/[,;，；]/.test(text)) return
  event.preventDefault()
  text
    .split(/[,;，；]/)
    .map((part) => part.trim())
    .filter(Boolean)
    .forEach(addTag)
  inputValue.value = ''
}

/**
 * 点击建议项添加
 * @param name - 建议标签名
 */
const handleSuggestionClick = (name: string) => {
  addTag(name)
  inputValue.value = ''
  activeSuggestionIndex.value = 0
}

/** 输入框获得焦点 */
const handleFocus = () => {
  focused.value = true
}

/** 输入框失焦：延迟收起，保证建议项点击先触发 */
const handleBlur = () => {
  window.setTimeout(() => {
    // 若焦点仍在组件内部（例如点了建议项）则不收起
    if (rootRef.value && rootRef.value.contains(document.activeElement)) return
    focused.value = false
  }, 120)
}

/**
 * 点击容器空白处时聚焦输入框
 * @param event - 鼠标事件
 */
const focusInput = (event: MouseEvent) => {
  const container = event.currentTarget as HTMLElement | null
  container?.querySelector('input')?.focus()
}
</script>

<template>
  <div ref="rootRef" class="relative">
    <!-- 标签容器：徽章 + 输入框 -->
    <div
      class="flex min-h-10 w-full flex-wrap items-center gap-1.5 rounded-md border border-input bg-card px-2 py-1.5 text-sm ring-offset-background transition-colors focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2"
      :class="{ 'opacity-60': disabled }"
      @click="focusInput"
    >
      <IconTag class="size-3.5 shrink-0 text-muted-foreground" />
      <span
        v-for="(tag, index) in modelValue"
        :key="tag"
        class="inline-flex max-w-[160px] items-center gap-1 rounded bg-secondary px-1.5 py-0.5 text-xs text-secondary-foreground"
        :title="tag"
      >
        <span class="truncate">{{ tag }}</span>
        <button
          type="button"
          class="shrink-0 rounded-sm text-muted-foreground transition-colors hover:text-destructive disabled:pointer-events-none"
          :disabled="disabled"
          :aria-label="`删除标签 ${tag}`"
          @click.stop="removeTag(index)"
        >
          <IconClose class="size-3" />
        </button>
      </span>
      <input
        v-model="inputValue"
        type="text"
        :placeholder="modelValue.length === 0 ? placeholder : ''"
        :disabled="disabled"
        class="min-w-[120px] flex-1 bg-transparent py-0.5 text-sm outline-none placeholder:text-muted-foreground/60 disabled:cursor-not-allowed"
        @keydown="handleKeydown"
        @paste="handlePaste"
        @focus="handleFocus"
        @blur="handleBlur"
      />
    </div>

    <!-- 建议下拉 -->
    <div
      v-if="focused && filteredSuggestions.length > 0"
      class="absolute z-50 mt-1 max-h-48 w-full overflow-y-auto rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
    >
      <button
        v-for="(name, index) in filteredSuggestions"
        :key="name"
        type="button"
        class="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm transition-colors"
        :class="index === activeSuggestionIndex ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/60'"
        @mousedown.prevent="handleSuggestionClick(name)"
        @mouseenter="activeSuggestionIndex = index"
      >
        <IconTag class="size-3.5 shrink-0 text-muted-foreground" />
        <span class="truncate">{{ name }}</span>
      </button>
    </div>

    <!-- 规则提示 / 上限提示 -->
    <p class="mt-1 text-xs" :class="limitHint ? 'text-destructive' : 'text-muted-foreground'">
      {{ limitHint || `回车或逗号添加标签，最多 ${maxTags} 个（单个标签不超过 ${MAX_TAG_CHARS} 个字符）` }}
    </p>
  </div>
</template>
