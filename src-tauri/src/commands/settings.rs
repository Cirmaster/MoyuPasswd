//! 设置命令模块
//!
//! 处理应用设置的读取和保存。
//!
//! # 命令列表
//!
//! - `get_settings`: 获取所有设置
//! - `get_setting`: 获取单个设置
//! - `save_setting`: 保存单个设置
//! - `save_settings`: 批量保存设置

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::state::AppState;

/// 设置项结构体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// 主题（light/dark/system）
    pub theme: String,
    /// 语言（zh-CN/en-US）
    pub language: String,
    /// 自动锁定时间（分钟）
    pub auto_lock_time: u64,
    /// 剪贴板清除时间（秒）
    pub clipboard_clear_time: u64,
    /// 是否开机自启
    pub auto_start: bool,
    /// 是否关闭时最小化到托盘
    pub close_to_tray: bool,
    /// 是否显示密码强度
    pub show_password_strength: bool,
    /// 启动时是否显示主窗口
    pub show_on_startup: bool,
    /// 是否允许向终端注入密码（默认拒绝，命中黑名单需二次确认）
    #[serde(default)]
    pub allow_terminal_inject: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            language: "zh-CN".to_string(),
            auto_lock_time: 5,
            clipboard_clear_time: 30,
            auto_start: false,
            close_to_tray: true,
            show_password_strength: true,
            show_on_startup: true,
            allow_terminal_inject: false,
        }
    }
}

/// 获取所有设置
///
/// 从数据库读取所有设置，如果不存在则返回默认值。
///
/// # Arguments
///
/// * `state` - 应用状态
///
/// # Returns
///
/// 设置项
///
/// # 前端调用
///
/// ```typescript
/// const settings = await invoke('get_settings');
/// ```
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
    crate::commands::auth::require_unlocked(&state)?;

    let db = state.get_db()?;
    let conn = db.conn();

    // 查询所有设置
    let mut stmt = conn
        .prepare("SELECT key, value FROM settings")
        .map_err(|e| e.to_string())?;

    let mut settings = Settings::default();

    let rows = stmt
        .query_map([], |row| {
            let key: String = row.get(0)?;
            let value: String = row.get(1)?;
            Ok((key, value))
        })
        .map_err(|e| e.to_string())?;

    for row in rows {
        let (key, value) = row.map_err(|e| e.to_string())?;

        match key.as_str() {
            "theme" => settings.theme = value,
            "language" => settings.language = value,
            "auto_lock_time" => settings.auto_lock_time = value.parse().unwrap_or(5),
            "clipboard_clear_time" => settings.clipboard_clear_time = value.parse().unwrap_or(30),
            "auto_start" => settings.auto_start = value == "true",
            "close_to_tray" => settings.close_to_tray = value == "true",
            "show_password_strength" => settings.show_password_strength = value == "true",
            "show_on_startup" => settings.show_on_startup = value == "true",
            "allow_terminal_inject" => settings.allow_terminal_inject = value == "true",
            _ => {}
        }
    }

    Ok(settings)
}

/// 获取单个设置
///
/// 根据 key 获取单个设置值。
///
/// # Arguments
///
/// * `key` - 设置项 key
/// * `state` - 应用状态
///
/// # Returns
///
/// 设置值
///
/// # 前端调用
///
/// ```typescript
/// const theme = await invoke('get_setting', { key: 'theme' });
/// ```
#[tauri::command]
pub async fn get_setting(
    key: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
    crate::commands::auth::require_unlocked(&state)?;

    let db = state.get_db()?;
    let conn = db.conn();

    // 查询单个设置
    let result = conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| {
            let value: String = row.get(0)?;
            Ok(value)
        },
    );

    match result {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// 保存单个设置
///
/// 保存单个设置项。
///
/// # Arguments
///
/// * `key` - 设置项 key
/// * `value` - 设置值
/// * `state` - 应用状态
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
///
/// # 前端调用
///
/// ```typescript
/// await invoke('save_setting', { key: 'theme', value: 'dark' });
/// ```
#[tauri::command]
pub async fn save_setting(
    key: String,
    value: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
    crate::commands::auth::require_unlocked(&state)?;

    let db = state.get_db()?;
    let conn = db.conn();

    // 使用 INSERT OR REPLACE 保存设置
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    ).map_err(|e| e.to_string())?;

    // 如果是自动锁定时间设置，同步更新空闲检测模块
    if key == "auto_lock_time" {
        let minutes: u64 = value.parse().unwrap_or(5);
        crate::idle::set_auto_lock_minutes(minutes);
    }

    Ok(())
}

/// 批量保存设置
///
/// 批量保存多个设置项。
///
/// # Arguments
///
/// * `settings` - 设置项
/// * `state` - 应用状态
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
///
/// # 前端调用
///
/// ```typescript
/// await invoke('save_settings', {
///   settings: {
///     theme: 'dark',
///     language: 'en-US'
///   }
/// });
/// ```
#[tauri::command]
pub async fn save_settings(
    settings: Settings,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
    crate::commands::auth::require_unlocked(&state)?;

    let db = state.get_db()?;
    let conn = db.conn();

    // 批量保存设置
    let settings_vec = vec![
        ("theme", settings.theme),
        ("language", settings.language),
        ("auto_lock_time", settings.auto_lock_time.to_string()),
        ("clipboard_clear_time", settings.clipboard_clear_time.to_string()),
        ("auto_start", settings.auto_start.to_string()),
        ("close_to_tray", settings.close_to_tray.to_string()),
        ("show_password_strength", settings.show_password_strength.to_string()),
        ("show_on_startup", settings.show_on_startup.to_string()),
        ("allow_terminal_inject", settings.allow_terminal_inject.to_string()),
    ];

    for (key, value) in settings_vec {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        ).map_err(|e| e.to_string())?;
    }

    // 同步更新空闲检测模块的自动锁定时间
    crate::idle::set_auto_lock_minutes(settings.auto_lock_time);

    Ok(())
}

/// 设置开机自启
///
/// 启用或禁用开机自动启动。
///
/// # Arguments
///
/// * `enable` - 是否启用
/// * `app` - Tauri 应用句柄
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
///
/// # 前端调用
///
/// ```typescript
/// await invoke('set_auto_start', { enable: true });
/// ```
#[tauri::command]
pub async fn set_auto_start(enable: bool, app: tauri::AppHandle) -> Result<(), String> {
    let autostart_manager = app.autolaunch();

    if enable {
        autostart_manager.enable().map_err(|e| e.to_string())?;
    } else {
        autostart_manager.disable().map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// 获取开机自启状态
///
/// # Arguments
///
/// * `app` - Tauri 应用句柄
///
/// # Returns
///
/// 是否启用开机自启
///
/// # 前端调用
///
/// ```typescript
/// const enabled = await invoke('is_auto_start_enabled');
/// ```
#[tauri::command]
pub async fn is_auto_start_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    let autostart_manager = app.autolaunch();
    autostart_manager.is_enabled().map_err(|e| e.to_string())
}

/// 最小化窗口到托盘
///
/// 隐藏主窗口，实现最小化到托盘的效果。
///
/// # Arguments
///
/// * `app` - Tauri 应用句柄
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
///
/// # 前端调用
///
/// ```typescript
/// await invoke('minimize_to_tray');
/// ```
#[tauri::command]
pub async fn minimize_to_tray(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())?;
        log::info!("窗口已最小化到托盘");
    }
    Ok(())
}
