//! 加密模块 - 密码哈希和数据加密
//!
//! 提供主密码的哈希验证和密码数据的 AES-256-GCM 加密/解密。
//!
//! # 安全设计
//!
//! - 主密码使用 Argon2id 进行哈希
//! - 密码数据使用 AES-256-GCM 加密
//! - 每条记录使用随机 IV（初始化向量）
//! - 加密密钥从主密码派生

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::{
    password_hash::{rand_core::RngCore, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2, Algorithm, Version, Params,
};

// ==================== Argon2 安全参数 ====================
// OWASP 推荐参数：64 MiB 内存、3 次迭代、1 并行度
const ARGON2_MEMORY_COST: u32 = 65536;  // 64 MiB
const ARGON2_ITERATIONS: u32 = 3;
const ARGON2_PARALLELISM: u32 = 1;

/// 创建配置了 OWASP 推荐参数的 Argon2 实例
///
/// 用于哈希主密码和派生 AES 密钥。
/// 验证旧哈希时不需要此函数，因为 Argon2 PHC 字符串自带参数。
fn argon2_instance<'a>() -> Argon2<'a> {
    let params = Params::new(
        ARGON2_MEMORY_COST,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        Some(32), // 输出长度 32 字节
    ).expect("Argon2 参数配置失败");

    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// 加密错误类型
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// Argon2 哈希错误
    #[error("哈希错误: {0}")]
    HashError(String),

    /// 密码验证错误
    #[error("密码验证错误: {0}")]
    VerifyError(String),

    /// AES 加密错误
    #[error("加密错误: {0}")]
    EncryptError(String),

    /// AES 解密错误
    #[error("解密错误: {0}")]
    DecryptError(String),

    /// 无效的密钥长度
    #[error("无效的密钥长度")]
    InvalidKeyLength,
}

/// 主密码哈希结果
#[derive(Debug, Clone)]
pub struct PasswordHashResult {
    /// 哈希值
    pub hash: String,
    /// 盐值
    pub salt: String,
}

/// 对主密码进行哈希
///
/// 使用 Argon2id 算法对主密码进行哈希处理。
/// Argon2id 是密码哈希的推荐算法，具有抗 GPU 和 ASIC 攻击的特性。
///
/// # Arguments
///
/// * `password` - 明文主密码
///
/// # Returns
///
/// 包含哈希值和盐值的结构体
///
/// # Errors
///
/// 如果哈希过程失败，返回 `CryptoError::HashError`
pub fn hash_master_password(password: &str) -> Result<PasswordHashResult, CryptoError> {
    // 生成随机盐值
    let salt = SaltString::generate(&mut OsRng);

    // 使用 OWASP 推荐参数的 Argon2id 实例
    let argon2 = argon2_instance();

    // 对密码进行哈希
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| CryptoError::HashError(e.to_string()))?
        .to_string();

    Ok(PasswordHashResult {
        hash,
        salt: salt.to_string(),
    })
}

/// 验证主密码
///
/// 验证明文密码是否与存储的哈希值匹配。
///
/// # Arguments
///
/// * `password` - 明文主密码
/// * `stored_hash` - 存储的哈希值
///
/// # Returns
///
/// 如果密码匹配返回 `true`，否则返回 `false`
///
/// # Errors
///
/// 如果验证过程失败，返回 `CryptoError::VerifyError`
pub fn verify_master_password(password: &str, stored_hash: &str) -> Result<bool, CryptoError> {
    // 解析存储的哈希值
    let parsed_hash = PasswordHash::new(stored_hash)
        .map_err(|e| CryptoError::VerifyError(e.to_string()))?;

    // 配置 Argon2 参数
    let argon2 = Argon2::default();

    // 验证密码
    match argon2.verify_password(password.as_bytes(), &parsed_hash) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// 生成 AES 密钥盐（16 字节随机数，hex 编码）
///
/// 与主密码验证哈希的盐相互独立，用于派生 AES-256 密钥。
/// 保证即使验证哈希被破解，加密密钥也无法从数据库直接还原。
pub fn generate_aes_salt() -> String {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    hex::encode(salt)
}

/// 生成数据库加密密钥盐（16 字节随机数，hex 编码）
///
/// 与 AES 密钥盐独立，用于派生 SQLCipher 数据库加密密钥。
pub fn generate_db_salt() -> String {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    hex::encode(salt)
}

/// 从主密码派生数据库加密密钥
///
/// 使用 Argon2id 从主密码和独立的数据库盐派生一个 32 字节密钥，
/// 用于 SQLCipher 的 PRAGMA key。
///
/// # Arguments
///
/// * `password` - 明文主密码
/// * `db_salt` - 数据库密钥盐（hex 字符串）
///
/// # Returns
///
/// 32 字节的数据库加密密钥
pub fn derive_db_key(password: &str, db_salt: &str) -> Result<[u8; 32], CryptoError> {
    let salt = hex::decode(db_salt)
        .map_err(|e| CryptoError::HashError(e.to_string()))?;

    let mut key = [0u8; 32];
    argon2_instance()
        .hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(|e| CryptoError::HashError(e.to_string()))?;

    Ok(key)
}

/// 从主密码派生 AES-256 密钥
///
/// 使用 Argon2id 从主密码和独立的 AES 盐派生一个 32 字节的 AES-256 密钥。
/// 相同的密码和盐值总是产生相同的密钥。
///
/// 注意：这里必须使用独立的 `aes_salt`，不能复用 `hash_master_password` 返回的盐，
/// 否则密钥会与验证哈希同源，导致密钥以可还原形式残留在数据库中。
///
/// # Arguments
///
/// * `password` - 明文主密码
/// * `aes_salt` - AES 密钥盐（由 `generate_aes_salt` 生成的 hex 字符串）
///
/// # Returns
///
/// 32 字节的 AES-256 密钥
///
/// # Errors
///
/// 如果密钥派生失败，返回 `CryptoError`
pub fn derive_aes_key(password: &str, aes_salt: &str) -> Result<[u8; 32], CryptoError> {
    // 解析 hex 盐
    let salt = hex::decode(aes_salt)
        .map_err(|e| CryptoError::HashError(e.to_string()))?;

    // 使用 OWASP 推荐参数的 Argon2id 派生 32 字节密钥
    let mut key = [0u8; 32];
    argon2_instance()
        .hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(|e| CryptoError::HashError(e.to_string()))?;

    Ok(key)
}

/// AES-256-GCM 加密结果
#[derive(Debug, Clone)]
pub struct EncryptedData {
    /// 加密后的数据
    pub ciphertext: Vec<u8>,
    /// 初始化向量（12 字节）
    pub nonce: Vec<u8>,
}

/// 使用 AES-256-GCM 加密数据
///
/// # Arguments
///
/// * `key` - 32 字节的 AES-256 密钥
/// * `plaintext` - 明文数据
///
/// # Returns
///
/// 包含密文和随机 IV 的结构体
///
/// # Errors
///
/// 如果加密过程失败，返回 `CryptoError::EncryptError`
pub fn encrypt_aes256gcm(key: &[u8; 32], plaintext: &[u8]) -> Result<EncryptedData, CryptoError> {
    // 创建 AES-256-GCM 加密器
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| CryptoError::EncryptError(e.to_string()))?;

    // 生成随机 IV（12 字节）
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // 加密数据
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| CryptoError::EncryptError(e.to_string()))?;

    Ok(EncryptedData {
        ciphertext,
        nonce: nonce_bytes.to_vec(),
    })
}

/// 使用 AES-256-GCM 解密数据
///
/// # Arguments
///
/// * `key` - 32 字节的 AES-256 密钥
/// * `encrypted_data` - 包含密文和 IV 的加密数据
///
/// # Returns
///
/// 解密后的明文数据
///
/// # Errors
///
/// 如果解密过程失败，返回 `CryptoError::DecryptError`
pub fn decrypt_aes256gcm(key: &[u8; 32], encrypted_data: &EncryptedData) -> Result<Vec<u8>, CryptoError> {
    // 创建 AES-256-GCM 解密器
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| CryptoError::DecryptError(e.to_string()))?;

    // 解密数据
    let nonce = Nonce::from_slice(&encrypted_data.nonce);
    let plaintext = cipher
        .decrypt(nonce, encrypted_data.ciphertext.as_ref())
        .map_err(|e| CryptoError::DecryptError(e.to_string()))?;

    Ok(plaintext)
}

/// 计算密码强度等级
///
/// 评分规则：长度 ≥8/≥12/≥16 各 +1，含小写/大写/数字/特殊字符各 +1，
/// 总分 0–7 映射到 1–4 档。
///
/// # Returns
///
/// 0(空/未知) | 1(弱) | 2(中) | 3(强) | 4(非常强)
pub fn password_strength_level(password: &str) -> i32 {
    if password.is_empty() {
        return 0;
    }

    let mut score = 0;
    if password.len() >= 8 {
        score += 1;
    }
    if password.len() >= 12 {
        score += 1;
    }
    if password.len() >= 16 {
        score += 1;
    }
    if password.chars().any(|c| c.is_ascii_lowercase()) {
        score += 1;
    }
    if password.chars().any(|c| c.is_ascii_uppercase()) {
        score += 1;
    }
    if password.chars().any(|c| c.is_ascii_digit()) {
        score += 1;
    }
    if password.chars().any(|c| !c.is_ascii_alphanumeric()) {
        score += 1;
    }

    if score <= 2 {
        1
    } else if score <= 4 {
        2
    } else if score <= 5 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify() {
        let password = "my_secure_password";
        let result = hash_master_password(password).unwrap();

        assert!(verify_master_password(password, &result.hash).unwrap());
        assert!(!verify_master_password("wrong_password", &result.hash).unwrap());
    }

    #[test]
    fn test_encrypt_and_decrypt() {
        let aes_salt = generate_aes_salt();
        let key = derive_aes_key("test_password", &aes_salt).unwrap();
        let plaintext = b"Hello, World!";

        let encrypted = encrypt_aes256gcm(&key, plaintext).unwrap();
        let decrypted = decrypt_aes256gcm(&key, &encrypted).unwrap();

        assert_eq!(plaintext.to_vec(), decrypted);
    }

    #[test]
    fn test_derive_key_deterministic() {
        let aes_salt = generate_aes_salt();
        let key1 = derive_aes_key("test_password", &aes_salt).unwrap();
        let key2 = derive_aes_key("test_password", &aes_salt).unwrap();
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_derive_key_different_salts_differ() {
        let salt1 = generate_aes_salt();
        let salt2 = generate_aes_salt();
        let key1 = derive_aes_key("test_password", &salt1).unwrap();
        let key2 = derive_aes_key("test_password", &salt2).unwrap();
        assert_ne!(key1, key2);
    }
}
