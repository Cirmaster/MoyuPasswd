//! 应用状态模块 - 管理全局状态
//!
//! 使用 Tauri 的状态管理功能存储全局状态。
//! 主要用于存储当前的 AES 密钥（解锁后）和数据库连接。
//!
//! # 安全设计
//!
//! - AES 密钥仅存储在内存中
//! - 锁定时清除密钥
//! - 使用 Mutex 保证线程安全

use std::sync::Mutex;
use crate::db::Database;

/// 应用状态
///
/// 存储应用运行时的全局状态
pub struct AppState {
    /// 数据库连接
    pub db: Database,

    /// 当前的 AES-256 密钥（解锁后存储，锁定时为 None）
    ///
    /// 使用 Mutex 保证线程安全
    /// 使用 Option 表示可能为空（未解锁状态）
    pub aes_key: Mutex<Option<[u8; 32]>>,

    /// 是否已解锁
    pub is_unlocked: Mutex<bool>,
}

impl AppState {
    /// 创建新的应用状态
    ///
    /// # Arguments
    ///
    /// * `db` - 数据库实例
    pub fn new(db: Database) -> Self {
        Self {
            db,
            aes_key: Mutex::new(None),
            is_unlocked: Mutex::new(false),
        }
    }

    /// 设置 AES 密钥
    ///
    /// 解锁时调用，存储从主密码派生的 AES 密钥
    ///
    /// # Arguments
    ///
    /// * `key` - 32 字节的 AES-256 密钥
    pub fn set_aes_key(&self, key: [u8; 32]) {
        let mut aes_key = self.aes_key.lock().unwrap();
        *aes_key = Some(key);

        let mut is_unlocked = self.is_unlocked.lock().unwrap();
        *is_unlocked = true;
    }

    /// 获取 AES 密钥
    ///
    /// 解密密码数据时调用
    ///
    /// # Returns
    ///
    /// 如果已解锁返回密钥的副本，否则返回 None
    pub fn get_aes_key(&self) -> Option<[u8; 32]> {
        let aes_key = self.aes_key.lock().unwrap();
        *aes_key
    }

    /// 清除 AES 密钥
    ///
    /// 锁定时调用，清除内存中的密钥
    pub fn clear_aes_key(&self) {
        let mut aes_key = self.aes_key.lock().unwrap();
        *aes_key = None;

        let mut is_unlocked = self.is_unlocked.lock().unwrap();
        *is_unlocked = false;
    }

    /// 检查是否已解锁
    ///
    /// # Returns
    ///
    /// 如果已解锁返回 true，否则返回 false
    pub fn is_unlocked(&self) -> bool {
        let is_unlocked = self.is_unlocked.lock().unwrap();
        *is_unlocked
    }
}
