<!--
  CountdownWindow.vue - 倒计时悬浮窗口

  只显示倒计时数字，点击数字关闭窗口。
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'

/** 剩余秒数 */
const remaining = ref(30)

/** 计时器 */
let timer: ReturnType<typeof setInterval> | null = null

/**
 * 开始倒计时
 */
const startCountdown = (seconds: number = 30) => {
  // 先停止之前的计时器
  if (timer) {
    clearInterval(timer)
    timer = null
  }

  remaining.value = seconds

  timer = setInterval(() => {
    remaining.value--

    if (remaining.value <= 0) {
      stopCountdown()
      clearClipboard()
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
 * 清除剪贴板并关闭
 */
const clearClipboard = async () => {
  try {
    await navigator.clipboard.writeText('')
  } catch {
    // 忽略错误
  }
  getCurrentWindow().close()
}

/**
 * 点击数字取消倒计时并关闭
 */
const handleClick = () => {
  stopCountdown()
  getCurrentWindow().close()
}

/**
 * 初始化
 */
onMounted(async () => {
  // 监听启动倒计时事件
  await listen<{ seconds: number }>('start-countdown', (event) => {
    startCountdown(event.payload.seconds || 30)
  })

  // 监听取消倒计时事件
  await listen('cancel-countdown', () => {
    stopCountdown()
    getCurrentWindow().close()
  })
})

onUnmounted(() => {
  stopCountdown()
})
</script>

<template>
  <div class="countdown-container" @click="handleClick">
    <span class="countdown-number">{{ remaining }}</span>
  </div>
</template>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body {
  overflow: hidden;
  background: transparent;
}
</style>

<style scoped>
.countdown-container {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(30, 30, 30, 0.9);
  border-radius: 6px;
  cursor: pointer;
  user-select: none;
}

.countdown-container:hover {
  background: rgba(50, 50, 50, 0.95);
}

.countdown-number {
  font-size: 20px;
  font-weight: bold;
  color: #60a5fa;
  font-family: monospace;
}
</style>
