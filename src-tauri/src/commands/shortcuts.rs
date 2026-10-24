//! 快捷键命令模块
//!
//! 处理快捷键配置的读取、保存和全局快捷键注册。
//!
//! # 命令列表
//!
//! - `get_shortcuts`: 获取快捷键配置
//! - `save_shortcuts`: 保存快捷键配置
//! - `update_global_shortcuts`: 动态更新全局快捷键（返回注册失败列表）
//! - `unregister_all_shortcuts`: 注销所有全局快捷键
//!
//! # 注册策略
//!
//! - 唯一注册实现 `register_all`：启动与动态更新共用，行为一致
//! - 启动时加密库未解锁、读不到用户配置，先注册默认配置；
//!   解锁成功后由 auth 命令调用 `reload_from_settings` 换成用户自定义配置
//! - 注册失败不 panic（快捷键可能被其他程序占用），收集后返回失败列表
//! - 不注册无修饰键的 Escape（全局裸 Esc 会吞系统级 Esc；快速搜索窗口内的
//!   Esc 由前端窗口级键盘监听处理）

use crate::response::{respond, ApiResult};
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
pub async fn get_shortcuts(state: State<'_, AppState>) -> ApiResult<ShortcutConfig> {
    respond(|| -> Result<ShortcutConfig, String> {
        // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
        crate::commands::auth::require_unlocked(&state)?;

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
                // 解析失败回退默认配置并告警（与 load_config_from_db 一致）：
                // 配置损坏不该把设置页整个挡住
                match serde_json::from_str(&json) {
                    Ok(config) => Ok(config),
                    Err(e) => {
                        log::error!("解析快捷键配置失败，回退默认配置: {}", e);
                        Ok(ShortcutConfig::default())
                    }
                }
            }
            Err(e) => {
                log::info!("未找到快捷键配置，使用默认值: {:?}", e);
                // 返回默认配置
                Ok(ShortcutConfig::default())
            }
        }

    })}

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
) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
        crate::commands::auth::require_unlocked(&state)?;

        log::info!("收到快捷键保存请求: {:?}", shortcuts);

        // 入库前校验快捷键串（空串 = 不绑定，放行）：
        // 任意字符串入库要等注册阶段才失败，应在保存时就拦截
        validate_accelerator(&shortcuts.quick_search, "快速搜索")?;
        validate_accelerator(&shortcuts.quick_add, "快速添加")?;
        validate_accelerator(&shortcuts.password_generator, "密码生成器")?;

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

    })}

// ==================== 注册实现（唯一入口） ====================

/// 校验快捷键串可被全局快捷键插件解析（空串 = 不绑定，放行）
fn validate_accelerator(value: &str, what: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Ok(());
    }
    if value.len() > 64 {
        return Err(format!("{what}快捷键过长: {value:?}"));
    }
    match value.parse::<tauri_plugin_global_shortcut::Shortcut>() {
        Ok(_) => Ok(()),
        Err(_) => Err(format!("{what}快捷键无法解析: {value:?}")),
    }
}

/// 从数据库读取快捷键配置（读不到/未解锁时用默认值）
pub fn load_config_from_db(state: &AppState) -> ShortcutConfig {
    let db = match state.get_db() {
        Ok(db) => db,
        Err(_) => {
            log::info!("数据库未解锁，快捷键使用默认配置");
            return ShortcutConfig::default();
        }
    };
    let conn = db.conn();
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'shortcuts'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|json| serde_json::from_str::<ShortcutConfig>(&json).ok())
    .unwrap_or_default()
}

/// 显示/聚焦快速搜索窗口（不存在则创建）
fn show_quick_search(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("quick-search") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.set_always_on_top(true);
        let _ = window.emit("window-shown", ());
    } else {
        match tauri::WebviewWindowBuilder::new(
            app,
            "quick-search",
            tauri::WebviewUrl::App("quick-search.html".into()),
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
        {
            Ok(window) => {
                let _ = window.set_focus();
                let _ = window.emit("window-shown", ());
            }
            Err(e) => log::error!("创建快速搜索窗口失败: {e}"),
        }
    }
}

/// 显示主窗口并向其发出事件
fn show_main_and_emit(app: &tauri::AppHandle, event: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit(event, ());
    } else {
        log::warn!("主窗口不存在，无法触发 {event}");
    }
}

/// 注册全部全局快捷键（唯一实现；启动与动态更新共用）
///
/// 先注销所有已注册快捷键再按配置注册，幂等。
/// 单个快捷键注册失败**不 panic**（可能被其他程序占用），收集后返回失败列表。
/// 不注册无修饰键的 Escape。
///
/// # Returns
///
/// 注册失败的快捷键描述列表（空表示全部成功）
pub fn register_all(app: &tauri::AppHandle, config: &ShortcutConfig) -> Result<Vec<String>, String> {
    log::info!("注册全局快捷键: {:?}", config);
    let global_shortcut = app.global_shortcut();
    global_shortcut.unregister_all().map_err(|e| {
        log::error!("注销快捷键失败: {}", e);
        e.to_string()
    })?;

    let mut failed: Vec<String> = Vec::new();

    // 快速搜索
    let quick_search = config.quick_search.clone();
    if let Err(e) = global_shortcut.on_shortcut(quick_search.as_str(), move |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            show_quick_search(app);
        }
    }) {
        log::error!("注册快速搜索快捷键 {} 失败: {}", quick_search, e);
        failed.push(format!("快速搜索（{}）", quick_search));
    }

    // 快速添加
    let quick_add = config.quick_add.clone();
    if let Err(e) = global_shortcut.on_shortcut(quick_add.as_str(), move |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            show_main_and_emit(app, "show-quick-add");
        }
    }) {
        log::error!("注册快速添加快捷键 {} 失败: {}", quick_add, e);
        failed.push(format!("快速添加（{}）", quick_add));
    }

    // 密码生成器
    let password_generator = config.password_generator.clone();
    if let Err(e) = global_shortcut.on_shortcut(password_generator.as_str(), move |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            show_main_and_emit(app, "show-password-generator");
        }
    }) {
        log::error!("注册密码生成器快捷键 {} 失败: {}", password_generator, e);
        failed.push(format!("密码生成器（{}）", password_generator));
    }

    // 紧急逃生热键（固定组合）：网页崩溃/卡死时的最后通道，见 register_emergency_shortcuts
    register_emergency_shortcuts(app);

    if failed.is_empty() {
        log::info!("全局快捷键注册完成");
    } else {
        log::warn!("以下快捷键注册失败（可能被占用）: {:?}", failed);
    }
    Ok(failed)
}

/// 注册紧急逃生热键（固定组合，不随用户配置）
///
/// 自绘标题栏模式下网页崩溃/卡死后界面按钮全部失效，这两个组合在 Rust 层响应，
/// 是最后的逃生通道：
/// - Ctrl+Alt+Shift+R：重载主窗口网页（自愈兜底；窗口隐藏时先唤起）
/// - Ctrl+Alt+Shift+E：强制退出（先尽力清剪贴板）
///
/// 必须在 `register_all` 的 `unregister_all()` **之后**调用，
/// 否则用户改配置整体重注册时会把它们抹掉。
fn register_emergency_shortcuts(app: &tauri::AppHandle) {
    let global_shortcut = app.global_shortcut();

    if let Err(e) = global_shortcut.on_shortcut("Ctrl+Alt+Shift+R", |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
                match win.reload() {
                    Ok(()) => {
                        app.state::<AppState>().mark_webview_alive();
                        log::info!("紧急热键：已重载主窗口网页");
                    }
                    Err(err) => {
                        app.state::<AppState>().set_webview_unhealthy(true);
                        log::error!("紧急热键：重载失败 {err}");
                    }
                }
            }
        }
    }) {
        log::warn!("注册紧急重载热键 Ctrl+Alt+Shift+R 失败: {e}");
    }

    if let Err(e) = global_shortcut.on_shortcut("Ctrl+Alt+Shift+E", |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            log::warn!("紧急热键：强制退出应用");
            if let Err(err) = crate::clipboard::clear_now(app) {
                log::warn!("退出前清空剪贴板失败: {err}");
            }
            app.exit(0);
        }
    }) {
        log::warn!("注册紧急退出热键 Ctrl+Alt+Shift+E 失败: {e}");
    }
}

/// 解锁后按用户配置重新注册全局快捷键
///
/// 启动时用户配置在加密库中读不到，只能注册默认快捷键；
/// 解锁成功后调用本函数换成用户自定义配置。
pub fn reload_from_settings(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<Vec<String>, String> {
    let config = load_config_from_db(state);
    register_all(app, &config)
}

/// 动态更新全局快捷键
///
/// 从数据库读取最新配置并重新注册（先注销再注册，幂等）。
/// 不注册无修饰键的 Escape。
///
/// # Arguments
///
/// * `app` - Tauri 应用句柄
/// * `state` - 应用状态
///
/// # Returns
///
/// 注册失败的快捷键描述列表（空表示全部成功）
///
/// # 前端调用
///
/// ```typescript
/// const failed = await invoke<string[]>('update_global_shortcuts');
/// ```
#[tauri::command]
pub async fn update_global_shortcuts(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> ApiResult<Vec<String>> {
    respond(|| -> Result<Vec<String>, String> {
        // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
        crate::commands::auth::require_unlocked(&state)?;

        log::info!("开始更新全局快捷键...");
        let config = load_config_from_db(&state);
        log::info!("新的快捷键配置: {:?}", config);
        let failed = register_all(&app, &config)?;
        log::info!("全局快捷键更新完成，失败: {:?}", failed);
        Ok(failed)

    })}

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
pub async fn unregister_all_shortcuts(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
        crate::commands::auth::require_unlocked(&state)?;

        log::info!("注销所有全局快捷键...");
        let global_shortcut = app.global_shortcut();
        global_shortcut.unregister_all().map_err(|e| {
            log::error!("注销快捷键失败: {}", e);
            e.to_string()
        })?;
        log::info!("已注销所有全局快捷键");
        Ok(())

    })}
