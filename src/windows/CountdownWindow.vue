<!--
  CountdownWindow.vue - 全局倒计时悬浮窗口

  在鼠标位置显示的倒计时，用于剪贴板清除。
  点击数字取消倒计时并关闭窗口。
  后端命中终端注入黑名单时发 terminal-inject-confirm 事件，
  本窗口扩成提示条告知用户「再按 Ctrl+V 确认」。
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { disableContextMenu } from '@/lib/windowSetup'

/** 剩余秒数 */
const remaining = ref(10)

/** 终端二次确认提示（命中注入黑名单时后端触发） */
const confirmHint = ref(false)

/** 计时器 */
let timer: ReturnType<typeof setInterval> | null = null

/** 提示条自动收起计时器 */
let hintTimer: ReturnType<typeof setTimeout> | null = null

/** 圆点原始尺寸（提示条收起时恢复） */
const DOT_SIZE = { width: 16, height: 16 }

/** 提示条尺寸 */
const HINT_SIZE = { width: 200, height: 24 }

/**
 * 调整窗口尺寸（圆点 ⇄ 提示条）
 */
const resizeWindow = async (size: { width: number; height: number }) => {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    const { LogicalSize } = await import('@tauri-apps/api/dpi')
    await getCurrentWindow().setSize(new LogicalSize(size.width, size.height))
  } catch (e) {
    console.warn('调整窗口尺寸失败:', e)
  }
}

/**
 * 显示终端二次确认提示条，5 秒后自动收起（与后端确认窗口 TTL 一致）
 */
const showConfirmHint = async () => {
  confirmHint.value = true
  await resizeWindow(HINT_SIZE)

  if (hintTimer) clearTimeout(hintTimer)
  hintTimer = setTimeout(() => {
    hideConfirmHint()
  }, 5000)
}

/**
 * 收起提示条并恢复圆点尺寸
 */
const hideConfirmHint = async () => {
  if (hintTimer) {
    clearTimeout(hintTimer)
    hintTimer = null
  }
  if (confirmHint.value) {
    confirmHint.value = false
    await resizeWindow(DOT_SIZE)
  }
}

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

  // 每次显示强制复位为圆点尺寸：治愈任何卡住的窗口尺寸
  // （提示条扩窗后复位失败等场景；圆点内容钉左上角，尺寸对了就贴着光标）
  hideConfirmHint()
  resizeWindow(DOT_SIZE)

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
  // 禁用浏览器默认右键菜单（入口已拦一道；组件内再拦，保证热替换后也生效）
  disableContextMenu()

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
      hideConfirmHint()
      startCountdown(event.payload.seconds || 10)
    })

    // 监听终端注入二次确认提示（命中黑名单时后端触发）
    await listen<{ process: string }>('terminal-inject-confirm', (event) => {
      console.log('Received terminal-inject-confirm event:', event.payload)
      showConfirmHint()
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
  if (hintTimer) {
    clearTimeout(hintTimer)
    hintTimer = null
  }
})
</script>

<template>
  <div class="countdown-container" @click="handleClick">
    <!-- 终端二次确认提示条 -->
    <div v-if="confirmHint" class="confirm-hint">
      目标是终端，再按 Ctrl+V 确认
    </div>
    <!-- 默认倒计时圆点 -->
    <div v-else class="countdown-circle">
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
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: transparent;
  -webkit-app-region: no-drag;
}
</style>

<style scoped>
.countdown-container {
  width: 100%;
  height: 100%;
  display: flex;
  /* 钉在窗口左上角：光标跟随锚定的是窗口左上角（cursor+15px），
     内容居中会在窗口大于 16×16 时（如提示条复位失败）让圆点飘离光标 */
  align-items: flex-start;
  justify-content: flex-start;
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

.confirm-hint {
  height: 24px;
  padding: 0 10px;
  border-radius: 12px;
  background: rgba(217, 119, 6, 0.95);
  color: #ffffff;
  font-size: 11px;
  font-weight: 600;
  line-height: 24px;
  white-space: nowrap;
  overflow: hidden;
}
</style>
