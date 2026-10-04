//! MoyuPasswd - Tauri 后端
//!
//! 这是 MoyuPasswd 应用的 Tauri 后端模块。
//! 负责处理数据库、加密、认证和业务逻辑。
//!
//! # 模块结构
//!
//! - `db`: 数据库初始化和迁移
//! - `db_meta`: 凭证元数据文件管理
//! - `crypto`: 密码哈希和数据加密
//! - `state`: 应用状态管理
//! - `commands`: Tauri 命令（前端调用）

/// 数据库模块
pub mod db;

/// 凭证元数据模块
pub mod db_meta;

/// 加密模块
pub mod crypto;

/// 剪贴板模块
pub mod clipboard;

/// 密码粘贴待处理槽
pub mod pending;

/// 全局热键与键盘钩子模块
pub mod hotkey;

/// 密码注入模块
pub mod inject;

/// 审计模块
pub mod audit;

/// 文件 ACL 加固模块
pub mod acl;

/// 空闲检测模块
pub mod idle;

/// 系统认证模块
pub mod system_auth;

/// 应用状态模块
pub mod state;

/// Tauri 命令模块
pub mod commands;

use state::AppState;
use tauri::{Emitter, Manager};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

/// 运行 Tauri 应用
///
/// 初始化所有插件、注册命令、设置状态管理。
///
/// # 启动流程
///
/// 1. 检查元数据文件是否存在，判断是否首次运行
/// 2. 首次运行：使用默认密钥创建数据库
/// 3. 后续运行：数据库待认证后创建
/// 4. 注册全局快捷键和系统托盘
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 注册全局快捷键插件
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // 注册剪贴板管理插件
        .plugin(tauri_plugin_clipboard_manager::init())
        // 注册开机自启插件
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // 注册通知插件
        .plugin(tauri_plugin_notification::init())
        // 注册 Shell 插件
        .plugin(tauri_plugin_shell::init())
        // 注册生物识别插件（Windows Hello / Touch ID）
        .plugin(tauri_plugin_biometry::init())
        .setup(|app| {
            // 开发模式下启用日志
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // 初始化空闲检测模块
            idle::init();

            // 初始化审计模块
            {
                let app_dir = app.path().app_data_dir().expect("无法获取应用数据目录");
                audit::init(&app_dir);
            }

            // 获取应用数据目录
            let app_dir = app.path().app_data_dir()
                .expect("无法获取应用数据目录");

            // 根据元数据文件判断是否首次运行
            let is_first_run = !db_meta::meta_exists(&app_dir);

            if is_first_run {
                // ========== 首次运行 ==========
                // 使用默认密钥创建数据库（后续设置主密码时会 rekey）
                log::info!("首次运行，使用默认密钥创建数据库");
                let database = db::Database::new(&app_dir, db::DEFAULT_DB_KEY)
                    .expect("无法初始化数据库");
                app.manage(AppState::new(app_dir.clone(), database));
            } else {
                // ========== 后续运行 ==========
                // 数据库待用户输入主密码后创建
                log::info!("检测到已有配置，等待用户解锁");
                app.manage(AppState::new_without_db(app_dir.clone()));
            }

            // 启动空闲检测后台线程
            {
                let app_handle = app.handle().clone();
                idle::start_idle_watcher(move || {
                    let state = app_handle.state::<AppState>();
                    state.clear_aes_key();
                    crate::pending::revoke("auto-lock");
                    crate::hotkey::deactivate();
                    if let Err(e) = crate::clipboard::clear_now(&app_handle) {
                        log::warn!("自动锁定时清空剪贴板失败: {e}");
                    }
                    let _ = app_handle.emit("app-locked", ());
                    log::info!("空闲超时，应用已自动锁定");
                });
            }

            // 创建系统托盘菜单
            let show_item = MenuItemBuilder::with_id("show", "显示主窗口").build(app)?;
            let lock_item = MenuItemBuilder::with_id("lock", "锁定").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;

            let menu = MenuBuilder::new(app)
                .item(&show_item)
                .item(&lock_item)
                .separator()
                .item(&quit_item)
                .build()?;

            // 创建系统托盘图标
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("MoyuPasswd")
                .on_menu_event(move |app, event| {
                    match event.id().as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "lock" => {
                            let state = app.state::<AppState>();
                            state.clear_aes_key();
                            crate::pending::revoke("manual-lock");
                            crate::hotkey::deactivate();
                            let _ = app.emit("app-locked", ());
                        }
                        "quit" => {
                            crate::pending::revoke("quit");
                            crate::hotkey::deactivate();
                            if let Err(e) = crate::clipboard::clear_now(app) {
                                log::warn!("退出时清空剪贴板失败: {e}");
                            }
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // 根据设置决定启动时是否显示主窗口
            {
                let app_state = app.state::<AppState>();
                if app_state.has_database() {
                    if let Ok(db) = app_state.get_db() {
                        let show_on_startup: bool = db.conn()
                            .query_row(
                                "SELECT value FROM settings WHERE key = 'show_on_startup'",
                                [],
                                |row| {
                                    let value: String = row.get(0)?;
                                    Ok(value == "true")
                                },
                            )
                            .unwrap_or(true);

                        if show_on_startup {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }

                        // 从数据库读取快捷键配置
                        let shortcuts_config = {
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
                                    serde_json::from_str::<commands::shortcuts::ShortcutConfig>(&json)
                                        .unwrap_or_default()
                                }
                                Err(_) => commands::shortcuts::ShortcutConfig::default(),
                            }
                        };
                        register_shortcuts(app, &shortcuts_config);
                    }
                } else {
                    // 数据库未初始化，显示主窗口让用户输入密码
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    let default_shortcuts = commands::shortcuts::ShortcutConfig::default();
                    register_shortcuts(app, &default_shortcuts);
                }
            }

            Ok(())
        })
        // 注册 Tauri 命令
        .invoke_handler(tauri::generate_handler![
            // 认证命令
            commands::set_master_password,
            commands::verify_master_password,
            commands::change_master_password,
            commands::lock_app,
            commands::is_unlocked,
            commands::has_master_password,
            // 系统认证命令
            commands::enable_system_auth,
            commands::disable_system_auth,
            commands::is_system_auth_enabled,
            commands::is_system_auth_available,
            commands::get_system_auth_method,
            commands::unlock_with_system_auth,
            // 密码命令
            commands::get_passwords,
            commands::add_password,
            commands::update_password,
            commands::delete_password,
            commands::toggle_favorite,
            commands::copy_password_to_clipboard,
            // 分类命令
            commands::get_categories,
            commands::add_category,
            commands::update_category,
            commands::delete_category,
            // 设置命令
            commands::get_settings,
            commands::get_setting,
            commands::save_setting,
            commands::save_settings,
            commands::set_auto_start,
            commands::is_auto_start_enabled,
            // 数据管理命令
            commands::export_data,
            commands::import_data,
            // 快捷键命令
            commands::get_shortcuts,
            commands::save_shortcuts,
            commands::update_global_shortcuts,
            commands::unregister_all_shortcuts,
            // 倒计时命令
            commands::show_countdown,
            commands::hide_countdown,
            commands::start_follow_cursor,
            commands::stop_follow_cursor,
            commands::get_countdown_seconds,
            commands::clear_clipboard,
            clipboard::copy_text_to_clipboard,
            // 窗口管理命令
            commands::minimize_to_tray,
            // 空闲检测命令
            idle::report_activity,
        ])
        // 监听窗口事件
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let app_handle = window.app_handle();
                    let state = app_handle.state::<AppState>();

                    let close_to_tray = if let Ok(db) = state.get_db() {
                        db.conn().query_row(
                            "SELECT value FROM settings WHERE key = 'close_to_tray'",
                            [],
                            |row| {
                                let value: String = row.get(0)?;
                                Ok(value == "true")
                            },
                        ).unwrap_or(true)
                    } else {
                        true
                    };

                    if window.label() == "main" {
                        if close_to_tray {
                            api.prevent_close();
                            let _ = window.hide();
                            log::info!("窗口关闭，最小化到托盘");
                        } else {
                            if let Err(e) = crate::clipboard::clear_now(app_handle) {
                                log::warn!("退出时清空剪贴板失败: {e}");
                            }
                            log::info!("窗口关闭，退出应用");
                        }
                    }
                }
                tauri::WindowEvent::Focused(focused) => {
                    // 快速搜索窗口失去焦点时隐藏（但认证进行中不隐藏）
                    if window.label() == "quick-search" && !focused {
                        if !crate::system_auth::is_auth_in_progress() {
                            let _ = window.hide();
                        }
                    }
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 注册全局快捷键
fn register_shortcuts(app: &tauri::App, shortcuts_config: &commands::shortcuts::ShortcutConfig) {
    log::info!("注册快捷键: {:?}", shortcuts_config);

    let quick_search_shortcut = shortcuts_config.quick_search.clone();
    app.global_shortcut().on_shortcut(quick_search_shortcut.as_str(), move |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            if let Some(window) = app.get_webview_window("quick-search") {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.set_always_on_top(true);
                let _ = window.emit("window-shown", ());
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
    }).expect("注册快速搜索快捷键失败");

    app.global_shortcut().on_shortcut("Escape", move |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            if let Some(window) = app.get_webview_window("quick-search") {
                let _ = window.hide();
            }
        }
    }).expect("注册 Esc 快捷键失败");

    let quick_add_shortcut = shortcuts_config.quick_add.clone();
    app.global_shortcut().on_shortcut(quick_add_shortcut.as_str(), move |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.emit("show-quick-add", ());
            }
        }
    }).expect("注册快速添加快捷键失败");

    let password_generator_shortcut = shortcuts_config.password_generator.clone();
    app.global_shortcut().on_shortcut(password_generator_shortcut.as_str(), move |app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.emit("show-password-generator", ());
            }
        }
    }).expect("注册密码生成器快捷键失败");
}
