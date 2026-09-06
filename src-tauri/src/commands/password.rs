//! 密码命令模块
//!
//! 处理密码的增删改查操作。
//! 所有密码数据都使用 AES-256-GCM 加密后存储。
//!
//! # 命令列表
//!
//! - `get_passwords`: 获取密码列表
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
    /// 密码强度等级（后端计算，0-4；未开启强度显示时为空）
    pub password_strength: Option<i32>,
    /// 创建时间（毫秒时间戳）
    pub created_at: i64,
    /// 更新时间（毫秒时间戳）
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
pub(crate) fn decrypt_password(aes_key: &[u8; 32], encrypted: &str) -> Result<String, String> {
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
pub(crate) fn encrypt_password(aes_key: &[u8; 32], password: &str) -> Result<String, String> {
    let encrypted = crypto::encrypt_aes256gcm(aes_key, password.as_bytes())
        .map_err(|e| e.to_string())?;

    // 格式：nonce_hex:ciphertext_hex
    let nonce_hex = hex::encode(&encrypted.nonce);
    let ciphertext_hex = hex::encode(&encrypted.ciphertext);

    Ok(format!("{}:{}", nonce_hex, ciphertext_hex))
}

/// 用新密钥重加密全部密码记录（密码 + 备注）
///
/// 在修改主密码时调用：用旧密钥解密所有记录，再用新密钥重新加密。
/// 本函数不自行开启事务，由调用方统一在事务中执行，
/// 以保证“重加密 + 更新 master_password”原子提交。
///
/// # Arguments
///
/// * `conn` - 数据库连接（应处于调用方开启的事务中）
/// * `old_key` - 旧 AES-256 密钥
/// * `new_key` - 新 AES-256 密钥
///
/// # Returns
///
/// 重加密的记录数量
///
/// # Errors
///
/// 如果解密或加密失败，返回错误信息
pub(crate) fn reencrypt_all_passwords(
    conn: &rusqlite::Connection,
    old_key: &[u8; 32],
    new_key: &[u8; 32],
) -> Result<usize, String> {
    // 先取出所有记录（密码 + 备注密文），再逐条重加密
    let mut stmt = conn
        .prepare("SELECT id, password_encrypted, notes_encrypted FROM passwords")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<(String, String, Option<String>)>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    let mut count = 0usize;
    for (id, encrypted_password, encrypted_notes) in rows {
        let plaintext = decrypt_password(old_key, &encrypted_password)?;
        let new_encrypted_password = encrypt_password(new_key, &plaintext)?;

        let new_encrypted_notes = match encrypted_notes {
            Some(n) => Some(encrypt_password(new_key, &decrypt_password(old_key, &n)?)?),
            None => None,
        };

        conn.execute(
            "UPDATE passwords SET password_encrypted = ?1, notes_encrypted = ?2 WHERE id = ?3",
            params![new_encrypted_password, new_encrypted_notes, id],
        )
        .map_err(|e| e.to_string())?;
        count += 1;
    }

    Ok(count)
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
/// 读取「是否显示密码强度」设置（默认开启）
fn is_show_strength_enabled(conn: &rusqlite::Connection) -> bool {
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'show_password_strength'",
        [],
        |row| {
            let v: String = row.get(0)?;
            Ok(v == "true")
        },
    )
    .unwrap_or(true)
}

/// 转义 LIKE 通配符（% _ \），配合 ESCAPE '\' 使用，使搜索按字面匹配
fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

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

    // 获取 AES 密钥（用于解密备注）
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    let conn = state.db.conn();

    // 是否显示密码强度（开启时解密密码计算强度，仍不返回明文）
    let show_strength = is_show_strength_enabled(&conn);

    // 构建参数化查询
    let mut sql = String::from(
        "SELECT id, title, username, password_encrypted, url, notes_encrypted, category_id, is_favorite, created_at, updated_at
         FROM passwords
         WHERE deleted_at IS NULL"
    );
    let mut params: Vec<String> = Vec::new();

    // 按分类筛选
    if let Some(cat) = &category {
        if cat == "favorite" {
            sql.push_str(" AND is_favorite = 1");
        } else if cat != "all" {
            sql.push_str(" AND category_id = ?");
            params.push(cat.clone());
        }
    }

    // 按关键词搜索（参数化 + LIKE 通配符转义）
    if let Some(search_text) = &search {
        if !search_text.is_empty() {
            sql.push_str(" AND (title LIKE ? ESCAPE '\\' OR username LIKE ? ESCAPE '\\' OR url LIKE ? ESCAPE '\\')");
            let pattern = format!("%{}%", escape_like(search_text));
            params.push(pattern.clone());
            params.push(pattern.clone());
            params.push(pattern);
        }
    }

    sql.push_str(" ORDER BY updated_at DESC");

    // 执行查询
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;

    let passwords = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let username: String = row.get(2)?;
            let password_encrypted: String = row.get(3)?;
            let url: Option<String> = row.get(4)?;
            let notes_encrypted: Option<String> = row.get(5)?;
            let category_id: String = row.get(6)?;
            let is_favorite: i32 = row.get(7)?;
            let created_at: i64 = row.get(8)?;
            let updated_at: i64 = row.get(9)?;

            Ok((id, title, username, password_encrypted, url, notes_encrypted, category_id, is_favorite, created_at, updated_at))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // 转换结果（列表不返回明文密码；解密备注；按需计算强度）
    let result = passwords
        .into_iter()
        .map(|(id, title, username, password_encrypted, url, notes_encrypted, category_id, is_favorite, created_at, updated_at)| {
            let notes = match notes_encrypted {
                Some(enc) => decrypt_password(&aes_key, &enc).ok(),
                None => None,
            };
            let password_strength = if show_strength {
                decrypt_password(&aes_key, &password_encrypted)
                    .ok()
                    .map(|p| crate::crypto::password_strength_level(&p))
            } else {
                None
            };
            PasswordItem {
                id,
                title,
                username,
                password: String::new(), // 列表不返回明文
                url,
                notes,
                category: category_id,
                is_favorite: is_favorite != 0,
                password_strength,
                created_at,
                updated_at,
            }
        })
        .collect();

    Ok(result)
}

/// 获取单个密码（内部使用，非对外命令）
///
/// 根据 ID 获取单个密码的详细信息，备注解密后返回。
/// 仅由 update_password 内部调用，不作为 Tauri 命令暴露。
fn get_password_internal(state: &AppState, id: &str) -> Result<PasswordItem, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    // 获取 AES 密钥
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    let conn = state.db.conn();

    // 查询数据库
    let result = conn.query_row(
        "SELECT id, title, username, password_encrypted, url, notes_encrypted, category_id, is_favorite, created_at, updated_at
         FROM passwords
         WHERE id = ?1 AND deleted_at IS NULL",
        params![id],
        |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let username: String = row.get(2)?;
            let password_encrypted: String = row.get(3)?;
            let url: Option<String> = row.get(4)?;
            let notes_encrypted: Option<String> = row.get(5)?;
            let category_id: String = row.get(6)?;
            let is_favorite: i32 = row.get(7)?;
            let created_at: i64 = row.get(8)?;
            let updated_at: i64 = row.get(9)?;

            Ok((id, title, username, password_encrypted, url, notes_encrypted, category_id, is_favorite, created_at, updated_at))
        },
    );

    match result {
        Ok((id, title, username, _password_encrypted, url, notes_encrypted, category_id, is_favorite, created_at, updated_at)) => {
            let notes = match notes_encrypted {
                Some(enc) => decrypt_password(&aes_key, &enc).ok(),
                None => None,
            };
            Ok(PasswordItem {
                id,
                title,
                username,
                password: String::new(), // 不返回明文
                url,
                notes,
                category: category_id,
                is_favorite: is_favorite != 0,
                password_strength: None, // 内部使用，不计算强度
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
    let now = chrono::Utc::now().timestamp_millis();

    // 加密密码
    let encrypted_password = encrypt_password(&aes_key, &data.password)?;

    // 加密备注（若有）
    let encrypted_notes = match &data.notes {
        Some(n) if !n.is_empty() => Some(encrypt_password(&aes_key, n)?),
        _ => None,
    };

    let conn = state.db.conn();

    // 是否显示密码强度
    let show_strength = is_show_strength_enabled(&conn);

    // 插入数据库
    conn.execute(
        "INSERT INTO passwords (id, title, username, password_encrypted, url, notes_encrypted, category_id, is_favorite, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            data.title,
            data.username,
            encrypted_password,
            data.url,
            encrypted_notes,
            data.category,
            data.is_favorite as i32,
            now,
            now,
        ],
    ).map_err(|e| e.to_string())?;

    // 返回新添加的密码项（不返回明文密码）
    Ok(PasswordItem {
        id,
        title: data.title,
        username: data.username,
        password: String::new(),
        url: data.url,
        notes: data.notes,
        category: data.category,
        is_favorite: data.is_favorite,
        password_strength: if show_strength {
            Some(crate::crypto::password_strength_level(&data.password))
        } else {
            None
        },
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
    let current = get_password_internal(&state, &id)?;

    // 构建更新字段
    let title = data.title.unwrap_or(current.title);
    let username = data.username.unwrap_or(current.username);
    let password = data.password.unwrap_or_default();
    let url = data.url.or(current.url);
    let notes = data.notes.or(current.notes);
    let category = data.category.unwrap_or(current.category);
    let is_favorite = data.is_favorite.unwrap_or(current.is_favorite);

    // 密码为空时保留原密码，否则重新加密
    let encrypted_password = if password.is_empty() {
        // 独立作用域：查询完立即释放数据库锁，避免与后续 UPDATE 死锁
        {
            let conn = state.db.conn();
            conn.query_row(
                "SELECT password_encrypted FROM passwords WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            ).map_err(|e| e.to_string())?
        }
    } else {
        encrypt_password(&aes_key, &password)?
    };

    // 加密备注
    let encrypted_notes = match &notes {
        Some(n) if !n.is_empty() => Some(encrypt_password(&aes_key, n)?),
        _ => None,
    };

    let now = chrono::Utc::now().timestamp_millis();

    let conn = state.db.conn();

    // 是否显示密码强度
    let show_strength = is_show_strength_enabled(&conn);

    // 更新数据库
    conn.execute(
        "UPDATE passwords
         SET title = ?1, username = ?2, password_encrypted = ?3, url = ?4, notes_encrypted = ?5,
             category_id = ?6, is_favorite = ?7, updated_at = ?8
         WHERE id = ?9",
        params![
            title,
            username,
            encrypted_password,
            url,
            encrypted_notes,
            category,
            is_favorite as i32,
            now,
            id,
        ],
    ).map_err(|e| e.to_string())?;

    // 计算密码强度（开启时；保留原密码则解密后计算）
    let password_strength = if show_strength {
        decrypt_password(&aes_key, &encrypted_password)
            .ok()
            .map(|p| crate::crypto::password_strength_level(&p))
    } else {
        None
    };

    // 返回更新后的密码项（不返回明文密码）
    Ok(PasswordItem {
        id,
        title,
        username,
        password: String::new(),
        url,
        notes,
        category,
        is_favorite,
        password_strength,
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
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    let conn = state.db.conn();
    let now = chrono::Utc::now().timestamp_millis();

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
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    let conn = state.db.conn();
    let now = chrono::Utc::now().timestamp_millis();

    // 切换 is_favorite 字段
    conn.execute(
        "UPDATE passwords SET is_favorite = NOT is_favorite, updated_at = ?1 WHERE id = ?2",
        params![now, id],
    ).map_err(|e| e.to_string())?;

    // 获取更新后的状态
    let is_favorite: i32 = conn.query_row(
        "SELECT is_favorite FROM passwords WHERE id = ?1",
        params![id],
        |row| row.get(0),
    ).map_err(|e| e.to_string())?;

    Ok(is_favorite != 0)
}

/// 解密密码并直接写入系统剪贴板
///
/// 前端全程不接触明文。Rust 解密后直接写入剪贴板，
/// 前端负责显示倒计时并在到期后清除剪贴板。
///
/// # Arguments
///
/// * `id` - 密码 ID
/// * `state` - 应用状态
/// * `app` - Tauri 应用句柄（用于访问剪贴板）
///
/// # Returns
///
/// 剪贴板清除时间（秒）
///
/// # 前端调用
///
/// ```typescript
/// const clearTime = await invoke('copy_password_to_clipboard', { id: '123' });
/// ```
#[tauri::command]
pub async fn copy_password_to_clipboard(
    id: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<u32, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    // 获取 AES 密钥
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    let conn = state.db.conn();

    // 查询加密的密码
    let encrypted: String = conn
        .query_row(
            "SELECT password_encrypted FROM passwords WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "未找到密码".to_string(),
            _ => e.to_string(),
        })?;

    // 解密密码
    let plaintext = decrypt_password(&aes_key, &encrypted)?;

    // 直接写入系统剪贴板（排除剪贴板历史与云端同步）
    crate::clipboard::write_text_excluded(&app, &plaintext)?;

    // 读取剪贴板清除时间设置
    let clear_time: u32 = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'clipboard_clear_time'",
            [],
            |row| {
                let v: String = row.get(0)?;
                Ok(v.parse().unwrap_or(30))
            },
        )
        .unwrap_or(30);

    // 后端兜底：定时清空剪贴板，不依赖前端倒计时窗口是否存活
    crate::clipboard::schedule_clear(&app, clear_time);

    log::info!("密码已写入剪贴板，将在 {} 秒后清除", clear_time);

    Ok(clear_time)
}
