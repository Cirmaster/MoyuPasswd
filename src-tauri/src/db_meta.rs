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
//! - 元数据读取区分「不存在」与「读取失败」（三态），
//!   读取失败绝不允许被上层当作「首次运行」，防止误删密码库
//! - 修改主密码时先写「事务日志」（旧 meta 副本），
//!   中途崩溃由启动恢复逻辑确定性回滚（见 `commands::auth`）

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

/// 元数据状态（三态）
///
/// 关键约束：**「从未设置」与「读取失败」必须可区分**。
/// 若读取失败被当成「不存在」，上层会走首次运行流程并可能毁掉已有密码库。
#[derive(Debug)]
pub enum MetaStatus {
    /// 元数据存在且可解析
    Present,
    /// 确认从未设置（可以安全地走首次运行流程）
    Absent,
    /// 读取失败或内容损坏（**绝不能当作「不存在」处理**）
    Unavailable(String),
}

/// 主元数据存储键（修改主密码事务日志以外的正常元数据）
const META_KEY: &str = "meta";
/// 修改主密码事务日志存储键（保存旧 meta 副本，作为回滚依据）
const JOURNAL_KEY: &str = "meta.old";

// ==================== Windows 实现 ====================

#[cfg(target_os = "windows")]
mod platform {
    use std::path::Path;
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS,
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };
    use windows::core::{PCWSTR, PWSTR};

    /// 凭据名称前缀
    const CREDENTIAL_PREFIX: &str = "com.moyu.passwd.";

    /// 存储键 → 凭据名称
    fn cred_name(key: &str) -> String {
        format!("{}{}", CREDENTIAL_PREFIX, key)
    }

    /// 读取凭据原始字符串
    ///
    /// # Returns
    ///
    /// - `Ok(None)`：确认不存在（`ERROR_NOT_FOUND`）
    /// - `Ok(Some(s)`：内容
    /// - `Err(_)`：读取失败或内容异常（**调用方不得当作「不存在」**）
    pub fn read_meta(_app_dir: &Path, key: &str) -> Result<Option<String>, String> {
        unsafe {
            let name_wide = to_wide(&cred_name(key));
            let mut credential: *mut CREDENTIALW = std::ptr::null_mut();

            let result = CredReadW(
                PCWSTR(name_wide.as_ptr()),
                CRED_TYPE_GENERIC,
                0,
                &mut credential,
            );

            if let Err(err) = result {
                // ERROR_NOT_FOUND = 1168：确认从未设置；其他错误一律上抛
                let code = (err.code().0 as u32) & 0xFFFF;
                if code == 1168 {
                    return Ok(None);
                }
                return Err(format!("读取凭据失败: {err}"));
            }

            let cred = &*credential;
            // 先把数据拷贝出来，随后立即 CredFree 释放（防止泄漏）
            let data = if cred.CredentialBlobSize == 0 || cred.CredentialBlob.is_null() {
                // 空凭据是异常状态：按「不可读」处理，绝不当作「不存在」
                Err("凭据内容为空（元数据损坏）".to_string())
            } else {
                let blob = std::slice::from_raw_parts(
                    cred.CredentialBlob,
                    cred.CredentialBlobSize as usize,
                );
                let utf16_data: Vec<u16> = blob
                    .chunks_exact(2)
                    .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();
                String::from_utf16(&utf16_data)
                    .map(Some)
                    .map_err(|e| format!("凭据内容编码错误: {e}"))
            };

            CredFree(credential.cast());
            data
        }
    }

    /// 写入凭据原始字符串
    pub fn write_meta(_app_dir: &Path, key: &str, data: &str) -> Result<(), String> {
        unsafe {
            let name_wide = to_wide(&cred_name(key));
            let target_wide = to_wide(&cred_name(key));

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

            log::info!("元数据已存入 Windows Credential Manager（key={}）", key);
            Ok(())
        }
    }

    /// 删除凭据（不存在视为成功，保证幂等）
    pub fn delete_meta(_app_dir: &Path, key: &str) -> Result<(), String> {
        unsafe {
            let name_wide = to_wide(&cred_name(key));
            let result = CredDeleteW(
                PCWSTR(name_wide.as_ptr()),
                CRED_TYPE_GENERIC,
                0,
            );

            match result {
                Ok(()) => {
                    log::info!("已从 Credential Manager 删除凭据（key={}）", key);
                    Ok(())
                }
                Err(err) => {
                    let code = (err.code().0 as u32) & 0xFFFF;
                    if code == 1168 {
                        Ok(())
                    } else {
                        Err(format!("删除凭据失败: {err}"))
                    }
                }
            }
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
    use std::fs;
    use std::path::{Path, PathBuf};

    /// 存储键 → 元数据文件名
    fn meta_path(app_dir: &Path, key: &str) -> PathBuf {
        app_dir.join(format!(".moyu_passwd_{}", key))
    }

    /// 读取元数据原始字符串
    ///
    /// # Returns
    ///
    /// - `Ok(None)`：文件不存在（确认从未设置）
    /// - `Ok(Some(s)`：内容
    /// - `Err(_)`：读取失败（**不得当作「不存在」**）
    pub fn read_meta(app_dir: &Path, key: &str) -> Result<Option<String>, String> {
        match fs::read_to_string(meta_path(app_dir, key)) {
            Ok(content) => Ok(Some(content)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读取元数据文件失败: {e}")),
        }
    }

    /// 写入元数据原始字符串
    pub fn write_meta(app_dir: &Path, key: &str, data: &str) -> Result<(), String> {
        let path = meta_path(app_dir, key);
        fs::write(&path, data).map_err(|e| format!("保存元数据文件失败: {e}"))?;

        // 收紧文件 ACL
        if let Err(e) = crate::acl::harden_file_acl(&path) {
            log::warn!("元数据文件 ACL 加固失败: {e}");
        }

        Ok(())
    }

    /// 删除元数据（不存在视为成功，保证幂等）
    pub fn delete_meta(app_dir: &Path, key: &str) -> Result<(), String> {
        let path = meta_path(app_dir, key);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("删除元数据文件失败: {e}")),
        }
    }
}

// ==================== 公共接口 ====================

/// 检查元数据状态（三态）
///
/// 首次运行判定、设置主密码前检查等关键路径必须使用本函数，
/// 根据 `MetaStatus` 分别处理，**绝不能把 `Unavailable` 当成 `Absent`**。
pub fn meta_status(app_dir: &Path) -> MetaStatus {
    match load_meta(app_dir) {
        Ok(Some(_)) => MetaStatus::Present,
        Ok(None) => MetaStatus::Absent,
        Err(e) => MetaStatus::Unavailable(e),
    }
}

/// 加载元数据
///
/// # Returns
///
/// - `Ok(None)`：确认从未设置
/// - `Ok(Some(meta)`：元数据
/// - `Err(_)`：读取失败或内容损坏
pub fn load_meta(app_dir: &Path) -> Result<Option<MetaInfo>, String> {
    match platform::read_meta(app_dir, META_KEY)? {
        Some(content) => serde_json::from_str(&content)
            .map(Some)
            .map_err(|e| format!("元数据损坏: {e}")),
        None => Ok(None),
    }
}

/// 保存元数据
pub fn save_meta(app_dir: &Path, meta: &MetaInfo) -> Result<(), String> {
    let content = serde_json::to_string(meta).map_err(|e| e.to_string())?;
    platform::write_meta(app_dir, META_KEY, &content)
}

/// 删除元数据
pub fn delete_meta(app_dir: &Path) -> Result<(), String> {
    platform::delete_meta(app_dir, META_KEY)
}

// ==================== 修改主密码事务日志 ====================
//
// 协议（详见 commands::auth::change_master_password）：
// 1. 备份数据库
// 2. save_meta_journal(旧 meta)   —— 进入可回滚窗口
// 3. 重加密 / rekey / save_meta(新 meta)
// 4. delete_meta_journal()        —— 提交点
// 任意时刻崩溃：启动时只要日志存在就按旧 meta + 备份回滚。

/// 写入修改主密码事务日志（保存旧 meta 副本）
pub fn save_meta_journal(app_dir: &Path, meta: &MetaInfo) -> Result<(), String> {
    let content = serde_json::to_string(meta).map_err(|e| e.to_string())?;
    platform::write_meta(app_dir, JOURNAL_KEY, &content)
}

/// 读取修改主密码事务日志
///
/// # Returns
///
/// - `Ok(None)`：无未完成的修改
/// - `Ok(Some(meta)`：回滚依据（旧 meta）
/// - `Err(_)`：日志读取失败或损坏（此时不应贸然回滚）
pub fn load_meta_journal(app_dir: &Path) -> Result<Option<MetaInfo>, String> {
    match platform::read_meta(app_dir, JOURNAL_KEY)? {
        Some(content) => serde_json::from_str(&content)
            .map(Some)
            .map_err(|e| format!("事务日志损坏: {e}")),
        None => Ok(None),
    }
}

/// 删除修改主密码事务日志（提交点；不存在视为成功）
pub fn delete_meta_journal(app_dir: &Path) -> Result<(), String> {
    platform::delete_meta(app_dir, JOURNAL_KEY)
}
