<!--
  Toast.vue - 全局提示组件

  用于显示操作成功、失败等提示信息。
  支持多种类型：success（成功）、error（错误）、info（信息）。

  @example
  ```vue
  <Toast v-model:show="showToast" type="success" message="操作成功" />
  ```
-->

<script setup lang="ts">
import { ref, watch } from 'vue'

/**
 * 组件 Props
 */
interface Props {
  /** 是否显示 */
  show: boolean
  /** 提示类型：success、error、info */
  type?: 'success' | 'error' | 'info'
  /** 提示消息 */
  message?: string
  /** 显示时长（毫秒），0 表示不自动关闭 */
  duration?: number
}

const props = withDefaults(defineProps<Props>(), {
  type: 'success',
  message: '',
  duration: 2000,
})

/**
 * 组件事件
 */
const emit = defineEmits<{
  'update:show': [value: boolean]
}>()

/** 内部显示状态 */
const visible = ref(props.show)

/**
 * 监听外部 show 变化
 */
watch(
  () => props.show,
  (val) => {
    visible.value = val
    if (val && props.duration > 0) {
      // 自动关闭
      setTimeout(() => {
        close()
      }, props.duration)
    }
  },
)

/**
 * 关闭提示
 */
const close = () => {
  visible.value = false
  emit('update:show', false)
}

/**
 * 根据类型获取图标颜色
 */
const getIconClass = () => {
  switch (props.type) {
    case 'success':
      return 'text-green-500'
    case 'error':
      return 'text-destructive'
    case 'info':
      return 'text-blue-500'
    default:
      return 'text-green-500'
  }
}
</script>

<template>
  <!-- 遮罩层 -->
  <Teleport to="body">
    <Transition
      enter-active-class="transition-all duration-300 ease-out"
      enter-from-class="opacity-0 -translate-y-4"
      enter-to-class="opacity-100 translate-y-0"
      leave-active-class="transition-all duration-200 ease-in"
      leave-from-class="opacity-100 translate-y-0"
      leave-to-class="opacity-0 -translate-y-4"
    >
      <div
        v-if="visible"
        class="fixed top-4 left-1/2 -translate-x-1/2 z-[9999] flex items-center gap-3 px-4 py-3 rounded-lg shadow-lg border bg-background"
        @click="close"
      >
        <!-- 图标 -->
        <svg
          v-if="type === 'success'"
          xmlns="http://www.w3.org/2000/svg"
          :class="['h-5 w-5 shrink-0', getIconClass()]"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
          <polyline points="22 4 12 14.01 9 11.01" />
        </svg>

        <svg
          v-else-if="type === 'error'"
          xmlns="http://www.w3.org/2000/svg"
          :class="['h-5 w-5 shrink-0', getIconClass()]"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <circle cx="12" cy="12" r="10" />
          <line x1="15" y1="9" x2="9" y2="15" />
          <line x1="9" y1="9" x2="15" y2="15" />
        </svg>

        <svg
          v-else
          xmlns="http://www.w3.org/2000/svg"
          :class="['h-5 w-5 shrink-0', getIconClass()]"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <circle cx="12" cy="12" r="10" />
          <line x1="12" y1="16" x2="12" y2="12" />
          <line x1="12" y1="8" x2="12.01" y2="8" />
        </svg>

        <!-- 消息 -->
        <span class="text-sm font-medium">{{ message }}</span>
      </div>
    </Transition>
  </Teleport>
</template>
