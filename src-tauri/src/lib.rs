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

/// X11 平台工具（仅 Linux）：注入链路的会话判定 / EWMH 归因 / 键盘映射助手
#[cfg(target_os = "linux")]
pub mod x11;

/// 非 Linux 的会话判定空实现
///
/// Wayland 只存在于 Linux，这里恒返回 false。保留同名模块与同名函数，
/// 是为了让 clipboard / countdown 里「Wayland 才改道」的判断不必加 `cfg`
/// （该分支在非 Linux 上本来就不成立）。注意只提供 `is_wayland_session`：
/// 其他 X11 函数在非 Linux 依然编译不过，避免被误用。
#[cfg(not(target_os = "linux"))]
pub mod x11 {
    pub fn is_wayland_session() -> bool {
        false
    }
}

/// 审计模块
pub mod audit;

/// 文件 ACL 加固模块
pub mod acl;

/// 空闲检测模块
pub mod idle;

/// 系统认证模块
pub mod system_auth;

/// 管理员身份模块（提权检测 + 启动时以管理员运行开关）
pub mod elevate;

/// 应用状态模块
pub mod state;

/// webview 心跳看门狗（崩溃/卡死自动重载；见 watchdog.rs）
pub mod watchdog;

/// WebView2 浏览器快捷键关闭（F3 查找栏、Ctrl+P 打印、F5 刷新等；见 browser_keys.rs）
pub mod browser_keys;

/// 统一响应格式模块（所有命令返回 { code, message, data } 包裹）
pub mod response;

/// 技术日志模块（区别于 audit 的审计日志；见 logging.rs）
pub mod logging;

/// 「自动」交付模式：目标进程在前台时才把密码写进剪贴板，失焦立刻抹掉（见 auto_clipboard.rs）
pub mod auto_clipboard;

/// 窗口「设计尺寸」常量（单一来源；见 window_sizes.rs 的维护约定）
pub mod window_sizes;

/// 窗口缩放换算模块（设计尺寸 ↔ 物理尺寸唯一入口；见 scale.rs 的缩放模型）
pub mod scale;

/// Tauri 命令模块
pub mod commands;

/// 拖放交付模块（Wayland 下密码不进剪贴板的唯一通道；见 dnd.rs）
///
/// 只有 Linux 有这条链路：底层依赖 GTK 的 `drag-begin` / `drag-data-get`。
/// 非 Linux 的空实现见下方同名模块。
#[cfg(target_os = "linux")]
pub mod dnd;

/// 拖拽池窗口（Wayland 下「点复制入池、从池里拖」的载体；见 pool_window.rs）
///
/// 同样是纯 GTK 实现，只在 Linux 存在。
#[cfg(target_os = "linux")]
pub mod pool_window;

/// 非 Linux 的 `dnd` 空实现
///
/// 拖放链路只在 Linux 存在，但它的调用点（剪贴板改道、锁定时清池、拖放命令）
/// 分布在跨平台代码里。这里按同名模块提供最小 API，让调用点不必到处写 `cfg`：
/// 非 Linux 上 Wayland 判定恒为 false，这些路径本来就走不到。
#[cfg(not(target_os = "linux"))]
pub mod dnd {
    use crate::pending::DecryptFn;

    /// 拖放武装有效期（与 Linux 实现同值，仅供命令返回值使用）
    pub const TTL_SECS: u64 = 20;

    /// 池中载荷（与 Linux 实现同构，保证调用点类型一致）
    #[derive(Clone)]
    pub enum PoolPayload {
        /// 密码：延迟解密
        Secret(DecryptFn),
        /// 普通文本：随条目一起清零
        Plain(zeroize::Zeroizing<String>),
    }

    pub fn arm(_entry_id: &str, _decrypt_fn: DecryptFn) -> u64 {
        0
    }

    pub fn revoke(_reason: &str) -> Option<String> {
        None
    }

    pub fn pool_put(_id: &str, _title: &str, _username: &str, _payload: PoolPayload) -> u64 {
        0
    }

    pub fn text_pool_id(_title: &str, _subtitle: &str, _text: &str) -> String {
        String::new()
    }

    pub fn pool_count() -> usize {
        0
    }
}

/// 非 Linux 的 `pool_window` 空实现（浮窗是 GTK 窗口，其他平台没有它）
#[cfg(not(target_os = "linux"))]
pub mod pool_window {
    pub fn toggle(_app: &tauri::AppHandle) {}

    pub fn shutdown(_app: &tauri::AppHandle) {}

    pub fn reveal_with_fly_in(_app: &tauri::AppHandle) {}
}

use state::AppState;
use tauri::Manager;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;

/// 显示致命错误弹窗（启动阶段数据安全相关的失败）
#[cfg(target_os = "windows")]
fn show_fatal_error(message: &str) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    use windows::core::PCWSTR;

    let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    let caption: Vec<u16> = "MoyuPasswd".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(
            HWND(std::ptr::null_mut()),
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// 显示致命错误（非 Windows 平台：rfd 原生错误对话框，Linux GTK3 / macOS NSAlert）
#[cfg(not(target_os = "windows"))]
fn show_fatal_error(message: &str) {
    eprintln!("MoyuPasswd: {message}");
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("MoyuPasswd")
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// 运行 Tauri 应用
///
/// 初始化所有插件、注册命令、设置状态管理。
///
/// # 启动流程
///
/// 1. 恢复可能中断的主密码修改（有事务日志则回滚）
/// 2. 检查元数据状态（三态）判断运行模式；读取失败中止启动，绝不当作首次运行
/// 3. 首次运行：使用默认密钥创建数据库
/// 4. 后续运行：数据库待认证后创建
/// 5. 注册全局快捷键和系统托盘
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例守卫：必须注册在所有插件之前，避免与其他插件产生启动竞态。
        // 效果：全局只允许一个进程运行；用户再次启动（开始菜单/快捷方式/开机自启）
        // 不会开出第二个实例，而是走下面的回调——把已运行实例的主窗口拉到前台，
        // 随后本进程由插件自动退出。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                // 窗口可能处于「最小化到托盘」（隐藏）或最小化状态，逐一恢复
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        // 注册全局快捷键插件
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // 注册剪贴板管理插件
        .plugin(tauri_plugin_clipboard_manager::init())
        // 注册开机自启插件
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // 注册生物识别插件（Windows Hello / Touch ID）
        .plugin(tauri_plugin_biometry::init())
        .setup(|app| {
            // 日志：release 也落盘（默认 Warn，异常与失败都能留下痕迹）。
            // 优先级最高——后面所有模块的失败都靠它留下线索（见 logging.rs）
            logging::init(app.handle())?;

            // 初始化空闲检测模块
            idle::init();

            // 启动 webview 心跳看门狗：网页崩溃/卡死时自动重载（自绘标题栏模式下
            // 界面按钮会随网页一起失效，自愈是第一道保命线）
            watchdog::spawn(app.handle().clone());

            // 关闭浏览器专属快捷键（F3 查找栏、Ctrl+P 打印、F5 刷新等，见 browser_keys.rs）。
            // 这里覆盖启动时已建的窗口（主窗口）；quick-search / countdown 是运行时建窗，
            // 在各自建窗后单独调用
            for window in app.webview_windows().into_values() {
                browser_keys::disable_browser_keys(&window);
            }

            // 拖放交付（Wayland）：在主窗口的 webview 上装拖放处理器。
            // 只在主窗口装——快速搜索窗是运行时建窗且 Wayland 下唤不起来（全局快捷键缺失）
            #[cfg(target_os = "linux")]
            {
                if let Some(main) = app.get_webview_window("main") {
                    dnd::install_gtk_handlers(&main, app.handle().clone());
                }
            }

            // 窗口缩放换算：登记设计尺寸并按渲染缩放应用窗口物理尺寸
            scale::init(app.handle());

            // 初始化审计模块
            {
                let app_dir = app.path().app_data_dir().expect("无法获取应用数据目录");
                audit::init(&app_dir);
            }

            // 注入模块需要应用句柄（读取设置中的注入黑名单）
            inject::set_app_handle(app.handle().clone());

            // 获取应用数据目录
            let app_dir = app.path().app_data_dir()
                .expect("无法获取应用数据目录");

            // 先恢复可能中断的主密码修改（幂等：无事务日志时不做任何事）
            commands::auth::recover_interrupted_change(&app_dir);

            // 根据元数据状态判断运行模式（三态：读取失败绝不当作首次运行）
            match db_meta::meta_status(&app_dir) {
                db_meta::MetaStatus::Present => {
                    // ========== 后续运行 ==========
                    // 数据库待用户输入主密码后创建
                    log::info!("检测到已有配置，等待用户解锁");
                    app.manage(AppState::new_without_db(app_dir.clone()));
                }
                db_meta::MetaStatus::Absent => {
                    // ========== 首次运行 ==========
                    // 使用默认密钥创建数据库（后续设置主密码时会 rekey）
                    log::info!("首次运行，使用默认密钥创建数据库");
                    let database = db::Database::new(&app_dir, db::DEFAULT_DB_KEY)
                        .expect("无法初始化数据库");
                    app.manage(AppState::new(app_dir.clone(), database));
                }
                db_meta::MetaStatus::Unavailable(reason) => {
                    // ========== 元数据不可读 ==========
                    // 中止启动以保护密码库：不删、不写、绝不进首跑流程
                    log::error!("元数据不可读，已中止启动以保护密码库: {reason}");
                    show_fatal_error(&format!(
                        "无法读取凭据元数据，已中止启动以保护密码库。\n\n原因：{reason}\n\n请检查系统凭据存储后重新启动应用。"
                    ));
                    std::process::exit(1);
                }
            }

            // 启动空闲检测后台线程
            {
                let app_handle = app.handle().clone();
                idle::start_idle_watcher(move || {
                    let state = app_handle.state::<AppState>();
                    // 只在已解锁时锁定：避免锁定后每轮空闲检查重复触发，
                    // 也避免首跑 setup 阶段（尚未解锁）被误锁死
                    if !state.is_unlocked() {
                        return;
                    }
                    // 统一锁定入口：作废 pending → 卸钩子 → 清密钥 → 清剪贴板 → 通知窗口
                    commands::auth::lockdown(&state, &app_handle, "auto-lock");
                });
            }

            // 创建系统托盘菜单
            let show_item = MenuItemBuilder::with_id("show", "显示主窗口").build(app)?;
            // 密码浮窗是 Wayland 拖放交付的载体（纯 GTK）：非 Linux 没有这条链路，
            // 菜单项整个不出现，免得点了没反应
            #[cfg(target_os = "linux")]
            let pool_item = MenuItemBuilder::with_id("pool", "显示/隐藏密码浮窗").build(app)?;
            let lock_item = MenuItemBuilder::with_id("lock", "锁定").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;

            let menu = {
                let builder = MenuBuilder::new(app).item(&show_item);
                #[cfg(target_os = "linux")]
                let builder = builder.item(&pool_item);
                builder
                    .item(&lock_item)
                    .separator()
                    .item(&quit_item)
                    .build()?
            };

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
                        "pool" => {
                            // 菜单项只在 Linux 建（见上面的 cfg）
                            #[cfg(target_os = "linux")]
                            pool_window::toggle(app);
                        }
                        "lock" => {
                            let state = app.state::<AppState>();
                            // 统一锁定入口（含清剪贴板，与命令锁定/空闲锁定语义一致）
                            commands::auth::lockdown(&state, app, "manual-lock");
                        }
                        "quit" => {
                            crate::pending::revoke("quit");
                            // 退出前把池子和它的窗口一起收掉，别留下一个孤儿浮层
                            //（浮窗只在 Linux 存在，其他平台是空实现）
                            pool_window::shutdown(app);
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
                        } else {
                            // 窗口必须以「可见」创建：隐藏态建窗会让 WebView2 不完成初始化、
                            // 丢掉创建时的导航，之后 show/reload 全部空转（页面永远空白）。
                            // 所以「启动不显示」只能在建好之后立刻收进托盘，顺序不能反
                            //（见 tauri.conf.json 的 visible: true）
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.hide();
                            }
                        }
                    }
                } else {
                    // 数据库未初始化，显示主窗口让用户输入密码
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
            }

            // 注册全局快捷键：启动时加密库未解锁、读不到用户配置，先注册默认配置；
            // 解锁成功后由 auth 命令调用 reload_from_settings 换成用户自定义配置。
            // 注册失败不 panic（快捷键可能被其他程序占用），只记日志。
            if let Err(e) = commands::shortcuts::register_all(
                app.handle(),
                &commands::shortcuts::ShortcutConfig::default(),
            ) {
                log::error!("注册全局快捷键失败: {e}");
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
            commands::get_password_stats,
            commands::get_password_detail,
            commands::add_password,
            commands::update_password,
            commands::delete_password,
            commands::toggle_favorite,
            commands::copy_password_to_clipboard,
            // 用户授权的剪贴板交付（注入失败后点提示条才走这里）
            commands::copy_pending_password_to_clipboard,
            commands::touch_password_usage,
            // 交付能力与拖放交付（Wayland）
            commands::get_capabilities,
            commands::prepare_dnd_payload,
            // 入浮窗（窗口本身是纯 GTK，见 pool_window.rs）
            commands::add_to_dnd_pool,
            commands::get_tags,
            commands::rename_tag,
            commands::delete_tag,
            // 分类命令
            commands::get_categories,
            commands::add_category,
            commands::update_category,
            commands::delete_category,
            commands::reorder_categories,
            // 设置命令
            commands::get_settings,
            commands::get_setting,
            commands::save_setting,
            commands::save_settings,
            commands::is_elevated,
            commands::set_auto_start,
            commands::is_auto_start_enabled,
            // 数据管理命令
            commands::export_data,
            commands::import_data,
            // 备份与恢复命令（设置 → 备份页签）
            commands::get_backups,
            commands::restore_backup,
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
            // 窗口缩放换算命令（设计尺寸模型；见 scale.rs）
            scale::sync_render_scale,
            scale::set_countdown_expanded,
            // webview 心跳（watchdog 自愈链路的输入端）
            commands::webview_heartbeat,
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
                        // 网页被判定异常（心跳停止且自动恢复失败）时关闭必须真正退出：
                        // 网页已无法交互，藏进托盘用户也点不出主窗口
                        let webview_unhealthy = state.is_webview_unhealthy();
                        if close_to_tray && !webview_unhealthy {
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
                tauri::WindowEvent::ScaleFactorChanged { .. } => {
                    // 几何缩放变化（拖到另一台显示器 / 改系统缩放）：按登记的
                    // 设计尺寸重排窗口，维持「CSS 视口 ≡ 设计尺寸」不变量（见 scale.rs）
                    scale::apply_all(window.app_handle());
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

