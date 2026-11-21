//! 密码注入模块
//!
//! 负责将密码注入到目标窗口：归因（识别目标进程）→ 解密 → SendInput → zeroize
//!
//! 注入失败不再只是「吞掉按键 + 写一行看不见的日志」：失败原因会带回调用方，
//! 通过 `inject-result` 事件告知前端（倒计时气泡扩成提示条）。
//!
//! **失败不自动写剪贴板**：剪贴板任何进程都能读，自动改投属于程序单方面降级。
//! 提示条给出「放入剪贴板」入口，用户点一下才走
//! `commands::copy_pending_password_to_clipboard`（一次点击一次授权，见 `fail` 的注释）。

use std::time::SystemTime;

use crate::pending::AuditAction;

/// 注入失败原因（全平台共用）
///
/// 早先这里返回 `bool`，调用方无法区分「没打到目标窗口」和「系统拒绝模拟输入」，
/// 失败只能写一行日志（release 构建根本没装 logger），用户侧表现为按键被吞。
///
/// 失败原因决定后续处置：目标窗口没了可以降级到剪贴板交付，平台不支持只能报错，
/// 而系统错误码（如 UIPI 拒绝 = 5）是排障的唯一线索。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectError {
    /// 前台窗口已不是归因时的目标窗口（用户切走了）
    TargetGone,
    /// 目标窗口句柄被复用给其他进程（归因后原窗口已销毁）
    TargetReused,
    /// 事件没进输入流；`u32` 为系统错误码（0 = 平台没有对应错误码）
    SendInputFailed(u32),
    /// 字符无法映射到当前键盘布局（X11 特有：XTest 没有 Unicode 直注）
    KeymapUnmappable,
    /// 当前平台或会话不支持注入（Wayland / macOS 桩）
    Unsupported,
}

impl InjectError {
    /// 稳定原因码：审计与前端提示都按它映射文案，文案可以改，码不能改
    pub fn code(&self) -> &'static str {
        match self {
            Self::TargetGone => "target_gone",
            Self::TargetReused => "target_reused",
            Self::SendInputFailed(_) => "send_input_failed",
            Self::KeymapUnmappable => "keymap_unmappable",
            Self::Unsupported => "unsupported",
        }
    }

    /// 系统错误码（仅 SendInput 失败时有；0 视为没有）
    pub fn os_error(&self) -> Option<u32> {
        match self {
            Self::SendInputFailed(code) if *code != 0 => Some(*code),
            _ => None,
        }
    }
}

#[cfg(windows)]
mod win_impl {
    use std::path::Path;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::core::PWSTR;

    use super::InjectError;

    /// 目标进程信息
    pub struct TargetProcess {
        pub pid: u32,
        pub name: String,
        pub path: String,
        pub hwnd: HWND,
    }

    /// 获取前台窗口对应的进程
    ///
    /// 归因链：前台窗口 → PID → 进程路径/进程名（QueryFullProcessImageNameW）。
    /// 进程名/路径解析失败时返回空字符串，由 `trigger_inject` 拒绝注入（fail closed）。
    pub fn get_target_process() -> Option<TargetProcess> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_invalid() {
                log::warn!("[INJECT] GetForegroundWindow 返回无效句柄");
                return None;
            }

            // 窗口 → PID
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                log::warn!("[INJECT] GetWindowThreadProcessId 返回 0");
                return None;
            }

            // PID → 进程路径/进程名
            let mut name = String::new();
            let mut path = String::new();
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(handle) => {
                    let mut buf = [0u16; 1024];
                    let mut size = buf.len() as u32;
                    let ok = QueryFullProcessImageNameW(
                        handle,
                        PROCESS_NAME_WIN32,
                        PWSTR(buf.as_mut_ptr()),
                        &mut size,
                    );
                    if ok.is_ok() {
                        path = String::from_utf16_lossy(&buf[..size as usize]);
                        name = Path::new(&path)
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                    } else {
                        log::warn!("[INJECT] QueryFullProcessImageNameW 失败: pid={}", pid);
                    }
                    let _ = CloseHandle(handle);
                }
                Err(e) => log::warn!("[INJECT] OpenProcess 失败: pid={}, {}", pid, e),
            }

            log::info!("[INJECT] 前台窗口句柄={:?}, PID={}, name={}", hwnd, pid, name);

            Some(TargetProcess {
                pid,
                name,
                path,
                hwnd,
            })
        }
    }
    
    /// SendInput 注入文本
    ///
    /// 注意事项（都是踩过的坑）：
    /// 1. 不要动键盘焦点：触发注入时目标窗口本来就是前台，键盘焦点就在用户
    ///    点选的输入框里。SetFocus(顶层窗口) 会把焦点从子输入框抢走，注入的
    ///    WM_CHAR 投给顶层窗口后被丢弃，表现为 SendInput 成功但没有字符上屏。
    /// 2. 不要泵消息：AttachThreadInput 之后消息队列是共享的，
    ///    PeekMessage(PM_REMOVE) 会把发往目标窗口的消息（包括刚注入的 WM_CHAR）
    ///    从队列里偷走，再从错误的线程 DispatchMessage，消息直接丢失。
    /// 3. 先合成释放修饰键：用户按 Ctrl+V 触发注入时 Ctrl 还物理按着，
    ///    不释放的话目标应用会把注入字符当成 Ctrl+组合键消费掉。
    ///
    /// # Arguments
    /// * `text` - 要注入的文本
    /// * `target_hwnd` - 目标窗口句柄
    /// * `target_pid` - 归因时的目标进程 ID（注入前复核，防句柄复用）
    ///
    /// # Returns
    /// 成功 = 全部按键事件已进输入流；失败带回具体原因（见 [`InjectError`]）
    pub fn inject_text(
        text: &str,
        target_hwnd: HWND,
        target_pid: u32,
    ) -> Result<(), InjectError> {
        log::info!("[INJECT] 开始注入文本: 长度={}, 目标窗口={:?}", text.len(), target_hwnd);

        unsafe {
            // 确认目标窗口仍是前台，防止把密码打进别的应用
            if GetForegroundWindow() != target_hwnd {
                log::warn!("[INJECT] 前台窗口已不是目标窗口，放弃注入");
                return Err(InjectError::TargetGone);
            }

            // 复核 hwnd 归属进程：归因后目标窗口可能已销毁、句柄被复用给其他进程，
            // 只比对句柄会把密码打进「顶着同一个 hwnd 的陌生进程」
            let mut current_pid: u32 = 0;
            GetWindowThreadProcessId(target_hwnd, Some(&mut current_pid));
            if target_pid == 0 || current_pid != target_pid {
                log::warn!(
                    "[INJECT] 目标窗口已易主（pid {} -> {}），放弃注入",
                    target_pid,
                    current_pid
                );
                return Err(InjectError::TargetReused);
            }

            // 释放修饰键，避免注入字符被目标应用当成组合键
            release_modifiers();

            // 注入文本（Zeroizing 包装：UTF-16 缓冲用完自动清零，不在内存里留密码）
            let chars: zeroize::Zeroizing<Vec<u16>> =
                zeroize::Zeroizing::new(text.encode_utf16().collect());
            log::info!("[INJECT] UTF-16 字符数: {}", chars.len());

            // 一次性把全部按键事件插入输入流，避免中途混入用户的物理按键
            let mut inputs: Vec<INPUT> = Vec::with_capacity(chars.len() * 2);
            for &ch in chars.iter() {
                // 绝不能把 ch 写进日志：ch 就是密码的一个码点，debug 构建下日志会落盘
                //（历史版本此处有一行 log::debug!("构造字符 U+{:04X}")，等于逐字符泄露密码）

                // 按下键
                inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0),
                            wScan: ch,
                            dwFlags: KEYEVENTF_UNICODE,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                });

                // 释放键
                inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0),
                            wScan: ch,
                            dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                });
            }

            let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            // 按键事件里含密码的 UTF-16 码点，发完就地清零缓冲
            for item in inputs.iter_mut() {
                *item = std::mem::zeroed();
            }
            if sent != inputs.len() as u32 {
                // 这里最常见的是 UIPI 拒绝：进程被降为低完整性、或目标窗口权限更高，
                // SendInput 返回 0 且 GetLastError = ERROR_ACCESS_DENIED(5)。
                // 错误码要原样带回，前端/审计才能区分「被系统拦了」和「窗口没了」
                let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32;
                log::error!("[INJECT] SendInput 失败: 发送={}/{}, 错误码={}",
                    sent, inputs.len(), code);
                return Err(InjectError::SendInputFailed(code));
            }

            log::info!("[INJECT] 所有字符注入完成，共 {} 个字符", chars.len());
            Ok(())
        }
    }

    /// 合成释放 Ctrl/Shift/Alt
    ///
    /// 用户按 Ctrl+V 触发注入时修饰键还物理按着，目标应用会把注入字符
    /// 当成组合键；先发一轮 keyup 让目标应用看到干净的按键状态。
    /// （用户随后物理松开修饰键时会多一次 keyup，无害）
    fn release_modifiers() {
        const MODIFIERS: &[VIRTUAL_KEY] = &[
            VK_CONTROL, VK_LCONTROL, VK_RCONTROL,
            VK_MENU, VK_LMENU, VK_RMENU,
            VK_SHIFT, VK_LSHIFT, VK_RSHIFT,
        ];

        let inputs: Vec<INPUT> = MODIFIERS
            .iter()
            .map(|&vk| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            })
            .collect();

        unsafe {
            let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            if sent != inputs.len() as u32 {
                log::warn!("[INJECT] 释放修饰键不完整: 发送={}/{}", sent, inputs.len());
            } else {
                log::info!("[INJECT] 已合成释放 Ctrl/Shift/Alt");
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod x11_impl {
    use x11rb::connection::Connection;
    use x11rb::protocol::xtest::ConnectionExt as _;

    use super::InjectError;

    /// 目标进程信息（hwnd 为 X11 窗口 id，字段与 win_impl 对齐）
    pub struct TargetProcess {
        pub pid: u32,
        pub name: String,
        pub path: String,
        pub hwnd: u32,
    }

    /// 获取前台窗口对应的进程（X11 实现）
    ///
    /// 归因链：EWMH `_NET_ACTIVE_WINDOW` → `_NET_WM_PID` → /proc 进程路径/名，
    /// 与 Windows 版「窗口 → PID → QueryFullProcessImageNameW」同构。
    /// Wayland 会话整体 fail closed（is_x11_session 判定），防止注入过期窗口。
    pub fn get_target_process() -> Option<TargetProcess> {
        if !crate::x11::is_x11_session() {
            log::warn!("[INJECT] 非 X11 会话，无法归因（Wayland 无全局窗口查询）");
            return None;
        }
        let (conn, screen_num) = crate::x11::connect()?;
        let root = conn.setup().roots[screen_num].root;

        let active = crate::x11::get_cardinal(&conn, root, "_NET_ACTIVE_WINDOW")?;
        if active == 0 {
            log::warn!("[INJECT] X11 无前台窗口");
            return None;
        }
        let pid = crate::x11::get_cardinal(&conn, active, "_NET_WM_PID").unwrap_or(0);

        // PID → 进程路径/名；解析失败留空串，由 trigger_inject 拒绝注入（fail closed）
        let mut path = String::new();
        let mut name = String::new();
        if pid > 0 {
            path = std::fs::read_link(format!("/proc/{pid}/exe"))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            name = std::path::Path::new(&path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| {
                    // exe 链接读不到（权限/已退出）时退回 comm
                    std::fs::read_to_string(format!("/proc/{pid}/comm"))
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default()
                });
        }

        log::info!("[INJECT] X11 前台窗口={}, PID={}, name={}", active, pid, name);
        Some(TargetProcess {
            pid,
            name,
            path,
            hwnd: active,
        })
    }

    /// XTest 注入文本
    ///
    /// 注意事项（对齐 Windows 版踩坑记录）：
    /// 1. 不动焦点：触发注入时目标窗口本来就是前台，X11 下同样不去碰焦点
    /// 2. 先合成释放修饰键：用户按 Ctrl+V 触发时 Ctrl 还物理按着，
    ///    不释放会被目标应用当成 Ctrl+组合键消费
    /// 3. XTest 没有 Unicode 直注（对不齐 Windows 的 KEYEVENTF_UNICODE），
    ///    字符必须按当前键盘映射折算 keycode；先整体解析、任一字符映射不到
    ///    就整次放弃，绝不打半截密码
    ///
    /// # Arguments
    /// * `text` - 要注入的文本
    /// * `target_hwnd` - 目标窗口 id（归因时记录）
    /// * `target_pid` - 归因时的目标进程 ID（注入前复核，防窗口易主）
    pub fn inject_text(text: &str, target_hwnd: u32, target_pid: u32) -> Result<(), InjectError> {
        if !crate::x11::is_x11_session() {
            log::error!("[INJECT] 非 X11 会话，注入不可用");
            return Err(InjectError::Unsupported);
        }
        let (conn, screen_num) = match crate::x11::connect() {
            Some(c) => c,
            None => return Err(InjectError::Unsupported),
        };
        let root = conn.setup().roots[screen_num].root;

        // 复核前台窗口归属（对齐 Windows 双复核语义）
        let active = crate::x11::get_cardinal(&conn, root, "_NET_ACTIVE_WINDOW").unwrap_or(0);
        if active != target_hwnd {
            log::warn!("[INJECT] 前台窗口已不是目标窗口，放弃注入");
            return Err(InjectError::TargetGone);
        }
        let current_pid = crate::x11::get_cardinal(&conn, active, "_NET_WM_PID").unwrap_or(0);
        if target_pid == 0 || current_pid != target_pid {
            log::warn!(
                "[INJECT] 目标窗口已易主（pid {} -> {}），放弃注入",
                target_pid,
                current_pid
            );
            return Err(InjectError::TargetReused);
        }

        // 先解析全部字符 → (keycode, shift)，任一失败整次放弃
        //（不能把 ch 写进日志：ch 就是密码的一个码点）
        let mut events: Vec<(u8, bool)> = Vec::with_capacity(text.len());
        for ch in text.chars() {
            let keysym = crate::x11::char_to_keysym(ch);
            match crate::x11::key_for_keysym(&conn, keysym) {
                Some(k) => events.push(k),
                None => {
                    log::error!("[INJECT] 字符无法映射到当前键盘布局（非 ASCII/AltGr 级），放弃注入");
                    return Err(InjectError::KeymapUnmappable);
                }
            }
        }
        let shift_kc = crate::x11::key_for_keysym(&conn, 0xffe1 /* XK_Shift_L */).map(|(kc, _)| kc);

        // 释放修饰键，避免注入字符被目标应用当成组合键
        release_modifiers(&conn);

        // 逐字符按下/抬起（按需 Shift）；发送完一次性 flush
        for (kc, shift) in &events {
            if *shift {
                if let Some(sk) = shift_kc {
                    fake_key(&conn, sk, true);
                }
            }
            fake_key(&conn, *kc, true);
            fake_key(&conn, *kc, false);
            if *shift {
                if let Some(sk) = shift_kc {
                    fake_key(&conn, sk, false);
                }
            }
        }
        if let Err(e) = conn.flush() {
            log::error!("[INJECT] XTest 发送失败: {e}");
            // X11 协议层没有 GetLastError，错误码位置记 0
            return Err(InjectError::SendInputFailed(0));
        }

        log::info!("[INJECT] X11 注入完成，共 {} 个字符", events.len());
        Ok(())
    }

    /// XTest 合成按键事件（press=true 按下 / false 抬起）
    ///
    /// FakeInput 的 type 字段用 X11 核心事件码：2=KeyPress、3=KeyRelease
    fn fake_key(conn: &impl x11rb::connection::Connection, keycode: u8, press: bool) {
        let ty = if press { 2u8 } else { 3u8 };
        // time=0（CurrentTime）、root/坐标=0、deviceid=0（核心键盘）
        let _ = conn.xtest_fake_input(ty, keycode, 0, 0, 0, 0, 0);
    }

    /// 合成释放 Ctrl/Shift/Alt/Super（对齐 Windows 版 release_modifiers 语义）
    fn release_modifiers(conn: &impl x11rb::connection::Connection) {
        // XK_Shift_L/R、XK_Control_L/R、XK_Alt_L/R(Meta)、XK_Super_L/R
        const MODIFIER_KEYSYMS: &[u32] = &[
            0xffe1, 0xffe2, 0xffe3, 0xffe4, 0xffe9, 0xffea, 0xffeb, 0xffec,
        ];
        let mut sent = 0;
        for &ks in MODIFIER_KEYSYMS {
            if let Some((kc, _)) = crate::x11::key_for_keysym(conn, ks) {
                fake_key(conn, kc, false);
                sent += 1;
            }
        }
        let _ = conn.flush();
        log::info!("[INJECT] 已合成释放修饰键 {} 个", sent);
    }
}

/// 非 Windows 且非 Linux 平台（macOS 等）桩：注入待实现
#[cfg(not(any(windows, target_os = "linux")))]
mod non_windows {
    use super::InjectError;

    /// 目标进程信息（字段与 `win_impl::TargetProcess` 对齐）
    pub struct TargetProcess {
        pub pid: u32,
        pub name: String,
        pub path: String,
        pub hwnd: u32,
    }

    pub fn get_target_process() -> Option<TargetProcess> {
        log::warn!("该平台进程归因待实现");
        None
    }

    /// 注入文本（桩：签名与 `win_impl::inject_text` 对齐，始终失败）
    pub fn inject_text(_text: &str, _target_hwnd: u32, _target_pid: u32) -> Result<(), InjectError> {
        log::error!("该平台注入待实现");
        Err(InjectError::Unsupported)
    }
}

#[cfg(windows)]
use win_impl::*;

#[cfg(target_os = "linux")]
use x11_impl::*;

#[cfg(not(any(windows, target_os = "linux")))]
use non_windows::*;

/// 前台窗口的进程名（归因失败返回 None）
///
/// `pub(crate)`：「自动」交付模式要判断前台是不是剪贴板目标进程（见 auto_clipboard.rs），
/// 必须与注入的终端黑名单用同一套归因、同一个进程名口径，各查各的迟早对不上。
pub(crate) fn foreground_process_name() -> Option<String> {
    get_target_process()
        .map(|t| t.name)
        .filter(|n| !n.is_empty())
}

/// 触发密码注入（钩子/热键回调）
pub fn trigger_inject() {
    log::info!("[INJECT] ========== 开始注入流程 ==========");

    // 1. 取出 pending（entry_id、seq 和解密闭包；倒计时内可多次粘贴，不消费）
    let (entry_id, seq, decrypt_fn) = match crate::pending::get_for_inject() {
        Some(x) => {
            log::info!("[INJECT] 取出 pending 成功: entry={:?}, seq={}", x.0, x.1);
            x
        }
        None => {
            // 没有待粘贴密码：用户没复制过，或会话已过期/作废。
            // 这是正常路径（会话结束后又按了一次 Ctrl+V），不提示也不降级
            log::warn!("[INJECT] 无待粘贴密码（未复制或已过期），忽略本次 Ctrl+V");
            return;
        }
    };

    // 失败收尾要同时写审计、发事件、可能降级，共用同一份上下文
    let mut ctx = InjectCtx::new(seq, entry_id);

    // 记一次触发（诊断用：会话到期时计数为 0 = 按键没到钩子，见 pending.rs）
    crate::pending::note_inject_attempt();

    // 并发互斥：快速连按 Ctrl+V 时多线程同时 SendInput 会交织乱码；
    // 拿不到锁直接拒绝并记审计（不排队，避免密码被连续注入多次）
    let _guard = match INJECT_LOCK.try_lock() {
        Ok(g) => g,
        Err(_) => {
            // 另一次注入正在进行，它自己会把结果报给前端；
            // 这里再报一条只会让用户看到两条互相矛盾的提示
            log::warn!("[INJECT] 已有注入进行中，忽略本次请求");
            ctx.audit(AuditAction::Denied("busy".to_string()));
            return;
        }
    };

    // 2. 归因：获取目标进程（窗口 → PID → 进程名/路径）
    log::info!("[INJECT] 步骤 2：归因");
    let target = match get_target_process() {
        Some(t) => {
            log::info!("[INJECT] 目标进程: pid={}, name={}, path={}, hwnd={:?}", t.pid, t.name, t.path, t.hwnd);
            t
        }
        None => {
            // 拿不到目标窗口：不注入（fail closed），也不猜用户想投到哪。
            // 但要把失败原因报出去——用户可以在提示条上点「放入剪贴板」显式放宽
            log::warn!("[INJECT] 归因失败：无法获取目标进程");
            fail(&ctx, "attribution_failed", None, None, true);
            return;
        }
    };

    ctx.target_pid = Some(target.pid);
    ctx.target_path = Some(target.path.clone());
    ctx.target_name = if target.name.is_empty() {
        None
    } else {
        Some(target.name.clone())
    };

    // 归因必须解析出真实进程名：解析失败时不得注入（fail closed），
    // 防止用空名/伪名绕过黑名单
    let Some(target_name) = ctx.target_name.clone() else {
        log::warn!("[INJECT] 归因失败：无法解析进程名，拒绝注入");
        fail(&ctx, "attribution_failed", None, None, true);
        return;
    };

    // 3. 策略检查（终端黑名单）
    // 默认拒绝向终端注入：密码落到命令行会进 shell 历史文件/回显在屏幕上。
    // 拒绝的同时打开 5 秒确认窗口，窗口内再次按 Ctrl+V 视为用户确认，放行一次；
    // 设置 allow_terminal_inject=true 可跳过本检查（设置页开关）。
    log::info!("[INJECT] 步骤 3：策略检查");
    if is_blacklisted(&target_name) && !allow_terminal_inject() {
        if !consume_terminal_grant(&target_name) {
            log::warn!("[INJECT] 目标进程在黑名单中，等待二次确认: {}", target_name);
            arm_terminal_grant(&target_name);
            notify_terminal_confirm(&target_name);
            ctx.audit(AuditAction::Denied(
                "blacklisted_confirm_required".to_string(),
            ));
            // 这里既不降级也不发 inject-result：提示条已由 terminal-inject-confirm 显示，
            // 而把密码放进剪贴板等于把终端保护整个绕开（终端里 Ctrl+Shift+V 照样能粘出来）
            return;
        }
        log::info!("[INJECT] 终端二次确认通过，放行注入: {}", target_name);
    }

    // 4. 调用解密闭包解密密码
    log::info!("[INJECT] 步骤 4：调用解密闭包");
    let plaintext = match decrypt_fn(&ctx.entry_id) {
        Ok(p) => {
            log::info!("[INJECT] 解密成功，长度={}", p.len());
            p
        }
        Err(e) => {
            log::error!("[INJECT] 解密密码失败: {}", e);
            // 没有明文就没有可交付的内容：连「放入剪贴板」也给不出，can_fallback = false
            fail(&ctx, "decrypt_failed", Some(e.clone()), None, false);
            return;
        }
    };

    // 5. 注入密码（SendInput 模拟键盘输入，不动焦点）
    log::info!("[INJECT] 步骤 5：SendInput 注入");
    let outcome = inject_text(&plaintext, target.hwnd, target.pid);
    log::info!(
        "[INJECT] SendInput 结果: ok={}, code={}, os_error={:?}",
        outcome.is_ok(),
        outcome.as_ref().err().map(|e| e.code()).unwrap_or("ok"),
        outcome.as_ref().err().and_then(|e| e.os_error())
    );

    // 6. 处理结果并立即 zeroize 明文
    //    — 成功：记审计即可，不弹提示（字符已上屏，提示只是噪音）
    //    — 失败：如实上报原因 + 提供「放入剪贴板」的授权入口，**不自动放宽**
    match outcome {
        Ok(()) => {
            ctx.audit(AuditAction::Injected);
            // pending 在整个倒计时内有效（多字段可反复 Ctrl+V），
            // 会话结束靠 TTL 到期 / Ctrl+C / Esc / 剪贴板哨兵（见 hotkey.rs）
            log::info!("[INJECT] ========== 注入完成 ==========");
        }
        Err(e) => {
            log::error!(
                "[INJECT] 注入失败: code={}, os_error={:?}",
                e.code(),
                e.os_error()
            );
            // pending 保留：用户可以直接再按一次 Ctrl+V 重试
            fail(&ctx, e.code(), None, e.os_error(), true);
            log::error!("[INJECT] ========== 注入失败 ==========");
        }
    }
    log::info!("[INJECT] 步骤 6：zeroize 清零");
    zeroize_plaintext(plaintext);
}

/// 一次注入的目标信息与代数
///
/// 失败收尾要同时写审计、发事件、可能降级，三处都需要同一份上下文；
/// 用结构体收拢，避免一路传五六个参数（也避免漏传目标信息导致审计对不上）。
struct InjectCtx {
    seq: u64,
    entry_id: String,
    target_pid: Option<u32>,
    target_name: Option<String>,
    target_path: Option<String>,
}

impl InjectCtx {
    fn new(seq: u64, entry_id: String) -> Self {
        Self {
            seq,
            entry_id,
            target_pid: None,
            target_name: None,
            target_path: None,
        }
    }

    /// 记一条审计事件（目标信息由上下文补齐；密码明文永不入审计）
    fn audit(&self, action: AuditAction) {
        crate::pending::record_audit(crate::pending::InjectAuditEvent {
            ts: SystemTime::now(),
            seq: self.seq,
            entry_id: self.entry_id.clone(),
            action,
            target_pid: self.target_pid,
            target_name: self.target_name.clone(),
            target_path: self.target_path.clone(),
        });
    }
}

/// 注入结果事件载荷（发给前端的 `inject-result`）
///
/// 约定与 `dnd-result` 一致：只报「送没送到、为什么没送到」，绝不带明文。
#[derive(Debug, Clone, serde::Serialize)]
pub struct InjectResult {
    /// 是否已把密码送进目标窗口
    pub ok: bool,
    /// 本次会话代数：前端「放入剪贴板」时回传，防止旧提示条对新会话授权
    pub seq: u64,
    /// 是否还能改为剪贴板交付
    ///
    /// 只有「密码已解密、目标没送到」或「归因失败（待粘贴槽仍在）」时为 true。
    /// 注意：这只是**提供入口**，真正写入剪贴板必须由用户点击授权
    /// （见 `copy_pending_password_to_clipboard`）。
    pub can_fallback: bool,
}

/// 失败收尾：记审计 + 通知前端（**不写剪贴板**）
///
/// # 为什么不在这里兜底写剪贴板
///
/// 剪贴板任何进程都能读，而注入只把字符送进目标窗口。失败时自动改投剪贴板，
/// 等于程序单方面把安全等级降了一档，且用户未必察觉。
/// 所以这里只负责「让失败可见 + 告知还能怎么补救」，真正写剪贴板由用户在提示条上
/// 点一下（`copy_pending_password_to_clipboard`），一次点击一次授权。
///
/// # Arguments
/// * `code` - 稳定原因码，前端按它映射文案
/// * `detail` - 写进审计的附加说明（如解密错误原文），**不含密码**
/// * `can_fallback` - 是否还有「放入剪贴板」这条路（待粘贴槽里还有密码时才有）
fn fail(
    ctx: &InjectCtx,
    code: &str,
    detail: Option<String>,
    os_error: Option<u32>,
    can_fallback: bool,
) {
    // 系统错误码（UIPI 拒绝 = 5 等）并进审计动作：事件载荷不带它之后，
    // 审计日志是唯一持久记录，排查环境问题（权限/拦截）就靠这一段
    let mut text = match detail {
        Some(d) => format!("{code}: {d}"),
        None => code.to_string(),
    };
    if let Some(e) = os_error {
        text.push_str(&format!(" (os_error={e})"));
    }
    ctx.audit(AuditAction::Denied(text));
    emit_result(InjectResult {
        ok: false,
        seq: ctx.seq,
        can_fallback,
    });
}

/// 把注入结果推给前端（倒计时气泡据此扩成提示条）
fn emit_result(result: InjectResult) {
    if let Some(app) = get_app_handle() {
        use tauri::Emitter;
        if let Err(e) = app.emit("inject-result", result) {
            log::warn!("[INJECT] 发送 inject-result 失败: {e}");
        }
    }
}

/// 默认注入黑名单
///
/// 终端/Shell 类进程：注入的密码可能回显在屏幕上或进 shell 历史文件（明文落盘）。
/// 注意 Windows 11 默认终端宿主是 WindowsTerminal.exe（cmd/powershell 都跑在它里面），
/// 归因拿到的是宿主进程名，因此宿主也必须在名单里才拦得住。
/// Linux 名单同理覆盖各桌面的终端（X11 注入链路用；其他平台无害，进程名不会撞）。
///
/// pub 给设置页展示/回退共用（get_settings 与 load_blacklist 必须同语义）。
pub fn default_blacklist() -> Vec<String> {
    [
        // Windows 终端 / Shell
        "cmd.exe",
        "powershell.exe",
        "pwsh.exe",
        "WindowsTerminal.exe",
        "conhost.exe",
        "bash.exe",
        "mintty.exe",
        "wsl.exe",
        // Linux 终端模拟器（含 GNOME 宿主进程）
        "xterm",
        "uxterm",
        "urxvt",
        "st",
        "kitty",
        "alacritty",
        "foot",
        "gnome-terminal",
        "gnome-terminal-server",
        "konsole",
        "xfce4-terminal",
        "lxterminal",
        "mate-terminal",
        "tilix",
        "terminator",
        "deepin-terminal",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// 加载注入黑名单
///
/// 从 settings 表读 `inject_blacklist`（逗号分隔的进程名）；
/// 读不到（未配置/未解锁）时回退默认黑名单。
fn load_blacklist() -> Vec<String> {
    let from_db = get_app_handle().and_then(|app| {
        use tauri::Manager;
        let state = app.state::<crate::state::AppState>();
        let db = state.get_db().ok()?;
        let conn = db.conn();
        conn.query_row(
            "SELECT value FROM settings WHERE key = 'inject_blacklist'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
    });

    match from_db {
        Some(raw) => {
            let list: Vec<String> = raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if list.is_empty() {
                default_blacklist()
            } else {
                list
            }
        }
        None => default_blacklist(),
    }
}

/// 检查进程名是否在黑名单（大小写不敏感）
fn is_blacklisted(process_name: &str) -> bool {
    load_blacklist()
        .iter()
        .any(|x| x.eq_ignore_ascii_case(process_name))
}

/// 是否允许向终端注入（设置项 `allow_terminal_inject`，默认 false）
///
/// 开启后跳过黑名单检查（不再二次确认），供明确知道自己在做什么的用户使用。
fn allow_terminal_inject() -> bool {
    get_app_handle()
        .and_then(|app| {
            use tauri::Manager;
            let state = app.state::<crate::state::AppState>();
            let db = state.get_db().ok()?;
            let conn = db.conn();
            conn.query_row(
                "SELECT value FROM settings WHERE key = 'allow_terminal_inject'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
        })
        .map(|v| v == "true")
        .unwrap_or(false)
}

/// 终端二次确认窗口（进程名 + 登记时刻）
///
/// 命中黑名单时先登记确认窗口并拒绝；窗口内再次 Ctrl+V 视为用户确认，放行一次。
static TERMINAL_GRANT: std::sync::Mutex<Option<(String, std::time::Instant)>> =
    std::sync::Mutex::new(None);

/// 确认窗口有效期：过期后需要重新走一次「拒绝 → 确认」流程
const TERMINAL_GRANT_TTL: std::time::Duration = std::time::Duration::from_secs(5);

/// 登记终端二次确认窗口
fn arm_terminal_grant(process_name: &str) {
    let mut guard = TERMINAL_GRANT.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some((process_name.to_string(), std::time::Instant::now()));
}

/// 消费确认窗口：同一进程且未过期则放行一次（窗口一次性用掉）
///
/// **先判后取**：进程名不匹配或已过期时不能消耗窗口——
/// 否则一次过期/串台的查询会吃掉合法用户的第二次确认，让他确认失效。
fn consume_terminal_grant(process_name: &str) -> bool {
    let mut guard = TERMINAL_GRANT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((name, at)) = guard.as_ref() {
        if name.eq_ignore_ascii_case(process_name) && at.elapsed() <= TERMINAL_GRANT_TTL {
            *guard = None;
            return true;
        }
    }
    false
}

/// 通知前端（倒计时窗口）提示用户二次确认
fn notify_terminal_confirm(process_name: &str) {
    if let Some(app) = get_app_handle() {
        use tauri::Emitter;
        let _ = app.emit(
            "terminal-inject-confirm",
            serde_json::json!({ "process": process_name }),
        );
    }
}

/// 应用句柄（读取设置中的黑名单用；由 lib.rs 启动时注入）
static APP_HANDLE: std::sync::Mutex<Option<tauri::AppHandle>> = std::sync::Mutex::new(None);

/// 注入互斥锁：同一时刻只允许一次注入在进行（防快速连按并发 SendInput 乱码）
static INJECT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 注入应用句柄（启动时调用一次）
pub fn set_app_handle(app: tauri::AppHandle) {
    *APP_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(app);
}

/// 获取应用句柄
fn get_app_handle() -> Option<tauri::AppHandle> {
    APP_HANDLE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// zeroize 明文（安全清零）
pub(crate) fn zeroize_plaintext(mut plaintext: String) {
    // 将字符串内容清零
    unsafe {
        let bytes = plaintext.as_bytes_mut();
        for b in bytes.iter_mut() {
            *b = 0;
        }
    }
    // Rust 的 String drop 时会释放内存，这里手动清零防止残留
    log::debug!("明文已清零");
}
