//! 快捷键命令模块
//!
//! 处理快捷键配置的读取和保存。
//!
//! # 命令列表
//!
//! - `get_shortcuts`: 获取快捷键配置
//! - `save_shortcuts`: 保存快捷键配置

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

/// 快捷键配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutConfig {
    /// 快速搜索快捷键
    pub quick_search: String,
    /// 快速添加快捷键
    pub quick_add: String,
    /// 密码生成器快捷键
    pub password_generator: String,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            quick_search: "CmdOrCtrl+K".to_string(),
            quick_add: "CmdOrCtrl+Shift+N".to_string(),
            password_generator: "CmdOrCtrl+Shift+G".to_string(),
        }
    }
}

/// 获取快捷键配置
///
/// 从数据库读取快捷键配置，如果不存在则返回默认值。
///
/// # Arguments
///
/// * `state` - 应用状态
///
/// # Returns
///
/// 快捷键配置
///
/// # 前端调用
///
/// ```typescript
/// const shortcuts = await invoke('get_shortcuts');
/// ```
#[tauri::command]
pub async fn get_shortcuts(state: State<'_, AppState>) -> Result<ShortcutConfig, String> {
    let conn = state.db.conn();

    // 查询快捷键配置
    let result = conn.query_row(
        "SELECT value FROM settings WHERE key = 'shortcuts'",
        [],
        |row| {
            let value: String = row.get(0)?;
            Ok(value)
        },
    );

    match result {
        Ok(json) => {
            // 解析 JSON
            serde_json::from_str(&json).map_err(|e| e.to_string())
        }
        Err(_) => {
            // 返回默认配置
            Ok(ShortcutConfig::default())
        }
    }
}

/// 保存快捷键配置
///
/// 将快捷键配置保存到数据库。
///
/// # Arguments
///
/// * `shortcuts` - 快捷键配置
/// * `state` - 应用状态
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
///
/// # 前端调用
///
/// ```typescript
/// await invoke('save_shortcuts', {
///   shortcuts: {
///     quick_search: 'CmdOrCtrl+K',
///     quick_add: 'CmdOrCtrl+Shift+N',
///     password_generator: 'CmdOrCtrl+Shift+G'
///   }
/// });
/// ```
#[tauri::command]
pub async fn save_shortcuts(
    shortcuts: ShortcutConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conn = state.db.conn();

    // 序列化为 JSON
    let json = serde_json::to_string(&shortcuts).map_err(|e| e.to_string())?;

    // 保存到数据库
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('shortcuts', ?1)",
        params![json],
    ).map_err(|e| e.to_string())?;

    Ok(())
}
