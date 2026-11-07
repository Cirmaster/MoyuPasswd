//! 窗口缩放换算模块 —— 「设计尺寸」↔「物理尺寸」的唯一换算入口
//!
//! # 缩放模型
//!
//! 症状（Linux 高分屏下窗口内容拥挤）的根因是两层缩放不同步：
//! 窗口几何层（tao/GTK）按 1x 开窗，渲染层（WebKitGTK）按 1.5x 作画，
//! 布局视口被压缩到 2/3。Windows 上 WebView2 与窗口几何共用系统缩放，
//! 天然同步，所以从未暴露。本模块把真源定死，任何平台恒满足：
//!
//! ```text
//! 窗口物理尺寸 = 设计尺寸 × 渲染缩放 R
//! CSS 布局视口 = 物理 ÷ R ≡ 设计尺寸        （不变量）
//! ```
//!
//! R 即 webview 的 `devicePixelRatio`。按 WebKitGTK 2.54 实测定律：
//!
//! ```text
//! R = 窗口几何缩放（tao/GDK scale factor） × 文字 DPI 缩放
//! 文字 DPI 缩放 = gtk-xft-dpi ÷ 1024 ÷ 96   （≤0 时按 96 即 1.0）
//! ```
//!
//! Windows / macOS 无独立文字 DPI 缩放（恒 1.0），R = 系统缩放因子，
//! 换算退化为恒等变换，行为与旧版完全一致。
//!
//! 前端启动时经 [`sync_render_scale`] 上报 `devicePixelRatio` 实测值做
//! 自检（安全网）：推断与实测不符时按实测校正文字缩放并重排所有窗口
//! （覆盖 Wayland 分数缩放等推断可能失准的环境）。
//!
//! # 用法约束（长期维护必读）
//!
//! - 窗口尺寸/定位涉及的换算**只允许**走本模块函数，禁止散落裸数值；
//!   设计尺寸常量统一定义在 [`crate::window_sizes`]。
//! - 屏幕坐标约定：光标查询（GetCursorPos / X11 query_pointer）返回**物理
//!   坐标**，直接用；前端 `event.screenX/screenY` 是 **CSS 逻辑坐标**，
//!   传入命令后由本模块乘 R 换算为物理坐标。

use crate::response::{respond, ApiResult};
use crate::window_sizes as ws;
use parking_lot::Mutex;
use std::collections::HashMap;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize};

/// 文字 DPI 缩放修正系数（前端实测自检校正；1.0 = 信任推断值）
static TEXT_SCALE_CORRECTION: Mutex<f64> = Mutex::new(1.0);

/// 窗口设计尺寸登记项（渲染缩放变化时按登记值重排窗口）
struct WindowDesign {
    /// 当前设计尺寸（随 UI 状态变化，如倒计时圆点 ⇄ 提示条）
    size: (f64, f64),
    /// 最小设计尺寸（None = 不约束）
    min: Option<(f64, f64)>,
    /// 最大设计尺寸（None = 不约束）
    max: Option<(f64, f64)>,
}

/// 设计尺寸登记表：label → 登记项（仅三个窗口，读写频率极低）
static DESIGN_REGISTRY: Mutex<Option<HashMap<&'static str, WindowDesign>>> = Mutex::new(None);

// ==================== 纯换算（可单测） ====================

/// 设计长度 → 物理像素长度（四舍五入，避免累积 1px 误差）
fn to_physical_len(design: f64, scale: f64) -> u32 {
    (design * scale).round().max(0.0) as u32
}

/// 设计坐标/偏移 → 物理像素坐标（四舍五入）
fn to_physical_pos(design: f64, scale: f64) -> i32 {
    (design * scale).round() as i32
}

// ==================== 渲染缩放 R ====================

/// 文字 DPI 缩放的推断基线（gtk-xft-dpi ÷ 1024 ÷ 96，≤0 按 1.0）
///
/// **只在主线程刷新**（见 [`refresh_text_scale_base`]），其余线程只读缓存：
/// `gtk::Settings::default()` 带 GTK 主线程断言，在 tokio 命令线程（如
/// async 的 `show_countdown`）/光标跟随线程里调用会 panic
/// （"GTK may only be used from the main thread."），故热路径绝不碰 GTK。
static TEXT_SCALE_BASE: Mutex<f64> = Mutex::new(1.0);

/// 从 GTK 读取文字 DPI 缩放基线写入缓存（**仅主线程调用**）
///
/// 刷新时机：启动 setup（scale::init）、前端 sync_render_scale 自检、
/// gtk-xft-dpi 变化回调——三者都在主线程。
#[cfg(target_os = "linux")]
fn refresh_text_scale_base() {
    use gtk::prelude::ObjectExt as _;
    let dpi: i32 = gtk::Settings::default()
        .and_then(|s| s.property_value("gtk-xft-dpi").get::<i32>().ok())
        .unwrap_or(-1);
    let inferred = if dpi <= 0 {
        1.0
    } else {
        dpi as f64 / 1024.0 / 96.0
    };
    *TEXT_SCALE_BASE.lock() = inferred;
}

#[cfg(not(target_os = "linux"))]
fn refresh_text_scale_base() {
    // 非 Linux 无独立文字 DPI 缩放，基线恒 1.0
}

/// 文字 DPI 缩放（含前端校正系数）——任何线程可调用（只读缓存，不碰 GTK）
#[cfg(target_os = "linux")]
fn text_scale() -> f64 {
    (*TEXT_SCALE_BASE.lock()) * (*TEXT_SCALE_CORRECTION.lock())
}

#[cfg(not(target_os = "linux"))]
fn text_scale() -> f64 {
    // 无独立文字 DPI 缩放，基线恒 1.0；前端校正系数（安全网）仍生效
    *TEXT_SCALE_CORRECTION.lock()
}

/// 指定窗口的渲染缩放 R = window.scale_factor() × 文字 DPI 缩放
pub fn render_scale(window: &tauri::WebviewWindow) -> f64 {
    let geo = window.scale_factor().unwrap_or(1.0);
    geo * text_scale()
}

/// 任意可用窗口的渲染缩放（建窗前换算初始尺寸/坐标时用；优先主窗口）
///
/// 兜底（理论上到不了——本应用恒有主窗口）：仅文字缩放。
pub fn render_scale_any(app: &AppHandle) -> f64 {
    let window = app
        .get_webview_window("main")
        .or_else(|| app.webview_windows().into_values().next());
    match window {
        Some(w) => w.scale_factor().unwrap_or(1.0) * text_scale(),
        None => text_scale(),
    }
}

// ==================== 设计尺寸 → 窗口属性 ====================

/// 设计尺寸 → 物理尺寸（按指定窗口的渲染缩放换算）
pub fn design_size(window: &tauri::WebviewWindow, w: f64, h: f64) -> PhysicalSize<u32> {
    let r = render_scale(window);
    PhysicalSize::new(to_physical_len(w, r), to_physical_len(h, r))
}

/// 设计尺寸 → tao 逻辑尺寸（**仅供 `WebviewWindowBuilder` 使用**）
///
/// builder 的 `(f64, f64)` 是 tao 逻辑像素——tao 建窗时会再乘几何缩放得到
/// 物理像素。目标物理 = 设计 × R = 设计 × 几何缩放 × 文字 DPI 缩放，
/// 消去几何缩放即得：**逻辑值 = 设计 × 文字 DPI 缩放**（与几何缩放无关）。
///
/// Windows/macOS 文字缩放恒 1.0，本函数退化为恒等变换，行为与旧版一致。
pub fn tao_logical_size(w: f64, h: f64) -> (f64, f64) {
    let t = text_scale();
    (w * t, h * t)
}

/// 设计坐标 → tao 逻辑坐标（builder `.position(x, y)` 使用；语义同 [`tao_logical_size`]）
pub fn tao_logical_pos(x: f64, y: f64) -> (f64, f64) {
    let t = text_scale();
    (x * t, y * t)
}

/// 设计偏移 → 物理偏移（屏幕坐标锚点偏移，如光标跟随 +15）
pub fn design_offset(window: &tauri::WebviewWindow, v: f64) -> i32 {
    to_physical_pos(v, render_scale(window))
}

/// 设计坐标 → 物理坐标（前端传入的 CSS 逻辑屏幕坐标，如 event.screenX）
pub fn design_to_physical_pos(app: &AppHandle, x: f64, y: f64) -> PhysicalPosition<i32> {
    let r = render_scale_any(app);
    PhysicalPosition::new(to_physical_pos(x, r), to_physical_pos(y, r))
}

// ==================== 设计尺寸登记与重排 ====================

/// 登记窗口设计尺寸（幂等覆盖；建窗时调用）
pub fn register_design(
    label: &'static str,
    size: (f64, f64),
    min: Option<(f64, f64)>,
    max: Option<(f64, f64)>,
) {
    let mut reg = DESIGN_REGISTRY.lock();
    reg.get_or_insert_with(HashMap::new).insert(
        label,
        WindowDesign { size, min, max },
    );
}

/// 更新窗口当前设计尺寸（UI 状态变化时调用，如倒计时圆点 ⇄ 提示条）
/// 并立即应用到已存在的窗口
pub fn set_design_size(app: &AppHandle, label: &str, w: f64, h: f64) {
    let entry = {
        let mut reg = DESIGN_REGISTRY.lock();
        reg.get_or_insert_with(HashMap::new)
            .get_mut(label)
            .map(|d| {
                d.size = (w, h);
                (d.size, d.min, d.max)
            })
    };
    if let Some((size, min, max)) = entry {
        if let Some(window) = app.get_webview_window(label) {
            apply_design_to(&window, size, min, max);
        }
    } else {
        log::warn!("set_design_size: 窗口 {label} 未登记设计尺寸，忽略");
    }
}

/// 把登记的设计尺寸换算后应用到单个窗口（size + min/max 约束）
fn apply_design_to(
    window: &tauri::WebviewWindow,
    size: (f64, f64),
    min: Option<(f64, f64)>,
    max: Option<(f64, f64)>,
) {
    let r = render_scale(window);
    let phys = PhysicalSize::new(to_physical_len(size.0, r), to_physical_len(size.1, r));
    if let Err(e) = window.set_size(phys) {
        log::warn!("set_size 失败({}): {:?}", window.label(), e);
    }
    if let Some((w, h)) = min {
        let _ = window.set_min_size(Some(PhysicalSize::new(
            to_physical_len(w, r),
            to_physical_len(h, r),
        )));
    }
    if let Some((w, h)) = max {
        let _ = window.set_max_size(Some(PhysicalSize::new(
            to_physical_len(w, r),
            to_physical_len(h, r),
        )));
    }
}

/// 按登记的设计尺寸重排所有已存在窗口
///
/// 触发时机：启动 setup、ScaleFactorChanged（换显示器/改系统缩放）、
/// gtk-xft-dpi 变化、前端实测校正。
pub fn apply_all(app: &AppHandle) {
    let entries: Vec<(&'static str, (f64, f64), Option<(f64, f64)>, Option<(f64, f64)>)> = {
        let reg = DESIGN_REGISTRY.lock();
        reg.as_ref()
            .map(|m| {
                m.iter()
                    .map(|(k, d)| (*k, d.size, d.min, d.max))
                    .collect()
            })
            .unwrap_or_default()
    };
    for (label, size, min, max) in entries {
        if let Some(window) = app.get_webview_window(label) {
            apply_design_to(&window, size, min, max);
        }
    }
}

// ==================== 启动挂钩 ====================

/// 模块初始化：主窗口登记 + 监听文字 DPI 运行时变化
///
/// 必须在 setup 中、主窗口 `show()` **之前**调用（窗口可见前完成尺寸换算，
/// 用户看不到尺寸跳动）。
pub fn init(app: &AppHandle) {
    // 主线程刷新文字 DPI 基线缓存（此后任何线程只读缓存；见 TEXT_SCALE_BASE）
    refresh_text_scale_base();

    register_design(
        "main",
        (ws::MAIN_WIDTH, ws::MAIN_HEIGHT),
        Some((ws::MAIN_MIN_WIDTH, ws::MAIN_MIN_HEIGHT)),
        None,
    );

    // 文字 DPI 运行时变化（如用户改 Xft.dpi / GTK 字体设置）→ 重排窗口
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::ObjectExt as _;
        if let Some(settings) = gtk::Settings::default() {
            let app_handle = app.clone();
            settings.connect_notify(Some("gtk-xft-dpi"), move |_, _| {
                log::info!("gtk-xft-dpi 变化，按新渲染缩放重排窗口");
                // 回调在主线程：顺带刷新文字 DPI 基线缓存
                refresh_text_scale_base();
                apply_all(&app_handle);
            });
        }
    }

    // 立即按换算应用主窗口尺寸（此时窗口仍不可见）
    apply_all(app);
    log::info!(
        "窗口缩放模型就绪: 渲染缩放 R={:.3}（主窗口）",
        render_scale_any(app)
    );
}

// ==================== Tauri 命令 ====================

/// 前端上报 `devicePixelRatio` 实测值（启动自检 + dpr 变化时）
///
/// 推断值与实测不符时按实测校正文字 DPI 缩放并重排所有窗口。
/// 这是推断律（见模块文档）之外的安全网，覆盖其未覆盖的环境。
///
/// # 前端调用
///
/// ```typescript
/// await invoke('sync_render_scale', { scale: window.devicePixelRatio })
/// ```
#[tauri::command]
pub fn sync_render_scale(
    scale: f64,
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(format!("非法渲染缩放值: {scale}"));
        }
        // 同步命令跑在主线程：顺带刷新文字 DPI 基线缓存，保证推断值用的是最新设置
        refresh_text_scale_base();
        let inferred = render_scale(&window);
        if (inferred - scale).abs() <= 0.01 {
            log::debug!("渲染缩放自检通过: R={scale}");
            return Ok(());
        }
        // 修正文字 DPI 缩放系数，使 render_scale == 实测值
        {
            let mut c = TEXT_SCALE_CORRECTION.lock();
            *c *= scale / inferred;
            log::warn!(
                "渲染缩放推断与实测不符（推断 {inferred:.3}，实测 {scale:.3}），文字缩放修正系数 → {:.3}",
                *c
            );
        }
        apply_all(&app);
        Ok(())
    })
}

/// 倒计时气泡形态切换（圆点 ⇄ 终端确认提示条）
///
/// 设计尺寸由后端 [`crate::window_sizes`] 常量统一管理，前端不传裸数值。
///
/// # 前端调用
///
/// ```typescript
/// await invoke('set_countdown_expanded', { expanded: true })
/// ```
#[tauri::command]
pub fn set_countdown_expanded(expanded: bool, app: tauri::AppHandle) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        let (w, h) = if expanded {
            (ws::COUNTDOWN_HINT_WIDTH, ws::COUNTDOWN_HINT_HEIGHT)
        } else {
            (ws::COUNTDOWN_DOT_WIDTH, ws::COUNTDOWN_DOT_HEIGHT)
        };
        set_design_size(&app, "countdown", w, h);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 恒等变换：1x 缩放下设计尺寸原样映射
    #[test]
    fn identity_at_1x() {
        assert_eq!(to_physical_len(900.0, 1.0), 900);
        assert_eq!(to_physical_pos(15.0, 1.0), 15);
    }

    /// Linux 高分屏（R=1.5）：主窗口换算结果
    #[test]
    fn linux_150_percent() {
        assert_eq!(to_physical_len(900.0, 1.5), 1350);
        assert_eq!(to_physical_len(650.0, 1.5), 975);
    }

    /// Windows 175%：与 tao 逻辑尺寸换算结果一致（行为不变的保证）
    #[test]
    fn windows_175_percent() {
        assert_eq!(to_physical_len(900.0, 1.75), 1575);
        assert_eq!(to_physical_len(650.0, 1.75), 1138); // 1137.5 四舍五入
    }

    /// macOS Retina（R=2）
    #[test]
    fn retina_2x() {
        assert_eq!(to_physical_len(16.0, 2.0), 32);
        assert_eq!(to_physical_pos(15.0, 2.0), 30);
    }

    /// 1.25x 分数缩放：四舍五入不留半像素
    #[test]
    fn fractional_rounding() {
        assert_eq!(to_physical_len(15.0, 1.25), 19); // 18.75 → 19
        assert_eq!(to_physical_len(16.0, 1.25), 20);
    }
}
