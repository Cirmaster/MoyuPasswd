//! 认证命令模块
//!
//! 处理主密码的设置、验证和锁定。
//!
//! # 命令列表
//!
//! - `set_master_password`: 首次设置主密码
//! - `verify_master_password`: 验证主密码并解锁
//! - `change_master_password`: 修改主密码
//! - `lock_app`: 锁定应用
//! - `is_unlocked`: 检查是否已解锁
//! - `has_master_password`: 检查是否已设置主密码

use rusqlite::params;
use tauri::{Emitter, State};

use crate::crypto;
use crate::state::AppState;

/// 认证错误类型
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// 数据库错误
    #[error("数据库错误: {0}")]
    Database(String),

    /// 加密错误
    #[error("加密错误: {0}")]
    Crypto(String),

    /// 主密码已存在
    #[error("主密码已设置，请使用修改密码功能")]
    PasswordAlreadyExists,

    /// 主密码未设置
    #[error("主密码未设置，请先设置主密码")]
    PasswordNotSet,

    /// 密码验证失败
    #[error("密码错误")]
    WrongPassword,

    /// 旧密码错误
    #[error("旧密码错误")]
    WrongOldPassword,
}

/// 首次设置主密码
///
/// 用户第一次使用应用时调用，设置主密码。
/// 主密码使用 Argon2id 哈希后存储。
///
/// # Arguments
///
/// * `password` - 明文主密码
/// * `state` - 应用状态
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
///
/// # 前端调用
///
/// ```typescript
/// await invoke('set_master_password', { password: 'my_password' });
/// ```
#[tauri::command]
pub async fn set_master_password(
    password: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 检查是否已设置主密码
    let has_password = has_master_password_internal(&state)?;
    if has_password {
        return Err(AuthError::PasswordAlreadyExists.to_string());
    }

    // 对主密码进行哈希
    let hash_result = crypto::hash_master_password(&password)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 生成独立的 AES 密钥盐
    let aes_salt = crypto::generate_aes_salt();

    // 保存到数据库
    save_master_password(&state, &hash_result.hash, &hash_result.salt, &aes_salt)?;

    // 派生 AES 密钥并存储到状态
    let aes_key = crypto::derive_aes_key(&password, &aes_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;
    state.set_aes_key(aes_key);

    Ok(())
}

/// 验证主密码并解锁
///
/// 验证用户输入的主密码是否正确，如果正确则解锁应用。
///
/// # Arguments
///
/// * `password` - 明文主密码
/// * `state` - 应用状态
///
/// # Returns
///
/// 成功返回 Ok(true)，密码错误返回 Ok(false)，其他错误返回错误信息
///
/// # 前端调用
///
/// ```typescript
/// const isValid = await invoke('verify_master_password', { password: 'my_password' });
/// ```
#[tauri::command]
pub async fn verify_master_password(
    password: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    // 获取存储的哈希值和盐值
    let (stored_hash, _salt, aes_salt) = get_stored_credentials(&state)?;

    // 如果没有设置主密码，返回错误
    if stored_hash.is_empty() {
        return Err(AuthError::PasswordNotSet.to_string());
    }

    // 验证密码
    let is_valid = crypto::verify_master_password(&password, &stored_hash)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    if is_valid {
        // 验证成功，派生 AES 密钥并存储
        let aes_salt = aes_salt
            .ok_or_else(|| "数据库缺少密钥盐，无法解密（旧版本数据需重新初始化）".to_string())?;
        let aes_key = crypto::derive_aes_key(&password, &aes_salt)
            .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;
        state.set_aes_key(aes_key);
        Ok(true)
    } else {
        Ok(false)
    }
}

/// 修改主密码
///
/// 验证旧密码，如果正确则更新为新密码。
///
/// # Arguments
///
/// * `old_password` - 旧主密码
/// * `new_password` - 新主密码
/// * `state` - 应用状态
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
///
/// # 前端调用
///
/// ```typescript
/// await invoke('change_master_password', {
///   oldPassword: 'old_password',
///   newPassword: 'new_password'
/// });
/// ```
#[tauri::command]
pub async fn change_master_password(
    old_password: String,
    new_password: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 获取存储的哈希值和盐值
    let (stored_hash, _salt, old_aes_salt) = get_stored_credentials(&state)?;

    // 验证旧密码
    let is_valid = crypto::verify_master_password(&old_password, &stored_hash)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    if !is_valid {
        return Err(AuthError::WrongOldPassword.to_string());
    }

    // 用旧盐派生旧密钥
    let old_aes_salt = old_aes_salt
        .ok_or_else(|| "数据库缺少密钥盐，无法解密（旧版本数据需重新初始化）".to_string())?;
    let old_key = crypto::derive_aes_key(&old_password, &old_aes_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 对新密码进行哈希
    let hash_result = crypto::hash_master_password(&new_password)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 生成新的独立 AES 密钥盐，并派生新密钥
    let new_aes_salt = crypto::generate_aes_salt();
    let new_key = crypto::derive_aes_key(&new_password, &new_aes_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 在同一个事务中：重加密全部已有密码 + 更新 master_password，保证原子提交
    {
        let conn = state.db.conn();
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        crate::commands::password::reencrypt_all_passwords(&tx, &old_key, &new_key)?;
        update_master_password(&tx, &hash_result.hash, &hash_result.salt, &new_aes_salt)?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    // 更新内存中的 AES 密钥
    state.set_aes_key(new_key);

    Ok(())
}

/// 锁定应用
///
/// 清除内存中的 AES 密钥，并通知所有窗口已锁定。
/// 前端只需调用此命令，不需要额外处理锁定逻辑。
///
/// # Arguments
///
/// * `state` - 应用状态
/// * `app` - Tauri 应用句柄
///
/// # 前端调用
///
/// ```typescript
/// await invoke('lock_app');
/// ```
#[tauri::command]
pub async fn lock_app(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    // 清除 AES 密钥
    state.clear_aes_key();

    // 清空剪贴板（避免刚复制的密码残留），失败不阻断锁定
    if let Err(e) = crate::clipboard::clear_now(&app) {
        log::warn!("锁定时清空剪贴板失败: {e}");
    }

    // 通知所有窗口已锁定
    let _ = app.emit("app-locked", ());

    log::info!("应用已锁定，已通知所有窗口");

    Ok(())
}

/// 检查是否已解锁
///
/// # Arguments
///
/// * `state` - 应用状态
///
/// # Returns
///
/// 如果已解锁返回 true，否则返回 false
///
/// # 前端调用
///
/// ```typescript
/// const isUnlocked = await invoke('is_unlocked');
/// ```
#[tauri::command]
pub async fn is_unlocked(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.is_unlocked())
}

/// 检查是否已设置主密码
///
/// # Arguments
///
/// * `state` - 应用状态
///
/// # Returns
///
/// 如果已设置主密码返回 true，否则返回 false
///
/// # 前端调用
///
/// ```typescript
/// const hasPassword = await invoke('has_master_password');
/// ```
#[tauri::command]
pub async fn has_master_password(state: State<'_, AppState>) -> Result<bool, String> {
    has_master_password_internal(&state)
}

// ==================== 内部辅助函数 ====================

/// 检查是否已设置主密码（内部实现）
fn has_master_password_internal(state: &State<'_, AppState>) -> Result<bool, String> {
    let conn = state.db.conn();

    // 查询 master_password 表是否有记录
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM master_password",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    Ok(count > 0)
}

/// 保存主密码哈希到数据库
fn save_master_password(
    state: &State<'_, AppState>,
    hash: &str,
    salt: &str,
    aes_salt: &str,
) -> Result<(), String> {
    let conn = state.db.conn();
    let now = chrono::Utc::now().timestamp_millis();

    // 插入 master_password 表
    conn.execute(
        "INSERT INTO master_password (id, hash, salt, aes_salt, created_at, updated_at) VALUES (1, ?1, ?2, ?3, ?4, ?4)",
        params![hash, salt, aes_salt, now],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// 更新主密码哈希（在调用方的事务中执行）
fn update_master_password(
    conn: &rusqlite::Connection,
    hash: &str,
    salt: &str,
    aes_salt: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();

    // 更新 master_password 表
    conn.execute(
        "UPDATE master_password SET hash = ?1, salt = ?2, aes_salt = ?3, updated_at = ?4 WHERE id = 1",
        params![hash, salt, aes_salt, now],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// 获取存储的凭证（哈希值、盐值、AES 密钥盐）
fn get_stored_credentials(state: &State<'_, AppState>) -> Result<(String, String, Option<String>), String> {
    let conn = state.db.conn();

    // 查询 master_password 表
    let result = conn.query_row(
        "SELECT hash, salt, aes_salt FROM master_password WHERE id = 1",
        [],
        |row| {
            let hash: String = row.get(0)?;
            let salt: String = row.get(1)?;
            let aes_salt: Option<String> = row.get(2)?;
            Ok((hash, salt, aes_salt))
        },
    );

    match result {
        Ok((hash, salt, aes_salt)) => Ok((hash, salt, aes_salt)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok((String::new(), String::new(), None)),
        Err(e) => Err(e.to_string()),
    }
}
