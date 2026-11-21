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
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::core::PWSTR;

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

    /// 「重装钩子」线程消息（投递给钩子线程，在它的消息循环里处理）
    ///
    /// 用 WM_APP 之后的自定义值：不与其他系统消息冲突，也不需要窗口句柄
    /// （PostThreadMessageW 直接投递到线程队列）。
    const WM_REARM: u32 = 0x8000 + 1;

    /// 远控客户端前台守望线程的参数
    ///
    /// 低级键盘钩子按「后装先调用」执行。远控客户端（ToDesk 等）在**获得前台时**
    /// 才安装自己的 WH_KEYBOARD_LL，把物理按键全部拦下转发给远端，于是比它先装的
    /// 钩子一个都收不到（RemoteDictation 在真实 ToDesk 环境的实测：启动时装 0/37，
    /// 远控到前台后重装 37/37）。
    ///
    /// 对策与它一致：
    /// 1. 远控窗口刚切到前台时等 300ms，让它先装完，我们立刻重装压到它前面；
    /// 2. 之后每 3 秒补装一次，防止它后续重装或别的软件插队。
    ///
    /// 只在「粘贴会话进行中」做，会话结束线程自行退出——钩子仍然是一次性的。
    const REMOTE_FOREGROUND_SETTLE_MS: u64 = 300;
    const REARM_INTERVAL_MS: u64 = 3000;
    const GUARD_TICK_MS: u64 = 300;

    /// 会「获得前台就吞键」的远控客户端进程名（小写比较）
    ///
    /// 只需覆盖「本机作为主控端」的常见客户端；列表外的软件不会重装钩子，
    /// 退化为原有行为（收不到按键）。新增客户端加进这个数组即可。
    const REMOTE_CLIENTS: &[&str] = &[
        "todesk.exe",
        "sunloginclient.exe",
        "awesun.exe",
        "rustdesk.exe",
        "anydesk.exe",
        "teamviewer.exe",
        "gameviewer.exe",
        "uuremote.exe",
        "uu_remote.exe",
        "mstsc.exe",
        "msrdc.exe",
        "vncviewer.exe",
        "tvnviewer.exe",
        "strwinclt.exe",
        "nxplayer.exe",
        "nxplayer.bin",
        "anyviewer.exe",
        "parsecd.exe",
        "moonlight.exe",
    ];

    /// 是否已经提示过「权限不够」——每个进程只提示一次，避免每 3 秒刷屏
    static ELEVATION_HINTED: AtomicBool = AtomicBool::new(false);

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

    /// 当前会话支持「Ctrl+V 注入」交付方式（Windows 恒支持）
    ///
    /// 供 copy_password_to_clipboard 决定交付方式：false 时回退剪贴板交付，
    /// 否则 pending 永远等不到 Ctrl+V，密码彻底交不出去。
    pub fn injection_supported() -> bool {
        true
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
                if install_hook() {
                    IS_ACTIVE.store(true, Ordering::SeqCst);
                    SHUTTING_DOWN.store(false, Ordering::SeqCst);
                    log::info!(
                        "Ctrl+V 钩子已激活，句柄={:?}",
                        HOOK_HANDLE.load(Ordering::SeqCst)
                    );

                    // 关键：运行消息循环（钩子依赖消息循环）；deactivate 发 WM_QUIT 使其返回 0 退出
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        // 重装请求：远控客户端抢到前台后，重装才能排到它前面。
                        // 必须在钩子线程上重装（SetWindowsHookEx 挂到哪个线程的
                        // 消息队列由调用线程决定），所以用线程消息而不是别的线程直接装。
                        if msg.message == WM_REARM {
                            install_hook();
                            continue;
                        }
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
                } else {
                    // IS_ACTIVE 已在 activate 里同步置位，失败必须复位
                    IS_ACTIVE.store(false, Ordering::SeqCst);
                    SHUTTING_DOWN.store(false, Ordering::SeqCst);
                    HOOK_THREAD_ID.store(0, Ordering::SeqCst);
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

        // 远控客户端守望：远控窗口在前台时持续把钩子重装到它前面
        //（ToDesk 等会吞掉物理按键，见 REMOTE_CLIENTS 的说明）
        std::thread::spawn(spawn_remote_client_guard);

        log::info!("钩子线程已启动");
    }

    /// 安装（或重装）低级键盘钩子——**只能在钩子线程上调用**
    ///
    /// 重装是「抢到远控软件前面」的唯一手段：低级钩子后装先调用，没有调优先级的 API。
    /// 先卸旧句柄再装新的；装失败也不留旧句柄（旧句柄若已被顶掉，留着只是幻觉）。
    ///
    /// # Returns
    /// 是否装成功。失败时 `HOOK_HANDLE` 为空，钩子线程仍继续跑（下一轮重装可能成功）。
    unsafe fn install_hook() -> bool {
        let old = HOOK_HANDLE.swap(ptr::null_mut(), Ordering::SeqCst);
        if !old.is_null() {
            let _ = UnhookWindowsHookEx(HHOOK(old));
        }
        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) {
            Ok(h) => {
                HOOK_HANDLE.store(h.0, Ordering::SeqCst);
                true
            }
            Err(e) => {
                log::error!("注册 Ctrl+V 钩子失败: {e}");
                false
            }
        }
    }

    /// 请求钩子线程重装钩子（线程消息，不在调用线程直接装）
    fn request_rearm() {
        let tid = HOOK_THREAD_ID.load(Ordering::SeqCst);
        if tid == 0 {
            return;
        }
        unsafe {
            if let Err(e) = PostThreadMessageW(tid, WM_REARM, WPARAM(0), LPARAM(0)) {
                log::warn!("投递重装钩子消息失败: {e}");
            }
        }
    }

    /// 前台窗口的进程名（小写；归因失败返回 None）
    ///
    /// 每 300ms 会被调用一次，所以**不写日志**——失败就当作「不是远控客户端」。
    /// 归因只需要进程名，不要路径、不落盘。
    unsafe fn foreground_process_name(hwnd: HWND) -> Option<String> {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok || size == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        std::path::Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
    }

    /// 当前进程是否以管理员（高完整性）运行
    ///
    /// 与「重装钩子」是两件事，必须分开判断：远控客户端常以管理员运行，
    /// UIPI 下普通权限进程**既收不到它前台时的按键，也不能向它发送输入**，
    /// 这种情况钩子装得再晚也没用。只用于提示，不改变任何行为。
    fn is_elevated() -> bool {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Security::{
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION::default();
            let mut len: u32 = 0;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(token);
            ok && elevation.TokenIsElevated != 0
        }
    }

    /// 远控客户端前台守望：保证钩子始终排在远控软件自己的钩子前面
    ///
    /// 只在粘贴会话进行中工作，会话结束（`IS_ACTIVE` 为 false）自行退出；
    /// 插件语义不变——钩子仍然只在复制密码后的那几十秒存在。
    fn spawn_remote_client_guard() {
        use std::time::{Duration, Instant};

        // 上次重装时的前台窗口（用来识别「刚切进远控窗口」）
        let mut last_hwnd: isize = 0;
        let mut last_rearm = Instant::now();

        loop {
            std::thread::sleep(Duration::from_millis(GUARD_TICK_MS));
            if !IS_ACTIVE.load(Ordering::SeqCst) {
                return;
            }
            // 没有粘贴状态时远控吞不吞键与我们无关，且此时钩子本来就该卸了
            if !has_paste_state() {
                last_hwnd = 0;
                continue;
            }
            let hwnd = unsafe { GetForegroundWindow() };
            if hwnd.is_invalid() {
                continue;
            }
            let Some(name) = (unsafe { foreground_process_name(hwnd) }) else {
                continue;
            };
            if !REMOTE_CLIENTS.contains(&name.as_str()) {
                last_hwnd = 0;
                continue;
            }

            let current = hwnd.0 as isize;
            if current != last_hwnd {
                // 刚切进远控窗口：等它先把自己的钩子装完，我们再压上去
                std::thread::sleep(Duration::from_millis(REMOTE_FOREGROUND_SETTLE_MS));
                log::info!("[HOOK] 远控客户端在前台（{name}），重装钩子以排到其前面");
                request_rearm();
                last_hwnd = current;
                last_rearm = Instant::now();

                // 权限不够时重装也没用，提示一次（用户需要把「以管理员身份运行」打开）
                if !is_elevated() && !ELEVATION_HINTED.swap(true, Ordering::SeqCst) {
                    log::warn!(
                        "[HOOK] 当前进程未提权，若 {name} 以管理员运行，UIPI 会导致\
                         收不到它的前台按键——请在设置里开启「以管理员身份运行」"
                    );
                }
            } else if last_rearm.elapsed() >= Duration::from_millis(REARM_INTERVAL_MS) {
                // 补装：远控软件可能在中途重装了自己的钩子，需要持续压着
                request_rearm();
                last_rearm = Instant::now();
            }
        }
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
                // 「自动」交付模式：目标进程在前台时放行原生粘贴（密码此刻在剪贴板里）。
                // 这里绝不能拦截——远控客户端不转发合成按键，拦下来走注入反而送不出去
                if crate::auto_clipboard::ctrl_v_pass_through() {
                    log::info!("[HOOK] 自动模式：目标进程在前台，放行 Ctrl+V 给原生粘贴");
                } else {
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
        // 「自动」交付模式：目标进程在前台时回放给原生粘贴（密码此刻在剪贴板里），
        // 与 Windows 侧语义一致（见 auto_clipboard.rs）
        if crate::auto_clipboard::ctrl_v_pass_through() {
            log::info!("[HOOK] 自动模式：目标进程在前台，放行 Ctrl+V 给原生粘贴");
            replay_key(conn);
            return;
        }
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

    /// 当前会话支持「Ctrl+V 注入」交付方式（仅 X11；Wayland 无全局输入接口）
    ///
    /// 供 copy_password_to_clipboard 决定交付方式：false 时回退剪贴板交付。
    pub fn injection_supported() -> bool {
        crate::x11::is_x11_session()
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

    /// 该平台不支持「Ctrl+V 注入」交付方式（回退剪贴板交付）
    pub fn injection_supported() -> bool {
        false
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
