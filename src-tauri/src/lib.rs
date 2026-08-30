//! 摸鱼密码 - Tauri 后端
//!
//! 这是摸鱼密码应用的 Tauri 后端模块。
//! 负责处理数据库、加密、认证和业务逻辑。
//!
//! # 模块结构
//!
//! - `db`: 数据库初始化和迁移
//! - `crypto`: 密码哈希和数据加密
//! - `state`: 应用状态管理
//! - `commands`: Tauri 命令（前端调用）

/// 数据库模块
pub mod db;

/// 加密模块
pub mod crypto;

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
/// 初始化所有插件、注册命令、设置状态管理
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
        .setup(|app| {
            // 开发模式下启用日志
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // 获取应用数据目录
            let app_dir = app.path().app_data_dir()
                .expect("无法获取应用数据目录");

            // 初始化数据库
            let database = db::Database::new(&app_dir)
                .expect("无法初始化数据库");

            // 设置应用状态
            app.manage(AppState::new(database));

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
                .tooltip("摸鱼密码")
                .on_menu_event(move |app, event| {
                    match event.id().as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "lock" => {
                            // 锁定应用
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.emit("lock-app", ());
                            }
                            // 通知快速搜索窗口已锁定
                            if let Some(window) = app.get_webview_window("quick-search") {
                                let _ = window.emit("app-locked", ());
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // 注册全局快捷键 Ctrl+K 呼出快速搜索窗口
            let _handle = app.handle().clone();
            app.global_shortcut().on_shortcut("CmdOrCtrl+K", move |app, _shortcut, event| {
                if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                    // 检查窗口是否已存在
                    if let Some(window) = app.get_webview_window("quick-search") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        let _ = window.set_always_on_top(true);
                    } else {
                        // 创建新窗口
                        let window = tauri::WebviewWindowBuilder::new(
                            app,
                            "quick-search",
                            tauri::WebviewUrl::App("quick-search.html".into())
                        )
                        .title("快速搜索")
                        .inner_size(600.0, 450.0)
                        .resizable(false)
                        .decorations(false)
                        .always_on_top(true)
                        .skip_taskbar(true)
                        .focused(true)
                        .center()
                        .build()
                        .expect("创建快速搜索窗口失败");

                        let _ = window.set_focus();
                    }
                }
            }).expect("注册全局快捷键失败");

            // 注册全局快捷键 Esc 关闭快速搜索窗口
            app.global_shortcut().on_shortcut("Escape", move |app, _shortcut, event| {
                if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                    if let Some(window) = app.get_webview_window("quick-search") {
                        let _ = window.hide();
                    }
                }
            }).expect("注册 Esc 快捷键失败");

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
            // 密码命令
            commands::get_passwords,
            commands::get_password_by_id,
            commands::add_password,
            commands::update_password,
            commands::delete_password,
            commands::toggle_favorite,
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
            // 倒计时命令
            commands::show_countdown,
            commands::hide_countdown,
            commands::start_follow_cursor,
            commands::stop_follow_cursor,
            commands::get_countdown_seconds,
            commands::clear_clipboard,
        ])
        // 监听窗口事件
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // 主窗口关闭时改为隐藏
                    if window.label() == "main" {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
                tauri::WindowEvent::Focused(focused) => {
                    // 快速搜索窗口失去焦点时隐藏
                    if window.label() == "quick-search" && !focused {
                        let _ = window.hide();
                    }
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
