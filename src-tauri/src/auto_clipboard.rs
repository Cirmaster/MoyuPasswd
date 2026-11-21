//! 「自动」交付模式：目标进程在前台时才把密码写进剪贴板
//!
//! # 为什么需要这个模式
//!
//! ToDesk 这类远控客户端不转发合成按键（SendInput 打的字进不了远端），注入在它们
//! 面前永远送不进去；但它们的剪贴板同步是通的。于是「自动」模式这样安排密码的去向：
//! 平时密码不落剪贴板（照常 Ctrl+V 注入），**只在名单内的目标进程获得前台时**写进
//! 剪贴板（远端靠剪贴板同步 + 原生粘贴拿到密码），失焦立刻抹掉。在名单内外来回
//! 切换，剪贴板就跟着来回「写入 / 清除」。
//!
//! # 数据安全边界（这三条是本模块的存在意义）
//!
//! - 写入只发生在目标进程在前台时，其余时间剪贴板里没有密码；
//! - 失焦 / 会话结束立刻清除，且只删「仍是本应用写入的那份」（指纹比对，见
//!   [`crate::clipboard::clear_if_still_ours`]）——用户复制了别的内容绝不误删；
//! - 明文只存在于「解密后到写入前」这几微秒，写完立刻 zeroize
//!   （[`crate::inject::zeroize_plaintext`]），日志里永不出现明文。
//!
//! # 实现要点
//!
//! - 守望线程每 [`TICK`] 看一次前台进程（[`crate::inject::foreground_process_name`]，
//!   与注入的终端黑名单同一套归因、同一个进程名口径）；一次复制一条线程，会话结束
//!   （TTL 到期 / Ctrl+C / Esc / 被下一次复制顶掉）自行退出。
//! - 钩子在目标进程前台时放行 Ctrl+V（密码已在剪贴板，让原生粘贴生效）——见
//!   [`ctrl_v_pass_through`] 与 hotkey.rs 的调用点。拦下来走注入反而送不出去。
//! - 解密是延迟的（复用 pending 的解密闭包）：进入目标进程时才解密，复制时不明文落内存。

use std::sync::Mutex;
use std::time::Duration;

use crate::clipboard::SecretConsent;
use crate::pending::DecryptFn;

/// 前台巡检间隔
///
/// 200ms 是「切进目标窗口」到「密码进剪贴板」的最坏延迟；再短只是白耗 CPU——
/// 用户从切窗口到按 Ctrl+V 至少要几百毫秒。
const TICK: Duration = Duration::from_millis(200);

/// 当前守望会话（一次复制对应一个）
struct AutoSession {
    /// 会话代数（与 pending 的 seq 一致）：旧线程靠它发现自己已被顶掉
    seq: u64,
    /// 剪贴板目标进程名（匹配忽略大小写；配置里写形如 `ToDesk.exe` 的完整进程名）
    targets: Vec<String>,
    /// 剪贴板自动清除的兜底秒数
    ///
    /// 主清除路径是「失焦立刻删」和「会话结束删」，这里只是守望线程万一失灵时的
    /// 最后防线（复用 write_secret 的定时清除）。
    clear_seconds: u32,
}

static SESSION: Mutex<Option<AutoSession>> = Mutex::new(None);

/// 启动「自动」交付守望（由 `copy_password_to_clipboard` 在 auto 模式下调用）
///
/// # Arguments
/// - `seq`：本次 pending 会话代数，会话一变旧线程即退出
/// - `entry_id`：条目 ID（延迟解密的入参）
/// - `decrypt_fn`：延迟解密闭包，进入目标进程时才调用
/// - `targets`：剪贴板目标进程名单（完整进程名，忽略大小写）
/// - `clear_seconds`：剪贴板自动清除兜底秒数（同 pending TTL）
pub fn start(
    app: tauri::AppHandle,
    seq: u64,
    entry_id: String,
    decrypt_fn: DecryptFn,
    targets: Vec<String>,
    clear_seconds: u32,
) {
    let targets: Vec<String> = targets.iter().map(|t| t.to_lowercase()).collect();
    {
        let mut guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(AutoSession {
            seq,
            targets,
            clear_seconds,
        });
    }
    std::thread::spawn(move || watch(app, seq, entry_id, decrypt_fn));
}

/// Ctrl+V 是否放行给系统原生粘贴（钩子调用，必须便宜）
///
/// 「自动」模式下目标进程在前台 = 密码已在剪贴板（或正要写入），此时绝不能拦截：
/// 拦下来走注入反而送不出去（远控不转发合成按键），原生粘贴才是正确通道。
/// 其余情形返回 false，钩子照常走「注入 / 放行」的老逻辑。
pub fn ctrl_v_pass_through() -> bool {
    let guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    let Some(session) = guard.as_ref() else {
        return false;
    };
    match crate::inject::foreground_process_name() {
        Some(name) => session
            .targets
            .iter()
            .any(|t| t.eq_ignore_ascii_case(&name)),
        None => false,
    }
}

/// 守望主循环（独立线程，会话结束自行退出）
fn watch(app: tauri::AppHandle, seq: u64, entry_id: String, decrypt_fn: DecryptFn) {
    let mut in_target = false;
    loop {
        std::thread::sleep(TICK);

        // 会话还在吗？TTL 到期 / Ctrl+C / Esc / 被下一次复制顶掉都会让代数变掉
        let alive = crate::pending::get_active().map(|(_, s)| s) == Some(seq);
        if !alive {
            // 离开前兜个底：会话死在目标进程前台时，剪贴板里的密码也得抹掉
            if in_target {
                clear_password(&app, seq);
            }
            // 只收掉自己那份会话，别把新会话的配置抹了
            let mut guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            if guard.as_ref().map(|s| s.seq) == Some(seq) {
                *guard = None;
            }
            log::info!("[AUTO] 守望线程退出（会话结束）");
            return;
        }

        let is_target = match crate::inject::foreground_process_name() {
            Some(name) => target_hit(&name),
            None => false,
        };

        if is_target && !in_target {
            // 进入目标进程：此刻才解密，写完立刻清零
            match decrypt_fn(&entry_id) {
                Ok(plaintext) => {
                    let seconds = {
                        let guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
                        guard.as_ref().map(|s| s.clear_seconds).unwrap_or(30)
                    };
                    let result = crate::clipboard::write_secret(
                        &app,
                        &plaintext,
                        seconds,
                        SecretConsent::from_explicit_setting(),
                    );
                    crate::inject::zeroize_plaintext(plaintext);
                    match result {
                        Ok(()) => log::info!(
                            "[AUTO] 目标进程在前台，密码已放入剪贴板（{}s 兜底清除）",
                            seconds
                        ),
                        Err(e) => log::error!("[AUTO] 写剪贴板失败: {e}"),
                    }
                }
                Err(e) => log::error!("[AUTO] 解密失败（可能已锁定）: {e}"),
            }
        }
        if !is_target && in_target {
            // 离开目标进程：马上抹掉，不等定时清除
            if clear_password(&app, seq) {
                log::info!("[AUTO] 已离开目标进程，剪贴板中的密码已清除");
            }
        }
        in_target = is_target;
    }
}

/// 前台进程是否命中目标名单
fn target_hit(name: &str) -> bool {
    let guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .as_ref()
        .map(|s| s.targets.iter().any(|t| t.eq_ignore_ascii_case(name)))
        .unwrap_or(false)
}

/// 清除剪贴板里的密码（只删自己的；被新会话接管时不动）
///
/// # Returns
/// 是否真的清除了内容
fn clear_password(app: &tauri::AppHandle, seq: u64) -> bool {
    {
        let guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        // 新会话已接管（它的密码可能刚写进剪贴板）：绝不能替它清
        if let Some(s) = guard.as_ref() {
            if s.seq != seq {
                return false;
            }
        }
    }
    crate::clipboard::clear_if_still_ours(app)
}
