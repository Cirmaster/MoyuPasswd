/**
 * dragGhost.ts - 拖拽幽灵（drag image）定位
 *
 * HTML5 默认把拖拽图像的**左上角**对齐指针，图像会盖住光标正下方，
 * 用户看不清目标输入框的插入点。`dataTransfer.setDragImage(el, x, y)` 里的
 * `(x, y)` 是「图像内哪个点对准指针」，给 `(0, 0)` 就是让图像从指针处
 * 向右下展开——幽灵在光标右下，不挡光标。
 */

/** 幽灵元素的通用样式：小卡片，贴近主题的深色底 */
const GHOST_STYLE: Partial<CSSStyleDeclaration> = {
  position: 'fixed',
  // 放进视口坐标系但移出可视区：WebKit 对完全离屏（display:none / top:-9999px）
  // 的元素可能拍不出快照，用 transform 位移最稳
  left: '0',
  top: '0',
  transform: 'translate3d(-200vw, -200vh, 0)',
  padding: '4px 10px',
  borderRadius: '6px',
  background: 'rgba(24, 24, 28, 0.92)',
  color: '#f2f2f5',
  font: '12px system-ui, -apple-system, "Noto Sans CJK SC", sans-serif',
  whiteSpace: 'nowrap',
  maxWidth: '220px',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  border: '1px solid rgba(255, 255, 255, 0.14)',
  pointerEvents: 'none',
  zIndex: '2147483647',
}

/**
 * 给当前拖拽设置一个位于光标右下方的幽灵
 *
 * 必须在 `dragstart` 里同步调用（`setDragImage` 只在事件处理期间有效）。
 *
 * @param e - dragstart 事件
 * @param text - 幽灵上显示的文本（标题这类非敏感信息，**不要传密码**）
 */
export function setDragGhost(e: DragEvent, text: string) {
  if (!e.dataTransfer) return

  const ghost = document.createElement('div')
  ghost.textContent = text
  Object.assign(ghost.style, GHOST_STYLE)
  document.body.appendChild(ghost)

  // 热点放在左上角 ⇒ 图像整体出现在光标右下方
  e.dataTransfer.setDragImage(ghost, 0, 0)

  // setDragImage 会立刻对元素拍快照，随后即可移除；放到下一个宏任务更稳妥
  setTimeout(() => ghost.remove(), 0)
}
