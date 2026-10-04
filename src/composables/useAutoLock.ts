/**
 * useAutoLock.ts - 用户活动上报
 *
 * 自动锁定由后端空闲检测统一负责（以系统空闲为准，见 idle.rs），
 * 前端不再各自计时锁定——此前前端只看应用内事件、后端取系统与应用空闲的较小值，
 * 两套语义互相矛盾。本 composable 只负责把前端活动上报给后端
 * （非 Windows 平台的空闲判定依据）。
 *
 * @example
 * ```ts
 * useAutoLock() // 挂载即开始活动上报
 * ```
 */

import { onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

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
   */
  const startTracking = () => {
    if (isTracking) return

    window.addEventListener('mousemove', handleActivity)
    window.addEventListener('mousedown', handleActivity)
    window.addEventListener('keypress', handleActivity)
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
    window.removeEventListener('keypress', handleActivity)
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
