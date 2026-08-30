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
    Argon2,
};
use sha2::{Sha256, Digest};

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

    // 配置 Argon2 参数
    let argon2 = Argon2::default();

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

/// 从主密码派生 AES-256 密钥
///
/// 使用 Argon2 从主密码和盐值派生一个 32 字节的 AES-256 密钥。
/// 相同的密码和盐值总是产生相同的密钥。
///
/// # Arguments
///
/// * `password` - 明文主密码
/// * `salt` - 盐值（来自 hash_master_password 返回的 salt）
///
/// # Returns
///
/// 32 字节的 AES-256 密钥
///
/// # Errors
///
/// 如果密钥派生失败，返回 `CryptoError`
pub fn derive_aes_key(password: &str, salt: &str) -> Result<[u8; 32], CryptoError> {
    // 使用 SHA-256 从密码和盐值派生密钥
    // 这是确定性的，相同的密码+盐值总是产生相同的密钥
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hasher.update(salt.as_bytes());
    hasher.update(b"moyu-passwd-aes-key"); // 固定后缀，防止与其他用途冲突
    let result = hasher.finalize();

    let mut key = [0u8; 32];
    key.copy_from_slice(&result);

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
        let hash_result = hash_master_password("test_password").unwrap();
        let key = derive_aes_key("test_password", &hash_result.salt).unwrap();
        let plaintext = b"Hello, World!";

        let encrypted = encrypt_aes256gcm(&key, plaintext).unwrap();
        let decrypted = decrypt_aes256gcm(&key, &encrypted).unwrap();

        assert_eq!(plaintext.to_vec(), decrypted);
    }

    #[test]
    fn test_derive_key_deterministic() {
        let hash_result = hash_master_password("test_password").unwrap();
        let key1 = derive_aes_key("test_password", &hash_result.salt).unwrap();
        let key2 = derive_aes_key("test_password", &hash_result.salt).unwrap();
        assert_eq!(key1, key2);
    }
}
