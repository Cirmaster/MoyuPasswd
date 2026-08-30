/**
 * useCountdown.ts - 全局倒计时管理 Composable
 *
 * 管理剪贴板清除倒计时，在光标旁边显示。
 */

import { ref } from 'vue'

/** 是否显示倒计时 */
const showCountdown = ref(false)

/** 倒计时秒数 */
const countdownSeconds = ref(10)

/** 显示位置 X */
const countdownX = ref(0)

/** 显示位置 Y */
const countdownY = ref(0)

/** 清除计时器 */
let clearTimer: ReturnType<typeof setTimeout> | null = null

/**
 * 全局倒计时管理
 */
export function useCountdown() {
  /**
   * 启动倒计时（在当前光标位置显示）
   * @param seconds - 倒计时秒数
   */
  const startCountdown = (seconds: number = 10) => {
    // 清除之前的计时器
    if (clearTimer) {
      clearTimeout(clearTimer)
      clearTimer = null
    }

    // 获取当前鼠标位置（如果没有鼠标位置，用屏幕中央）
    countdownX.value = window.innerWidth / 2
    countdownY.value = window.innerHeight / 2

    countdownSeconds.value = seconds
    showCountdown.value = true

    // 设置自动清除计时器
    clearTimer = setTimeout(async () => {
      await clearClipboard()
      showCountdown.value = false
    }, seconds * 1000)
  }

  /**
   * 启动倒计时（在指定位置显示）
   * @param x - X 坐标
   * @param y - Y 坐标
   * @param seconds - 倒计时秒数
   */
  const startCountdownAt = (x: number, y: number, seconds: number = 10) => {
    // 清除之前的计时器
    if (clearTimer) {
      clearTimeout(clearTimer)
      clearTimer = null
    }

    countdownX.value = x
    countdownY.value = y
    countdownSeconds.value = seconds
    showCountdown.value = true

    // 设置自动清除计时器
    clearTimer = setTimeout(async () => {
      await clearClipboard()
      showCountdown.value = false
    }, seconds * 1000)
  }

  /**
   * 取消倒计时
   */
  const cancelCountdown = () => {
    showCountdown.value = false

    if (clearTimer) {
      clearTimeout(clearTimer)
      clearTimer = null
    }
  }

  /**
   * 清除剪贴板
   */
  const clearClipboard = async () => {
    try {
      await navigator.clipboard.writeText('')
    } catch {
      // 忽略错误
    }
  }

  /**
   * 倒计时结束回调
   */
  const onCountdownEnd = async () => {
    await clearClipboard()
    showCountdown.value = false
  }

  return {
    showCountdown,
    countdownSeconds,
    countdownX,
    countdownY,
    startCountdown,
    startCountdownAt,
    cancelCountdown,
    onCountdownEnd,
  }
}
