//! 密码注入模块
//!
//! 负责将密码注入到目标窗口：归因（识别目标进程）→ 解密 → SendInput → zeroize

use std::time::SystemTime;

#[cfg(windows)]
mod win_impl {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::HWND;

    /// 目标进程信息
    pub struct TargetProcess {
        pub pid: u32,
        pub name: String,
        pub path: String,
        pub hwnd: HWND,
    }

    /// 获取前台窗口对应的进程
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

            log::info!("[INJECT] 前台窗口句柄={:?}, PID={}", hwnd, pid);

            Some(TargetProcess {
                pid,
                name: format!("PID_{}", pid),
                path: String::new(),
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

    // 2. 归因：获取目标进程
    log::info!("[INJECT] 步骤 2：归因");
    let target = get_target_process();
    let (target_pid, target_name, target_path, target_hwnd) = match target {
        Some(t) => {
            log::info!("[INJECT] 目标进程: pid={}, name={}, hwnd={:?}", t.pid, t.name, t.hwnd);
            (Some(t.pid), Some(t.name), Some(t.path), t.hwnd)
        }
        None => {
            log::warn!("[INJECT] 归因失败：无法获取目标进程");
            return;
        }
    };
    let _ = target_path;

    // 3. 策略检查（黑名单等）
    log::info!("[INJECT] 步骤 3：策略检查");
    if let Some(ref name) = target_name {
        if is_blacklisted(name) {
            log::warn!("[INJECT] 目标进程在黑名单中，拒绝注入: {}", name);
            crate::pending::record_audit(crate::pending::InjectAuditEvent {
                ts: SystemTime::now(),
                seq,
                entry_id,
                action: crate::pending::AuditAction::Denied("blacklisted".to_string()),
                target_pid,
                target_name,
                target_path,
            });
            return;
        }
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
                target_name,
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
        target_name,
        target_path,
    });

    if success {
        log::info!("[INJECT] ========== 注入完成 ==========");
    } else {
        log::error!("[INJECT] ========== 注入失败 ==========");
    }
}

/// 检查进程名是否在黑名单
fn is_blacklisted(process_name: &str) -> bool {
    // TODO: 从配置读取黑名单
    let blacklist = vec!["cmd.exe", "powershell.exe", "pwsh.exe"];
    blacklist.iter().any(|&x| x.eq_ignore_ascii_case(process_name))
}

/// 从数据库解密密码（已废弃）
///
/// # Deprecated
/// 此函数已废弃，现在使用闭包解密
#[deprecated(note = "使用闭包解密")]
fn decrypt_password_from_db(_entry_id: &str) -> Result<String, String> {
    Err("已废弃：使用闭包解密".to_string())
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
