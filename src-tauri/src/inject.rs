//! 密码注入模块
//!
//! 负责将密码注入到目标窗口：归因（识别目标进程）→ 解密 → SendInput → zeroize

use std::time::SystemTime;

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
    ///
    /// # Returns
    /// 是否注入成功
    pub fn inject_text(text: &str, target_hwnd: HWND) -> bool {
        log::info!("[INJECT] 开始注入文本: 长度={}, 目标窗口={:?}", text.len(), target_hwnd);

        unsafe {
            // 确认目标窗口仍是前台，防止把密码打进别的应用
            if GetForegroundWindow() != target_hwnd {
                log::warn!("[INJECT] 前台窗口已不是目标窗口，放弃注入");
                return false;
            }

            // 释放修饰键，避免注入字符被目标应用当成组合键
            release_modifiers();

            // 注入文本
            let chars: Vec<u16> = text.encode_utf16().collect();
            log::info!("[INJECT] UTF-16 字符数: {}", chars.len());

            // 一次性把全部按键事件插入输入流，避免中途混入用户的物理按键
            let mut inputs: Vec<INPUT> = Vec::with_capacity(chars.len() * 2);
            for &ch in &chars {
                log::debug!("[INJECT] 构造字符 U+{:04X}", ch);

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
            if sent != inputs.len() as u32 {
                log::error!("[INJECT] SendInput 失败: 发送={}/{}, 错误={}",
                    sent, inputs.len(), std::io::Error::last_os_error());
                return false;
            }

            log::info!("[INJECT] 所有字符注入完成，共 {} 个字符", chars.len());
            true
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

#[cfg(not(windows))]
mod non_windows {
    use super::*;
    
    pub struct TargetProcess {
        pub pid: u32,
        pub name: String,
        pub path: String,
    }
    
    pub fn get_target_process() -> Option<TargetProcess> {
        log::warn!("非 Windows 平台进程归因待实现");
        None
    }
    
    pub fn inject_text(_text: &str) -> bool {
        log::error!("非 Windows 平台注入待实现");
        false
    }
}

#[cfg(windows)]
use win_impl::*;

#[cfg(not(windows))]
use non_windows::*;

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
            log::warn!("[INJECT] 失败：无待粘贴密码（未复制或已过期）");
            return;
        }
    };

    // 并发互斥：快速连按 Ctrl+V 时多线程同时 SendInput 会交织乱码；
    // 拿不到锁直接拒绝并记审计（不排队，避免密码被连续注入多次）
    let _guard = match INJECT_LOCK.try_lock() {
        Ok(g) => g,
        Err(_) => {
            log::warn!("[INJECT] 已有注入进行中，忽略本次请求");
            crate::pending::record_audit(crate::pending::InjectAuditEvent {
                ts: SystemTime::now(),
                seq,
                entry_id,
                action: crate::pending::AuditAction::Denied("busy".to_string()),
                target_pid: None,
                target_name: None,
                target_path: None,
            });
            return;
        }
    };

    // 2. 归因：获取目标进程（窗口 → PID → 进程名/路径）
    log::info!("[INJECT] 步骤 2：归因");
    let target = get_target_process();
    let (target_pid, target_name, target_path, target_hwnd) = match target {
        Some(t) => {
            log::info!("[INJECT] 目标进程: pid={}, name={}, path={}, hwnd={:?}", t.pid, t.name, t.path, t.hwnd);
            (Some(t.pid), t.name, Some(t.path), t.hwnd)
        }
        None => {
            log::warn!("[INJECT] 归因失败：无法获取目标进程");
            return;
        }
    };

    // 归因必须解析出真实进程名：解析失败时拒绝注入（fail closed），
    // 防止用空名/伪名绕过黑名单
    if target_name.is_empty() {
        log::warn!("[INJECT] 归因失败：无法解析进程名，拒绝注入");
        crate::pending::record_audit(crate::pending::InjectAuditEvent {
            ts: SystemTime::now(),
            seq,
            entry_id,
            action: crate::pending::AuditAction::Denied("attribution_failed".to_string()),
            target_pid,
            target_name: None,
            target_path,
        });
        return;
    }

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
            crate::pending::record_audit(crate::pending::InjectAuditEvent {
                ts: SystemTime::now(),
                seq,
                entry_id,
                action: crate::pending::AuditAction::Denied("blacklisted_confirm_required".to_string()),
                target_pid,
                target_name: Some(target_name),
                target_path,
            });
            return;
        }
        log::info!("[INJECT] 终端二次确认通过，放行注入: {}", target_name);
    }

    // 4. 调用解密闭包解密密码
    log::info!("[INJECT] 步骤 4：调用解密闭包");
    let plaintext = match decrypt_fn(&entry_id) {
        Ok(p) => {
            log::info!("[INJECT] 解密成功，长度={}", p.len());
            p
        }
        Err(e) => {
            log::error!("[INJECT] 解密密码失败: {}", e);
            crate::pending::record_audit(crate::pending::InjectAuditEvent {
                ts: SystemTime::now(),
                seq,
                entry_id,
                action: crate::pending::AuditAction::Denied(format!("decrypt_failed: {}", e)),
                target_pid,
                target_name: Some(target_name),
                target_path,
            });
            return;
        }
    };

    // 5. 注入密码（SendInput 模拟键盘输入，不动焦点）
    log::info!("[INJECT] 步骤 5：SendInput 注入");
    let success = inject_text(&plaintext, target_hwnd);
    log::info!("[INJECT] SendInput 结果: {}", success);
    let _ = target_path;

    // 6. 立即 zeroize 明文
    log::info!("[INJECT] 步骤 6：zeroize 清零");
    zeroize_plaintext(plaintext);

    // 7. 记审计事件
    let action = if success {
        crate::pending::AuditAction::Injected
    } else {
        crate::pending::AuditAction::Denied("inject_failed".to_string())
    };

    crate::pending::record_audit(crate::pending::InjectAuditEvent {
        ts: SystemTime::now(),
        seq,
        entry_id,
        action,
        target_pid,
        target_name: Some(target_name),
        target_path,
    });

    if success {
        log::info!("[INJECT] ========== 注入完成 ==========");
    } else {
        log::error!("[INJECT] ========== 注入失败 ==========");
    }
}

/// 默认注入黑名单
///
/// 终端/Shell 类进程：注入的密码可能回显在屏幕上或进 shell 历史文件（明文落盘）。
/// 注意 Windows 11 默认终端宿主是 WindowsTerminal.exe（cmd/powershell 都跑在它里面），
/// 归因拿到的是宿主进程名，因此宿主也必须在名单里才拦得住。
fn default_blacklist() -> Vec<String> {
    [
        "cmd.exe",
        "powershell.exe",
        "pwsh.exe",
        "WindowsTerminal.exe",
        "conhost.exe",
        "bash.exe",
        "mintty.exe",
        "wsl.exe",
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
    if let Ok(mut guard) = TERMINAL_GRANT.lock() {
        *guard = Some((process_name.to_string(), std::time::Instant::now()));
    }
}

/// 消费确认窗口：同一进程且未过期则放行一次（窗口一次性用掉）
fn consume_terminal_grant(process_name: &str) -> bool {
    if let Ok(mut guard) = TERMINAL_GRANT.lock() {
        if let Some((name, at)) = guard.take() {
            return name.eq_ignore_ascii_case(process_name) && at.elapsed() <= TERMINAL_GRANT_TTL;
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
    *APP_HANDLE.lock().unwrap() = Some(app);
}

/// 获取应用句柄
fn get_app_handle() -> Option<tauri::AppHandle> {
    APP_HANDLE.lock().unwrap().clone()
}

/// zeroize 明文（安全清零）
fn zeroize_plaintext(mut plaintext: String) {
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
