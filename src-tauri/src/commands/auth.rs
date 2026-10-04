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
//!
//! 另提供非命令辅助函数：
//! - `lockdown`: 统一锁定入口（所有锁定路径必须走这里）
//! - `recover_interrupted_change`: 启动时回滚被中断的主密码修改

use std::path::Path;

use tauri::{Emitter, State};

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

/// 校验主密码强度（后端兜底）
///
/// 与前端规则一致：至少 6 个字符。
/// 前端校验可被绕过，后端必须独立校验。
fn validate_password_strength(password: &str) -> Result<(), String> {
    if password.chars().count() < 6 {
        return Err("主密码长度至少 6 位".to_string());
    }
    Ok(())
}

/// 首次设置主密码
///
/// 用户第一次使用应用时调用，设置主密码。
/// 主密码使用 Argon2id 哈希后存储到元数据文件，
/// 同时生成 AES 密钥盐和数据库加密密钥盐。
/// 如果系统认证可用，自动启用系统快速解锁。
///
/// # Arguments
///
/// * `password` - 明文主密码
/// * `state` - 应用状态
/// * `app` - Tauri 应用句柄
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
#[tauri::command]
pub async fn set_master_password(
    password: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // 检查元数据状态（三态）：读取失败绝不当作「未设置」，避免覆盖已有配置
    match db_meta::meta_status(&state.app_dir) {
        db_meta::MetaStatus::Present => {
            return Err(AuthError::PasswordAlreadyExists.to_string());
        }
        db_meta::MetaStatus::Unavailable(reason) => {
            return Err(format!("元数据不可读，已中止设置: {reason}"));
        }
        db_meta::MetaStatus::Absent => {}
    }

    // 后端强度校验（与前端规则一致，前端校验可被绕过）
    validate_password_strength(&password)?;

    // 对主密码进行哈希
    let hash_result = crypto::hash_master_password(&password)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 生成独立的 AES 密钥盐和数据库加密密钥盐
    let aes_salt = crypto::generate_aes_salt();
    let db_salt = crypto::generate_db_salt();

    // 派生数据库加密密钥和 AES 密钥
    let db_key = crypto::derive_db_key(&password, &db_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;
    let aes_key = crypto::derive_aes_key(&password, &aes_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 使用 PRAGMA rekey 将数据库从默认密钥切换为派生密钥
    //
    // 顺序约定：先 rekey、后存 meta。
    // - rekey 失败：meta 未写入，用户可直接重试；
    // - rekey 成功而 save_meta 失败：下次启动走首跑路径，旧库无法用默认密钥打开
    //   会被隔离保留（db::quarantine_existing_db），不会困死用户。
    {
        let db = state.get_db()?;
        db.rekey(&db_key)?;
    }

    // 保存凭证到元数据文件
    let meta = db_meta::MetaInfo {
        master_hash: hash_result.hash,
        master_salt: hash_result.salt,
        aes_salt: aes_salt.clone(),
        db_salt: db_salt.clone(),
    };
    db_meta::save_meta(&state.app_dir, &meta)?;

    state.set_db_key(db_key);
    state.set_aes_key(aes_key);

    // 如果系统认证可用，自动启用系统快速解锁
    if crate::system_auth::is_available(&app) {
        // 将两个密钥合并存储（64 字节）
        let mut combined_key = [0u8; 64];
        combined_key[..32].copy_from_slice(&aes_key);
        combined_key[32..].copy_from_slice(&db_key);

        // 存入系统安全存储（会触发系统认证）
        if let Err(e) = crate::system_auth::store_key(&combined_key, &state.app_dir, &app) {
            // 启用失败不影响主流程，只记录日志
            log::warn!("自动启用系统快速解锁失败: {}", e);
        }

        // 清零临时变量
        use zeroize::Zeroize;
        combined_key.zeroize();
    }

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
        .map_err(|e| format!("读取元数据失败: {e}"))?
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
/// # 崩溃一致性
///
/// 重加密、rekey、更新元数据横跨数据库与 Credential Manager 两个存储，无法真原子。
/// 采用「备份 + 事务日志 + 回滚」协议：任一步失败或中途崩溃，
/// 都会由 `rollback_change` / `recover_interrupted_change` 确定性回滚到修改前状态，
/// 绝不留下半新半旧的数据。
///
/// # Arguments
///
/// * `old_password` - 旧主密码
/// * `new_password` - 新主密码
/// * `state` - 应用状态
/// * `app` - Tauri 应用句柄（轮换系统快速解锁密钥用）
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
#[tauri::command]
pub async fn change_master_password(
    old_password: String,
    new_password: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // ---- 前置检查 ----
    // 暴力破解防护（与 verify_master_password 共用失败计数/锁定）
    state.clear_expired_lockout();
    if let Err(remaining) = state.check_lockout() {
        return Err(format!("密码错误次数过多，请等待 {} 秒后重试", remaining));
    }

    // 先要求已解锁，再验证密码：未解锁时对/错旧密码返回同一错误，消除明文 oracle
    if !state.is_unlocked() {
        return Err("应用未解锁，请先解锁应用".to_string());
    }

    // 读取元数据文件
    let meta = db_meta::load_meta(&state.app_dir)
        .map_err(|e| format!("读取元数据失败: {e}"))?
        .ok_or_else(|| AuthError::PasswordNotSet.to_string())?;

    // 验证旧密码
    let is_valid = crypto::verify_master_password(&old_password, &meta.master_hash)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;
    if !is_valid {
        // 验证失败，记录并可能触发锁定
        if let Some(lock_msg) = state.record_failure() {
            return Err(lock_msg);
        }
        return Err(AuthError::WrongOldPassword.to_string());
    }

    // 新密码强度校验（后端兜底，不依赖前端）
    validate_password_strength(&new_password)?;

    // 验证成功，重置锁定计数器
    state.reset_lockout();

    // ---- 派生旧/新密钥 ----
    // 用旧盐派生旧 AES 密钥
    let old_aes_key = crypto::derive_aes_key(&old_password, &meta.aes_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 对新密码进行哈希
    let hash_result = crypto::hash_master_password(&new_password)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;

    // 生成新的独立盐值并派生新密钥
    let new_aes_salt = crypto::generate_aes_salt();
    let new_db_salt = crypto::generate_db_salt();
    let new_aes_key = crypto::derive_aes_key(&new_password, &new_aes_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;
    let new_db_key = crypto::derive_db_key(&new_password, &new_db_salt)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;
    let new_meta = db_meta::MetaInfo {
        master_hash: hash_result.hash,
        master_salt: hash_result.salt,
        aes_salt: new_aes_salt,
        db_salt: new_db_salt,
    };

    // ---- 备份 + 事务日志协议 ----
    // 步骤 2：备份当前数据库（失败则中止；此时未改动任何状态）
    {
        let db = state.get_db()?;
        backup_database(&state.app_dir, &db)?;
    }

    // 步骤 3：写事务日志（旧 meta 副本），从此进入可回滚窗口
    db_meta::save_meta_journal(&state.app_dir, &meta)?;

    // 步骤 4-6：重加密 → rekey → 更新 meta。任一步失败 → 回滚。
    // 注意：db.conn() 的 guard 与 rekey 内部锁的是同一把互斥锁，必须分块持有，不能嵌套。
    let apply_result: Result<(), String> = (|| {
        // 步骤 4：在同一事务中重加密全部已有密码 + 提交
        {
            let db = state.get_db()?;
            let conn = db.conn();
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            crate::commands::password::reencrypt_all_passwords(&tx, &old_aes_key, &new_aes_key)?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        // 步骤 5：使用新密钥重新加密数据库
        {
            let db = state.get_db()?;
            db.rekey(&new_db_key)?;
        }
        // 步骤 6：更新元数据文件
        db_meta::save_meta(&state.app_dir, &new_meta)?;
        Ok(())
    })();

    if let Err(e) = apply_result {
        // 先完整锁定（作废 pending/钩子、清密钥丢弃连接），再从备份还原文件
        lockdown(&state, &app, "change-password-rollback");
        return match rollback_change(&state.app_dir) {
            Ok(()) => Err(format!(
                "修改主密码失败，已恢复到修改前状态，请重新解锁。原因: {e}"
            )),
            Err(re) => Err(format!(
                "修改主密码失败（{e}），且回滚失败（{re}）；数据库备份保留在 backup 目录，请勿删除应用数据并联系支持。"
            )),
        };
    }

    // 步骤 7：删除事务日志（提交点）
    db_meta::delete_meta_journal(&state.app_dir)?;

    // 旧 pending 的解密闭包仍持有旧密钥（可解旧密文），立即作废
    crate::pending::revoke("change-master-password");
    crate::hotkey::deactivate();

    // 更新内存中的密钥
    state.set_aes_key(new_aes_key);
    state.set_db_key(new_db_key);

    // 轮换系统快速解锁密钥：先作废旧组合密钥，宁可失效也不残留旧密钥
    if crate::system_auth::is_enabled(&state.app_dir) {
        if let Err(e) = crate::system_auth::delete_key(&state.app_dir, &app) {
            log::warn!("作废旧系统快速解锁密钥失败: {e}");
        }
        let mut combined_key = [0u8; 64];
        combined_key[..32].copy_from_slice(&new_aes_key);
        combined_key[32..].copy_from_slice(&new_db_key);
        if let Err(e) = crate::system_auth::store_key(&combined_key, &state.app_dir, &app) {
            log::warn!("重新写入系统快速解锁密钥失败（快速解锁已禁用，请手动重新启用）: {e}");
        }
        use zeroize::Zeroize;
        combined_key.zeroize();
    }

    log::info!("主密码修改成功");
    Ok(())
}

/// 修改主密码前备份数据库文件
///
/// 先把 WAL 落盘，保证备份完整；备份成功是进入事务日志窗口的前提。
fn backup_database(app_dir: &Path, db: &db::Database) -> Result<(), String> {
    // WAL checkpoint：把 WAL 中的改动并入主库，尽量得到单文件备份
    {
        let conn = db.conn();
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }

    let backup_dir = app_dir.join("backup");
    std::fs::create_dir_all(&backup_dir).map_err(|e| format!("创建备份目录失败: {e}"))?;

    let db_path = app_dir.join(db::DB_NAME);
    let bak_path = backup_dir.join(format!("{}.bak", db::DB_NAME));
    std::fs::copy(&db_path, &bak_path).map_err(|e| format!("备份数据库失败: {e}"))?;

    // checkpoint 后 WAL/SHM 通常已清空；若仍存在也一并备份
    for suffix in ["-wal", "-shm"] {
        let src = app_dir.join(format!("{}{}", db::DB_NAME, suffix));
        if src.exists() {
            let _ = std::fs::copy(&src, backup_dir.join(format!("{}.bak{}", db::DB_NAME, suffix)));
        }
    }
    log::info!("修改主密码前已备份数据库: {:?}", bak_path);
    Ok(())
}

/// 回滚被中断的主密码修改
///
/// 从备份还原数据库文件，用事务日志中的旧 meta 覆盖主 meta，删除日志。
/// 调用前提：数据库连接已关闭（进程内回滚前需先 `clear_aes_key`）。
///
/// 崩溃矩阵：
/// | 崩溃点              | 磁盘状态            | 恢复动作          |
/// |---------------------|---------------------|-------------------|
/// | 备份后、写日志前    | 全旧                | 无需恢复          |
/// | 写日志后、提交前    | 日志在 + 库半新     | 回滚 → 全旧       |
/// | 存 meta 后、删日志前| 日志在 + 全新       | 回滚 → 全旧（用户重做修改） |
/// | 删日志后            | 日志无 + 全新       | 提交成功          |
fn rollback_change(app_dir: &Path) -> Result<(), String> {
    // 事务日志是回滚依据
    let journal = db_meta::load_meta_journal(app_dir)?.ok_or("事务日志不存在，无法回滚")?;

    let backup_dir = app_dir.join("backup");
    let bak_path = backup_dir.join(format!("{}.bak", db::DB_NAME));
    if !bak_path.exists() {
        return Err("数据库备份不存在，无法回滚".to_string());
    }

    // 还原数据库文件：先清掉当前的 db/wal/shm，再拷回备份
    let db_path = app_dir.join(db::DB_NAME);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(app_dir.join(format!("{}-wal", db::DB_NAME)));
    let _ = std::fs::remove_file(app_dir.join(format!("{}-shm", db::DB_NAME)));
    std::fs::copy(&bak_path, &db_path).map_err(|e| format!("还原数据库备份失败: {e}"))?;
    for suffix in ["-wal", "-shm"] {
        let bak = backup_dir.join(format!("{}.bak{}", db::DB_NAME, suffix));
        if bak.exists() {
            let _ = std::fs::copy(&bak, app_dir.join(format!("{}{}", db::DB_NAME, suffix)));
        }
    }

    // 还原旧 meta 并删除事务日志
    db_meta::save_meta(app_dir, &journal)?;
    db_meta::delete_meta_journal(app_dir)?;
    log::warn!("已回滚中断的主密码修改，恢复到修改前状态");
    Ok(())
}

/// 启动时调用：若上次修改主密码被中断，确定性回滚到修改前状态
///
/// 幂等：无事务日志时不做任何事。不需要用户密码。
pub fn recover_interrupted_change(app_dir: &Path) {
    match db_meta::load_meta_journal(app_dir) {
        Ok(Some(_)) => {
            log::warn!("检测到未完成的主密码修改，执行回滚");
            if let Err(e) = rollback_change(app_dir) {
                log::error!("回滚未完成的主密码修改失败（备份保留在 backup 目录）: {e}");
            }
        }
        Ok(None) => {}
        Err(e) => log::error!("读取主密码修改事务日志失败: {e}"),
    }
}

/// 统一锁定入口
///
/// 所有锁定路径（命令/空闲/托盘）必须走这里，避免遗漏步骤：
///
/// 1. 作废待粘贴密码（含解密闭包），杜绝锁定后仍可注入
/// 2. 卸载 Ctrl+V 钩子
/// 3. 清零内存中的密钥与数据库引用
/// 4. 清空剪贴板（避免刚复制的密码残留），失败不阻断锁定
/// 5. 通知所有窗口已锁定
pub fn lockdown(state: &AppState, app: &tauri::AppHandle, reason: &str) {
    crate::pending::revoke(reason);
    crate::hotkey::deactivate();
    state.clear_aes_key();
    if let Err(e) = crate::clipboard::clear_now(app) {
        log::warn!("锁定时清空剪贴板失败: {e}");
    }
    let _ = app.emit("app-locked", ());
    log::info!("应用已锁定（reason={}），已通知所有窗口", reason);
}

/// 锁定应用
///
/// 清除内存中的所有密钥和数据库引用，通知所有窗口已锁定。
/// 前端只需调用此命令，不需要额外处理锁定逻辑。
#[tauri::command]
pub async fn lock_app(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    lockdown(&state, &app, "lock-app");
    Ok(())
}

/// 检查是否已解锁
#[tauri::command]
pub async fn is_unlocked(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.is_unlocked())
}

/// 检查是否已设置主密码
///
/// 通过元数据状态（三态）判断：读取失败返回错误，绝不当作「未设置」。
#[tauri::command]
pub async fn has_master_password(state: State<'_, AppState>) -> Result<bool, String> {
    match db_meta::meta_status(&state.app_dir) {
        db_meta::MetaStatus::Present => Ok(true),
        db_meta::MetaStatus::Absent => Ok(false),
        db_meta::MetaStatus::Unavailable(reason) => Err(format!("元数据不可读: {reason}")),
    }
}

// ==================== 系统认证命令 ====================

/// 启用系统快速解锁
///
/// 将密钥存入安全存储（插件会自动触发认证）。
#[tauri::command]
pub async fn enable_system_auth(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    // 检查系统认证是否可用
    if !crate::system_auth::is_available(&app) {
        return Err("当前系统不支持系统认证".to_string());
    }

    // 检查是否已解锁（需要密钥）
    let aes_key = state.get_aes_key()
        .ok_or("请先解锁应用，再启用系统快速解锁")?;
    let db_key = state.get_db_key()
        .ok_or("数据库密钥不存在，请重新登录")?;

    // 将两个密钥合并存储（64 字节）
    // 插件的 set_data 会自动触发 Windows Hello 认证
    let mut combined_key = [0u8; 64];
    combined_key[..32].copy_from_slice(&aes_key);
    combined_key[32..].copy_from_slice(&db_key);

    // 存入系统安全存储
    crate::system_auth::store_key(&combined_key, &state.app_dir, &app)?;

    // 清零临时变量
    use zeroize::Zeroize;
    combined_key.zeroize();

    log::info!("系统快速解锁已启用");
    Ok(())
}

/// 禁用系统快速解锁
///
/// 从系统安全存储中删除密钥。
#[tauri::command]
pub async fn disable_system_auth(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    crate::system_auth::delete_key(&state.app_dir, &app)?;
    log::info!("系统快速解锁已禁用");
    Ok(())
}

/// 检查系统快速解锁是否已启用
#[tauri::command]
pub async fn is_system_auth_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(crate::system_auth::is_enabled(&state.app_dir))
}

/// 检查系统认证是否可用
#[tauri::command]
pub async fn is_system_auth_available(app: tauri::AppHandle) -> Result<bool, String> {
    Ok(crate::system_auth::is_available(&app))
}

/// 获取系统认证方式名称（用于前端显示）
#[tauri::command]
pub async fn get_system_auth_method() -> Result<String, String> {
    Ok(crate::system_auth::auth_method_name().to_string())
}

/// 使用系统认证解锁
///
/// 直接从安全存储读取密钥（插件会自动触发认证），读取成功即解锁。
#[tauri::command]
pub async fn unlock_with_system_auth(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<bool, String> {
    // 检查系统认证是否已启用
    if !crate::system_auth::is_enabled(&state.app_dir) {
        return Err("系统快速解锁未启用".to_string());
    }

    // 从安全存储读取密钥（插件会自动触发 Windows Hello 认证）
    let auth_method = crate::system_auth::auth_method_name();
    let reason = format!("使用 {} 解锁 MoyuPasswd", auth_method);

    let combined_key = match crate::system_auth::retrieve_key(&app, &reason) {
        Ok(key) => key,
        Err(e) => {
            // 用户取消或认证失败
            let err_str = e.to_string();
            if err_str.contains("userCancel") || err_str.contains("UserCancel") {
                return Ok(false);
            }
            return Err(e);
        }
    };

    // 拆分为两个密钥
    let mut aes_key = [0u8; 32];
    let mut db_key = [0u8; 32];
    aes_key.copy_from_slice(&combined_key[..32]);
    db_key.copy_from_slice(&combined_key[32..]);

    // 使用数据库密钥打开数据库
    let database = match db::Database::new(&state.app_dir, &db_key) {
        Ok(d) => d,
        Err(e) => {
            // 存储的密钥已失效（例如改密后残留的旧密钥）：自动作废，避免永久报错
            if let Err(de) = crate::system_auth::delete_key(&state.app_dir, &app) {
                log::warn!("作废失效的系统快速解锁密钥失败: {de}");
            }
            return Err(format!(
                "系统快速解锁已失效，请使用主密码解锁（可在设置中重新启用）: {e}"
            ));
        }
    };

    // 将数据库和密钥存入状态
    state.set_database(database);
    state.set_db_key(db_key);
    state.set_aes_key(aes_key);

    // 清零临时变量
    use zeroize::Zeroize;
    aes_key.zeroize();
    db_key.zeroize();
    let mut combined_key = combined_key;
    combined_key.zeroize();

    log::info!("系统认证解锁成功");
    Ok(true)
}

