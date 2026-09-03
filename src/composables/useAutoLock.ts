/**
 * useAutoLock.ts - 自动锁定 Composable
 *
 * 监听用户活动，无操作一段时间后自动锁定应用。
 *
 * @example
 * ```ts
 * const { startTracking, stopTracking, resetTimer } = useAutoLock()
 *
 * // 开始监听
 * startTracking()
 *
 * // 停止监听
 * stopTracking()
 *
 * // 重置计时器（用户有操作时调用）
 * resetTimer()
 * ```
 */

import { ref, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

/** 自动锁定时间（分钟） */
const lockTimeoutMinutes = ref(5)

/** 锁定计时器 ID */
let lockTimer: ReturnType<typeof setTimeout> | null = null

/** 是否正在监听 */
let isTracking = false

/**
 * 自动锁定 Hook
 */
export function useAutoLock() {

  /**
   * 锁定应用
   * 只需调用后端命令，后端负责清除密钥并通知所有窗口
   */
  const lockApp = async () => {
    try {
      await invoke('lock_app')
    } catch (e) {
      console.error('锁定失败:', e)
    }
  }

  /**
   * 重置锁定计时器
   * 用户有操作时调用
   */
  const resetTimer = () => {
    if (lockTimer) {
      clearTimeout(lockTimer)
    }

    // 设置新的计时器
    lockTimer = setTimeout(() => {
      lockApp()
    }, lockTimeoutMinutes.value * 60 * 1000)
  }

  /**
   * 用户活动事件处理函数
   */
  const handleActivity = () => {
    resetTimer()
  }

  /**
   * 开始监听用户活动
   */
  const startTracking = () => {
    if (isTracking) return

    // 监听各种用户活动
    window.addEventListener('mousemove', handleActivity)
    window.addEventListener('mousedown', handleActivity)
    window.addEventListener('keypress', handleActivity)
    window.addEventListener('touchmove', handleActivity)
    window.addEventListener('scroll', handleActivity)

    isTracking = true

    // 启动计时器
    resetTimer()
  }

  /**
   * 停止监听用户活动
   */
  const stopTracking = () => {
    if (!isTracking) return

    window.removeEventListener('mousemove', handleActivity)
    window.removeEventListener('mousedown', handleActivity)
    window.removeEventListener('keypress', handleActivity)
    window.removeEventListener('touchmove', handleActivity)
    window.removeEventListener('scroll', handleActivity)

    if (lockTimer) {
      clearTimeout(lockTimer)
      lockTimer = null
    }

    isTracking = false
  }

  /**
   * 设置锁定时间
   * @param minutes - 锁定时间（分钟）
   */
  const setLockTimeout = (minutes: number) => {
    lockTimeoutMinutes.value = minutes
    // 重置计时器
    if (isTracking) {
      resetTimer()
    }
  }

  // 组件挂载时开始监听
  onMounted(() => {
    startTracking()
  })

  // 组件卸载时停止监听
  onUnmounted(() => {
    stopTracking()
  })

  return {
    startTracking,
    stopTracking,
    resetTimer,
    setLockTimeout,
    lockTimeoutMinutes,
  }
}
