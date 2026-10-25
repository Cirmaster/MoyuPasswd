//! 剪贴板模块
//!
//! 提供带隐私保护的剪贴板操作：在 Windows 上写入文本时附带
//! `CanIncludeInClipboardHistory = 0` 和 `CanUploadToCloudClipboard = 0`
//! 两个剪贴板格式，避免敏感内容进入 Win+V 剪贴板历史或跨设备/云端同步。
//! 非 Windows 平台回退到 Tauri 剪贴板插件。
//!
//! 同时提供「后端兜底清除」：复制敏感内容后由后端定时清空剪贴板，
//! 不依赖前端倒计时窗口是否存活。

use crate::response::{respond, ApiResult};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::Emitter;

/// 剪贴板清除任务的代数计数。
/// 每次调度新的清除任务时自增，旧任务醒来后若发现代数不匹配则放弃清除，
/// 从而避免「旧任务把新复制的内容误清」。
static CLEAR_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 最近一次经本应用写入剪贴板的文本指纹（SHA-256）。
///
/// 定时清除前读回剪贴板比对：内容若已不是本应用写入的文本
/// （用户右键复制了别的东西——不经 Ctrl+C 钩子，代数计数不会失效），
/// 跳过清除，避免把用户的新内容误删。
/// 只存指纹不存明文，避免敏感文本在内存中二次驻留。
static LAST_WRITTEN: std::sync::Mutex<Option<[u8; 32]>> = std::sync::Mutex::new(None);

/// 计算文本指纹（SHA-256）
fn fingerprint(text: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher.finalize().into()
}

/// 取 LAST_WRITTEN 锁（毒化时降级恢复，不连环 panic）
fn lock_last_written() -> std::sync::MutexGuard<'static, Option<[u8; 32]>> {
    LAST_WRITTEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// 写入文本到剪贴板，并排除剪贴板历史与云端同步（Windows）
///
/// 返回值：
/// - `Ok(true)`：文本已写入，且已成功附带「排除历史/云同步」标记
/// - `Ok(false)`：**软失败**——文本已写入剪贴板，但排除标记失败（内容可能进
///   Win+V 剪贴板历史/云同步）。调用方必须继续安排兜底清除，
///   绝不能因错误提前返回，否则明文会永久残留在剪贴板
/// - `Err(_)`：写入失败，文本未进入剪贴板
pub fn write_text_excluded(app: &tauri::AppHandle, text: &str) -> Result<bool, String> {
    #[cfg(windows)]
    {
        let _ = app;
        win_impl::write_text_excluded(text)
    }
    #[cfg(not(windows))]
    {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        app.clipboard()
            .write_text(text.to_string())
            .map_err(|e| e.to_string())?;
        // 非 Windows 无「排除历史」标记概念，视为已排除
        Ok(true)
    }
}

/// 读取当前剪贴板文本（定时清除前比对用，确认内容仍是本应用写入的）
pub fn read_text(app: &tauri::AppHandle) -> Result<String, String> {
    #[cfg(windows)]
    {
        let _ = app;
        win_impl::read_text()
    }
    #[cfg(not(windows))]
    {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        app.clipboard().read_text().map_err(|e| e.to_string())
    }
}

/// 清空剪贴板
///
/// Windows 下直接清空当前剪贴板内容（不会向历史追加空记录）；
/// 其他平台写空字符串。
pub fn clear(app: &tauri::AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = app;
        win_impl::clear()
    }
    #[cfg(not(windows))]
    {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        app.clipboard()
            .write_text(String::new())
            .map_err(|e| e.to_string())
    }
}

/// 调度剪贴板清除任务（后端兜底）
///
/// 在指定秒数后清空剪贴板，并通过 `clipboard-cleared` 事件通知前端。
/// 新任务会使旧任务失效，连续复制时以最后一次为准。
///
/// 清除前做内容比对：剪贴板若已被用户换成别的内容（右键复制不经 Ctrl+C 钩子，
/// 代数计数不会失效），跳过清除，避免误删用户的新内容。
pub fn schedule_clear(app: &tauri::AppHandle, seconds: u32) {
    let gen = CLEAR_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(seconds.max(1) as u64));
        if CLEAR_GENERATION.load(Ordering::SeqCst) != gen {
            return;
        }

        // 内容比对：确认剪贴板里仍是本应用最后写入的文本。
        // 读取失败时按「仍是我们的内容」处理（安全优先，宁可多清一次密码残留）。
        let expected = lock_last_written().take();
        let still_ours = match expected {
            Some(hash) => read_text(&app)
                .map(|t| fingerprint(&t) == hash)
                .unwrap_or(true),
            None => true,
        };
        if still_ours {
            let _ = clear(&app);
        }

        // 无论是否真正清除，都通知前端收掉倒计时 UI（本次复制的生命周期已结束）
        let _ = app.emit("clipboard-cleared", ());
    });
}

/// 立即清空剪贴板，并使所有 pending 的清除任务失效
///
/// **无条件清除、不做内容比对**：用于锁屏 / Esc 主动放弃等安全动作，
/// 目标是保证敏感残留被销毁，属于「宁可清掉用户内容」的场景。
pub fn clear_now(app: &tauri::AppHandle) -> Result<(), String> {
    CLEAR_GENERATION.fetch_add(1, Ordering::SeqCst);
    *lock_last_written() = None;
    clear(app)?;
    let _ = app.emit("clipboard-cleared", ());
    Ok(())
}

/// 作废所有 pending 的清除任务（**不清空**剪贴板）
///
/// 用户复制了新内容（如 Ctrl+C）时调用：旧的定时清除任务若照常触发，
/// 会把用户刚复制的新内容误清掉。自增代数让旧任务醒来后自行放弃。
pub fn invalidate_scheduled_clear() {
    CLEAR_GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// 复制文本到剪贴板（前端调用；排除历史与云端同步）
///
/// `clear_after` 语义：
/// - `None`：回退读取设置中的 `clipboard_clear_time`（后端兜底，防前端漏排清除）
/// - `Some(0)`：明确不自动清除
/// - `Some(n)`：n 秒后自动清空剪贴板（后端调度，不依赖前端倒计时窗口存活）
#[tauri::command]
pub async fn copy_text_to_clipboard(
    text: String,
    clear_after: Option<u32>,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        // 解锁门禁：锁定状态下不允许写系统剪贴板（防剪贴板劫持：篡改用户复制的内容）
        crate::commands::auth::require_unlocked(&state)?;

        // 文本复制与密码注入共用 Ctrl+V 语义：用户改为复制文本（用户名/自定义字段等）
        // 说明意图已切换，必须作废之前登记的待粘贴密码——否则 Ctrl+V 钩子会继续拦截
        // 按键注入密码，用户刚复制的文本被吞掉（粘贴出来的是密码）。
        crate::pending::revoke("text-copy");

        // 写入 + 历史/云同步排除标记。
        // Ok(false) 是软失败：文本已在剪贴板但缺排除标记（会进 Win+V 历史），
        // 此时绝不能提前返回——必须走到下面的清除调度，否则明文永久残留；
        // 软失败时把清除时间压到 5 秒兜底。
        let excluded = write_text_excluded(&app, &text)?;
        if !excluded {
            log::warn!("剪贴板排除标记设置失败，压缩自动清除时间兜底");
        }
        // 记录写入指纹：定时清除前比对，避免误删用户后续复制的无关内容
        *lock_last_written() = Some(fingerprint(&text));

        let mut seconds = match clear_after {
            Some(0) => None, // 明确不自动清除
            Some(n) => Some(n),
            // 设置值为 0 同样表示「不清除」，与 Some(0) 语义一致
            //（此前 0 会被 schedule_clear 钳成 1 秒即清，语义自相矛盾）
            None => {
                let t = read_clipboard_clear_time(&state);
                if t == 0 { None } else { Some(t) }
            }
        };
        // 软失败兜底：无论用户配置如何，最迟 5 秒必须清掉（内容已进剪贴板历史面）
        if !excluded {
            seconds = Some(seconds.unwrap_or(u32::MAX).min(5));
        }
        if let Some(seconds) = seconds {
            schedule_clear(&app, seconds);
        }

        // 挂上一次性 Ctrl+C/Esc 监察钩子：用户随后按 Ctrl+C 复制了别的内容时，
        // 销毁本次复制的倒计时与定时清除任务（否则定时清除会把用户的新内容误清）。
        // 钩子对 Ctrl+V 的拦截以「有待粘贴密码」为前提，文本复制场景不会劫持正常粘贴。
        crate::hotkey::activate(app);

        Ok(())

    })}

/// 读取剪贴板清除时间设置（秒），默认 30
fn read_clipboard_clear_time(state: &crate::state::AppState) -> u32 {
    state
        .get_db()
        .ok()
        .and_then(|db| {
            let conn = db.conn();
            conn.query_row(
                "SELECT value FROM settings WHERE key = 'clipboard_clear_time'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
}

#[cfg(windows)]
mod win_impl {
    use std::ptr;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GlobalFree, HANDLE};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    /// CF_UNICODETEXT（Unicode 文本剪贴板格式）
    const CF_UNICODETEXT: u32 = 13;

    /// 写入 UTF-16 文本并附带"排除历史/云端"格式
    ///
    /// 返回 `Ok(false)` = 软失败：文本已写入但排除标记失败（见外层同名函数注释）
    pub fn write_text_excluded(text: &str) -> Result<bool, String> {
        unsafe {
            OpenClipboard(None).map_err(|e| format!("打开剪贴板失败: {e}"))?;
            // 清空当前内容（不产生历史记录）
            let _ = EmptyClipboard();

            // 先写文本：失败 = 剪贴板里没有内容，直接报错
            if let Err(e) = write_unicode_text(text) {
                let _ = CloseClipboard();
                return Err(e);
            }
            // 排除标记失败 = 软失败：文本已在剪贴板，如实上报让外层安排兜底清除
            let excluded = set_exclusion_formats().is_ok();

            let _ = CloseClipboard();
            Ok(excluded)
        }
    }

    /// 读取当前剪贴板文本（清除前比对用）
    pub fn read_text() -> Result<String, String> {
        unsafe {
            OpenClipboard(None).map_err(|e| format!("打开剪贴板失败: {e}"))?;
            let result = read_unicode_text();
            let _ = CloseClipboard();
            result
        }
    }

    unsafe fn read_unicode_text() -> Result<String, String> {
        use windows::Win32::System::DataExchange::GetClipboardData;
        use windows::Win32::System::Memory::GlobalSize;

        let handle =
            GetClipboardData(CF_UNICODETEXT).map_err(|e| format!("读取剪贴板失败: {e}"))?;
        let hmem = windows::Win32::Foundation::HGLOBAL(handle.0);

        // 先取真实分配大小：剪贴板内容由任意进程写入，NUL 终止符只是「写入方应遵守的
        // 契约」而非保证。此前直接 while 扫描到 NUL，遇到不带终止符的恶意数据块会一路
        // 越界读到未映射页 —— 访问违例属 SEH 级异常，catch_unwind 兜不住，进程直接崩。
        let cap_bytes = GlobalSize(hmem);
        if cap_bytes < 2 {
            return Err("剪贴板数据为空或大小异常".to_string());
        }
        let max_units = cap_bytes / 2;

        let p = GlobalLock(hmem);
        if p.is_null() {
            return Err("锁定剪贴板内存失败".to_string());
        }
        // 在容量范围内扫描 NUL；找不到就按容量截断，绝不越界
        let units = p as *const u16;
        let mut len = 0usize;
        while len < max_units && *units.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(units, len);
        let text = String::from_utf16_lossy(slice);
        let _ = GlobalUnlock(hmem);
        Ok(text)
    }

    /// 清空剪贴板（不追加历史记录）
    pub fn clear() -> Result<(), String> {
        unsafe {
            OpenClipboard(None).map_err(|e| format!("打开剪贴板失败: {e}"))?;
            let _ = EmptyClipboard();
            let _ = CloseClipboard();
        }
        Ok(())
    }

    unsafe fn write_unicode_text(text: &str) -> Result<(), String> {
        // UTF-16 + 结尾 NUL；Zeroizing 保证本地副本用完清零（内容可能是用户名等敏感文本）
        let utf16: zeroize::Zeroizing<Vec<u16>> = {
            let mut v: Vec<u16> = text.encode_utf16().collect();
            v.push(0);
            zeroize::Zeroizing::new(v)
        };
        let byte_len = utf16.len() * 2;

        let h = GlobalAlloc(GMEM_MOVEABLE, byte_len).map_err(|e| format!("分配内存失败: {e}"))?;
        let p = GlobalLock(h);
        if p.is_null() {
            // 锁定失败必须释放刚分配的全局内存，否则 GMEM 句柄泄漏
            let _ = GlobalFree(h);
            return Err("锁定内存失败".to_string());
        }
        ptr::copy_nonoverlapping(utf16.as_ptr() as *const u8, p as *mut u8, byte_len);
        let _ = GlobalUnlock(h);

        // 失败路径同样要释放：SetClipboardData 成功即所有权移交系统，不再由我们释放
        if let Err(e) = SetClipboardData(CF_UNICODETEXT, HANDLE(h.0)) {
            let _ = GlobalFree(h);
            return Err(format!("写入文本失败: {e}"));
        }
        Ok(())
    }

    unsafe fn set_exclusion_formats() -> Result<(), String> {
        set_dword_format("CanIncludeInClipboardHistory", 0)?;
        set_dword_format("CanUploadToCloudClipboard", 0)?;
        Ok(())
    }

    unsafe fn set_dword_format(name: &str, value: u32) -> Result<(), String> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let fmt = RegisterClipboardFormatW(PCWSTR(wide.as_ptr()));
        if fmt == 0 {
            return Err("注册剪贴板格式失败".to_string());
        }

        let h = GlobalAlloc(GMEM_MOVEABLE, 4).map_err(|e| format!("分配内存失败: {e}"))?;
        let p = GlobalLock(h);
        if p.is_null() {
            let _ = GlobalFree(h);
            return Err("锁定内存失败".to_string());
        }
        *(p as *mut u32) = value;
        let _ = GlobalUnlock(h);

        if SetClipboardData(fmt, HANDLE(h.0)).is_err() {
            // 失败路径必须释放，否则 GMEM 句柄泄漏
            let _ = GlobalFree(h);
            return Err("写入排除格式失败".to_string());
        }
        Ok(())
    }
}
