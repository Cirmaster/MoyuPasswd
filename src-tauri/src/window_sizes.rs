//! 窗口「设计尺寸」常量的单一来源
//!
//! 这里的数值全部是**设计尺寸**（CSS px，即 webview 布局视口的逻辑像素），
//! 不是屏幕物理像素。窗口的实际物理尺寸由 `scale.rs` 按渲染缩放换算：
//!
//! ```text
//! 窗口物理尺寸 = 设计尺寸 × 渲染缩放 R
//! ```
//!
//! # 约定（长期维护必读）
//!
//! 1. **禁止在别处散落裸数值**：任何新窗口、弹窗的尺寸/定位偏移都必须
//!    在此定义命名常量，并经 `scale.rs` 的换算入口应用到窗口。
//! 2. `tauri.conf.json` 里的 width/height/minWidth/minHeight 只是启动占位
//!    初值（conf 格式无法表达换算），setup 阶段会用本文件常量 × R 覆盖。
//! 3. 前端 CSS 里写死 px 是**允许且推荐**的：CSS px 即设计尺寸，视口恒等于
//!    设计尺寸（见 scale.rs 的缩放模型），写死即安全、即还原设计稿。

/// 主窗口设计尺寸（与 tauri.conf.json 占位初值一致，启动时按 R 覆盖）
pub const MAIN_WIDTH: f64 = 900.0;
pub const MAIN_HEIGHT: f64 = 650.0;

/// 主窗口最小设计尺寸（对应 tauri.conf.json minWidth/minHeight）
pub const MAIN_MIN_WIDTH: f64 = 780.0;
pub const MAIN_MIN_HEIGHT: f64 = 560.0;

/// 快速搜索窗口设计尺寸（commands/shortcuts.rs 建窗）
pub const QUICK_SEARCH_WIDTH: f64 = 600.0;
pub const QUICK_SEARCH_HEIGHT: f64 = 450.0;

/// 倒计时气泡「圆点」态设计尺寸（前端 CSS 的 .countdown-circle 同为 16px，两处需一致）
pub const COUNTDOWN_DOT_WIDTH: f64 = 16.0;
pub const COUNTDOWN_DOT_HEIGHT: f64 = 16.0;

/// 倒计时气泡「终端二次确认提示条」态设计尺寸
///（前端 CSS 的 .confirm-hint 同为 24px 高，两处需一致）
pub const COUNTDOWN_HINT_WIDTH: f64 = 200.0;
pub const COUNTDOWN_HINT_HEIGHT: f64 = 24.0;

/// 倒计时气泡窗口设计尺寸上限（提示条态的放宽空间）
pub const COUNTDOWN_MAX_WIDTH: f64 = 320.0;
pub const COUNTDOWN_MAX_HEIGHT: f64 = 48.0;

/// 光标跟随锚点偏移（设计 px）：气泡左上角相对光标位置的右下偏移
pub const CURSOR_FOLLOW_OFFSET: f64 = 15.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// 常量自洽：提示条不得超过上限（否则高分屏换算后会被窗口上限裁剪）
    #[test]
    fn hint_within_max() {
        assert!(COUNTDOWN_HINT_WIDTH <= COUNTDOWN_MAX_WIDTH);
        assert!(COUNTDOWN_HINT_HEIGHT <= COUNTDOWN_MAX_HEIGHT);
    }

    /// 常量自洽：圆点不得超过上限
    #[test]
    fn dot_within_max() {
        assert!(COUNTDOWN_DOT_WIDTH <= COUNTDOWN_MAX_WIDTH);
        assert!(COUNTDOWN_DOT_HEIGHT <= COUNTDOWN_MAX_HEIGHT);
    }
}
