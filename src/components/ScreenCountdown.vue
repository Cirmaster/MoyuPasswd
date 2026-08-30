<!--
  ScreenCountdown.vue - 屏幕右下角倒计时悬浮窗

  这是一个独立的悬浮窗，始终显示在屏幕右下角。
  用于显示剪贴板清除倒计时。
-->

<script setup lang="ts">
import { ref, watch, onUnmounted } from 'vue'

/**
 * 组件 Props
 */
interface Props {
  /** 是否显示 */
  show: boolean
  /** 倒计时秒数 */
  seconds: number
}

const props = withDefaults(defineProps<Props>(), {
  seconds: 30,
})

/**
 * 组件事件
 */
const emit = defineEmits<{
  'update:show': [value: boolean]
  'countdown-end': []
}>()

/** 剩余秒数 */
const remaining = ref(props.seconds)

/** 计时器 */
let timer: ReturnType<typeof setInterval> | null = null

/**
 * 开始倒计时
 */
const startCountdown = () => {
  remaining.value = props.seconds

  timer = setInterval(() => {
    remaining.value--

    if (remaining.value <= 0) {
      stopCountdown()
      emit('countdown-end')
      emit('update:show', false)
    }
  }, 1000)
}

/**
 * 停止倒计时
 */
const stopCountdown = () => {
  if (timer) {
    clearInterval(timer)
    timer = null
  }
}

/**
 * 取消倒计时
 */
const cancel = () => {
  stopCountdown()
  emit('update:show', false)
}

/**
 * 监听 show 变化
 */
watch(
  () => props.show,
  (val) => {
    if (val) {
      startCountdown()
    } else {
      stopCountdown()
    }
  },
  { immediate: true },
)

/**
 * 组件卸载时停止倒计时
 */
onUnmounted(() => {
  stopCountdown()
})
</script>

<template>
  <Teleport to="body">
    <Transition
      enter-active-class="transition-all duration-300 ease-out"
      enter-from-class="opacity-0 translate-y-2 scale-95"
      enter-to-class="opacity-100 translate-y-0 scale-100"
      leave-active-class="transition-all duration-200 ease-in"
      leave-from-class="opacity-100 translate-y-0 scale-100"
      leave-to-class="opacity-0 translate-y-2 scale-95"
    >
      <div
        v-if="show"
        class="fixed bottom-6 right-6 z-[99999] flex items-center gap-3 px-4 py-3 rounded-xl shadow-2xl border backdrop-blur-md bg-background/95"
      >
        <!-- 动画图标 -->
        <div class="relative">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            class="h-5 w-5 text-blue-500"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <circle cx="12" cy="12" r="10" />
            <polyline points="12 6 12 12 16 14" />
          </svg>
          <div class="absolute inset-0 rounded-full bg-blue-500/20 animate-ping" />
        </div>

        <!-- 文字 -->
        <div class="flex flex-col">
          <span class="text-xs text-muted-foreground">剪贴板自动清除</span>
          <span class="text-sm font-medium">
            <span class="text-blue-500 font-mono tabular-nums">{{ remaining }}</span>
            <span class="text-muted-foreground ml-1">秒后</span>
          </span>
        </div>

        <!-- 分隔线 -->
        <div class="w-px h-8 bg-border" />

        <!-- 取消按钮 -->
        <button
          class="text-xs text-muted-foreground hover:text-foreground transition-colors px-2 py-1 rounded hover:bg-muted"
          @click="cancel"
        >
          取消
        </button>
      </div>
    </Transition>
  </Teleport>
</template>
