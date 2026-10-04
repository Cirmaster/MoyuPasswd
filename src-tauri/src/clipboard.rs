//! 剪贴板模块
//!
//! 提供带隐私保护的剪贴板操作：在 Windows 上写入文本时附带
//! `CanIncludeInClipboardHistory = 0` 和 `CanUploadToCloudClipboard = 0`
//! 两个剪贴板格式，避免敏感内容进入 Win+V 剪贴板历史或跨设备/云端同步。
//! 非 Windows 平台回退到 Tauri 剪贴板插件。
//!
//! 同时提供「后端兜底清除」：复制敏感内容后由后端定时清空剪贴板，
//! 不依赖前端倒计时窗口是否存活。

use std::sync::atomic::{AtomicU64, Ordering};
use tauri::Emitter;

/// 剪贴板清除任务的代数计数。
/// 每次调度新的清除任务时自增，旧任务醒来后若发现代数不匹配则放弃清除，
/// 从而避免「旧任务把新复制的内容误清」。
static CLEAR_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 写入文本到剪贴板，并排除剪贴板历史与云端同步（Windows）
pub fn write_text_excluded(app: &tauri::AppHandle, text: &str) -> Result<(), String> {
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
            .map_err(|e| e.to_string())
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
pub fn schedule_clear(app: &tauri::AppHandle, seconds: u32) {
    let gen = CLEAR_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(seconds.max(1) as u64));
        if CLEAR_GENERATION.load(Ordering::SeqCst) == gen {
            let _ = clear(&app);
            let _ = app.emit("clipboard-cleared", ());
        }
    });
}

/// 立即清空剪贴板，并使所有 pending 的清除任务失效
pub fn clear_now(app: &tauri::AppHandle) -> Result<(), String> {
    CLEAR_GENERATION.fetch_add(1, Ordering::SeqCst);
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
) -> Result<(), String> {
    write_text_excluded(&app, &text)?;
    let seconds = match clear_after {
        Some(0) => None,
        Some(n) => Some(n),
        None => Some(read_clipboard_clear_time(&state)),
    };
    if let Some(seconds) = seconds {
        schedule_clear(&app, seconds);
    }
    Ok(())
}

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
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    /// CF_UNICODETEXT（Unicode 文本剪贴板格式）
    const CF_UNICODETEXT: u32 = 13;

    /// 写入 UTF-16 文本并附带"排除历史/云端"格式
    pub fn write_text_excluded(text: &str) -> Result<(), String> {
        unsafe {
            OpenClipboard(None).map_err(|e| format!("打开剪贴板失败: {e}"))?;
            // 清空当前内容（不产生历史记录）
            let _ = EmptyClipboard();

            let result = write_unicode_text(text).and_then(|_| set_exclusion_formats());

            let _ = CloseClipboard();
            result
        }
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
        // UTF-16 + 结尾 NUL
        let mut utf16: Vec<u16> = text.encode_utf16().collect();
        utf16.push(0);
        let byte_len = utf16.len() * 2;

        let h = GlobalAlloc(GMEM_MOVEABLE, byte_len).map_err(|e| format!("分配内存失败: {e}"))?;
        let p = GlobalLock(h);
        if p.is_null() {
            return Err("锁定内存失败".to_string());
        }
        ptr::copy_nonoverlapping(utf16.as_ptr() as *const u8, p as *mut u8, byte_len);
        let _ = GlobalUnlock(h);

        SetClipboardData(CF_UNICODETEXT, HANDLE(h.0)).map_err(|e| format!("写入文本失败: {e}"))?;
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
            return Err("锁定内存失败".to_string());
        }
        *(p as *mut u32) = value;
        let _ = GlobalUnlock(h);

        SetClipboardData(fmt, HANDLE(h.0)).map_err(|_| "写入排除格式失败".to_string())?;
        Ok(())
    }
}
