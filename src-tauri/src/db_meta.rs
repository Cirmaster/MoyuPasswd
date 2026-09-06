//! 数据库凭证元数据模块
//!
//! 管理存储在独立文件中的认证信息（主密码哈希、各种盐值）。
//! 这些信息需要在数据库打开之前就能读取（用于验证密码和派生密钥），
//! 因此不能存在加密的数据库中。
//!
//! # 文件格式
//!
//! JSON 文件，与数据库文件同目录，文件名 `.moyu_passwd_meta`。
//! 文件 ACL 与数据库文件同样收紧（仅当前用户可访问）。
//!
//! # 安全设计
//!
//! - 主密码哈希使用 Argon2id，与数据库加密密钥独立
//! - 每种密钥（AES、数据库）使用独立的随机盐
//! - 文件 ACL 收紧，防止同机其他账户读取

use std::fs;
use std::path::{Path, PathBuf};

/// 元数据文件名
const META_FILENAME: &str = ".moyu_passwd_meta";

/// 凭证元数据
///
/// 存储在独立文件中，用于在数据库打开之前验证密码和派生密钥。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MetaInfo {
    /// 主密码的 Argon2id 哈希（PHC 格式）
    pub master_hash: String,
    /// 主密码验证哈希的盐（PHC 字符串内嵌，此字段为冗余备份）
    pub master_salt: String,
    /// AES-256 密钥派生盐（hex 编码，16 字节随机数）
    pub aes_salt: String,
    /// 数据库加密密钥派生盐（hex 编码，16 字节随机数）
    pub db_salt: String,
}

/// 获取元数据文件路径
pub fn meta_path(app_dir: &Path) -> PathBuf {
    app_dir.join(META_FILENAME)
}

/// 检查元数据文件是否存在
pub fn meta_exists(app_dir: &Path) -> bool {
    meta_path(app_dir).exists()
}

/// 加载元数据
///
/// 如果文件不存在或解析失败返回 None。
pub fn load_meta(app_dir: &Path) -> Option<MetaInfo> {
    let path = meta_path(app_dir);
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// 保存元数据
///
/// 写入文件后收紧 ACL（仅当前用户可访问）。
pub fn save_meta(app_dir: &Path, meta: &MetaInfo) -> Result<(), String> {
    let path = meta_path(app_dir);
    let content = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
    fs::write(&path, content).map_err(|e| format!("保存元数据文件失败: {e}"))?;

    // 收紧文件 ACL（Windows；失败不致命，仅记录日志）
    if let Err(e) = crate::acl::harden_file_acl(&path) {
        log::warn!("元数据文件 ACL 加固失败: {e}");
    }

    Ok(())
}

/// 更新元数据（保留现有字段，覆盖指定字段）
pub fn update_meta(app_dir: &Path, meta: &MetaInfo) -> Result<(), String> {
    save_meta(app_dir, meta)
}
