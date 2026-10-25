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

use crate::response::{respond, ApiResult};
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
    /// 启动时以管理员身份运行（写系统兼容层标记，下次启动弹 UAC 提权；
    /// 用于向以管理员身份运行的目标窗口注入密码）
    #[serde(default)]
    pub run_as_admin: bool,
    /// 注入排除名单（逗号分隔的进程名，如终端类；命中需二次确认）。
    /// 空 = 恢复默认名单（防误清空裸奔），与 inject::load_blacklist 同语义
    #[serde(default)]
    pub inject_blacklist: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // 明暗模式默认深色，与前端 useTheme 的默认值一致；
            // 若与前端不一致，首装后进设置页会把默认值误当用户偏好盖掉前端主题
            theme: "dark".to_string(),
            language: "zh-CN".to_string(),
            auto_lock_time: 5,
            clipboard_clear_time: 30,
            auto_start: false,
            close_to_tray: true,
            show_password_strength: true,
            show_on_startup: true,
            allow_terminal_inject: false,
            run_as_admin: false,
            inject_blacklist: crate::inject::default_blacklist().join(", "),
        }
    }
}

/// 已知设置键白名单（与 get_settings 的解析分支一致，另含快捷键配置）
///
/// 任意键值直写 settings 表会破坏不变量（如伪造 shortcuts 键让设置页解析报错、
/// 无限写垃圾键膨胀数据库），保存前必须过白名单。
const KNOWN_SETTING_KEYS: &[&str] = &[
    "theme",
    "language",
    "auto_lock_time",
    "clipboard_clear_time",
    "auto_start",
    "close_to_tray",
    "show_password_strength",
    "show_on_startup",
    "allow_terminal_inject",
    "run_as_admin",
    "inject_blacklist",
    "shortcuts",
];

/// 校验单个设置项（键白名单 + 值域/长度）
///
/// 数值项：0 = 禁用（自动锁定/自动清除的既有语义），其余按 UI 范围 10~300 钳制——
/// 安全边界不能只靠前端的 min/max 提示（手动输入 0/负数/9999 都能提交）。
fn validate_setting(key: &str, value: &str) -> Result<(), String> {
    if !KNOWN_SETTING_KEYS.contains(&key) {
        return Err(format!("未知的设置项: {key}"));
    }
    if value.len() > 4096 {
        return Err(format!("设置值过长（{key}）"));
    }
    match key {
        // 数值项：0 有「禁用/不自动清除」语义，其余按 UI 输入范围钳制——
        // 安全边界不能只靠前端的 min/max 提示（手动输入 0/负数/9999 都能提交）
        "auto_lock_time" => {
            let n: u64 = value
                .parse()
                .map_err(|_| format!("{key} 必须是非负整数（分钟）"))?;
            if !(n == 0 || (1..=60).contains(&n)) {
                return Err(format!("{key} 取值须为 0（禁用）或 1~60"));
            }
        }
        "clipboard_clear_time" => {
            let n: u64 = value
                .parse()
                .map_err(|_| format!("{key} 必须是非负整数（秒）"))?;
            if !(n == 0 || (10..=300).contains(&n)) {
                return Err(format!("{key} 取值须为 0（不自动清除）或 10~300"));
            }
        }
        "theme" | "language" => {
            let ok = !value.is_empty()
                && value.len() <= 32
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !ok {
                return Err(format!("{key} 取值非法: {value:?}"));
            }
        }
        "inject_blacklist" => {
            // 逗号分隔的进程名（空名单合法 = 恢复默认）；每段限长、
            // 字符集与 Windows 进程名一致（字母数字 . - _），防塞脚本/路径
            for item in value.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    continue;
                }
                let ok = item.len() <= 64
                    && item
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
                if !ok {
                    return Err(format!("inject_blacklist 含非法进程名: {item:?}"));
                }
            }
        }
        "auto_start" | "close_to_tray" | "show_password_strength" | "show_on_startup"
        | "allow_terminal_inject" | "run_as_admin" => {
            if value != "true" && value != "false" {
                return Err(format!("{key} 只能是 true/false"));
            }
        }
        "shortcuts" => {
            // 必须是合法 JSON 对象（快捷键配置）
            match serde_json::from_str::<serde_json::Value>(value) {
                Ok(v) if v.is_object() => {}
                _ => return Err("shortcuts 必须是 JSON 对象".to_string()),
            }
        }
        _ => {}
    }
    Ok(())
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
pub async fn get_settings(state: State<'_, AppState>) -> ApiResult<Settings> {
    respond(|| -> Result<Settings, String> {
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
                "run_as_admin" => settings.run_as_admin = value == "true",
                "inject_blacklist" => {
                    // 与 inject::load_blacklist 同语义：解析后为空同样回退默认名单
                    let list: Vec<&str> = value
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect();
                    settings.inject_blacklist = if list.is_empty() {
                        crate::inject::default_blacklist().join(", ")
                    } else {
                        list.join(", ")
                    };
                }
                _ => {}
            }
        }

        // 同步自动锁定阈值到空闲检测线程。
        // 否则重启后空闲检测一直用默认 5 分钟，直到用户再次保存设置才生效。
        crate::idle::set_auto_lock_minutes(settings.auto_lock_time);

        Ok(settings)

    })}

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
) -> ApiResult<Option<String>> {
    respond(|| -> Result<Option<String>, String> {
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

    })}

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
    app: tauri::AppHandle,
) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
        crate::commands::auth::require_unlocked(&state)?;

        // 键白名单 + 值域校验（后端兜底）
        validate_setting(&key, &value)?;

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
        // 开机自启：写库与注册表必须一致，否则 UI 显示与实际不符
        if key == "auto_start" {
            sync_auto_start(&app, value == "true")?;
        }
        // 管理员启动：写库与系统兼容层标记必须一致，否则开关不生效
        if key == "run_as_admin" {
            sync_run_as_admin(value == "true")?;
        }

        Ok(())

    })}

/// 把注册表中的开机自启状态同步为目标值（幂等，状态一致时不写注册表）
fn sync_auto_start(app: &tauri::AppHandle, enable: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    let current = manager.is_enabled().unwrap_or(false);
    if current == enable {
        return Ok(());
    }
    if enable {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
}

/// 把系统兼容层里的「以管理员身份运行」标记同步为目标值
///
/// 与 sync_auto_start 同理：库里的设置值和实际生效的标记必须一致，
/// 否则 UI 显示开了、下次启动却仍是普通权限。写入失败直接报错，不静默。
fn sync_run_as_admin(enable: bool) -> Result<(), String> {
    crate::elevate::set_run_as_admin(enable)
}

/// 解锁后按库里的设置补同步「以管理员身份运行」标记（自愈，失败不影响解锁）
///
/// 该标记以 exe 路径为键，而开关可能在别的实例里拨的：
/// dev 与安装包路径不同（dev 里拨开标记还写不进，见 elevate.rs）、
/// 更新换安装路径也会失配。解锁拿到库后按当前 exe 路径补写/清理，
/// 保证无论开关在哪里拨的，下次启动都按设置生效。
pub fn sync_run_as_admin_from_db(state: &AppState) {
    let enabled = {
        let db = match state.get_db() {
            Ok(db) => db,
            Err(_) => return,
        };
        // 先落局部变量再收尾：让 db 守卫的借用在块尾表达式求值后立即结束
        let value = db.conn().query_row(
            "SELECT value FROM settings WHERE key = 'run_as_admin'",
            [],
            |row| row.get::<_, String>(0),
        );
        // 没有记录 = 默认关闭，走清理路径把历史标记删掉
        value.map(|v| v == "true").unwrap_or(false)
    };
    if let Err(e) = sync_run_as_admin(enabled) {
        log::warn!("解锁后同步管理员启动标记失败: {e}");
    }
}

/// 查询当前进程是否以管理员身份运行
///
/// 设置页展示用：解释「启动时以管理员身份运行」开关为何需要下次启动才生效。
///
/// # 前端调用
///
/// ```typescript
/// const elevated = await invoke('is_elevated');
/// ```
#[tauri::command]
pub async fn is_elevated(state: State<'_, AppState>) -> ApiResult<bool> {
    respond(|| -> Result<bool, String> {
        // 解锁门禁：与其余设置查询保持一致（设置页本身解锁后才可见）
        crate::commands::auth::require_unlocked(&state)?;
        Ok(crate::elevate::is_elevated())
    })
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
    app: tauri::AppHandle,
) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
        crate::commands::auth::require_unlocked(&state)?;

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
            ("run_as_admin", settings.run_as_admin.to_string()),
            ("inject_blacklist", settings.inject_blacklist),
        ];

        // 逐项校验（键白名单 + 值域），任一非法整体拒绝
        for (key, value) in &settings_vec {
            validate_setting(key, value)?;
        }

        let db = state.get_db()?;
        let conn = db.conn();

        // 整包事务：中途失败不留下半新半旧的配置
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for (key, value) in settings_vec {
            tx.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                params![key, value],
            ).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;

        // 同步更新空闲检测模块的自动锁定时间
        crate::idle::set_auto_lock_minutes(settings.auto_lock_time);

        // 开机自启：写库与注册表必须一致（此前只写库，UI 显示与实际不符）
        sync_auto_start(&app, settings.auto_start)?;

        // 管理员启动：写库与系统兼容层标记必须一致，否则开关不生效
        sync_run_as_admin(settings.run_as_admin)?;

        Ok(())

    })}

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
pub async fn set_auto_start(
    enable: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        // 解锁门禁：开机自启是持久化的系统级行为（写注册表 Run 键），
        // 锁定状态下不允许修改（防被注入的代码静默开启自启做持久化）
        crate::commands::auth::require_unlocked(&state)?;

        let autostart_manager = app.autolaunch();

        if enable {
            autostart_manager.enable().map_err(|e| e.to_string())?;
        } else {
            autostart_manager.disable().map_err(|e| e.to_string())?;
        }

        Ok(())

    })}

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
pub async fn is_auto_start_enabled(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::state::AppState>,
) -> ApiResult<bool> {
    respond(|| -> Result<bool, String> {
        // 解锁门禁：与 set_auto_start 保持一致（状态查询同样不给锁定态留口子）
        crate::commands::auth::require_unlocked(&state)?;
        let autostart_manager = app.autolaunch();
        autostart_manager.is_enabled().map_err(|e| e.to_string())

    })}

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
pub async fn minimize_to_tray(app: tauri::AppHandle) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        if let Some(window) = app.get_webview_window("main") {
            window.hide().map_err(|e| e.to_string())?;
            log::info!("窗口已最小化到托盘");
        }
        Ok(())

    })}
