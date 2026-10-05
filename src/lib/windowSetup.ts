/**
 * windowSetup.ts - 窗口通用环境设置
 *
 * 各窗口入口（主窗/快速搜索/倒计时）在启动时调用。
 */

/**
 * 禁用浏览器默认右键菜单
 *
 * WebView2 会弹出「后退/刷新/复制」等浏览器菜单，桌面应用不需要，
 * 全局拦截 contextmenu 事件屏蔽。输入框内同样禁用（复制粘贴用快捷键）。
 */
export function disableContextMenu() {
  document.addEventListener('contextmenu', (e) => e.preventDefault())
}
