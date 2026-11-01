/**
 * renderScale.ts - 渲染缩放自检上报
 *
 * 窗口缩放模型（见 src-tauri/src/scale.rs）的前端配合方。
 *
 * 后端按「窗口几何缩放 × gtk-xft-dpi 文字 DPI 缩放」推断 webview 渲染缩放
 * R（devicePixelRatio），本模块在窗口启动时把实测值上报后端自检：
 * 推断与实测不符时后端按实测校正并重排所有窗口（安全网，覆盖
 * Wayland 分数缩放等推断可能失准的环境）。
 *
 * 同时监听 devicePixelRatio 运行时变化（换显示器 / 改系统缩放），
 * 变化后重新上报，后端随之重排窗口尺寸。
 *
 * 三个窗口入口（main / quick-search / countdown）启动时各调用一次。
 */

/** 是否已武装（幂等：一个 webview 只需一次） */
let armed = false

/** 上报一次当前渲染缩放（devicePixelRatio 实测值） */
async function report(): Promise<void> {
  try {
    const { invoke } = await import('@/lib/api')
    await invoke('sync_render_scale', { scale: window.devicePixelRatio })
  } catch (e) {
    // 自检失败不影响窗口启动（后端仍有推断值兜底）
    console.warn('渲染缩放上报失败:', e)
  }
}

/**
 * 监听 devicePixelRatio 变化
 *
 * matchMedia 的 resolution 查询绑定的是具体分辨率，触发后失效，
 * 需重新武装下一次变化。
 */
function armDprWatcher(): void {
  const mq = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`)
  const onChange = (): void => {
    mq.removeEventListener('change', onChange)
    void report()
    armDprWatcher()
  }
  mq.addEventListener('change', onChange)
}

/** 启动自检：上报渲染缩放并监听变化（幂等，可重复调用） */
export function syncRenderScale(): void {
  if (armed) return
  armed = true
  void report()
  armDprWatcher()
}
