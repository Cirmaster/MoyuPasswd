/**
 * useAutoLock.ts - 用户活动上报
 *
 * 自动锁定由后端空闲检测统一负责（双条件取先到：系统空闲 / 应用内无操作，
 * 见 idle.rs），前端不自行计时锁定，只负责把应用内的用户活动上报给后端
 * （`report_activity`），作为「应用内无操作」条件的判定依据。
 *
 * @example
 * ```ts
 * useAutoLock() // 挂载即开始活动上报
 * ```
 */

import { onMounted, onUnmounted } from 'vue'
import { invoke } from '@/lib/api'

/** 是否正在监听 */
let isTracking = false

/** 上次上报时间（节流用） */
let lastReport = 0

/** 上报节流间隔（毫秒） */
const REPORT_THROTTLE_MS = 2000

/**
 * 自动锁定 Hook（仅活动上报，锁定由后端负责）
 */
export function useAutoLock() {
  /**
   * 用户活动事件处理函数
   * 通知后端更新活动时间（节流，避免鼠标移动刷 IPC）
   */
  const handleActivity = () => {
    const now = Date.now()
    if (now - lastReport < REPORT_THROTTLE_MS) return
    lastReport = now
    invoke('report_activity').catch(() => {
      // 忽略错误，不影响前端功能
    })
  }

  /**
   * 开始监听用户活动
   *
   * keydown 覆盖全部按键（keypress 对功能键/修饰键不触发）；
   * wheel 覆盖滚轮滚动（scroll 事件不冒泡，部分滚动场景收不到）。
   * 漏报活动会导致「人在用却被锁」，事件宁多勿少（节流已兜底）。
   */
  const startTracking = () => {
    if (isTracking) return

    window.addEventListener('mousemove', handleActivity)
    window.addEventListener('mousedown', handleActivity)
    window.addEventListener('keydown', handleActivity)
    window.addEventListener('keypress', handleActivity)
    window.addEventListener('wheel', handleActivity)
    window.addEventListener('touchmove', handleActivity)
    window.addEventListener('scroll', handleActivity)

    isTracking = true
  }

  /**
   * 停止监听用户活动
   */
  const stopTracking = () => {
    if (!isTracking) return

    window.removeEventListener('mousemove', handleActivity)
    window.removeEventListener('mousedown', handleActivity)
    window.removeEventListener('keydown', handleActivity)
    window.removeEventListener('keypress', handleActivity)
    window.removeEventListener('wheel', handleActivity)
    window.removeEventListener('touchmove', handleActivity)
    window.removeEventListener('scroll', handleActivity)

    isTracking = false
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
  }
}
