//! 数据库凭证元数据模块
//!
//! 管理认证信息（主密码哈希、各种盐值）的存储。
//! 这些信息需要在数据库打开之前就能读取（用于验证密码和派生密钥），
//! 因此不能存在加密的数据库中。
//!
//! # 存储策略
//!
//! - **Windows**: 使用 Windows Credential Manager（系统加密存储）
//! - **其他平台**: 使用文件存储 + ACL 保护
//!
//! # 安全设计
//!
//! - 主密码哈希使用 Argon2id，与数据库加密密钥独立
//! - 每种密钥（AES、数据库）使用独立的随机盐
//! - Windows Credential Manager 由系统加密保护，绑定当前用户

use std::path::Path;

/// 凭证元数据
///
/// 用于在数据库打开之前验证密码和派生密钥。
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

// ==================== Windows 实现 ====================

#[cfg(target_os = "windows")]
mod platform {
    use super::MetaInfo;
    use std::path::Path;
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::Security::Credentials::{
        CredDeleteW, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    };
    use windows::core::{PCWSTR, PWSTR};

    /// Credential Manager 中的凭据名称
    const CREDENTIAL_NAME: &str = "com.moyu.passwd.meta";

    /// 检查元数据是否存在
    pub fn meta_exists(_app_dir: &Path) -> bool {
        read_credential().is_some()
    }

    /// 加载元数据
    pub fn load_meta(_app_dir: &Path) -> Option<MetaInfo> {
        let data = read_credential()?;
        serde_json::from_str(&data).ok()
    }

    /// 保存元数据
    pub fn save_meta(_app_dir: &Path, meta: &MetaInfo) -> Result<(), String> {
        let content = serde_json::to_string(meta).map_err(|e| e.to_string())?;
        write_credential(&content)
    }

    /// 读取凭据
    fn read_credential() -> Option<String> {
        unsafe {
            let name_wide = to_wide(CREDENTIAL_NAME);
            let mut credential: *mut CREDENTIALW = std::ptr::null_mut();

            let result = CredReadW(
                PCWSTR(name_wide.as_ptr()),
                CRED_TYPE_GENERIC,
                0,
                &mut credential,
            );

            if result.is_err() {
                return None;
            }

            let cred = &*credential;
            if cred.CredentialBlobSize == 0 || cred.CredentialBlob.is_null() {
                return None;
            }

            // 凭据数据是 UTF-16 编码
            let blob = std::slice::from_raw_parts(
                cred.CredentialBlob,
                cred.CredentialBlobSize as usize,
            );

            // 转换为 UTF-8 字符串
            let utf16_data: Vec<u16> = blob
                .chunks_exact(2)
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                .collect();

            String::from_utf16(&utf16_data).ok()
        }
    }

    /// 写入凭据
    fn write_credential(data: &str) -> Result<(), String> {
        unsafe {
            let name_wide = to_wide(CREDENTIAL_NAME);
            let target_wide = to_wide(CREDENTIAL_NAME);

            // 将字符串转为 UTF-16 字节
            let utf16: Vec<u16> = data.encode_utf16().collect();
            let blob: Vec<u8> = utf16
                .iter()
                .flat_map(|&c| c.to_le_bytes().to_vec())
                .collect();

            let credential = CREDENTIALW {
                Flags: CRED_FLAGS(0),
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(name_wide.as_ptr() as *mut u16),
                Comment: PWSTR::null(),
                LastWritten: FILETIME::default(),
                CredentialBlobSize: blob.len() as u32,
                CredentialBlob: blob.as_ptr() as *mut u8,
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                AttributeCount: 0,
                Attributes: std::ptr::null_mut(),
                TargetAlias: PWSTR::null(),
                UserName: PWSTR(target_wide.as_ptr() as *mut u16),
            };

            let result = CredWriteW(&credential, 0);
            if result.is_err() {
                return Err(format!("写入凭据失败: {:?}", result));
            }

            log::info!("元数据已存入 Windows Credential Manager");
            Ok(())
        }
    }

    /// 删除凭据
    pub fn delete_meta(_app_dir: &Path) -> Result<(), String> {
        unsafe {
            let name_wide = to_wide(CREDENTIAL_NAME);
            let result = CredDeleteW(
                PCWSTR(name_wide.as_ptr()),
                CRED_TYPE_GENERIC,
                0,
            );

            if result.is_ok() {
                log::info!("已从 Credential Manager 删除元数据");
            }
            Ok(())
        }
    }

    /// 字符串转 UTF-16（以 null 结尾）
    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

// ==================== 其他平台实现（文件存储） ====================

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::MetaInfo;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// 元数据文件名
    const META_FILENAME: &str = ".moyu_passwd_meta";

    fn meta_path(app_dir: &Path) -> PathBuf {
        app_dir.join(META_FILENAME)
    }

    pub fn meta_exists(app_dir: &Path) -> bool {
        meta_path(app_dir).exists()
    }

    pub fn load_meta(app_dir: &Path) -> Option<MetaInfo> {
        let content = fs::read_to_string(meta_path(app_dir)).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub fn save_meta(app_dir: &Path, meta: &MetaInfo) -> Result<(), String> {
        let path = meta_path(app_dir);
        let content = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
        fs::write(&path, content).map_err(|e| format!("保存元数据文件失败: {e}"))?;

        // 收紧文件 ACL
        if let Err(e) = crate::acl::harden_file_acl(&path) {
            log::warn!("元数据文件 ACL 加固失败: {e}");
        }

        Ok(())
    }

    pub fn delete_meta(app_dir: &Path) -> Result<(), String> {
        let path = meta_path(app_dir);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("删除元数据文件失败: {e}"))?;
        }
        Ok(())
    }
}

// ==================== 公共接口 ====================

/// 检查元数据是否存在
pub fn meta_exists(app_dir: &Path) -> bool {
    platform::meta_exists(app_dir)
}

/// 加载元数据
pub fn load_meta(app_dir: &Path) -> Option<MetaInfo> {
    platform::load_meta(app_dir)
}

/// 保存元数据
pub fn save_meta(app_dir: &Path, meta: &MetaInfo) -> Result<(), String> {
    platform::save_meta(app_dir, meta)
}

/// 删除元数据
pub fn delete_meta(app_dir: &Path) -> Result<(), String> {
    platform::delete_meta(app_dir)
}
