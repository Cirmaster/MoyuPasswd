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

use crate::response::{respond, ApiResult};
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

/// 获取全局鼠标位置（X11 query_pointer，相对根窗口的屏幕坐标）
///
/// Wayland 无全局光标查询接口（安全设计），返回 None 整体跳过跟随。
/// 连接按线程缓存：跟随线程 16ms 一轮，逐次建连开销太大；查询失败时
/// 丢弃连接下次重连（X 服务器重启等场景可自愈）。
#[cfg(target_os = "linux")]
fn get_cursor_position() -> Option<(i32, i32)> {
    use std::cell::RefCell;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt as _;

    thread_local! {
        static CONN: RefCell<Option<(x11rb::rust_connection::RustConnection, usize)>> =
            RefCell::new(None);
    }

    if !crate::x11::is_x11_session() {
        return None;
    }

    CONN.with(|cell| {
        let need_connect = cell.borrow().is_none();
        if need_connect {
            *cell.borrow_mut() = crate::x11::connect();
        }
        // 查询期间只读借用，结束后再决定是否丢弃连接（避免借用冲突）
        let reply = {
            let slot = cell.borrow();
            let Some((conn, screen)) = slot.as_ref() else {
                return None;
            };
            let root = conn.setup().roots[*screen].root;
            conn.query_pointer(root).ok().and_then(|c| c.reply().ok())
        };
        match reply {
            Some(r) => Some((r.root_x as i32, r.root_y as i32)),
            None => {
                // 查询失败：连接可能已死（如 X 服务器重启），丢弃后下次重连
                *cell.borrow_mut() = None;
                None
            }
        }
    })
}

/// 获取全局鼠标位置（其他平台桩：无光标位置 API，跟随跳过）
#[cfg(not(any(windows, target_os = "linux")))]
fn get_cursor_position() -> Option<(i32, i32)> {
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
    // X11 下 WM 会在窗口 map 时按焦点策略给焦点，把目标应用的输入抢走
    //（Windows 用 SW_SHOWNA 规避的正是这件事）。set_accept_focus(false) 让
    // 气泡永不接受键盘焦点，与 Windows 语义对齐；鼠标交互不受影响。
    let _ = window.set_focusable(false);
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
pub async fn start_follow_cursor(app: tauri::AppHandle) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        // 停止并回收旧线程，避免重复启动导致多线程竞态
        stop_follow_cursor_internal();

        // 光标跟随依赖系统光标位置：Windows API / X11 query_pointer 可用；
        // Wayland（无全局光标查询）与未实现平台返回 None，跳过启动避免空转
        if get_cursor_position().is_none() {
            log::info!("当前平台/会话不支持光标跟随，跳过启动");
            return Ok(());
        }

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

    })}

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
pub async fn stop_follow_cursor() -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        stop_follow_cursor_internal();
        log::info!("Cursor follow stopped");
        Ok(())

    })}

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
pub async fn get_countdown_seconds() -> ApiResult<u32> {
    respond(|| -> Result<u32, String> {
        Ok(COUNTDOWN_SECONDS.load(Ordering::SeqCst))

    })}

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
pub async fn show_countdown(seconds: u32, x: f64, y: f64, app: tauri::AppHandle) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
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
            // 上限放宽：终端二次确认提示时前端会把小圆点扩成提示条（见 CountdownWindow.vue）
            .max_inner_size(320.0, 48.0)
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

            // 气泡不接受键盘焦点（X11 下 WM map 时会给焦点，抢走目标应用输入），
            // 对齐 Windows SW_SHOWNA 语义；见 show_without_activate
            let _ = _window.set_focusable(false);

            // 新建窗口：关闭 WebView2 浏览器专属快捷键（F3 查找栏等，见 browser_keys.rs）
            crate::browser_keys::disable_browser_keys(&_window);

            log::info!("Countdown window created");
        }

        Ok(())

    })}

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
pub async fn hide_countdown(app: tauri::AppHandle) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        // 先停止光标跟随（走统一的 join 回收路径，防止线程变孤儿）；
        // 只置 FOLLOW_CURSOR_RUNNING=false 不 join 的旧写法会残留 JoinHandle
        stop_follow_cursor_internal();

        if let Some(window) = app.get_webview_window("countdown") {
            let _ = window.hide();
        }

        Ok(())

    })}

/// 清除剪贴板
///
/// # 前端调用
///
/// ```typescript
/// await invoke('clear_clipboard');
/// ```
#[tauri::command]
pub async fn clear_clipboard(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        // 解锁门禁：锁定状态下不允许清空用户剪贴板（防骚扰/破坏）
        crate::commands::auth::require_unlocked(&state)?;
        crate::clipboard::clear_now(&app)?;
        log::info!("Clipboard cleared");
        Ok(())

    })}

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
