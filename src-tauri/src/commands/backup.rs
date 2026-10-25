//! 备份与恢复命令模块
//!
//! 暴露「设置 → 备份」页签所需的两个命令：
//! - `get_backups`: 列出可恢复的历史副本（旧库隔离 / 恢复前的当前库 / 改密前备份）
//! - `restore_backup`: 用选中的副本替换当前库
//!
//! # 安全约束
//!
//! - 仅主窗口可调用：这两个命令会整体替换密码库，小窗不得触达
//! - 必须已解锁：锁定状态下不允许改写磁盘上的库文件
//! - 恢复后统一走 `lockdown`：meta/密钥可能与新库不匹配，必须让用户
//!   用该库当时的主密码重新解锁，绝不能沿用内存里的旧密钥
//! - 文件名走 `db::validate_backup_name` 白名单校验，不接受任意路径

use tauri::State;

use crate::db;
use crate::response::{respond, ApiResult};
use crate::state::AppState;

/// 列出可恢复的历史备份（按时间倒序）
#[tauri::command]
pub async fn get_backups(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
) -> ApiResult<Vec<db::BackupEntry>> {
    respond(|| -> Result<Vec<db::BackupEntry>, String> {
        // 仅主窗口：备份列表含历史库元信息，小窗无此需求
        if window.label() != "main" {
            return Err("该操作仅允许在主窗口执行".to_string());
        }
        crate::commands::auth::require_unlocked(&state)?;

        Ok(db::list_backups(&state.app_dir))

    })}

/// 用选中的备份恢复数据库
///
/// 恢复成功后应用会被锁定（前端收到 `app-locked` 后跳解锁页），
/// 用户需用**该备份当时的主密码**解锁。
#[tauri::command]
pub async fn restore_backup(
    file_name: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        if window.label() != "main" {
            return Err("该操作仅允许在主窗口执行".to_string());
        }
        crate::commands::auth::require_unlocked(&state)?;

        // 先做纯字符串校验：非法文件名直接拒绝，避免「校验失败前就把用户锁了」
        db::validate_backup_name(&file_name)?;

        // 必须先关掉数据库连接：Windows 下被进程打开的文件无法替换，
        // 且恢复后旧连接指向的已是旧库文件，继续用会写坏数据
        state.clear_aes_key();

        db::restore_backup(&state.app_dir, &file_name)?;

        // 统一锁定入口：作废 pending、卸钩子、清剪贴板、通知所有窗口跳解锁页
        crate::commands::auth::lockdown(&state, &app, "restore-backup");

        Ok(())

    })}
