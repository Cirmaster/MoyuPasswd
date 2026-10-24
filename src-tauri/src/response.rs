//! 统一响应格式模块
//!
//! 所有 Tauri 命令返回给前端的 JSON 统一为如下包裹结构（成功、失败一致）：
//!
//! ```json
//! { "code": 0, "message": "ok", "data": <业务数据> }
//! { "code": 1, "message": "错误信息", "data": null }
//! ```
//!
//! - `code`：`0` = 成功；非 `0` = 失败（当前统一为 `1`，预留细分错误码空间）
//! - `message`：人类可读信息（成功固定为 `"ok"`，失败为具体原因）
//! - `data`：业务数据；失败时为 `null`
//!
//! 命令内部实现保持 `Result<T, String>` 的既有写法不变，
//! 只需把函数体包进 `respond(|| ...)`，由本模块统一完成包裹与错误归一。

use serde::Serialize;

/// 成功响应码
pub const CODE_OK: u32 = 0;

/// 失败响应码（通用业务错误；后续可按需细分类别）
pub const CODE_ERROR: u32 = 1;

/// 统一响应包裹结构
///
/// 字段名即线上 JSON 字段（`code` / `message` / `data`），与前端 `api.ts` 的
/// `ApiResponse` 接口一一对应，改动时两边必须同步。
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    /// 响应码：0 成功，非 0 失败
    pub code: u32,
    /// 人类可读信息
    pub message: String,
    /// 业务数据；失败时为 None（序列化为 null）
    pub data: Option<T>,
}

impl<T: Serialize> ApiResponse<T> {
    /// 构造成功响应（code=0，message="ok"）
    pub fn ok(data: T) -> Self {
        Self {
            code: CODE_OK,
            message: "ok".to_string(),
            data: Some(data),
        }
    }

    /// 构造失败响应（code=1，data=null）
    pub fn err(message: impl Into<String>) -> Self {
        Self {
            code: CODE_ERROR,
            message: message.into(),
            data: None,
        }
    }
}

/// 命令统一返回类型
///
/// `Ok` 与 `Err` 装的是**同一包裹结构**，因此线上 JSON 形状永远一致：
/// 成功 `{ code: 0, message: "ok", data: ... }`，失败 `{ code: 1, message: "...", data: null }`。
///
/// 必须是 `Result` 形态：Tauri 规定带引用参数（如 `State<'_, AppState>`）
/// 的异步命令只能返回 `Result`。
pub type ApiResult<T> = Result<ApiResponse<T>, ApiResponse<()>>;

/// 把「函数体返回 `Result<T, String>`」的命令包装成统一响应格式
///
/// 用法（命令签名改为返回 `ApiResult<T>`，函数体原样包进闭包）：
///
/// ```ignore
/// #[tauri::command]
/// pub async fn get_settings(state: State<'_, AppState>) -> ApiResult<Settings> {
///     respond(|| -> Result<Settings, String> {
///         // ...原有函数体不变...
///         Ok(settings)
///     })
/// }
/// ```
pub fn respond<T: Serialize, F: FnOnce() -> Result<T, String>>(f: F) -> ApiResult<T> {
    // catch_unwind 兜底：命令体若 panic（如解密路径遇到非法输入），
    // 统一归一为普通错误响应，不让 panic 逸出命令层——
    // 否则会崩溃整个进程（sync 命令）或让 IPC 永不响应（async 命令）。
    // AssertUnwindSafe：闭包捕获的 State 引用等跨 unwind 不做自动检查，
    // 这里兜的是崩溃面；panic 后应用状态由 lockdown/重新解锁流程恢复。
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(data)) => Ok(ApiResponse::ok(data)),
        Ok(Err(e)) => Err(ApiResponse::err(e)),
        Err(panic) => {
            // 记录 panic 载荷便于排查；对外只给稳定文案，不泄漏内部细节
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown".to_string());
            log::error!("命令执行 panic（已兜底）: {msg}");
            Err(ApiResponse::err("内部错误：命令执行异常"))
        }
    }
}
