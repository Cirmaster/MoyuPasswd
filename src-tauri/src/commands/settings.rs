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
    /// 诊断日志（默认关）：打开后技术日志级别升到 Info，记录注入流程细节。
    /// 关闭时 release 只记 Warn——失败与异常仍然有，不会因为关着而丢失排障线索
    #[serde(default)]
    pub diagnostic_log: bool,
    /// 密码交付方式：`inject`（默认，Ctrl+V 注入，密码不进剪贴板）、
    /// `clipboard`（直接写系统剪贴板，靠目标端剪贴板同步/原生 Ctrl+V 粘贴）或
    /// `auto`（名单内的目标进程在前台时才写剪贴板，失焦立刻抹掉）。
    ///
    /// clipboard/auto 是给「不转发合成按键」的远控客户端用的（ToDesk 前台时按键在
    /// 到低级钩子之前就被吃掉，注入怎么调都送不进去，而它的剪贴板同步是通的）。
    /// 代价是密码明文会经过系统剪贴板，所以必须用户显式选择；默认值永远是 `inject`
    /// ——安全默认只能是「密码不落剪贴板」。auto 的目标进程名单见
    /// [`Self::auto_clipboard_processes`]。
    #[serde(default = "default_password_delivery")]
    pub password_delivery: String,
    /// 「自动」交付模式的剪贴板目标进程名单（逗号分隔的完整进程名，如 `ToDesk.exe`）。
    /// 命中名单的进程获得前台时密码才进剪贴板，失焦立刻抹掉。
    /// **空 = 使用默认名单**（[`default_auto_clipboard_processes`]，常见远程控制客户端）——
    /// 占位符容易被当成已填内容，所以宁可给真默认值，也不要让人误以为配好了
    #[serde(default = "default_auto_clipboard_processes")]
    pub auto_clipboard_processes: String,
}

/// `password_delivery` 的默认值（`inject`）
///
/// 单独成函数是因为 String 的 `#[serde(default)]` 会给出空串，
/// 空串得再走一遍「当作 inject」的兜底，不如让默认值本身就是合法值。
fn default_password_delivery() -> String {
    "inject".to_string()
}

/// `auto_clipboard_processes` 的默认名单（常见远程控制客户端）
///
/// 这类客户端不转发合成按键、只同步剪贴板，正是「自动」交付模式的服务对象；
/// 用户可在设置里增删。空名单一律回退到它（见 [`read_auto_clipboard_processes`]）。
fn default_auto_clipboard_processes() -> String {
    "ToDesk.exe, SunloginClient.exe".to_string()
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
            diagnostic_log: false,
            password_delivery: default_password_delivery(),
            auto_clipboard_processes: default_auto_clipboard_processes(),
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
    "diagnostic_log",
    "password_delivery",
    "auto_clipboard_processes",
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
        // 进程名单：逗号分隔的进程名。空名单合法（inject_blacklist = 恢复默认名单，
        // auto_clipboard_processes = 读取时回退默认远控名单）；每段限长、
        // 字符集与 Windows 进程名一致（字母数字 . - _），防塞脚本/路径
        "inject_blacklist" | "auto_clipboard_processes" => {
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
                    return Err(format!("{key} 含非法进程名: {item:?}"));
                }
            }
        }
        "password_delivery" => {
            // 只有这三种：其余值会让复制密码时悄悄改变密码的去向，必须在这里挡死
            if value != "inject" && value != "clipboard" && value != "auto" {
                return Err(format!("{key} 只能是 inject / clipboard / auto"));
            }
        }
        "auto_start" | "close_to_tray" | "show_password_strength" | "show_on_startup"
        | "allow_terminal_inject" | "run_as_admin" | "diagnostic_log" => {
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
                "diagnostic_log" => settings.diagnostic_log = value == "true",
                "password_delivery" => settings.password_delivery = value,
                "auto_clipboard_processes" => {
                    // 空名单一律回退默认名单：占位符曾被误当成已填内容，宁可给真默认值
                    settings.auto_clipboard_processes = if value.trim().is_empty() {
                        default_auto_clipboard_processes()
                    } else {
                        value
                    };
                }
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
        // 诊断日志：开关即生效（运行期改日志级别，不需要重启）
        if key == "diagnostic_log" {
            crate::logging::apply_level(value == "true");
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

/// 解锁后按库里的「诊断日志」开关校正日志级别（失败不影响解锁）
///
/// 启动时数据库还没解锁，日志只能按构建类型给默认级别；解锁拿到库后必须立刻校正，
/// 否则用户开了诊断日志、重启后却要等到打开设置页才生效。
pub fn apply_log_level_from_db(state: &AppState) {
    let enabled = {
        let db = match state.get_db() {
            Ok(db) => db,
            Err(_) => return,
        };
        // 先落局部变量：让 db 守卫的借用在块尾表达式求值前结束（同 sync_run_as_admin_from_db）
        let value = db.conn().query_row(
            "SELECT value FROM settings WHERE key = 'diagnostic_log'",
            [],
            |row| row.get::<_, String>(0),
        );
        value.map(|v| v == "true").unwrap_or(false)
    };
    crate::logging::apply_level(enabled);
}

/// 读取密码交付方式（`inject` / `clipboard`）
///
/// 给 `copy_password_to_clipboard` 在分流前读一次用；取不到或值非法一律按 `inject`——
/// 安全默认只能是「密码不进剪贴板」，宁可退到注入路径也不要悄悄把明文写进剪贴板。
pub fn read_password_delivery(conn: &rusqlite::Connection) -> &'static str {
    let value = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'password_delivery'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();
    match value.as_deref() {
        Some("clipboard") => "clipboard",
        Some("auto") => "auto",
        _ => "inject",
    }
}

/// 读取「自动」交付模式的剪贴板目标进程名单
///
/// 逗号分隔 → 去空去空白的 `Vec`（匹配时忽略大小写，见 auto_clipboard.rs）。
/// 名单缺失或为空时回退默认名单（常见远程控制客户端）——空名单退化为注入的
/// 老语义容易让人白试一次（配了「自动」却没生效），默认带上更符合直觉。
pub fn read_auto_clipboard_processes(conn: &rusqlite::Connection) -> Vec<String> {
    let raw = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'auto_clipboard_processes'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(default_auto_clipboard_processes);
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
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
            ("diagnostic_log", settings.diagnostic_log.to_string()),
            ("password_delivery", settings.password_delivery),
            ("auto_clipboard_processes", settings.auto_clipboard_processes),
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

        // 诊断日志：连批保存也要即时生效
        crate::logging::apply_level(settings.diagnostic_log);

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
