//! 审计模块
//!
//! 密码注入审计事件的持久化、查询、推送

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};

/// 审计日志文件
static LOG_FILE: OnceLock<PathBuf> = OnceLock::new();

/// 初始化审计模块
///
/// # Arguments
/// * `app_dir` - 应用数据目录
pub fn init(app_dir: &PathBuf) {
    let log_path = app_dir.join("audit.log");
    let _ = LOG_FILE.set(log_path.clone());
    log::info!("审计日志: {:?}", log_path);
}

/// 写入审计事件到日志文件
///
/// # Arguments
/// * `event` - 审计事件
pub fn write_to_file(event: &crate::pending::InjectAuditEvent) {
    if let Some(path) = LOG_FILE.get() {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let ts = event.ts.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let line = format!(
                "{} seq={} entry={} action={:?} target_pid={:?} target_name={:?}\n",
                ts,
                event.seq,
                event.entry_id,
                event.action,
                event.target_pid,
                event.target_name
            );

            let _ = file.write_all(line.as_bytes());
        }
    }
}

/// 推送审计事件到前端（Tauri 事件）
///
/// # Arguments
/// * `app` - Tauri 应用句柄
/// * `event` - 审计事件
pub fn emit_to_frontend(app: &tauri::AppHandle, event: &crate::pending::InjectAuditEvent) {
    use tauri::Emitter;
    let _ = app.emit("password-inject-audit", event);
}

/// 查询审计事件
///
/// # Arguments
/// * `limit` - 返回数量上限
///
/// # Returns
/// 审计事件列表
pub fn query_events(limit: usize) -> Vec<crate::pending::InjectAuditEvent> {
    crate::pending::get_audit_events(limit)
}

/// 清空审计日志
pub fn clear_logs() {
    crate::pending::clear_audit();

    if let Some(path) = LOG_FILE.get() {
        let _ = std::fs::remove_file(path);
        log::info!("审计日志已清空");
    }
}
