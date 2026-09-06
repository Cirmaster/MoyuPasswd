//! 快捷键命令模块
//!
//! 处理快捷键配置的读取和保存。
//!
//! # 命令列表
//!
//! - `get_shortcuts`: 获取快捷键配置
//! - `save_shortcuts`: 保存快捷键配置
//! - `update_global_shortcuts`: 动态更新全局快捷键

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

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
    let db = state.get_db()?;
    let conn = db.conn();

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
            log::info!("读取到快捷键配置: {}", json);
            // 解析 JSON
            serde_json::from_str(&json).map_err(|e| {
                log::error!("解析快捷键配置失败: {}", e);
                e.to_string()
            })
        }
        Err(e) => {
            log::info!("未找到快捷键配置，使用默认值: {:?}", e);
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
    log::info!("收到快捷键保存请求: {:?}", shortcuts);
    
    let db = state.get_db()?;
    let conn = db.conn();

    // 序列化为 JSON
    let json = serde_json::to_string(&shortcuts).map_err(|e| {
        log::error!("序列化快捷键配置失败: {}", e);
        e.to_string()
    })?;
    
    log::info!("序列化后的 JSON: {}", json);

    // 保存到数据库
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('shortcuts', ?1)",
        params![json],
    ).map_err(|e| {
        log::error!("保存快捷键到数据库失败: {}", e);
        e.to_string()
    })?;

    log::info!("快捷键配置保存成功");
    Ok(())
}

/// 动态更新全局快捷键
///
/// 注销所有现有快捷键并重新注册新的快捷键。
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
/// await invoke('update_global_shortcuts');
/// ```
#[tauri::command]
pub async fn update_global_shortcuts(app: tauri::AppHandle) -> Result<(), String> {
    log::info!("开始更新全局快捷键...");
    
    // 获取应用状态
    let state = app.state::<AppState>();
    
    // 从数据库读取快捷键配置
    let shortcuts_config = {
        let db = state.get_db()?;
        let conn = db.conn();
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
                serde_json::from_str::<ShortcutConfig>(&json)
                    .unwrap_or_default()
            }
            Err(_) => ShortcutConfig::default(),
        }
    };
    
    log::info!("新的快捷键配置: {:?}", shortcuts_config);
    
    // 注销所有现有快捷键
    let global_shortcut = app.global_shortcut();
    global_shortcut.unregister_all().map_err(|e| {
        log::error!("注销快捷键失败: {}", e);
        e.to_string()
    })?;
    log::info!("已注销所有快捷键");
    
    // 重新注册快速搜索快捷键
    let quick_search = shortcuts_config.quick_search.clone();
    let quick_search_for_closure = quick_search.clone();
    global_shortcut.on_shortcut(quick_search.as_str(), move |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            log::info!("快速搜索快捷键触发: {}", quick_search_for_closure);
            if let Some(window) = app.get_webview_window("quick-search") {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.set_always_on_top(true);
            } else {
                let window = tauri::WebviewWindowBuilder::new(
                    app,
                    "quick-search",
                    tauri::WebviewUrl::App("quick-search.html".into())
                )
                .title("快速搜索")
                .inner_size(600.0, 450.0)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(true)
                .center()
                .build()
                .expect("创建快速搜索窗口失败");
                let _ = window.set_focus();
            }
        }
    }).map_err(|e| {
        log::error!("注册快速搜索快捷键失败: {}", e);
        e.to_string()
    })?;
    log::info!("注册快速搜索快捷键: {}", shortcuts_config.quick_search);
    
    // 重新注册 Esc 快捷键
    global_shortcut.on_shortcut("Escape", move |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            if let Some(window) = app.get_webview_window("quick-search") {
                let _ = window.hide();
            }
        }
    }).map_err(|e| {
        log::error!("注册 Esc 快捷键失败: {}", e);
        e.to_string()
    })?;
    
    // 重新注册快速添加快捷键
    let quick_add = shortcuts_config.quick_add.clone();
    let quick_add_for_closure = quick_add.clone();
    app.global_shortcut().on_shortcut(quick_add.as_str(), move |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            log::info!("快速添加快捷键触发: {}", quick_add_for_closure);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.emit("show-quick-add", ());
            }
        }
    }).map_err(|e| {
        log::error!("注册快速添加快捷键失败: {}", e);
        e.to_string()
    })?;
    log::info!("注册快速添加快捷键: {}", shortcuts_config.quick_add);
    
    // 重新注册密码生成器快捷键
    let password_generator = shortcuts_config.password_generator.clone();
    let password_generator_for_closure = password_generator.clone();
    app.global_shortcut().on_shortcut(password_generator.as_str(), move |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            log::info!("密码生成器快捷键触发: {}", password_generator_for_closure);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.emit("show-password-generator", ());
            }
        }
    }).map_err(|e| {
        log::error!("注册密码生成器快捷键失败: {}", e);
        e.to_string()
    })?;
    log::info!("注册密码生成器快捷键: {}", shortcuts_config.password_generator);
    
    log::info!("全局快捷键更新完成");
    Ok(())
}

/// 注销所有全局快捷键
///
/// 在编辑快捷键时调用，防止编辑时触发应用快捷键。
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
/// await invoke('unregister_all_shortcuts');
/// ```
#[tauri::command]
pub async fn unregister_all_shortcuts(app: tauri::AppHandle) -> Result<(), String> {
    log::info!("注销所有全局快捷键...");
    let global_shortcut = app.global_shortcut();
    global_shortcut.unregister_all().map_err(|e| {
        log::error!("注销快捷键失败: {}", e);
        e.to_string()
    })?;
    log::info!("已注销所有全局快捷键");
    Ok(())
}
