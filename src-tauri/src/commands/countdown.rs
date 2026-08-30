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
use std::sync::Arc;
use tauri::{Emitter, Manager};

/// 全局标志，用于停止光标跟随线程
static FOLLOW_CURSOR_RUNNING: AtomicBool = AtomicBool::new(false);

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
    // 如果已经在运行，先停止
    if FOLLOW_CURSOR_RUNNING.load(Ordering::SeqCst) {
        FOLLOW_CURSOR_RUNNING.store(false, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    FOLLOW_CURSOR_RUNNING.store(true, Ordering::SeqCst);
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    std::thread::spawn(move || {
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

    Ok(())
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
    FOLLOW_CURSOR_RUNNING.store(false, Ordering::SeqCst);
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
        let _ = window.show();
        let _ = window.set_focus();
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
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(String::new()).map_err(|e| e.to_string())?;
    log::info!("Clipboard cleared");
    Ok(())
}
