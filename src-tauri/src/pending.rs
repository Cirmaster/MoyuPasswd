//! 密码粘贴待处理槽
//!
//! 管理"复制密码"后的状态：登记待粘贴的密码条目、过期控制、审计事件。
//! 复制动作**不解密**，只记录 entry_id 和解密闭包，真正粘贴时才按需解密。
//! 倒计时（TTL）内可多次粘贴，每次注入都重新解密并立即清零；
//! 过期/锁定/再次复制/退出，以及用户按 Ctrl+C（复制别的东西）或 Esc（主动放弃）后作废。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use serde::{Deserialize, Serialize};

/// 解密函数类型
pub type DecryptFn = Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// 待粘贴密码
struct PendingClip {
    /// 密码条目 ID（只存 ID，不存明文）
    entry_id: String,
    /// 过期时间
    deadline: Instant,
    /// 代数计数（沿用 CLEAR_GENERATION 思想，防止旧任务误清新任务）
    seq: u64,
    /// 解密闭包（延迟解密，粘贴时才调用）
    decrypt_fn: DecryptFn,
}

/// 全局待粘贴状态
static PENDING: Mutex<Option<PendingClip>> = Mutex::new(None);
/// 代数计数器
static SEQ_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 审计事件动作类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditAction {
    /// 用户复制了密码（登记 pending）
    Pending,
    /// 密码已注入到目标窗口
    Injected,
    /// 拒绝注入
    Denied(String),
    /// 已过期
    Expired,
    /// 已作废（锁定/再次复制/手动取消）
    Revoked,
}

/// 注入审计事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectAuditEvent {
    /// 时间戳
    pub ts: SystemTime,
    /// 代数（第几次复制/注入）
    pub seq: u64,
    /// 密码条目 ID（不记明文！）
    pub entry_id: String,
    /// 动作
    pub action: AuditAction,
    /// 目标进程 PID
    pub target_pid: Option<u32>,
    /// 目标进程名
    pub target_name: Option<String>,
    /// 目标进程路径
    pub target_path: Option<String>,
}

/// 审计事件环形缓冲
static AUDIT_BUFFER: Mutex<Vec<InjectAuditEvent>> = Mutex::new(Vec::new());
/// 环形缓冲容量
const AUDIT_BUFFER_SIZE: usize = 1000;

/// 登记待粘贴密码（带解密闭包）
///
/// # Arguments
/// * `entry_id` - 密码条目 ID
/// * `ttl_seconds` - 有效期（秒）
/// * `decrypt_fn` - 解密闭包（延迟解密）
///
/// # Returns
/// 代数 seq
pub fn register_with_decrypt(entry_id: &str, ttl_seconds: u32, decrypt_fn: DecryptFn) -> u64 {
    let seq = SEQ_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let deadline = Instant::now() + Duration::from_secs(ttl_seconds.max(1) as u64);

    let mut pending = PENDING.lock().unwrap();
    // 再次复制：旧 pending 作废（沿用「再次复制后作废」语义，记 Revoked 审计）
    if let Some(old) = pending.take() {
        log::info!("[PENDING] 再次复制，作废旧 pending: seq={}", old.seq);
        record_audit(InjectAuditEvent {
            ts: SystemTime::now(),
            seq: old.seq,
            entry_id: old.entry_id,
            action: AuditAction::Revoked,
            target_pid: None,
            target_name: None,
            target_path: None,
        });
    }
    *pending = Some(PendingClip {
        entry_id: entry_id.to_string(),
        deadline,
        seq,
        decrypt_fn,
    });

    // 记审计事件
    record_audit(InjectAuditEvent {
        ts: SystemTime::now(),
        seq,
        entry_id: entry_id.to_string(),
        action: AuditAction::Pending,
        target_pid: None,
        target_name: None,
        target_path: None,
    });

    log::info!("[PENDING] 登记成功: entry_id={}, seq={}, ttl={}s, deadline={:?}", entry_id, seq, ttl_seconds, deadline);
    seq
}

/// 查询待粘贴密码（未过期）
///
/// # Returns
/// Some((entry_id, seq)) 如果存在且未过期
pub fn get_active() -> Option<(String, u64)> {
    let mut pending = PENDING.lock().unwrap();
    expire_if_stale_locked(&mut pending);
    if let Some(ref p) = *pending {
        let now = Instant::now();
        log::info!("[PENDING] 有效: entry_id={}, seq={}, 剩余={:?}", p.entry_id, p.seq, p.deadline - now);
        Some((p.entry_id.clone(), p.seq))
    } else {
        log::warn!("[PENDING] 无 pending（PENDING 是 None）");
        None
    }
}

/// 静默检查是否存在有效的待粘贴密码（供钩子高频路径使用，不刷日志）
pub fn has_active() -> bool {
    let mut pending = PENDING.lock().unwrap();
    expire_if_stale_locked(&mut pending);
    pending.is_some()
}

/// 取出待粘贴密码用于注入（**不消费**，倒计时内可多次粘贴）
///
/// 每次注入都通过解密闭包重新解密，注入后立即清零；
/// pending 保留到倒计时结束/作废为止。
///
/// # Returns
/// Some((entry_id, seq, decrypt_fn)) 如果存在且未过期
pub fn get_for_inject() -> Option<(String, u64, DecryptFn)> {
    let mut pending = PENDING.lock().unwrap();
    expire_if_stale_locked(&mut pending);
    if let Some(ref p) = *pending {
        let now = Instant::now();
        log::info!("[PENDING] 取出注入: entry_id={}, seq={}, 剩余={:?}", p.entry_id, p.seq, p.deadline - now);
        Some((p.entry_id.clone(), p.seq, Arc::clone(&p.decrypt_fn)))
    } else {
        None
    }
}

/// 清理已过期的 pending 并记 Expired 审计
///
/// # Arguments
/// * `pending` - 待清理槽（调用方需持有 PENDING 锁）
///
/// # Returns
/// 是否清理了过期条目
fn expire_if_stale_locked(pending: &mut Option<PendingClip>) -> bool {
    if let Some(ref p) = *pending {
        if Instant::now() >= p.deadline {
            let entry_id = p.entry_id.clone();
            let seq = p.seq;
            *pending = None;

            record_audit(InjectAuditEvent {
                ts: SystemTime::now(),
                seq,
                entry_id,
                action: AuditAction::Expired,
                target_pid: None,
                target_name: None,
                target_path: None,
            });
            log::info!("[PENDING] 已过期，清理: seq={}", seq);
            return true;
        }
    }
    false
}

/// 作废待粘贴密码
pub fn revoke(reason: &str) {
    let mut pending = PENDING.lock().unwrap();
    if let Some(p) = pending.take() {
        record_audit(InjectAuditEvent {
            ts: SystemTime::now(),
            seq: p.seq,
            entry_id: p.entry_id,
            action: AuditAction::Revoked,
            target_pid: None,
            target_name: None,
            target_path: None,
        });
        log::info!("作废待粘贴密码: reason={}", reason);
    }
}

/// 检查是否过期并清理
pub fn cleanup_expired() {
    let mut pending = PENDING.lock().unwrap();
    expire_if_stale_locked(&mut pending);
}

/// 记录审计事件
pub fn record_audit(event: InjectAuditEvent) {
    let mut buffer = AUDIT_BUFFER.lock().unwrap();

    // 环形缓冲：满了就丢弃最旧的
    if buffer.len() >= AUDIT_BUFFER_SIZE {
        buffer.remove(0);
    }
    buffer.push(event.clone());

    // 持久化到审计日志文件
    crate::audit::write_to_file(&event);

    // 输出到日志
    log::info!("[AUDIT] seq={} entry={} action={:?} target={:?}",
        event.seq, event.entry_id, event.action,
        event.target_name.clone().unwrap_or_default());
}

/// 获取审计事件列表（供前端查询）
pub fn get_audit_events(limit: usize) -> Vec<InjectAuditEvent> {
    let buffer = AUDIT_BUFFER.lock().unwrap();
    let start = if buffer.len() > limit { buffer.len() - limit } else { 0 };
    buffer[start..].to_vec()
}

/// 清空审计缓冲
pub fn clear_audit() {
    let mut buffer = AUDIT_BUFFER.lock().unwrap();
    buffer.clear();
}
