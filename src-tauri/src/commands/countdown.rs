//! 倒计时命令模块
//!
//! 处理剪贴板清除倒计时。
//!
//! # 命令列表
//!
//! - `show_countdown`: 显示倒计时窗口
//! - `hide_countdown`: 隐藏倒计时窗口
//! - `start_follow_cursor`: 开始跟随光标
//! - `stop_follow_cursor`: 停止跟随光标
//! - `get_countdown_seconds`: 获取倒计时秒数

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;
use tauri::{Emitter, Manager};

/// 全局标志，用于停止光标跟随线程
static FOLLOW_CURSOR_RUNNING: AtomicBool = AtomicBool::new(false);

/// 光标跟随线程句柄（受管，用于停止时回收线程）
static FOLLOW_CURSOR_HANDLE: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// 全局倒计时秒数
static COUNTDOWN_SECONDS: AtomicU32 = AtomicU32::new(10);

/// 获取全局鼠标位置（Windows API）
#[cfg(windows)]
fn get_cursor_position() -> Option<(i32, i32)> {
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    use windows::Win32::Foundation::POINT;

    let mut point = POINT { x: 0, y: 0 };
    unsafe {
        if GetCursorPos(&mut point).is_ok() {
            Some((point.x, point.y))
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn get_cursor_position() -> Option<(i32, i32)> {
    // 非 Windows 平台暂不支持
    None
}

/// 显示窗口但不激活（不抢焦点）
///
/// 不能用 show()+set_focus()，否则倒计时小窗会把前台焦点从目标应用抢走，
/// Ctrl+V 钩子检测到前台是自己就会拒绝拦截。
#[cfg(windows)]
fn show_without_activate(window: &tauri::WebviewWindow) {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNA};

    if let Ok(hwnd) = window.hwnd() {
        // tauri 依赖的 windows crate 版本与本项目不同，HWND 是两个类型，
        // 底层都是 *mut c_void，手动桥接
        let hwnd = windows::Win32::Foundation::HWND(hwnd.0);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        }
    }
}

#[cfg(not(windows))]
fn show_without_activate(window: &tauri::WebviewWindow) {
    let _ = window.show();
}

/// 开始跟随光标移动
///
/// 启动一个后台线程，持续更新倒计时窗口位置使其跟随鼠标。
///
/// # Arguments
///
/// * `app` - Tauri 应用句柄
///
/// # 前端调用
///
/// ```typescript
/// await invoke('start_follow_cursor');
/// ```
#[tauri::command]
pub async fn start_follow_cursor(app: tauri::AppHandle) -> Result<(), String> {
    // 停止并回收旧线程，避免重复启动导致多线程竞态
    stop_follow_cursor_internal();

    FOLLOW_CURSOR_RUNNING.store(true, Ordering::SeqCst);

    let handle = std::thread::spawn(move || {
        log::info!("Cursor follow thread started");

        while FOLLOW_CURSOR_RUNNING.load(Ordering::SeqCst) {
            if let Some((x, y)) = get_cursor_position() {
                // 更新窗口位置（鼠标右下角偏移 15px）
                if let Some(window) = app.get_webview_window("countdown") {
                    let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
                        x: x + 15,
                        y: y + 15,
                    }));
                }
            }
            // 每 16ms 更新一次（约 60fps）
            std::thread::sleep(std::time::Duration::from_millis(16));
        }

        log::info!("Cursor follow thread stopped");
    });

    if let Ok(mut guard) = FOLLOW_CURSOR_HANDLE.lock() {
        *guard = Some(handle);
    }

    Ok(())
}

/// 停止跟随光标（内部实现：置标志 + join 回收线程）
fn stop_follow_cursor_internal() {
    FOLLOW_CURSOR_RUNNING.store(false, Ordering::SeqCst);
    if let Ok(mut guard) = FOLLOW_CURSOR_HANDLE.lock() {
        if let Some(handle) = guard.take() {
            // 线程最多再运行一个循环（约 16ms）即退出
            let _ = handle.join();
        }
    }
}

/// 停止跟随光标
///
/// # 前端调用
///
/// ```typescript
/// await invoke('stop_follow_cursor');
/// ```
#[tauri::command]
pub async fn stop_follow_cursor() -> Result<(), String> {
    stop_follow_cursor_internal();
    log::info!("Cursor follow stopped");
    Ok(())
}

/// 获取倒计时秒数
///
/// 前端调用此命令获取当前倒计时秒数。
///
/// # 前端调用
///
/// ```typescript
/// const seconds = await invoke('get_countdown_seconds');
/// ```
#[tauri::command]
pub async fn get_countdown_seconds() -> Result<u32, String> {
    Ok(COUNTDOWN_SECONDS.load(Ordering::SeqCst))
}

/// 显示倒计时窗口
///
/// 创建或显示倒计时窗口，并启动倒计时。
///
/// # Arguments
///
/// * `seconds` - 倒计时秒数
/// * `x` - 鼠标 X 坐标（屏幕坐标）
/// * `y` - 鼠标 Y 坐标（屏幕坐标）
/// * `app` - Tauri 应用句柄
///
/// # 前端调用
///
/// ```typescript
/// await invoke('show_countdown', { seconds: 30, x: 100, y: 200 });
/// ```
#[tauri::command]
pub async fn show_countdown(seconds: u32, x: f64, y: f64, app: tauri::AppHandle) -> Result<(), String> {
    log::info!("show_countdown called: seconds={}, x={}, y={}", seconds, x, y);

    // 存储倒计时秒数
    COUNTDOWN_SECONDS.store(seconds, Ordering::SeqCst);

    // 计算窗口位置（鼠标右下角偏移 15px）
    let window_x = x + 15.0;
    let window_y = y + 15.0;

    // 检查窗口是否已存在
    if let Some(window) = app.get_webview_window("countdown") {
        log::info!("Countdown window exists, updating position");
        // 窗口已存在，更新位置并显示
        let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition {
            x: window_x as i32,
            y: window_y as i32,
        }));
        // 发送重置事件
        let _ = window.emit("reset-countdown", serde_json::json!({ "seconds": seconds }));
        // 显示但不抢焦点
        show_without_activate(&window);
    } else {
        log::info!("Creating new countdown window");
        // 创建新窗口 - 透明无边框，始终置顶，无阴影
        let _window = tauri::WebviewWindowBuilder::new(
            &app,
            "countdown",
            tauri::WebviewUrl::App("countdown.html".into())
        )
        .title("")
        .inner_size(16.0, 16.0)
        .min_inner_size(16.0, 16.0)
        .max_inner_size(16.0, 16.0)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .position(window_x, window_y)
        .transparent(true)
        .skip_taskbar(true)
        .shadow(false)
        .focused(false)
        .visible(true)
        .build()
        .map_err(|e| e.to_string())?;

        log::info!("Countdown window created");
    }

    Ok(())
}

/// 隐藏倒计时窗口
///
/// # Arguments
///
/// * `app` - Tauri 应用句柄
///
/// # 前端调用
///
/// ```typescript
/// await invoke('hide_countdown');
/// ```
#[tauri::command]
pub async fn hide_countdown(app: tauri::AppHandle) -> Result<(), String> {
    // 先停止光标跟随
    FOLLOW_CURSOR_RUNNING.store(false, Ordering::SeqCst);

    if let Some(window) = app.get_webview_window("countdown") {
        let _ = window.hide();
    }

    Ok(())
}

/// 清除剪贴板
///
/// # 前端调用
///
/// ```typescript
/// await invoke('clear_clipboard');
/// ```
#[tauri::command]
pub async fn clear_clipboard(app: tauri::AppHandle) -> Result<(), String> {
    crate::clipboard::clear_now(&app)?;
    log::info!("Clipboard cleared");
    Ok(())
}

/// 统一销毁「粘贴/倒计时」状态（钩子检测到 Ctrl+C / Esc 时调用）
///
/// 作废 pending 密码槽、收掉倒计时窗口和光标跟随，并按需处理剪贴板：
///
/// * `clear_clipboard = true`（Esc 主动放弃）：立即清空剪贴板，把敏感残留销毁干净
/// * `clear_clipboard = false`（Ctrl+C，用户正在复制别的东西）：只作废遗留的
///   定时清除任务，**绝不能清剪贴板**，否则会毁掉用户刚复制的新内容
pub fn cancel_paste_state(app: &tauri::AppHandle, reason: &str, clear_clipboard: bool) {
    log::info!("销毁粘贴状态: reason={}, clear_clipboard={}", reason, clear_clipboard);

    // 1. 作废待粘贴密码（无 pending 时是空操作）
    crate::pending::revoke(reason);

    // 2. 处理剪贴板
    if clear_clipboard {
        if let Err(e) = crate::clipboard::clear_now(app) {
            log::warn!("销毁状态时清空剪贴板失败: {e}");
        }
    } else {
        crate::clipboard::invalidate_scheduled_clear();
    }

    // 3. 停止光标跟随线程（join 回收，防止窗口关掉后线程变孤儿）
    stop_follow_cursor_internal();

    // 4. 收掉倒计时窗口（前端监听 cancel-countdown 自行关窗，hide 是兜底）
    let _ = app.emit("cancel-countdown", ());
    if let Some(window) = app.get_webview_window("countdown") {
        let _ = window.hide();
    }
}
