//! 认证命令模块
//!
//! 处理主密码的设置、验证和锁定。
//!
//! # 认证流程
//!
//! ## 首次运行（设置主密码）
//!
//! 1. 应用启动时，数据库已用默认密钥创建
//! 2. 用户设置主密码
//! 3. 生成各种盐值，派生 AES 密钥和数据库密钥
//! 4. 将凭证保存到独立元数据文件
//! 5. 使用 `PRAGMA rekey` 将数据库切换为派生密钥
//!
//! ## 后续运行（验证主密码）
//!
//! 1. 应用启动时，读取元数据文件获取盐值和哈希
//! 2. 用户输入主密码
//! 3. 验证密码（与元数据中的哈希比对）
//! 4. 派生 AES 密钥和数据库密钥
//! 5. 使用派生密钥打开数据库
//!
//! # 命令列表
//!
//! - `set_master_password`: 首次设置主密码
//! - `verify_master_password`: 验证主密码并解锁
//! - `change_master_password`: 修改主密码
//! - `lock_app`: 锁定应用
//! - `is_unlocked`: 检查是否已解锁
//! - `has_master_password`: 检查是否已设置主密码

use tauri::{Emitter, Manager, State};

use crate::crypto;
use crate::db;
use crate::db_meta;
use crate::state::AppState;

/// 认证错误类型
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
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
/// 主密码使用 Argon2id 哈希后存储到元数据文件，
/// 同时生成 AES 密钥盐和数据库加密密钥盐。
///
/// # Arguments
///
/// * `password` - 明文主密码
/// * `state` - 应用状态
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
#[tauri::command]
pub async fn set_master_password(
    password: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 检查是否已设置主密码
    if db_meta::meta_exists(&state.app_dir) {
        return Err(AuthError::PasswordAlreadyExists.to_string());
    }

    // 对主密码进行哈希
    let hash_result = crypto::hash_master_password(&password)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 生成独立的 AES 密钥盐和数据库加密密钥盐
    let aes_salt = crypto::generate_aes_salt();
    let db_salt = crypto::generate_db_salt();

    // 保存凭证到元数据文件
    let meta = db_meta::MetaInfo {
        master_hash: hash_result.hash.clone(),
        master_salt: hash_result.salt,
        aes_salt: aes_salt.clone(),
        db_salt: db_salt.clone(),
    };
    db_meta::save_meta(&state.app_dir, &meta)?;

    // 派生数据库加密密钥
    let db_key = crypto::derive_db_key(&password, &db_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 使用 PRAGMA rekey 将数据库从默认密钥切换为派生密钥
    {
        let db = state.get_db()?;
        db.rekey(&db_key)?;
    }
    state.set_db_key(db_key);

    // 派生 AES 密钥并存储到状态
    let aes_key = crypto::derive_aes_key(&password, &aes_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;
    state.set_aes_key(aes_key);

    Ok(())
}

/// 验证主密码并解锁
///
/// 验证用户输入的主密码是否正确，如果正确则解锁应用。
/// 包含暴力破解防护：连续失败 5 次后锁定。
///
/// # 流程
///
/// 1. 读取元数据文件获取盐值和哈希
/// 2. 验证密码（与哈希比对）
/// 3. 派生 AES 密钥和数据库密钥
/// 4. 使用数据库密钥打开数据库
/// 5. 将数据库和密钥存入应用状态
///
/// # Arguments
///
/// * `password` - 明文主密码
/// * `state` - 应用状态
///
/// # Returns
///
/// 成功返回 Ok(true)，密码错误返回 Ok(false)，其他错误返回错误信息
#[tauri::command]
pub async fn verify_master_password(
    password: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    // 检查是否处于锁定状态
    state.clear_expired_lockout();
    if let Err(remaining) = state.check_lockout() {
        return Err(format!("密码错误次数过多，请等待 {} 秒后重试", remaining));
    }

    // 读取元数据文件
    let meta = db_meta::load_meta(&state.app_dir)
        .ok_or_else(|| AuthError::PasswordNotSet.to_string())?;

    // 验证密码
    let is_valid = crypto::verify_master_password(&password, &meta.master_hash)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    if is_valid {
        // 验证成功，重置锁定计数器
        state.reset_lockout();

        // 派生 AES 密钥
        let aes_key = crypto::derive_aes_key(&password, &meta.aes_salt)
            .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

        // 派生数据库加密密钥
        let db_key = crypto::derive_db_key(&password, &meta.db_salt)
            .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

        // 使用派生密钥打开数据库
        let database = db::Database::new(&state.app_dir, &db_key)
            .map_err(|e| format!("打开数据库失败: {e}"))?;

        // 将数据库和密钥存入状态
        state.set_database(database);
        state.set_db_key(db_key);
        state.set_aes_key(aes_key);

        Ok(true)
    } else {
        // 验证失败，记录并可能触发锁定
        if let Some(lock_msg) = state.record_failure() {
            return Err(lock_msg);
        }
        Ok(false)
    }
}

/// 修改主密码
///
/// 验证旧密码，如果正确则更新为新密码。
/// 同时更新元数据文件中的凭证，并重新加密数据库。
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
#[tauri::command]
pub async fn change_master_password(
    old_password: String,
    new_password: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 读取元数据文件
    let meta = db_meta::load_meta(&state.app_dir)
        .ok_or_else(|| AuthError::PasswordNotSet.to_string())?;

    // 验证旧密码
    let is_valid = crypto::verify_master_password(&old_password, &meta.master_hash)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    if !is_valid {
        return Err(AuthError::WrongOldPassword.to_string());
    }

    // 用旧盐派生旧 AES 密钥
    let old_aes_key = crypto::derive_aes_key(&old_password, &meta.aes_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 对新密码进行哈希
    let hash_result = crypto::hash_master_password(&new_password)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 生成新的独立盐值
    let new_aes_salt = crypto::generate_aes_salt();
    let new_db_salt = crypto::generate_db_salt();

    // 派生新密钥
    let new_aes_key = crypto::derive_aes_key(&new_password, &new_aes_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;
    let new_db_key = crypto::derive_db_key(&new_password, &new_db_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 获取旧数据库密钥（用于 rekey）
    let _old_db_key = state.get_db_key()
        .ok_or("数据库密钥不存在")?;

    // 在同一个事务中：重加密全部已有密码 + 原子提交
    {
        let db = state.get_db()?;
        let conn = db.conn();
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        crate::commands::password::reencrypt_all_passwords(&tx, &old_aes_key, &new_aes_key)?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    // 使用新密钥重新加密数据库
    {
        let db = state.get_db()?;
        db.rekey(&new_db_key)?;
    }

    // 更新元数据文件
    let new_meta = db_meta::MetaInfo {
        master_hash: hash_result.hash,
        master_salt: hash_result.salt,
        aes_salt: new_aes_salt,
        db_salt: new_db_salt,
    };
    db_meta::save_meta(&state.app_dir, &new_meta)?;

    // 更新内存中的密钥
    state.set_aes_key(new_aes_key);
    state.set_db_key(new_db_key);

    Ok(())
}

/// 锁定应用
///
/// 清除内存中的所有密钥和数据库引用，通知所有窗口已锁定。
/// 前端只需调用此命令，不需要额外处理锁定逻辑。
#[tauri::command]
pub async fn lock_app(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    // 清除所有密钥和数据库引用
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
#[tauri::command]
pub async fn is_unlocked(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.is_unlocked())
}

/// 检查是否已设置主密码
///
/// 通过检查元数据文件是否存在来判断。
#[tauri::command]
pub async fn has_master_password(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(db_meta::meta_exists(&state.app_dir))
}
