//! 数据管理命令模块
//!
//! 处理数据的导入导出操作。
//!
//! # 命令列表
//!
//! - `export_data`: 导出所有密码数据为 JSON
//! - `import_data`: 从 JSON 导入密码数据

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::crypto;
use crate::state::AppState;

/// 导出数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportData {
    /// 导出时间
    pub exported_at: i64,
    /// 密码列表
    pub passwords: Vec<ExportPasswordItem>,
    /// 分类列表
    pub categories: Vec<ExportCategory>,
}

/// 导出密码项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportPasswordItem {
    pub id: String,
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub category: String,
    pub is_favorite: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 导出分类
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportCategory {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub sort_order: i32,
}

/// 导入数据请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportData {
    pub passwords: Vec<ExportPasswordItem>,
    pub categories: Vec<ExportCategory>,
}

/// 解密密码数据
fn decrypt_password(aes_key: &[u8; 32], encrypted: &str) -> Result<String, String> {
    let parts: Vec<&str> = encrypted.split(':').collect();
    if parts.len() != 2 {
        return Err("无效的加密数据格式".to_string());
    }

    let nonce = hex::decode(parts[0]).map_err(|e| e.to_string())?;
    let ciphertext = hex::decode(parts[1]).map_err(|e| e.to_string())?;

    let encrypted_data = crypto::EncryptedData { ciphertext, nonce };
    let decrypted = crypto::decrypt_aes256gcm(aes_key, &encrypted_data)
        .map_err(|e| e.to_string())?;

    String::from_utf8(decrypted).map_err(|e| e.to_string())
}

/// 加密密码数据
fn encrypt_password(aes_key: &[u8; 32], password: &str) -> Result<String, String> {
    let encrypted = crypto::encrypt_aes256gcm(aes_key, password.as_bytes())
        .map_err(|e| e.to_string())?;

    let nonce_hex = hex::encode(&encrypted.nonce);
    let ciphertext_hex = hex::encode(&encrypted.ciphertext);

    Ok(format!("{}:{}", nonce_hex, ciphertext_hex))
}

/// 导出所有密码数据
///
/// 将所有密码和分类导出为 JSON 格式。
/// 密码会被解密后导出（方便用户查看）。
///
/// # Arguments
///
/// * `state` - 应用状态
///
/// # Returns
///
/// 导出数据的 JSON 字符串
///
/// # 前端调用
///
/// ```typescript
/// const data = await invoke('export_data');
/// // 保存到文件或显示
/// ```
#[tauri::command]
pub async fn export_data(state: State<'_, AppState>) -> Result<String, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    // 获取 AES 密钥
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    let conn = state.db.conn();

    // 查询所有密码
    let mut stmt = conn
        .prepare(
            "SELECT id, title, username, password_encrypted, url, notes, category_id, is_favorite, created_at, updated_at
             FROM passwords WHERE deleted_at IS NULL"
        )
        .map_err(|e| e.to_string())?;

    let passwords = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let username: String = row.get(2)?;
            let password_encrypted: String = row.get(3)?;
            let url: Option<String> = row.get(4)?;
            let notes: Option<String> = row.get(5)?;
            let category_id: String = row.get(6)?;
            let is_favorite: i32 = row.get(7)?;
            let created_at: i64 = row.get(8)?;
            let updated_at: i64 = row.get(9)?;

            Ok((id, title, username, password_encrypted, url, notes, category_id, is_favorite, created_at, updated_at))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // 解密并转换
    let mut export_passwords = Vec::new();
    for (id, title, username, password_encrypted, url, notes, category_id, is_favorite, created_at, updated_at) in passwords {
        let password = decrypt_password(&aes_key, &password_encrypted)?;

        export_passwords.push(ExportPasswordItem {
            id,
            title,
            username,
            password,
            url,
            notes,
            category: category_id,
            is_favorite: is_favorite != 0,
            created_at,
            updated_at,
        });
    }

    // 查询所有分类
    let mut stmt = conn
        .prepare("SELECT id, name, icon, sort_order FROM categories ORDER BY sort_order")
        .map_err(|e| e.to_string())?;

    let categories = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let icon: Option<String> = row.get(2)?;
            let sort_order: i32 = row.get(3)?;

            Ok(ExportCategory { id, name, icon, sort_order })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // 构建导出数据
    let export_data = ExportData {
        exported_at: chrono::Utc::now().timestamp(),
        passwords: export_passwords,
        categories,
    };

    // 序列化为 JSON
    serde_json::to_string_pretty(&export_data).map_err(|e| e.to_string())
}

/// 导入密码数据
///
/// 从 JSON 格式导入密码和分类。
///
/// # Arguments
///
/// * `json` - JSON 数据字符串
/// * `state` - 应用状态
///
/// # Returns
///
/// 导入的密码数量
///
/// # 前端调用
///
/// ```typescript
/// const count = await invoke('import_data', { json: data });
/// ```
#[tauri::command]
pub async fn import_data(json: String, state: State<'_, AppState>) -> Result<usize, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    // 获取 AES 密钥
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    // 解析 JSON
    let import_data: ImportData = serde_json::from_str(&json)
        .map_err(|e| format!("解析数据失败: {}", e))?;

    let conn = state.db.conn();
    let now = chrono::Utc::now().timestamp();
    let mut imported_count = 0;

    // 导入分类
    for category in &import_data.categories {
        conn.execute(
            "INSERT OR IGNORE INTO categories (id, name, icon, sort_order, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![category.id, category.name, category.icon, category.sort_order, now],
        ).map_err(|e| e.to_string())?;
    }

    // 导入密码
    for password in &import_data.passwords {
        // 加密密码
        let encrypted_password = encrypt_password(&aes_key, &password.password)?;

        conn.execute(
            "INSERT OR REPLACE INTO passwords (id, title, username, password_encrypted, url, notes, category_id, is_favorite, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                password.id,
                password.title,
                password.username,
                encrypted_password,
                password.url,
                password.notes,
                password.category,
                password.is_favorite as i32,
                password.created_at,
                password.updated_at,
            ],
        ).map_err(|e| e.to_string())?;

        imported_count += 1;
    }

    Ok(imported_count)
}
