/**
 * useClipboard.ts - 剪贴板管理 Composable
 *
 * 提供复制到剪贴板和自动清除功能。
 * 支持显示倒计时。
 *
 * @example
 * ```ts
 * const { copyToClipboard, showCountdown, countdownSeconds } = useClipboard()
 *
 * // 复制文本，30秒后自动清除
 * await copyToClipboard('password123')
 * ```
 */

import { ref } from 'vue'

/** 剪贴板清除时间（秒） */
const clearTimeSeconds = ref(30)

/** 清除计时器 */
let clearTimer: ReturnType<typeof setTimeout> | null = null

/** 是否显示倒计时 */
const showCountdown = ref(false)

/**
 * 剪贴板管理 Hook
 */
export function useClipboard() {
  /**
   * 复制文本到剪贴板
   * @param text - 要复制的文本
   * @param showToast - 显示提示的回调函数
   * @returns 是否复制成功
   */
  const copyToClipboard = async (
    text: string,
    showToast?: (type: 'success' | 'error', message: string) => void,
  ): Promise<boolean> => {
    try {
      await navigator.clipboard.writeText(text)

      if (showToast) {
        showToast('success', '已复制到剪贴板')
      }

      // 显示倒计时
      showCountdown.value = true

      // 设置自动清除
      scheduleClear()

      return true
    } catch {
      if (showToast) {
        showToast('error', '复制失败')
      }
      return false
    }
  }

  /**
   * 计划清除剪贴板
   */
  const scheduleClear = () => {
    // 清除之前的计时器
    if (clearTimer) {
      clearTimeout(clearTimer)
    }

    // 设置新的计时器
    clearTimer = setTimeout(async () => {
      await clearClipboard()
    }, clearTimeSeconds.value * 1000)
  }

  /**
   * 设置剪贴板清除时间
   * @param seconds - 清除时间（秒）
   */
  const setClearTime = (seconds: number) => {
    clearTimeSeconds.value = seconds
  }

  /**
   * 立即清除剪贴板
   */
  const clearClipboard = async () => {
    try {
      // 只清空当前内容，不清除历史记录
      await navigator.clipboard.writeText('')
      console.log('剪贴板已清除')
    } catch {
      console.error('清除剪贴板失败')
    }

    // 隐藏倒计时
    showCountdown.value = false

    // 清除计时器
    if (clearTimer) {
      clearTimeout(clearTimer)
      clearTimer = null
    }
  }

  /**
   * 取消清除（用户点击取消）
   */
  const cancelClear = () => {
    showCountdown.value = false

    if (clearTimer) {
      clearTimeout(clearTimer)
      clearTimer = null
    }
  }

  return {
    copyToClipboard,
    setClearTime,
    clearClipboard,
    cancelClear,
    showCountdown,
    clearTimeSeconds,
  }
}
