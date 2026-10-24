/**
 * useWebviewHeartbeat.ts - webview 心跳
 *
 * 主线程每 5 秒向 Rust 上报一次心跳。Rust 侧 watchdog 据此判断网页是否存活：
 * 心跳停止（崩溃/卡死）超 15 秒会自动重载主窗口网页，多数渲染崩溃自动消化。
 *
 * 特点：
 * - 页面崩溃/卡死后定时器自然停止 → 心跳停止，正好触发自愈，无需崩溃事件 API
 * - 心跳失败静默（解锁前后、窗口隐藏时都可能短暂失败，不影响主流程）
 * - 全窗口生命周期只启动一次（App.vue 挂载时调用）
 */

import { invoke } from '@/lib/api'

/** 心跳上报间隔（毫秒），需小于 Rust 侧的判定阈值 15 秒 */
const HEARTBEAT_INTERVAL_MS = 5_000

/**
 * 启动 webview 心跳（幂等性由调用方保证：App 挂载时调用一次）
 */
export function startWebviewHeartbeat() {
  const beat = () => {
    invoke('webview_heartbeat').catch(() => {
      // 心跳失败静默：看门狗以「超时」为判定依据，偶发失败无影响
    })
  }

  setInterval(() => {
    // 隐藏/最小化时不上报：WebView2 会节流定时器、看门狗对隐藏窗口也不检查，
    // 省掉无谓 IPC；恢复可见的瞬间由 visibilitychange 立刻补一发
    if (document.visibilityState === 'hidden') return
    beat()
  }, HEARTBEAT_INTERVAL_MS)

  // 恢复可见时立即上报：把「后台被节流」造成的心跳空窗立刻补上
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') beat()
  })
}
