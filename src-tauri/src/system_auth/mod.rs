//! 系统认证模块
//!
//! 使用 `tauri-plugin-biometry` 插件实现生物识别认证和安全密钥存储。
//!
//! # 支持平台
//!
//! - **Windows**: Windows Hello（指纹、面部、PIN）
//! - **macOS**: Touch ID
//! - **iOS**: Face ID / Touch ID
//! - **Android**: 指纹 / 面部
//!
//! # 安全设计
//!
//! - 密钥存储在系统安全存储中（Windows Credential Manager / macOS Keychain）
//! - 读取密钥需要通过生物识别认证
//! - 密钥在内存中使用后立即清零

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
use tauri_plugin_biometry::{
    BiometryExt, AuthOptions, SetDataOptions, GetDataOptions, DataOptions, RemoveDataOptions,
};

/// 系统认证是否正在进行中
/// 用于防止认证期间窗口因失焦被隐藏
static AUTH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// 安全存储的域名
const DOMAIN: &str = "com.moyu.passwd";

/// 密钥存储的名称
const KEY_NAME: &str = "encryption-keys";

/// 系统认证凭证文件名（存储启用状态标记）
const SYSTEM_AUTH_MARKER: &str = ".system_auth_enabled";

/// 获取主窗口的 WebviewWindow
fn get_main_window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or("无法获取主窗口".to_string())
}

/// 检查系统认证是否可用
pub fn is_available(app: &tauri::AppHandle) -> bool {
    match app.biometry().status() {
        Ok(status) => status.is_available,
        Err(e) => {
            log::warn!("检查生物识别状态失败: {}", e);
            false
        }
    }
}

/// 触发系统认证
///
/// # Arguments
///
/// * `app` - Tauri 应用句柄
/// * `reason` - 显示给用户的认证原因
pub fn request_authentication(app: &tauri::AppHandle, reason: &str) -> Result<bool, String> {
    log::info!("触发系统认证: {}", reason);

    let window = get_main_window(app)?;

    let options = AuthOptions {
        allow_device_credential: Some(true),  // 允许使用设备密码作为备用
        cancel_title: Some("取消".to_string()),
        ..Default::default()
    };

    match app.biometry().authenticate(window, reason.to_string(), options) {
        Ok(()) => {
            log::info!("系统认证成功");
            Ok(true)
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("userCancel") || err_str.contains("UserCancel") {
                log::info!("用户取消认证");
                Ok(false)
            } else {
                log::warn!("系统认证失败: {}", e);
                Err(format!("认证失败: {}", e))
            }
        }
    }
}

/// 检查系统认证是否正在进行中
///
/// 用于窗口焦点事件处理，防止认证期间窗口被隐藏
pub fn is_auth_in_progress() -> bool {
    AUTH_IN_PROGRESS.load(Ordering::SeqCst)
}

/// 设置认证进行中状态
fn set_auth_in_progress(in_progress: bool) {
    AUTH_IN_PROGRESS.store(in_progress, Ordering::SeqCst);
}

/// 将密钥存储到安全存储
///
/// # Arguments
///
/// * `key_data` - 要存储的密钥数据（64 字节）
/// * `app_dir` - 应用数据目录（用于存储标记文件）
/// * `app` - Tauri 应用句柄
pub fn store_key(key_data: &[u8], app_dir: &Path, app: &tauri::AppHandle) -> Result<(), String> {
    let key_hex = hex::encode(key_data);

    let options = SetDataOptions {
        domain: DOMAIN.to_string(),
        name: KEY_NAME.to_string(),
        data: key_hex,
    };

    let window = get_main_window(app)?;

    // 认证前：临时取消 quick-search 窗口的置顶，避免遮挡系统认证弹窗
    let qs_window = app.get_webview_window("quick-search");
    if let Some(ref qs) = qs_window {
        let _ = qs.set_always_on_top(false);
    }

    // 标记认证开始（防止窗口失焦被隐藏）
    set_auth_in_progress(true);
    let result = app.biometry().set_data(window, options);
    set_auth_in_progress(false);

    // 认证后：恢复 quick-search 窗口的置顶
    if let Some(ref qs) = qs_window {
        let _ = qs.set_always_on_top(true);
    }

    result.map_err(|e| format!("存储密钥失败: {}", e))?;

    let marker_path = app_dir.join(SYSTEM_AUTH_MARKER);
    std::fs::write(&marker_path, "enabled").map_err(|e| format!("写入标记文件失败: {e}"))?;

    log::info!("密钥已存入安全存储");
    Ok(())
}

/// 从安全存储读取密钥
///
/// 读取时会触发生物识别认证。
pub fn retrieve_key(app: &tauri::AppHandle, reason: &str) -> Result<[u8; 64], String> {
    let has = app.biometry().has_data(DataOptions {
        domain: DOMAIN.to_string(),
        name: KEY_NAME.to_string(),
    }).map_err(|e| format!("检查密钥失败: {}", e))?;

    if !has {
        return Err("未找到存储的密钥".to_string());
    }

    let window = get_main_window(app)?;

    // 认证前：临时取消 quick-search 窗口的置顶，避免遮挡系统认证弹窗
    let qs_window = app.get_webview_window("quick-search");
    if let Some(ref qs) = qs_window {
        let _ = qs.set_always_on_top(false);
    }

    // 标记认证开始（防止窗口失焦被隐藏）
    set_auth_in_progress(true);
    let result = app.biometry().get_data(window, GetDataOptions {
        domain: DOMAIN.to_string(),
        name: KEY_NAME.to_string(),
        reason: reason.to_string(),
        cancel_title: Some("取消".to_string()),
    });
    set_auth_in_progress(false);

    // 认证后：恢复 quick-search 窗口的置顶
    if let Some(ref qs) = qs_window {
        let _ = qs.set_always_on_top(true);
    }

    let response = result.map_err(|e| format!("读取密钥失败: {}", e))?;

    let key_bytes = hex::decode(&response.data)
        .map_err(|e| format!("解码密钥失败: {}", e))?;

    if key_bytes.len() != 64 {
        return Err(format!("密钥长度不正确: {}（期望 64）", key_bytes.len()));
    }

    let mut result = [0u8; 64];
    result.copy_from_slice(&key_bytes);
    Ok(result)
}

/// 从安全存储删除密钥
pub fn delete_key(app_dir: &Path, app: &tauri::AppHandle) -> Result<(), String> {
    let has = app.biometry().has_data(DataOptions {
        domain: DOMAIN.to_string(),
        name: KEY_NAME.to_string(),
    }).unwrap_or(false);

    if has {
        app.biometry().remove_data(RemoveDataOptions {
            domain: DOMAIN.to_string(),
            name: KEY_NAME.to_string(),
        }).map_err(|e| format!("删除密钥失败: {}", e))?;
    }

    let marker_path = app_dir.join(SYSTEM_AUTH_MARKER);
    if marker_path.exists() {
        std::fs::remove_file(&marker_path).map_err(|e| format!("删除标记文件失败: {e}"))?;
    }

    log::info!("系统快速解锁已禁用");
    Ok(())
}

/// 检查系统认证是否已启用
pub fn is_enabled(app_dir: &Path) -> bool {
    app_dir.join(SYSTEM_AUTH_MARKER).exists()
}

/// 获取当前平台的认证方式名称
pub fn auth_method_name() -> &'static str {
    #[cfg(target_os = "windows")]
    { "Windows Hello" }

    #[cfg(target_os = "macos")]
    { "Touch ID" }

    #[cfg(target_os = "ios")]
    { "Face ID / Touch ID" }

    #[cfg(target_os = "linux")]
    { "系统认证" }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios", target_os = "linux")))]
    { "系统认证" }
}
