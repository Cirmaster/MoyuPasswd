<!--
  CountdownWindow.vue - 全局倒计时悬浮窗口

  在鼠标位置显示的倒计时，用于剪贴板清除。
  点击数字取消倒计时并关闭窗口。
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'

/** 剩余秒数 */
const remaining = ref(10)

/** 计时器 */
let timer: ReturnType<typeof setInterval> | null = null

/**
 * 开始倒计时
 */
const startCountdown = (seconds: number = 10) => {
  console.log('Starting countdown with', seconds, 'seconds')

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
      // 剪贴板清除由后端在复制时调度（copy_text_to_clipboard → schedule_clear），
      // 不依赖本窗口存活，前端只需关闭窗口
      closeWindow()
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
    // 调用后端原生清空剪贴板（不向历史追加空记录）
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('clear_clipboard')
    console.log('Clipboard cleared')
  } catch (e) {
    console.error('清除剪贴板失败:', e)
  }
  closeWindow()
}

/**
 * 关闭窗口
 */
const closeWindow = async () => {
  console.log('closeWindow called')

  // 先停止光标跟随（不等待结果）
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    invoke('stop_follow_cursor').catch(err => console.warn('stop_follow_cursor failed:', err))
  } catch (e) {
    console.warn('Failed to invoke stop_follow_cursor:', e)
  }

  // 关闭窗口（必须执行）
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    const win = getCurrentWindow()
    console.log('Closing window...')
    await win.close()
    console.log('Window closed')
  } catch (e) {
    console.error('Failed to close window:', e)
    // 尝试隐藏
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      await getCurrentWindow().hide()
    } catch (e2) {
      console.error('Failed to hide window:', e2)
    }
  }
}

/**
 * 点击数字取消倒计时并关闭
 */
const handleClick = async () => {
  console.log('Countdown clicked, clearing clipboard and closing...')
  stopCountdown()
  await clearClipboard()
}

/**
 * 初始化
 */
onMounted(async () => {
  try {
    // 动态导入 Tauri API
    const { invoke } = await import('@tauri-apps/api/core')
    const { listen } = await import('@tauri-apps/api/event')

    // 从后端获取倒计时秒数并启动
    const seconds = await invoke<number>('get_countdown_seconds')
    console.log('Got countdown seconds from backend:', seconds)
    startCountdown(seconds)

    // 监听重置倒计时事件
    await listen<{ seconds: number }>('reset-countdown', (event) => {
      console.log('Received reset-countdown event:', event.payload)
      startCountdown(event.payload.seconds || 10)
    })

    // 监听取消倒计时事件
    await listen('cancel-countdown', () => {
      console.log('Received cancel-countdown event')
      stopCountdown()
      closeWindow()
    })

    // 后端已完成清除（兜底），前端只需关闭窗口
    await listen('clipboard-cleared', () => {
      console.log('Received clipboard-cleared event')
      stopCountdown()
      closeWindow()
    })

    console.log('Countdown window initialized')
  } catch (e) {
    console.error('Failed to initialize countdown window:', e)
  }
})

onUnmounted(() => {
  stopCountdown()
})
</script>

<template>
  <div class="countdown-container" @click="handleClick">
    <div class="countdown-circle">
      <span class="countdown-number">{{ remaining }}</span>
    </div>
  </div>
</template>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body {
  width: 16px;
  height: 16px;
  overflow: hidden;
  background: transparent;
  -webkit-app-region: no-drag;
}
</style>

<style scoped>
.countdown-container {
  width: 16px;
  height: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  cursor: pointer;
  user-select: none;
}

.countdown-circle {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: rgba(37, 99, 235, 0.9);
  display: flex;
  align-items: center;
  justify-content: center;
  transition: transform 0.15s ease, background 0.15s ease;
}

.countdown-circle:hover {
  transform: scale(1.2);
  background: rgba(59, 130, 246, 0.95);
}

.countdown-number {
  font-size: 10px;
  font-weight: 700;
  color: #ffffff;
  font-family: 'SF Mono', 'Fira Code', monospace;
}
</style>
