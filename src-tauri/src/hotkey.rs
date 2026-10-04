//! 全局热键与键盘钩子模块
//!
//! Ctrl+V 钩子：捕获用户的 Ctrl+V 按键，拦截并改为注入密码。
//! 一次性钩子语义：复制密码时激活，粘贴状态（pending/倒计时）结束后自动卸载，不常驻。
//!
//! 同时监听退出信号（按键放行，只销毁状态）：
//! * Ctrl+C：用户已经在复制别的东西，密码自然不粘贴了（不碰剪贴板）
//! * Esc：用户主动放弃，彻底销毁（含清空剪贴板残留）

#[cfg(windows)]
mod win_impl {
    use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
    use std::ptr;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;

    /// 一次性钩子句柄
    static HOOK_HANDLE: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(ptr::null_mut());
    /// 是否已激活
    static IS_ACTIVE: AtomicBool = AtomicBool::new(false);
    /// 正在卸载中（WM_QUIT 已投递、清理未完成）
    static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
    /// 钩子线程 ID（deactivate 用 PostThreadMessageW 退出其消息循环）
    static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
    /// 主应用句柄（钩子回调里用于取消倒计时窗口等 UI 操作）
    static HOOK_APP: std::sync::Mutex<Option<tauri::AppHandle>> = std::sync::Mutex::new(None);

    /// 激活 Ctrl+V 钩子（一次性）
    ///
    /// 粘贴状态（pending / 倒计时）结束后由监视线程自动卸载钩子，不常驻。
    ///
    /// # Arguments
    /// * `app` - 应用句柄（钩子线程里销毁粘贴状态时要用）
    pub fn activate(app: tauri::AppHandle) {
        // 保存应用句柄，供钩子回调取消倒计时/作废 pending 使用
        if let Ok(mut guard) = HOOK_APP.lock() {
            *guard = Some(app);
        }

        // 若上一个钩子正在卸载，等它清理完成再启动，避免新旧钩子交叠
        for _ in 0..20 {
            if !SHUTTING_DOWN.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if IS_ACTIVE.load(Ordering::SeqCst) {
            log::warn!("钩子已激活，忽略重复激活");
            return;
        }

        // 启动钩子线程（必须有消息循环）
        std::thread::spawn(|| {
            unsafe {
                // 先创建线程消息队列（否则 PostThreadMessageW 可能投递失败），
                // 再发布线程 ID，供 deactivate 发 WM_QUIT 退出消息循环
                let mut msg: MSG = std::mem::zeroed();
                let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                HOOK_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);

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
                        SHUTTING_DOWN.store(false, Ordering::SeqCst);
                        log::info!("Ctrl+V 钩子已激活，句柄={:?}", h.0);

                        // 关键：运行消息循环（钩子依赖消息循环）；deactivate 发 WM_QUIT 使其返回 0 退出
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
                        SHUTTING_DOWN.store(false, Ordering::SeqCst);
                        HOOK_THREAD_ID.store(0, Ordering::SeqCst);
                        log::info!("钩子线程退出，钩子已卸载");
                    }
                    Err(e) => {
                        log::error!("注册 Ctrl+V 钩子失败: {}", e);
                        SHUTTING_DOWN.store(false, Ordering::SeqCst);
                        HOOK_THREAD_ID.store(0, Ordering::SeqCst);
                    }
                }
            }
        });

        // 启动监视线程：粘贴状态结束后自动卸钩（一次性钩子语义）
        std::thread::spawn(|| {
            // 等待钩子真正激活
            for _ in 0..50 {
                if IS_ACTIVE.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let mut idle_count = 0u32;
            while IS_ACTIVE.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_secs(1));
                if !IS_ACTIVE.load(Ordering::SeqCst) {
                    return;
                }
                if has_paste_state() {
                    idle_count = 0;
                } else {
                    idle_count += 1;
                    // 连续 2 秒无粘贴状态（pending 过期/作废、倒计时关闭）→ 自动卸钩
                    if idle_count >= 2 {
                        log::info!("[HOOK] 粘贴状态结束，自动卸载钩子");
                        deactivate();
                        return;
                    }
                }
            }
        });

        log::info!("钩子线程已启动");
    }

    /// 反激活钩子
    ///
    /// 向钩子线程投递 WM_QUIT，令 GetMessageW 返回并走卸载清理。
    pub fn deactivate() {
        if !IS_ACTIVE.load(Ordering::SeqCst) && !SHUTTING_DOWN.load(Ordering::SeqCst) {
            return;
        }
        // 已在卸载中则不重复投递
        if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
            return;
        }
        let tid = HOOK_THREAD_ID.load(Ordering::SeqCst);
        if tid == 0 {
            SHUTTING_DOWN.store(false, Ordering::SeqCst);
            return;
        }
        unsafe {
            match PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0)) {
                Ok(()) => log::info!("已向钩子线程发送 WM_QUIT，卸载钩子"),
                Err(e) => {
                    SHUTTING_DOWN.store(false, Ordering::SeqCst);
                    log::warn!("发送 WM_QUIT 失败: {}", e);
                }
            }
        }
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
}

#[cfg(not(windows))]
mod non_windows {
    /// 非 Windows 平台：不支持钩子
    pub fn activate(_app: tauri::AppHandle) {
        log::warn!("非 Windows 平台不支持 Ctrl+V 钩子");
    }

    pub fn deactivate() {
        // 无操作
    }
}

#[cfg(windows)]
pub use win_impl::*;

#[cfg(not(windows))]
pub use non_windows::*;
