//! 拖放交付命令（Wayland）
//!
//! 与 `pending`（Ctrl+V 注入）并列的另一条交付路径，核心状态机在 `crate::dnd`，
//! 浮窗本身在 `crate::pool_window`（纯 GTK 实现，不走 webview）。
//!
//! 只有两条命令，分工是固定的：
//!
//! - `add_to_dnd_pool`：主窗口 / 快捷弹窗点「复制」时调。查元数据 + 构造解密闭包
//!   → 入浮窗 → 播入池动效。
//! - `prepare_dnd_payload`：**主窗口整行**拖拽时在 `pointerdown` 调。
//!   浮窗不需要它——浮窗每条自带闭包，GTK 的 `drag_data_get` 按行直接取。

use serde::Serialize;
use tauri::State;

use crate::response::{respond, ApiResult};
use crate::state::AppState;

/// 交付方式能力
#[derive(Debug, Serialize)]
pub struct Capabilities {
    /// 交付方式：
    /// - `inject`：全局按键注入（Windows / Linux X11），密码不进剪贴板
    /// - `dnd`：拖放交付（Linux Wayland），密码不进剪贴板
    /// - `none`：两者都不支持，当前会话交不出去
    pub delivery: String,
}

/// 武装结果
#[derive(Debug, Serialize)]
pub struct PrepareDndResult {
    /// 本次拖放的代数（日志对照用）
    pub seq: u64,
    /// 武装有效期（秒），前端可用于超时提示
    pub ttl: u64,
}

/// 入池结果
#[derive(Debug, Serialize)]
pub struct PoolAddResult {
    /// 入池后的条目数（前端 toast 用它区分「第几条」）
    pub count: usize,
}

/// 查询当前会话的交付能力
#[tauri::command]
pub async fn get_capabilities() -> ApiResult<Capabilities> {
    respond(|| -> Result<Capabilities, String> {
        #[cfg(target_os = "linux")]
        let delivery = {
            if crate::hotkey::injection_supported() {
                "inject"
            } else if crate::x11::is_wayland_session() {
                "dnd"
            } else {
                "none"
            }
        };

        #[cfg(not(target_os = "linux"))]
        let delivery = if crate::hotkey::injection_supported() {
            "inject"
        } else {
            "none"
        };

        log::info!("[CMD] get_capabilities: delivery={delivery}");
        Ok(Capabilities {
            delivery: delivery.to_string(),
        })
    })
}

/// 武装一次拖放交付（不解密）—— 供**主窗口整行拖拽**使用
///
/// 必须在 `pointerdown` 调用而不是 `dragstart`：`invoke` 是异步的，而 `dragstart`
/// 必须同步完成数据设置，来不及等 IPC 返回。
///
/// # 参数
/// * `id` - 密码条目 ID
#[tauri::command]
pub async fn prepare_dnd_payload(
    id: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> ApiResult<PrepareDndResult> {
    respond(|| -> Result<PrepareDndResult, String> {
        log::info!("[CMD] prepare_dnd_payload 被调用: id={}", id);

        if !state.is_unlocked() {
            log::warn!("[CMD] 应用未解锁，拒绝武装拖放");
            return Err("应用未解锁".to_string());
        }

        // 只取密文，不解密：明文在 GTK 取数据那一刻才产生
        let db = state.get_db()?;
        let conn = db.conn();
        let encrypted: String = conn
            .query_row(
                "SELECT password_encrypted FROM passwords WHERE id = ?1 AND deleted_at IS NULL",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    log::warn!("[CMD] 拖放：未找到密码 id={}", id);
                    "未找到密码".to_string()
                }
                other => other.to_string(),
            })?;
        drop(conn);

        let decrypt_fn = super::password::build_decrypt_fn(&app, encrypted);
        let seq = crate::dnd::arm(&id, decrypt_fn);

        Ok(PrepareDndResult {
            seq,
            ttl: crate::dnd::TTL_SECS,
        })
    })
}

/// 把一个密码条目放进浮窗
///
/// 浮窗的拖拽由 GTK 原生拖源处理，`drag_data_get` 按行回调，直接按 id 取闭包，
/// 不需要「先武装、再拖」的跨进程时序（那正是之前「有时能拖、有时拖不动」的根源）。
///
/// # 参数
/// * `id` - 密码条目 ID
#[tauri::command]
pub async fn add_to_dnd_pool(
    id: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> ApiResult<PoolAddResult> {
    respond(|| -> Result<PoolAddResult, String> {
        log::info!("[CMD] add_to_dnd_pool 被调用: id={id}");

        if !state.is_unlocked() {
            log::warn!("[CMD] 应用未解锁，拒绝入池");
            return Err("应用未解锁".to_string());
        }

        // 一次查出元数据和密文：标题/用户名只用于浮窗展示，与主界面可见等级一致
        let (title, username, encrypted): (String, String, String) = {
            let db = state.get_db()?;
            let conn = db.conn();
            conn.query_row(
                "SELECT title, username, password_encrypted FROM passwords \
                 WHERE id = ?1 AND deleted_at IS NULL",
                rusqlite::params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    log::warn!("[CMD] 入池：未找到密码 id={id}");
                    "未找到密码".to_string()
                }
                other => other.to_string(),
            })?
        };

        // 只登记解密闭包、不解密：明文只在拖拽取数据那一刻产生
        let decrypt_fn = super::password::build_decrypt_fn(&app, encrypted);
        crate::dnd::pool_put(
            &id,
            &title,
            &username,
            crate::dnd::PoolPayload::Secret(decrypt_fn),
        );
        let count = crate::dnd::pool_count();

        // 先播一段入池动效（内部失败会直接显示，不会什么都不出现）。
        // 之前用户收起过浮窗，这次点了复制就该再亮出来——所以走 reveal 语义。
        crate::pool_window::reveal_with_fly_in(&app);

        Ok(PoolAddResult { count })
    })
}
