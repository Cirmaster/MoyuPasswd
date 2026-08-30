<!--
  CaretCountdown.vue - 光标旁倒计时组件

  显示在光标旁边的倒计时，用于剪贴板清除。
  粘贴完成后或倒计时归0时自动关闭。
-->

<script setup lang="ts">
import { ref, watch, onUnmounted, nextTick } from 'vue'

/**
 * 组件 Props
 */
interface Props {
  /** 是否显示 */
  show: boolean
  /** 倒计时秒数 */
  seconds: number
  /** 显示位置 X */
  x: number
  /** 显示位置 Y */
  y: number
}

const props = withDefaults(defineProps<Props>(), {
  seconds: 10,
  x: 0,
  y: 0,
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

  if (timer) {
    clearInterval(timer)
  }

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
      enter-active-class="transition-all duration-200 ease-out"
      enter-from-class="opacity-0 scale-75"
      enter-to-class="opacity-100 scale-100"
      leave-active-class="transition-all duration-150 ease-in"
      leave-from-class="opacity-100 scale-100"
      leave-to-class="opacity-0 scale-75"
    >
      <div
        v-if="show"
        class="fixed z-[99999] pointer-events-none"
        :style="{
          left: `${x + 15}px`,
          top: `${y - 30}px`,
        }"
      >
        <div class="flex items-center justify-center w-8 h-8 rounded-full bg-blue-600/90 shadow-lg">
          <span class="text-xs font-bold text-white font-mono">{{ remaining }}</span>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
