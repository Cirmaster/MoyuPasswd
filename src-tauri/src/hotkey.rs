//! 全局热键与键盘钩子模块
//!
//! 提供双触发机制：
//! 1. Ctrl+V 钩子（主推）：捕获用户的 Ctrl+V 按键，拦截并改为注入密码
//! 2. Ctrl+Shift+V 热键（备用）：高级用户/兜底方案
//!
//! 同时监听退出信号（按键放行，只销毁状态）：
//! * Ctrl+C：用户已经在复制别的东西，密码自然不粘贴了（不碰剪贴板）
//! * Esc：用户主动放弃，彻底销毁（含清空剪贴板残留）

#[cfg(windows)]
mod win_impl {
    use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
    use std::ptr;
    use std::sync::mpsc;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};

    /// 一次性钩子句柄
    static HOOK_HANDLE: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(ptr::null_mut());
    /// 是否已激活
    static IS_ACTIVE: AtomicBool = AtomicBool::new(false);
    /// 钩子线程消息发送器
    static HOOK_SENDER: std::sync::Mutex<Option<mpsc::Sender<u32>>> = std::sync::Mutex::new(None);
    /// 主应用句柄（钩子回调里用于取消倒计时窗口等 UI 操作）
    static HOOK_APP: std::sync::Mutex<Option<tauri::AppHandle>> = std::sync::Mutex::new(None);

    /// 激活 Ctrl+V 钩子（一次性）
    ///
    /// # Arguments
    /// * `app` - 应用句柄（钩子线程里销毁粘贴状态时要用）
    pub fn activate(app: tauri::AppHandle) {
        if IS_ACTIVE.load(Ordering::SeqCst) {
            log::warn!("钩子已激活，忽略重复激活");
            return;
        }

        // 保存应用句柄，供钩子回调取消倒计时/作废 pending 使用
        if let Ok(mut guard) = HOOK_APP.lock() {
            *guard = Some(app);
        }

        // 创建消息通道
        let (tx, rx) = mpsc::channel::<u32>();
        {
            let mut sender = HOOK_SENDER.lock().unwrap();
            *sender = Some(tx);
        }

        // 启动钩子线程（必须有消息循环）
        std::thread::spawn(move || {
            unsafe {
                // 注册低级键盘钩子
                let hook = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(hook_proc),
                    None,
                    0,
                );

                match hook {
                    Ok(h) => {
                        HOOK_HANDLE.store(h.0, Ordering::SeqCst);
                        IS_ACTIVE.store(true, Ordering::SeqCst);
                        log::info!("Ctrl+V 钩子已激活，句柄={:?}", h.0);

                        // 关键：运行消息循环（钩子依赖消息循环）
                        let mut msg: MSG = std::mem::zeroed();
                        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }

                        // 消息循环退出，清理钩子
                        let hook = HOOK_HANDLE.load(Ordering::SeqCst);
                        if !hook.is_null() {
                            let _ = UnhookWindowsHookEx(HHOOK(hook));
                            HOOK_HANDLE.store(ptr::null_mut(), Ordering::SeqCst);
                        }
                        IS_ACTIVE.store(false, Ordering::SeqCst);
                        log::info!("钩子线程退出");
                    }
                    Err(e) => {
                        log::error!("注册 Ctrl+V 钩子失败: {}", e);
                    }
                }
            }

            // 接收退出信号
            drop(rx);
        });

        log::info!("钩子线程已启动");
    }

    /// 反激活钩子
    pub fn deactivate() {
        if !IS_ACTIVE.load(Ordering::SeqCst) {
            return;
        }

        // 发送退出信号给钩子线程
        {
            let sender = HOOK_SENDER.lock().unwrap();
            if let Some(ref tx) = *sender {
                let _ = tx.send(0);  // 发送任意值触发退出
            }
        }

        log::info!("已发送钩子退出信号");
    }

    /// 钩子过程（低级键盘钩子）
    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let is_keydown = wparam.0 == WM_KEYDOWN as usize || wparam.0 == WM_SYSKEYDOWN as usize;

            // 检测 Ctrl+V
            if is_keydown && kb.vkCode == 'V' as u32 && is_ctrl_pressed() {
                log::info!("[HOOK] 检测到 Ctrl+V");

                // 检查：有待粘贴密码 && 前台不是我们自己的窗口
                let has_pending = crate::pending::get_active().is_some();
                let is_our_window = is_foreground_our_window();

                if has_pending && !is_our_window {
                    log::info!("[HOOK] 拦截 Ctrl+V，触发注入");

                    // 触发注入（在独立线程执行，避免阻塞钩子）
                    std::thread::spawn(|| {
                        crate::inject::trigger_inject();
                    });

                    // 吞掉这次按键（不传给系统）
                    return LRESULT(1);
                } else {
                    log::info!("[HOOK] 不拦截：pending={}, 前台是我们={}", has_pending, is_our_window);
                }
            }

            // 检测退出信号：Ctrl+C / Esc
            //
            // 语义：Ctrl+C = 用户已经在复制别的东西，密码自然不粘贴了；
            //       Esc    = 用户主动放弃。
            // 两者都销毁粘贴状态，但按键照常放行（不吞掉）。
            let is_esc = kb.vkCode == VK_ESCAPE.0 as u32;
            let is_ctrl_c = kb.vkCode == 'C' as u32 && is_ctrl_pressed();
            if is_keydown && (is_esc || is_ctrl_c) && has_paste_state() {
                log::info!("[HOOK] 检测到 {}，销毁粘贴状态", if is_esc { "Esc" } else { "Ctrl+C" });

                // Esc 是主动放弃，清掉剪贴板残留；Ctrl+C 时用户正在复制新内容，
                // 绝不能清剪贴板，否则会毁掉他刚复制的东西。
                let clear_clipboard = is_esc;
                std::thread::spawn(move || {
                    let reason = if is_esc { "user-esc" } else { "user-ctrl-c" };
                    match get_hook_app() {
                        Some(app) => {
                            crate::commands::countdown::cancel_paste_state(&app, reason, clear_clipboard);
                        }
                        None => {
                            // 拿不到句柄时至少作废 pending
                            crate::pending::revoke(reason);
                        }
                    }
                });
            }
        }

        // 其他按键正常传递
        CallNextHookEx(None, code, wparam, lparam)
    }

    /// 检查 Ctrl 键是否按下
    fn is_ctrl_pressed() -> bool {
        unsafe {
            (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0
        }
    }

    /// 取出 activate 时保存的应用句柄
    fn get_hook_app() -> Option<tauri::AppHandle> {
        HOOK_APP.lock().unwrap().clone()
    }

    /// 是否存在待销毁的粘贴/倒计时状态
    ///
    /// 两种情况之一即算有：
    /// 1. 有待粘贴密码（copy_password_to_clipboard 登记的 pending）
    /// 2. 倒计时窗口正在显示（文本复制场景，无 pending 但 UI 在跑）
    fn has_paste_state() -> bool {
        if crate::pending::has_active() {
            return true;
        }
        if let Some(app) = get_hook_app() {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("countdown") {
                return window.is_visible().unwrap_or(false);
            }
        }
        false
    }

    /// 检查前台窗口是否是我们自己的窗口
    fn is_foreground_our_window() -> bool {
        unsafe {
            let foreground = GetForegroundWindow();
            if foreground.is_invalid() {
                return false;
            }

            // 获取前台窗口的进程 ID
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(foreground, Some(&mut pid));

            // 比较是否是我们自己的进程
            let current_pid = std::process::id();
            let result = pid == current_pid;
            log::debug!("[HOOK] 前台 PID={}, 我们 PID={}, 是我们={}", pid, current_pid, result);
            result
        }
    }

    /// 注册全局热键 Ctrl+Shift+V（备用）
    pub fn register_backup_hotkey() -> bool {
        log::info!("备用热键 Ctrl+Shift+V 注册（待实现）");
        true
    }

    /// 反注册备用热键
    pub fn unregister_backup_hotkey() {
        log::info!("备用热键反注册（待实现）");
    }
}

#[cfg(not(windows))]
mod non_windows {
    /// 非 Windows 平台：不支持钩子，只能用热键
    pub fn activate(_app: tauri::AppHandle) {
        log::warn!("非 Windows 平台不支持 Ctrl+V 钩子，请使用热键触发");
    }

    pub fn deactivate() {
        // 无操作
    }

    pub fn register_backup_hotkey() -> bool {
        log::warn!("非 Windows 平台热键功能待实现");
        false
    }

    pub fn unregister_backup_hotkey() {
        // 无操作
    }
}

#[cfg(windows)]
pub use win_impl::*;

#[cfg(not(windows))]
pub use non_windows::*;
