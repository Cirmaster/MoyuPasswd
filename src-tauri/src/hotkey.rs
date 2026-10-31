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
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
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
    /// 激活互斥：把「检查 IS_ACTIVE + 置位 + 启动线程」包成整体。
    /// 并发两次 activate 都通过检查时会起两个钩子线程，句柄/线程 ID 互相覆盖，
    /// deactivate 只收一个 → 全局键盘钩子永久驻留（keylogger 面常开）。
    static ACTIVATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 剪贴板哨兵基线序号（系统每换一次剪贴板内容就 +1）
    ///
    /// 会话期间序号一变就说明用户复制了新内容（右键/菜单复制不经 Ctrl+C，
    /// 钩子收不到退出信号），此时应立即销毁粘贴状态，防旧密码劫持后续 Ctrl+V。
    static CLIP_BASELINE: AtomicU32 = AtomicU32::new(0);

    /// 读取当前剪贴板序号（0 = 读取失败/不可用）
    fn clipboard_seq() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }

    /// 重置剪贴板哨兵基线到当前序号
    ///
    /// 「我们自己」写剪贴板（复制文本）后、以及每次会话建立时调用，
    /// 之后出现的任何变化才视为「用户复制了新内容」。
    pub fn rebase_clipboard_sentinel() {
        CLIP_BASELINE.store(clipboard_seq(), Ordering::SeqCst);
    }

    /// 会话期间剪贴板是否出现过新内容（发现即重置基线，避免重复触发）
    fn clipboard_changed() -> bool {
        let cur = clipboard_seq();
        // 读不到序号时跳过本轮，宁可漏判不误杀会话（该 API 基本不会失败）
        if cur == 0 {
            return false;
        }
        let base = CLIP_BASELINE.load(Ordering::SeqCst);
        if cur != base {
            CLIP_BASELINE.store(cur, Ordering::SeqCst);
            return true;
        }
        false
    }

    /// 取激活锁（毒化降级恢复，不连环 panic）
    fn lock_activate() -> std::sync::MutexGuard<'static, ()> {
        ACTIVATE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 激活 Ctrl+V 钩子（一次性）
    ///
    /// 粘贴状态（pending / 倒计时）结束后由监视线程自动卸载钩子，不常驻。
    ///
    /// # Arguments
    /// * `app` - 应用句柄（钩子线程里销毁粘贴状态时要用）
    pub fn activate(app: tauri::AppHandle) {
        // 保存应用句柄，供钩子回调取消倒计时/作废 pending 使用
        {
            let mut guard = HOOK_APP.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(app);
        }

        // 哨兵基线对齐到「此刻」：复制流程刚写完剪贴板/刚登记 pending，
        // 这之后的剪贴板变化才算用户复制了新内容（每次调用都重基，
        // 钩子已激活时重复 activate 也覆盖，避免基线停在旧内容上）
        rebase_clipboard_sentinel();

        // 全程持锁：检查 + 置位 + 启动不可分割
        let _guard = lock_activate();

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
        // 同步置位「激活中/已激活」，让并发的第二次 activate 走「已激活」分支；
        // 钩子线程退出/失败时复位
        IS_ACTIVE.store(true, Ordering::SeqCst);

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
                        // IS_ACTIVE 已在 activate 里同步置位，失败必须复位
                        IS_ACTIVE.store(false, Ordering::SeqCst);
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
                    // 剪贴板哨兵：会话期间剪贴板出现新内容（右键/菜单复制等不经
                    // Ctrl+C 的复制）→ 立即销毁粘贴状态——倒计时收窗、pending 作废，
                    // 防旧密码劫持随后的 Ctrl+V
                    if clipboard_changed() {
                        log::info!("[HOOK] 剪贴板出现新内容，销毁粘贴状态");
                        match get_hook_app() {
                            Some(app) => {
                                crate::commands::countdown::cancel_paste_state(
                                    &app,
                                    "clipboard-changed",
                                    false,
                                );
                            }
                            None => crate::pending::revoke("clipboard-changed"),
                        }
                    }
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
        // 等钩子线程发布线程 ID（刚启动的窗口期），最多等 1 秒；
        // 否则会漏发 WM_QUIT，钩子线程变成无人能收的孤儿
        let mut tid = HOOK_THREAD_ID.load(Ordering::SeqCst);
        for _ in 0..20 {
            if tid != 0 || !IS_ACTIVE.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            tid = HOOK_THREAD_ID.load(Ordering::SeqCst);
        }
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

                // 检查：有待粘贴密码就拦截。
                // 自己的窗口同样拦截：密码不进剪贴板，放行只会在本应用内贴出空/旧内容；
                // 应用内表单（如新建条目里粘贴刚复制的密码）也走注入。
                // 锁屏/解锁框不受影响：lockdown 已作废 pending。
                let has_pending = crate::pending::get_active().is_some();

                if has_pending {
                    // 剪贴板哨兵兜底（轮询间隔内可能漏判）：剪贴板已换过新内容时，
                    // 本次 Ctrl+V 还给系统做普通粘贴，异步销毁残留会话
                    if clipboard_changed() {
                        log::info!("[HOOK] 剪贴板已换新内容，放行 Ctrl+V 并销毁粘贴状态");
                        let app = get_hook_app();
                        std::thread::spawn(move || {
                            match app {
                                Some(app) => {
                                    crate::commands::countdown::cancel_paste_state(
                                        &app,
                                        "clipboard-changed",
                                        false,
                                    );
                                }
                                None => crate::pending::revoke("clipboard-changed"),
                            }
                        });
                    } else {
                        log::info!("[HOOK] 拦截 Ctrl+V，触发注入");

                        // 触发注入（在独立线程执行，避免阻塞钩子）
                        std::thread::spawn(|| {
                            crate::inject::trigger_inject();
                        });

                        // 吞掉这次按键（不传给系统）
                        return LRESULT(1);
                    }
                } else {
                    log::info!("[HOOK] 不拦截：无待粘贴密码");
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
        HOOK_APP.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
}

/// X11 实现（仅 Linux）
///
/// Linux 没有 WH_KEYBOARD_LL 对应物，Ctrl+V 触发改用 XGrabKey：
/// 粘贴状态（pending/倒计时）期间被动抓取 Ctrl+V 组合键，语义与 Windows
/// 钩子一致——复制时激活、状态结束自动卸载、期间 Ctrl+V 触发注入。
/// 组合键抓取是「消费」型（按键到不了应用），因此无待注入密码的场景
///（文本复制会话）要用 XTest 回放 Ctrl+V，把粘贴还给应用。
/// 剪贴板哨兵用 CLIPBOARD selection 属主变化近似 Windows 的剪贴板序号。
/// Wayland 无此能力，activate 直接跳过（is_x11_session 判定）。
#[cfg(target_os = "linux")]
mod x11_impl {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{Allow, ConnectionExt as _, GrabMode, ModMask};
    use x11rb::protocol::Event;

    /// 是否已激活（抓取线程在跑）
    static IS_ACTIVE: AtomicBool = AtomicBool::new(false);
    /// 退出信号（deactivate 置位，线程在一个轮询间隔内自行清理退出）
    static STOP: AtomicBool = AtomicBool::new(false);
    /// 激活互斥（对齐 Windows 版：检查 + 置位 + 启动不可分割）
    static ACTIVATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    /// 主应用句柄（销毁粘贴状态时取消倒计时窗口等 UI 操作）
    static HOOK_APP: std::sync::Mutex<Option<tauri::AppHandle>> = std::sync::Mutex::new(None);
    /// 剪贴板哨兵基线：CLIPBOARD selection 当前属主窗口（0 = 无属主）
    ///
    /// X11 没有 Windows 的剪贴板序号，用「属主窗口 id」近似：任何应用复制
    /// 新内容都会接管 CLIPBOARD，属主即变。剪贴板管理器接管属主时可能误判
    /// ——误判方向是「提前作废会话、退化为剪贴板粘贴」，安全且不误注入。
    static CLIP_BASELINE: AtomicU32 = AtomicU32::new(0);

    fn get_hook_app() -> Option<tauri::AppHandle> {
        HOOK_APP.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn lock_activate() -> std::sync::MutexGuard<'static, ()> {
        ACTIVATE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 查询 CLIPBOARD selection 的属主窗口
    fn clipboard_owner(conn: &impl Connection) -> Option<u32> {
        let atom = crate::x11::intern(conn, "CLIPBOARD")?;
        Some(conn.get_selection_owner(atom).ok()?.reply().ok()?.owner)
    }

    /// 重置哨兵基线到当前属主（自己写完剪贴板后调用，对齐 Windows 语义）
    pub fn rebase_clipboard_sentinel() {
        if let Some((conn, _)) = crate::x11::connect() {
            if let Some(owner) = clipboard_owner(&conn) {
                CLIP_BASELINE.store(owner, Ordering::SeqCst);
            }
        }
    }

    /// 会话期间剪贴板是否易主（发现即重置基线，避免重复触发）
    fn clipboard_changed(conn: &impl Connection) -> bool {
        let cur = match clipboard_owner(conn) {
            Some(o) => o,
            None => return false, // 查询失败跳过本轮，宁可漏判不误杀会话
        };
        let base = CLIP_BASELINE.load(Ordering::SeqCst);
        if cur != base {
            CLIP_BASELINE.store(cur, Ordering::SeqCst);
            return true;
        }
        false
    }

    /// 是否存在待销毁的粘贴/倒计时状态（对齐 Windows 版同名逻辑）
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

    /// 异步销毁粘贴状态（对齐 Windows 版的两种降级路径）
    fn cancel_async(reason: &'static str, clear_clipboard: bool) {
        std::thread::spawn(move || match get_hook_app() {
            Some(app) => {
                crate::commands::countdown::cancel_paste_state(&app, reason, clear_clipboard)
            }
            None => crate::pending::revoke(reason),
        });
    }

    /// Ctrl+V 的 4 个抓取组合（覆盖 CapsLock(Lock) / NumLock(Mod2) 开关态）
    fn grab_combos(conn: &impl Connection, root: u32, v_kc: u8) {
        use ModMask as M;
        let combos = [
            M::CONTROL,
            M::CONTROL | M::LOCK,
            M::CONTROL | M::M2,
            M::CONTROL | M::LOCK | M::M2,
        ];
        for mods in combos {
            if let Err(e) = conn.grab_key(false, root, mods, v_kc, GrabMode::ASYNC, GrabMode::ASYNC)
            {
                log::warn!("[HOOK] X11 grab_key 失败: {e}");
            }
        }
        let _ = conn.flush();
    }

    /// 解除抓取（含组合键触发后的隐式活动抓取）
    fn ungrab_combos(conn: &impl Connection, root: u32, v_kc: u8) {
        use ModMask as M;
        let combos = [
            M::CONTROL,
            M::CONTROL | M::LOCK,
            M::CONTROL | M::M2,
            M::CONTROL | M::LOCK | M::M2,
        ];
        for mods in combos {
            let _ = conn.ungrab_key(v_kc, root, mods);
        }
        // 关键：Ctrl+V 触发后键盘处于隐式活动抓取状态，不显式放开的话
        // XTest 合成的按键会被重定向给抓取方（本客户端），目标窗口收不到注入
        let _ = conn.ungrab_keyboard(0u32); // 0 = CurrentTime
        let _ = conn.flush();
    }

    /// 抓取线程：Ctrl+V 事件分发 + 剪贴板哨兵 + 无状态自动卸载
    fn worker() {
        let (conn, screen_num) = match crate::x11::connect() {
            Some(c) => c,
            None => {
                IS_ACTIVE.store(false, Ordering::SeqCst);
                return;
            }
        };
        let root = conn.setup().roots[screen_num].root;

        // 'v' 的 keysym = 0x76（Ctrl 由抓取掩码约束）
        let v_kc = match crate::x11::key_for_keysym(&conn, 0x76) {
            Some((kc, _)) => kc,
            None => {
                log::error!("[HOOK] X11 键盘映射里找不到 'v'，无法抓取 Ctrl+V");
                IS_ACTIVE.store(false, Ordering::SeqCst);
                return;
            }
        };

        log::info!("[HOOK] X11 Ctrl+V 监听线程已启动（keycode={v_kc}，抓取跟随 pending）");

        let mut idle_ticks = 0u32;
        let mut sentinel_ticks = 0u32;
        // 抓取跟随 pending：只在有待注入密码时拦 Ctrl+V（对齐 Windows 钩子语义）。
        // 文本复制会话（无 pending）不挂抓取，Ctrl+V 原生通过——不必回放，
        // 也就没有「回放时按键已松开」的时序竞态。
        let mut grabbed = false;
        while !STOP.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(10));

            while let Ok(Some(event)) = conn.poll_for_event() {
                if let Event::KeyPress(kp) = event {
                    if kp.detail == v_kc {
                        handle_ctrl_v(&conn, root, v_kc, &mut grabbed);
                    }
                }
            }

            // 按 pending 状态挂/摘抓取
            let should_grab = crate::pending::has_active();
            if should_grab && !grabbed {
                grab_combos(&conn, root, v_kc);
                grabbed = true;
                log::info!("[HOOK] 待注入密码就绪，抓取 Ctrl+V");
            } else if !should_grab && grabbed {
                ungrab_combos(&conn, root, v_kc);
                grabbed = false;
                log::info!("[HOOK] 待注入密码已消耗/作废，放行 Ctrl+V");
            }

            // 剪贴板哨兵（~1s 一轮，对齐 Windows 监视线程节奏）
            sentinel_ticks += 1;
            if sentinel_ticks >= 100 {
                sentinel_ticks = 0;
                if clipboard_changed(&conn) {
                    log::info!("[HOOK] 剪贴板出现新内容，销毁粘贴状态");
                    cancel_async("clipboard-changed", false);
                }
            }

            // 一次性钩子语义：粘贴状态持续 2s 缺失即自动卸载（对齐 Windows）
            if has_paste_state() {
                idle_ticks = 0;
            } else {
                idle_ticks += 1;
                if idle_ticks >= 200 {
                    log::info!("[HOOK] X11 粘贴状态结束，自动卸载抓取");
                    break;
                }
            }
        }

        if grabbed {
            ungrab_combos(&conn, root, v_kc);
        }
        IS_ACTIVE.store(false, Ordering::SeqCst);
        log::info!("[HOOK] X11 Ctrl+V 抓取已卸载");
    }

    /// Ctrl+V 触发处理（仅在有待注入密码时会被调用——抓取跟随 pending）
    ///
    /// 抓取是消费型的，「不该拦」的按键必须还回去：用 XAllowEvents(ReplayKeyboard)
    /// 把触发按键原样回放给焦点窗口（X11 为被动抓取设计的原语，物理修饰键状态
    /// 天然正确）。注意 ReplayKeyboard 必须在隐式抓取存续期内（按键松开前）发出，
    /// 所以事件轮询间隔不能太长。
    /// 三种情况：
    /// 1. 无待注入密码（pending 刚被消耗/作废）→ 回放，把粘贴还给应用
    /// 2. 剪贴板已换新内容 → 异步销毁会话 + 回放（对齐 Windows「放行做普通粘贴」）
    /// 3. 正常 → 解开抓取、注入（注入后由循环按 pending 决定是否重新抓取）
    fn handle_ctrl_v(conn: &impl Connection, root: u32, v_kc: u8, grabbed: &mut bool) {
        log::info!("[HOOK] X11 检测到 Ctrl+V");
        let active = crate::x11::get_cardinal(conn, root, "_NET_ACTIVE_WINDOW").unwrap_or(0);
        log::info!("[HOOK] 当前前台窗口={}", active);

        if crate::pending::get_active().is_none() {
            log::info!("[HOOK] 不拦截：无待粘贴密码，回放 Ctrl+V");
            replay_key(conn);
            return;
        }

        if clipboard_changed(conn) {
            log::info!("[HOOK] 剪贴板已换新内容，销毁粘贴状态并回放本次 Ctrl+V");
            cancel_async("clipboard-changed", false);
            replay_key(conn);
            return;
        }

        log::info!("[HOOK] 拦截 Ctrl+V，触发注入");
        ungrab_combos(conn, root, v_kc);
        *grabbed = false;
        crate::inject::trigger_inject();
    }

    /// 回放触发按键给焦点窗口（XAllowEvents(ReplayKeyboard)）
    ///
    /// 必须在隐式抓取存续期内调用（按键松开前）；失败记日志——
    /// 那次按键会被吞掉，但下一次 Ctrl+V 恢复正常。
    fn replay_key(conn: &impl Connection) {
        let result = conn
            .allow_events(Allow::REPLAY_KEYBOARD, 0u32)
            .map_err(|e| e.to_string())
            .and_then(|cookie| cookie.check().map_err(|e| e.to_string()));
        match result {
            Ok(()) => log::info!("[HOOK] Ctrl+V 已回放给焦点窗口"),
            Err(e) => log::warn!("[HOOK] 回放 Ctrl+V 失败（按键可能已松开）: {e}"),
        }
    }

    /// 激活 Ctrl+V 抓取（一次性，对齐 Windows 版 activate 契约）
    ///
    /// # Arguments
    /// * `app` - 应用句柄（销毁粘贴状态时要用）
    pub fn activate(app: tauri::AppHandle) {
        {
            let mut guard = HOOK_APP.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(app);
        }

        // 哨兵基线对齐到「此刻」（对齐 Windows：自己刚写完剪贴板/刚登记 pending）
        rebase_clipboard_sentinel();

        // Wayland 无全局输入接口，不抓取（剪贴板粘贴体验不受影响）
        if !crate::x11::is_x11_session() {
            log::warn!("非 X11 会话（Wayland），不支持 Ctrl+V 触发注入，本次仅剪贴板粘贴");
            return;
        }

        let _guard = lock_activate();
        // 若上一个抓取线程还在清理，等它退出，避免新旧交叠
        for _ in 0..20 {
            if !IS_ACTIVE.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if IS_ACTIVE.load(Ordering::SeqCst) {
            log::warn!("抓取已激活，忽略重复激活");
            return;
        }
        IS_ACTIVE.store(true, Ordering::SeqCst);
        STOP.store(false, Ordering::SeqCst);
        std::thread::spawn(worker);
    }

    /// 反激活：置停止信号，抓取线程自行清理退出（最多一个轮询间隔）
    pub fn deactivate() {
        if !IS_ACTIVE.load(Ordering::SeqCst) {
            return;
        }
        STOP.store(true, Ordering::SeqCst);
    }
}

/// 非 Windows 且非 Linux 平台（macOS 等）桩：不支持钩子
#[cfg(not(any(windows, target_os = "linux")))]
mod non_windows {
    /// 该平台：不支持钩子
    pub fn activate(_app: tauri::AppHandle) {
        log::warn!("该平台不支持 Ctrl+V 钩子");
    }

    pub fn deactivate() {
        // 无操作
    }

    /// 该平台无剪贴板序号，哨兵空操作
    pub fn rebase_clipboard_sentinel() {
        // 无操作
    }
}

#[cfg(windows)]
pub use win_impl::*;

#[cfg(target_os = "linux")]
pub use x11_impl::*;

#[cfg(not(any(windows, target_os = "linux")))]
pub use non_windows::*;
