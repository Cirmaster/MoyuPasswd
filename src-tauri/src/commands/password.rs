//! 密码命令模块
//!
//! 处理密码的增删改查操作。
//! 所有密码数据都使用 AES-256-GCM 加密后存储。
//!
//! # 命令列表
//!
//! - `get_passwords`: 获取密码列表
//! - `get_password_by_id`: 获取单个密码
//! - `add_password`: 添加密码
//! - `update_password`: 更新密码
//! - `delete_password`: 删除密码（软删除）
//! - `toggle_favorite`: 切换收藏状态

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::crypto;
use crate::state::AppState;

/// 密码项结构体
///
/// 对应数据库中的 passwords 表
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordItem {
    /// 唯一标识符
    pub id: String,
    /// 网站/应用名称
    pub title: String,
    /// 登录用户名或邮箱
    pub username: String,
    /// 登录密码（解密后的明文）
    pub password: String,
    /// 网站 URL
    pub url: Option<String>,
    /// 备注信息
    pub notes: Option<String>,
    /// 所属分类 ID
    pub category: String,
    /// 是否收藏
    pub is_favorite: bool,
    /// 创建时间（时间戳）
    pub created_at: i64,
    /// 更新时间（时间戳）
    pub updated_at: i64,
}

/// 新增密码请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewPassword {
    /// 网站/应用名称
    pub title: String,
    /// 登录用户名或邮箱
    pub username: String,
    /// 登录密码
    pub password: String,
    /// 网站 URL
    pub url: Option<String>,
    /// 备注信息
    pub notes: Option<String>,
    /// 所属分类 ID
    pub category: String,
    /// 是否收藏
    pub is_favorite: bool,
}

/// 更新密码请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePassword {
    /// 网站/应用名称
    pub title: Option<String>,
    /// 登录用户名或邮箱
    pub username: Option<String>,
    /// 登录密码
    pub password: Option<String>,
    /// 网站 URL
    pub url: Option<String>,
    /// 备注信息
    pub notes: Option<String>,
    /// 所属分类 ID
    pub category: Option<String>,
    /// 是否收藏
    pub is_favorite: Option<bool>,
}

/// 解密密码数据
fn decrypt_password(aes_key: &[u8; 32], encrypted: &str) -> Result<String, String> {
    // 解析加密数据（格式：nonce_hex:ciphertext_hex）
    let parts: Vec<&str> = encrypted.split(':').collect();
    if parts.len() != 2 {
        return Err("无效的加密数据格式".to_string());
    }

    let nonce = hex::decode(parts[0]).map_err(|e| e.to_string())?;
    let ciphertext = hex::decode(parts[1]).map_err(|e| e.to_string())?;

    let encrypted_data = crypto::EncryptedData {
        ciphertext,
        nonce,
    };

    let decrypted = crypto::decrypt_aes256gcm(aes_key, &encrypted_data)
        .map_err(|e| e.to_string())?;

    String::from_utf8(decrypted).map_err(|e| e.to_string())
}

/// 加密密码数据
fn encrypt_password(aes_key: &[u8; 32], password: &str) -> Result<String, String> {
    let encrypted = crypto::encrypt_aes256gcm(aes_key, password.as_bytes())
        .map_err(|e| e.to_string())?;

    // 格式：nonce_hex:ciphertext_hex
    let nonce_hex = hex::encode(&encrypted.nonce);
    let ciphertext_hex = hex::encode(&encrypted.ciphertext);

    Ok(format!("{}:{}", nonce_hex, ciphertext_hex))
}

/// 获取密码列表
///
/// 获取所有未删除的密码，支持按分类筛选和关键词搜索。
///
/// # Arguments
///
/// * `search` - 搜索关键词（可选）
/// * `category` - 分类 ID（可选，"all" 表示全部，"favorite" 表示收藏）
/// * `state` - 应用状态
///
/// # Returns
///
/// 密码列表
///
/// # 前端调用
///
/// ```typescript
/// const passwords = await invoke('get_passwords', {
///   search: 'github',
///   category: 'work'
/// });
/// ```
#[tauri::command]
pub async fn get_passwords(
    search: Option<String>,
    category: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<PasswordItem>, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    // 获取 AES 密钥
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    let conn = state.db.conn();

    // 构建查询
    let mut sql = String::from(
        "SELECT id, title, username, password_encrypted, url, notes, category_id, is_favorite, created_at, updated_at
         FROM passwords
         WHERE deleted_at IS NULL"
    );

    let mut conditions = Vec::new();

    // 按分类筛选
    if let Some(cat) = &category {
        if cat == "favorite" {
            conditions.push("is_favorite = 1".to_string());
        } else if cat != "all" {
            conditions.push(format!("category_id = '{}'", cat));
        }
    }

    // 按关键词搜索
    if let Some(search_text) = &search {
        if !search_text.is_empty() {
            let escaped = search_text.replace("'", "''");
            conditions.push(format!(
                "(title LIKE '%{}%' OR username LIKE '%{}%' OR url LIKE '%{}%')",
                escaped, escaped, escaped
            ));
        }
    }

    if !conditions.is_empty() {
        sql.push_str(" AND ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(" ORDER BY updated_at DESC");

    // 执行查询
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;

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

    // 解密并转换结果
    let mut result = Vec::new();
    for (id, title, username, password_encrypted, url, notes, category_id, is_favorite, created_at, updated_at) in passwords {
        let password = decrypt_password(&aes_key, &password_encrypted)?;

        result.push(PasswordItem {
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

    Ok(result)
}

/// 获取单个密码
///
/// 根据 ID 获取单个密码的详细信息。
///
/// # Arguments
///
/// * `id` - 密码 ID
/// * `state` - 应用状态
///
/// # Returns
///
/// 密码项
///
/// # 前端调用
///
/// ```typescript
/// const password = await invoke('get_password_by_id', { id: '123' });
/// ```
#[tauri::command]
pub async fn get_password_by_id(
    id: String,
    state: State<'_, AppState>,
) -> Result<PasswordItem, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    // 获取 AES 密钥
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    let conn = state.db.conn();

    // 查询数据库
    let result = conn.query_row(
        "SELECT id, title, username, password_encrypted, url, notes, category_id, is_favorite, created_at, updated_at
         FROM passwords
         WHERE id = ?1 AND deleted_at IS NULL",
        params![id],
        |row| {
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
        },
    );

    match result {
        Ok((id, title, username, password_encrypted, url, notes, category_id, is_favorite, created_at, updated_at)) => {
            let password = decrypt_password(&aes_key, &password_encrypted)?;

            Ok(PasswordItem {
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
            })
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Err("未找到密码".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// 添加密码
///
/// 加密密码数据后存储到数据库。
///
/// # Arguments
///
/// * `data` - 新密码数据
/// * `state` - 应用状态
///
/// # Returns
///
/// 新添加的密码项
///
/// # 前端调用
///
/// ```typescript
/// const password = await invoke('add_password', {
///   data: {
///     title: 'GitHub',
///     username: 'user@example.com',
///     password: 'my_password',
///     url: 'https://github.com',
///     notes: null,
///     category: 'work',
///     is_favorite: true
///   }
/// });
/// ```
#[tauri::command]
pub async fn add_password(
    data: NewPassword,
    state: State<'_, AppState>,
) -> Result<PasswordItem, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    // 获取 AES 密钥
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    // 生成唯一 ID
    let id = uuid::Uuid::new_v4().to_string();

    // 获取当前时间戳
    let now = chrono::Utc::now().timestamp();

    // 加密密码
    let encrypted_password = encrypt_password(&aes_key, &data.password)?;

    let conn = state.db.conn();

    // 插入数据库
    conn.execute(
        "INSERT INTO passwords (id, title, username, password_encrypted, url, notes, category_id, is_favorite, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            data.title,
            data.username,
            encrypted_password,
            data.url,
            data.notes,
            data.category,
            data.is_favorite as i32,
            now,
            now,
        ],
    ).map_err(|e| e.to_string())?;

    // 返回新添加的密码项
    Ok(PasswordItem {
        id,
        title: data.title,
        username: data.username,
        password: data.password,
        url: data.url,
        notes: data.notes,
        category: data.category,
        is_favorite: data.is_favorite,
        created_at: now,
        updated_at: now,
    })
}

/// 更新密码
///
/// 更新指定密码的数据。
///
/// # Arguments
///
/// * `id` - 密码 ID
/// * `data` - 更新数据
/// * `state` - 应用状态
///
/// # Returns
///
/// 更新后的密码项
///
/// # 前端调用
///
/// ```typescript
/// const password = await invoke('update_password', {
///   id: '123',
///   data: {
///     title: 'GitHub Updated',
///     password: 'new_password'
///   }
/// });
/// ```
#[tauri::command]
pub async fn update_password(
    id: String,
    data: UpdatePassword,
    state: State<'_, AppState>,
) -> Result<PasswordItem, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    // 获取 AES 密钥
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    // 获取当前密码数据
    let current = get_password_by_id(id.clone(), state.clone()).await?;

    // 构建更新字段
    let title = data.title.unwrap_or(current.title);
    let username = data.username.unwrap_or(current.username);
    let password = data.password.unwrap_or(current.password);
    let url = data.url.or(current.url);
    let notes = data.notes.or(current.notes);
    let category = data.category.unwrap_or(current.category);
    let is_favorite = data.is_favorite.unwrap_or(current.is_favorite);

    // 加密新密码
    let encrypted_password = encrypt_password(&aes_key, &password)?;

    let now = chrono::Utc::now().timestamp();

    let conn = state.db.conn();

    // 更新数据库
    conn.execute(
        "UPDATE passwords
         SET title = ?1, username = ?2, password_encrypted = ?3, url = ?4, notes = ?5,
             category_id = ?6, is_favorite = ?7, updated_at = ?8
         WHERE id = ?9",
        params![
            title,
            username,
            encrypted_password,
            url,
            notes,
            category,
            is_favorite as i32,
            now,
            id,
        ],
    ).map_err(|e| e.to_string())?;

    // 返回更新后的密码项
    Ok(PasswordItem {
        id,
        title,
        username,
        password,
        url,
        notes,
        category,
        is_favorite,
        created_at: current.created_at,
        updated_at: now,
    })
}

/// 删除密码（软删除）
///
/// 将密码标记为已删除，不实际删除数据。
///
/// # Arguments
///
/// * `id` - 密码 ID
/// * `state` - 应用状态
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
///
/// # 前端调用
///
/// ```typescript
/// await invoke('delete_password', { id: '123' });
/// ```
#[tauri::command]
pub async fn delete_password(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conn = state.db.conn();
    let now = chrono::Utc::now().timestamp();

    // 更新 deleted_at 字段（软删除）
    conn.execute(
        "UPDATE passwords SET deleted_at = ?1 WHERE id = ?2",
        params![now, id],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// 切换收藏状态
///
/// 切换密码的收藏状态。
///
/// # Arguments
///
/// * `id` - 密码 ID
/// * `state` - 应用状态
///
/// # Returns
///
/// 更新后的收藏状态
///
/// # 前端调用
///
/// ```typescript
/// const isFavorite = await invoke('toggle_favorite', { id: '123' });
/// ```
#[tauri::command]
pub async fn toggle_favorite(
    id: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let conn = state.db.conn();

    // 切换 is_favorite 字段
    conn.execute(
        "UPDATE passwords SET is_favorite = NOT is_favorite, updated_at = strftime('%s', 'now') WHERE id = ?1",
        params![id],
    ).map_err(|e| e.to_string())?;

    // 获取更新后的状态
    let is_favorite: i32 = conn.query_row(
        "SELECT is_favorite FROM passwords WHERE id = ?1",
        params![id],
        |row| row.get(0),
    ).map_err(|e| e.to_string())?;

    Ok(is_favorite != 0)
}
