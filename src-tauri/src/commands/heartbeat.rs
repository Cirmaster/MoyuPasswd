//! webview 心跳命令
//!
//! 前端主线程每 5 秒上报一次「我还活着」。Rust 侧看门狗（watchdog.rs）发现心跳停止
//! 即判定网页崩溃/卡死并自动重载，是自愈链路的输入端。
//!
//! 注意：**不能加 require_unlocked 门禁**——解锁页也要打心跳，
//! 否则停留在解锁页时会被误判为网页死亡、触发无谓重载。

use tauri::State;

use crate::response::{respond, ApiResult};
use crate::state::AppState;

/// 更新 webview 心跳时间戳并清除异常标记（幂等、无副作用）
#[tauri::command]
pub async fn webview_heartbeat(state: State<'_, AppState>) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        state.mark_webview_alive();
        Ok(())
    })
}
