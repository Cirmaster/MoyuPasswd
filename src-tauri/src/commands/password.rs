//! 密码命令模块
//!
//! 处理密码的增删改查操作。
//! 所有密码数据都使用 AES-256-GCM 加密后存储。
//!
//! # 命令列表
//!
//! - `get_passwords`: 获取密码列表（零解密：不含备注明文，强度读预存列；自定义字段仅明文列）
//! - `get_password_detail`: 获取单条详情（按需解密备注与敏感自定义字段）
//! - `add_password`: 添加密码
//! - `update_password`: 更新密码（url/notes/extra_fields 支持「不修改/清空/赋值」三层语义）
//! - `delete_password`: 删除密码（软删除）
//! - `toggle_favorite`: 切换收藏状态

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::crypto;
use crate::state::AppState;

/// 自定义字段（接口层）
///
/// 一条密码可携带任意多个附加字段（如数据库连接地址/端口/连接命令）。
/// `sensitive` 为 true 的字段值加密存储、列表只下发空值占位，
/// 明文按需通过 `get_password_detail` 获取；非敏感字段值明文存储并参与搜索。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomField {
    /// 字段名（如 "连接地址"）
    pub label: String,
    /// 字段值（列表响应中敏感字段为空串）
    pub value: String,
    /// 是否敏感（敏感字段值加密存储）
    pub sensitive: bool,
}

/// 自定义字段明文列存储项
///
/// 全部字段的 label/order 都在明文列（label 非机密，便于列表展示与搜索）；
/// 敏感字段的 value 置 null，真值存密文列。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlainField {
    label: String,
    value: Option<String>,
    order: usize,
}

/// 自定义字段密文列存储项（AES 解密后的形态）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SecretField {
    order: usize,
    value: String,
}

/// 拆分自定义字段为「明文列 JSON + 密文列密文」
///
/// 无字段时两列都为 NULL。无名（label 为空）的行直接丢弃。
pub(crate) fn split_extra_fields(
    aes_key: &[u8; 32],
    fields: &[CustomField],
) -> Result<(Option<String>, Option<String>), String> {
    let mut plain: Vec<PlainField> = Vec::new();
    let mut secrets: Vec<SecretField> = Vec::new();

    for (i, f) in fields.iter().enumerate() {
        let label = f.label.trim();
        if label.is_empty() {
            continue;
        }
        if f.sensitive {
            plain.push(PlainField {
                label: label.to_string(),
                value: None,
                order: i,
            });
            secrets.push(SecretField {
                order: i,
                value: f.value.clone(),
            });
        } else {
            plain.push(PlainField {
                label: label.to_string(),
                value: Some(f.value.clone()),
                order: i,
            });
        }
    }

    if plain.is_empty() {
        return Ok((None, None));
    }

    let plain_json = serde_json::to_string(&plain).map_err(|e| e.to_string())?;
    let enc = if secrets.is_empty() {
        None
    } else {
        // 敏感字段值整体走与备注同一 AES 通道（nonce_hex:ciphertext_hex）
        let secrets_json = serde_json::to_string(&secrets).map_err(|e| e.to_string())?;
        Some(encrypt_password(aes_key, &secrets_json)?)
    };
    Ok((Some(plain_json), enc))
}

/// 合并两列为自定义字段列表（按 order 排序还原用户设定顺序）
///
/// `aes_key` 为 Some 时解密敏感字段值（详情/导出用），为 None 时敏感值留空（列表用）。
/// 详情模式下密文解密失败直接报错，绝不静默返回空值。
pub(crate) fn merge_extra_fields(
    plain_json: Option<&str>,
    enc: Option<&str>,
    aes_key: Option<&[u8; 32]>,
) -> Result<Vec<CustomField>, String> {
    let Some(pj) = plain_json else {
        return Ok(Vec::new());
    };
    let plain: Vec<PlainField> = serde_json::from_str(pj).map_err(|e| e.to_string())?;

    let secrets: Vec<SecretField> = match (enc, aes_key) {
        (Some(enc), Some(key)) => {
            let decrypted = decrypt_password(key, enc)?;
            serde_json::from_str(&decrypted).map_err(|e| e.to_string())?
        }
        _ => Vec::new(),
    };

    let mut out: Vec<(usize, CustomField)> = Vec::with_capacity(plain.len());
    for pf in plain {
        let sensitive = pf.value.is_none();
        let value = match pf.value {
            Some(v) => v,
            None => secrets
                .iter()
                .find(|s| s.order == pf.order)
                .map(|s| s.value.clone())
                .unwrap_or_default(),
        };
        out.push((
            pf.order,
            CustomField {
                label: pf.label,
                value,
                sensitive,
            },
        ));
    }
    out.sort_by_key(|(o, _)| *o);
    Ok(out.into_iter().map(|(_, f)| f).collect())
}

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
    /// 备注明文（仅 `get_password_detail` 返回；列表不下发）
    pub notes: Option<String>,
    /// 是否有备注（列表用；备注明文按需通过 `get_password_detail` 获取）
    pub has_notes: bool,
    /// 自定义字段（列表中敏感字段 value 为空串，详情解密后回填）
    pub extra_fields: Vec<CustomField>,
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
    /// 自定义字段（可选）
    pub extra_fields: Option<Vec<CustomField>>,
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
    /// 网站 URL（三层语义：字段缺失=不修改、null/空串=清空、字符串=赋值）
    #[serde(default, deserialize_with = "de_double_option")]
    pub url: Option<Option<String>>,
    /// 备注信息（三层语义同 url）
    #[serde(default, deserialize_with = "de_double_option")]
    pub notes: Option<Option<String>>,
    /// 自定义字段（三层语义同 url：缺失=不修改、null/空数组=清空、数组=整体替换）
    #[serde(default, deserialize_with = "de_double_option")]
    pub extra_fields: Option<Option<Vec<CustomField>>>,
    /// 所属分类 ID
    pub category: Option<String>,
    /// 是否收藏
    pub is_favorite: Option<bool>,
}

/// 双层 Option 反序列化：区分「字段缺失（不修改）」与「显式 null（清空）」
fn de_double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let value: Option<T> = Deserialize::deserialize(deserializer)?;
    Ok(Some(value))
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

/// 根据密码 ID 解密密码（供 inject 模块调用）
///
/// # Arguments
/// * `entry_id` - 密码条目 ID
///
/// # Returns
/// 解密后的明文密码
///
/// # Deprecated
/// 此函数已废弃，请使用 register_with_decrypt 传递解密闭包
#[deprecated(note = "请使用 register_with_decrypt 传递解密闭包")]
pub fn decrypt_password_by_id(_entry_id: &str) -> Result<String, String> {
    Err("已废弃：请使用 register_with_decrypt 传递解密闭包".to_string())
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

/// 用新密钥重加密全部密码记录（密码 + 备注 + 敏感自定义字段）
///
/// 在修改主密码时调用：用旧密钥解密所有记录，再用新密钥重新加密。
/// 本函数不自行开启事务，由调用方统一在事务中执行。
/// 注意：meta（主密码哈希/盐值）不在该事务内——跨存储无法真原子，
/// 由调用方的「备份 + 事务日志 + 回滚」协议保证崩溃一致性（见 auth::change_master_password）。
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
    // 先取出所有记录（密码 + 备注 + 敏感字段密文），再逐条重加密
    let mut stmt = conn
        .prepare(
            "SELECT id, password_encrypted, notes_encrypted, extra_fields_encrypted FROM passwords",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<(String, String, Option<String>, Option<String>)>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    let mut count = 0usize;
    for (id, encrypted_password, encrypted_notes, encrypted_fields) in rows {
        let plaintext = decrypt_password(old_key, &encrypted_password)?;
        let new_encrypted_password = encrypt_password(new_key, &plaintext)?;

        let new_encrypted_notes = match encrypted_notes {
            Some(n) => Some(encrypt_password(new_key, &decrypt_password(old_key, &n)?)?),
            None => None,
        };

        // 敏感字段值 JSON 整体重加密（内容无需解析，按密文通道整体换钥）
        let new_encrypted_fields = match encrypted_fields {
            Some(n) => Some(encrypt_password(new_key, &decrypt_password(old_key, &n)?)?),
            None => None,
        };

        conn.execute(
            "UPDATE passwords
             SET password_encrypted = ?1, notes_encrypted = ?2, extra_fields_encrypted = ?3
             WHERE id = ?4",
            params![new_encrypted_password, new_encrypted_notes, new_encrypted_fields, id],
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

    // 获取 AES 密钥（仅用于存量数据强度回填）
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    let db = state.get_db()?;
    let conn = db.conn();

    // 是否显示密码强度（仅控制返回值，不影响落库）
    let show_strength = is_show_strength_enabled(&conn);

    // 存量数据强度回填（strength 为空的行补算一次并落库，之后列表零解密）
    backfill_strength(&conn, &aes_key);

    // 构建参数化查询（列表不解密任何数据：备注只给 has_notes，强度读预存列，
    // 自定义字段读明文列、敏感值只给空串占位）
    let mut sql = String::from(
        "SELECT id, title, username, url, notes_encrypted, extra_fields_plain, category_id, is_favorite, strength, created_at, updated_at
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

    // 按关键词搜索（参数化 + LIKE 通配符转义）；自定义字段明文列（label + 非敏感值）参与搜索
    if let Some(search_text) = &search {
        if !search_text.is_empty() {
            sql.push_str(
                " AND (title LIKE ? ESCAPE '\\' OR username LIKE ? ESCAPE '\\' OR url LIKE ? ESCAPE '\\' OR extra_fields_plain LIKE ? ESCAPE '\\')",
            );
            let pattern = format!("%{}%", escape_like(search_text));
            params.push(pattern.clone());
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
            let url: Option<String> = row.get(3)?;
            let notes_encrypted: Option<String> = row.get(4)?;
            let extra_fields_plain: Option<String> = row.get(5)?;
            let category_id: String = row.get(6)?;
            let is_favorite: i32 = row.get(7)?;
            let strength: Option<i32> = row.get(8)?;
            let created_at: i64 = row.get(9)?;
            let updated_at: i64 = row.get(10)?;

            Ok((id, title, username, url, notes_encrypted, extra_fields_plain, category_id, is_favorite, strength, created_at, updated_at))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // 转换结果：列表零解密——不返回明文密码、不下发备注（只给 has_notes）、强度读预存列，
    // 自定义字段敏感值留空（明文列里本来就是 null）
    let result = passwords
        .into_iter()
        .map(|(id, title, username, url, notes_encrypted, extra_fields_plain, category_id, is_favorite, strength, created_at, updated_at)| {
            let extra_fields = merge_extra_fields(extra_fields_plain.as_deref(), None, None)
                .unwrap_or_default();
            PasswordItem {
                id,
                title,
                username,
                password: String::new(), // 列表不返回明文
                url,
                notes: None,
                has_notes: notes_encrypted.is_some(),
                extra_fields,
                category: category_id,
                is_favorite: is_favorite != 0,
                password_strength: if show_strength { strength } else { None },
                created_at,
                updated_at,
            }
        })
        .collect();

    Ok(result)
}

/// 存量数据强度一次性回填
///
/// 强度列上线前的旧记录没有 strength 值；首次列表加载时补算并落库，
/// 之后列表读取列即可（不再解密全库）。解密失败的行跳过，下次再试。
fn backfill_strength(conn: &rusqlite::Connection, aes_key: &[u8; 32]) {
    let rows: Vec<(String, String)> = match conn
        .prepare("SELECT id, password_encrypted FROM passwords WHERE strength IS NULL")
    {
        Ok(mut stmt) => match stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => return,
        },
        Err(_) => return,
    };

    if rows.is_empty() {
        return;
    }

    let mut filled = 0usize;
    for (id, encrypted) in rows {
        if let Ok(plaintext) = decrypt_password(aes_key, &encrypted) {
            let level = crate::crypto::password_strength_level(&plaintext);
            if conn
                .execute(
                    "UPDATE passwords SET strength = ?1 WHERE id = ?2",
                    params![level, id],
                )
                .is_ok()
            {
                filled += 1;
            }
        }
    }
    log::info!("密码强度回填完成: {} 条", filled);
}

/// 获取单个密码详情（含备注明文）
///
/// 备注按需获取：列表不下发备注明文（只给 `has_notes`），
/// 编辑/查看详情时才调用本命令。备注解密失败直接报错，绝不静默返回空。
#[tauri::command]
pub async fn get_password_detail(
    id: String,
    state: State<'_, AppState>,
) -> Result<PasswordItem, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    // 获取 AES 密钥
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    let db = state.get_db()?;
    let conn = db.conn();

    // 查询数据库
    let result = conn.query_row(
        "SELECT id, title, username, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, strength, created_at, updated_at
         FROM passwords
         WHERE id = ?1 AND deleted_at IS NULL",
        params![id],
        |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let username: String = row.get(2)?;
            let url: Option<String> = row.get(3)?;
            let notes_encrypted: Option<String> = row.get(4)?;
            let extra_fields_plain: Option<String> = row.get(5)?;
            let extra_fields_encrypted: Option<String> = row.get(6)?;
            let category_id: String = row.get(7)?;
            let is_favorite: i32 = row.get(8)?;
            let strength: Option<i32> = row.get(9)?;
            let created_at: i64 = row.get(10)?;
            let updated_at: i64 = row.get(11)?;

            Ok((id, title, username, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, strength, created_at, updated_at))
        },
    );

    match result {
        Ok((id, title, username, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, strength, created_at, updated_at)) => {
            // 备注解密失败必须报错：静默返回 None 会让更新流程把原备注覆盖清空
            let notes = match notes_encrypted {
                Some(enc) => Some(decrypt_password(&aes_key, &enc)?),
                None => None,
            };
            // 敏感字段值按需解密；解密失败同样必须报错，绝不静默空值
            let extra_fields = merge_extra_fields(
                extra_fields_plain.as_deref(),
                extra_fields_encrypted.as_deref(),
                Some(&aes_key),
            )?;
            Ok(PasswordItem {
                id,
                title,
                username,
                password: String::new(), // 不返回明文
                url,
                has_notes: notes.is_some(),
                notes,
                extra_fields,
                category: category_id,
                is_favorite: is_favorite != 0,
                password_strength: strength,
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

    // 密码强度写入时计算并落库（列表读列即可，不再全量解密）
    let strength = crate::crypto::password_strength_level(&data.password);

    // 加密备注（若有）
    let encrypted_notes = match &data.notes {
        Some(n) if !n.is_empty() => Some(encrypt_password(&aes_key, n)?),
        _ => None,
    };

    // 拆分自定义字段（明文列 + 敏感值密文列）
    let (extra_fields_plain, extra_fields_encrypted) =
        split_extra_fields(&aes_key, data.extra_fields.as_deref().unwrap_or_default())?;

    let db = state.get_db()?;
    let conn = db.conn();

    // 是否显示密码强度（仅控制返回值）
    let show_strength = is_show_strength_enabled(&conn);

    // 插入数据库
    conn.execute(
        "INSERT INTO passwords (id, title, username, password_encrypted, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, strength, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            id,
            data.title,
            data.username,
            encrypted_password,
            data.url,
            encrypted_notes,
            extra_fields_plain,
            extra_fields_encrypted,
            data.category,
            data.is_favorite as i32,
            strength,
            now,
            now,
        ],
    ).map_err(|e| e.to_string())?;

    // 返回新添加的密码项（不返回明文密码、不回显备注明文、敏感字段值留空）
    let extra_fields =
        merge_extra_fields(extra_fields_plain.as_deref(), None, None).unwrap_or_default();
    Ok(PasswordItem {
        id,
        title: data.title,
        username: data.username,
        password: String::new(),
        url: data.url,
        notes: None,
        has_notes: encrypted_notes.is_some(),
        extra_fields,
        category: data.category,
        is_favorite: data.is_favorite,
        password_strength: if show_strength { Some(strength) } else { None },
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

    let now = chrono::Utc::now().timestamp_millis();

    // 读取当前原始行：备注/敏感字段密文不改动则完全不解密、不重写，
    // 从根上消除「解密失败 → 静默清空」的问题
    let db = state.get_db()?;
    let conn = db.conn();

    let (
        cur_title,
        cur_username,
        cur_password_enc,
        cur_url,
        cur_notes_enc,
        cur_extra_plain,
        cur_extra_enc,
        cur_category,
        cur_is_favorite,
        cur_strength,
        created_at,
    ): (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        i32,
        Option<i32>,
        i64,
    ) = conn
        .query_row(
            "SELECT title, username, password_encrypted, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, strength, created_at
             FROM passwords
             WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                ))
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => "未找到密码".to_string(),
            _ => e.to_string(),
        })?;

    // 标量字段：None = 不修改
    let title = data.title.unwrap_or(cur_title);
    let username = data.username.unwrap_or(cur_username);
    let category = data.category.unwrap_or(cur_category);
    let is_favorite = data.is_favorite.unwrap_or(cur_is_favorite != 0);

    // URL 三层语义：缺失=不修改、null/空串=清空、字符串=赋值
    let url = match data.url {
        None => cur_url,
        Some(None) => None,
        Some(Some(v)) => {
            let v = v.trim().to_string();
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        }
    };

    // 备注三层语义：缺失=保持原密文不动、null/空串=清空、字符串=重新加密
    let encrypted_notes = match &data.notes {
        None => cur_notes_enc,
        Some(None) => None,
        Some(Some(n)) if n.trim().is_empty() => None,
        Some(Some(n)) => Some(encrypt_password(&aes_key, n)?),
    };
    let has_notes = encrypted_notes.is_some();

    // 自定义字段三层语义：缺失=不修改、null/空数组=清空、数组=整体替换
    let (extra_fields_plain, extra_fields_encrypted) = match &data.extra_fields {
        None => (cur_extra_plain, cur_extra_enc),
        Some(None) => (None, None),
        Some(Some(fields)) => split_extra_fields(&aes_key, fields)?,
    };

    // 密码：为空/缺失保留原密文与原强度，否则重新加密并重算强度
    let (encrypted_password, strength) = match &data.password {
        Some(p) if !p.is_empty() => (
            encrypt_password(&aes_key, p)?,
            crate::crypto::password_strength_level(p),
        ),
        _ => {
            let level = match cur_strength {
                Some(l) => l,
                // 旧数据无强度值：解密补算一次（单条，可接受）
                None => decrypt_password(&aes_key, &cur_password_enc)
                    .map(|p| crate::crypto::password_strength_level(&p))
                    .unwrap_or(0),
            };
            (cur_password_enc, level)
        }
    };

    // 是否显示密码强度（仅控制返回值）
    let show_strength = is_show_strength_enabled(&conn);

    // 更新数据库（含强度列与自定义字段两列）
    conn.execute(
        "UPDATE passwords
         SET title = ?1, username = ?2, password_encrypted = ?3, url = ?4, notes_encrypted = ?5,
             extra_fields_plain = ?6, extra_fields_encrypted = ?7,
             category_id = ?8, is_favorite = ?9, strength = ?10, updated_at = ?11
         WHERE id = ?12",
        params![
            title,
            username,
            encrypted_password,
            url,
            encrypted_notes,
            extra_fields_plain,
            extra_fields_encrypted,
            category,
            is_favorite as i32,
            strength,
            now,
            id,
        ],
    ).map_err(|e| e.to_string())?;

    // 返回更新后的密码项（不返回明文密码、不回显备注明文、敏感字段值留空）
    let extra_fields =
        merge_extra_fields(extra_fields_plain.as_deref(), None, None).unwrap_or_default();
    Ok(PasswordItem {
        id,
        title,
        username,
        password: String::new(),
        url,
        notes: None,
        has_notes,
        extra_fields,
        category,
        is_favorite,
        password_strength: if show_strength { Some(strength) } else { None },
        created_at,
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

    let db = state.get_db()?;
    let conn = db.conn();
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

    let db = state.get_db()?;
    let conn = db.conn();
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

/// 解密密码并准备粘贴注入（延迟解密）
///
/// 复制动作**不解密**，只登记待粘贴状态和解密闭包并激活 Ctrl+V 钩子。
/// 用户在目标窗口按 Ctrl+V 时，钩子拦截并触发注入流程：
/// 归因（识别目标进程）→ 调用闭包解密 → SendInput 注入 → 立即清零。
/// 倒计时（TTL）内可多次 Ctrl+V 粘贴，每次都重新解密并立即清零；
/// 密码不进系统剪贴板。
///
/// # Arguments
///
/// * `id` - 密码 ID
/// * `state` - 应用状态
/// * `app` - Tauri 应用句柄（用于访问剪贴板）
///
/// # Returns
///
/// 待粘贴有效期（秒）
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
    log::info!("[CMD] copy_password_to_clipboard 被调用: id={}", id);

    // 检查是否已解锁
    if !state.is_unlocked() {
        log::warn!("[CMD] 应用未解锁");
        return Err("应用未解锁".to_string());
    }

    let db = state.get_db()?;
    let conn = db.conn();

    // 验证密码条目存在并获取密文
    let encrypted: String = conn
        .query_row(
            "SELECT password_encrypted FROM passwords WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                log::warn!("[CMD] 未找到密码: id={}", id);
                "未找到密码".to_string()
            }
            _ => e.to_string(),
        })?;

    // 读取剪贴板清除时间设置（作为 pending TTL）
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

    log::info!("[CMD] 开始登记 pending: id={}, clear_time={}s", id, clear_time);

    // 创建解密闭包：不捕获密钥副本，调用时从状态现场取密钥。
    // 锁定后任何迟到的调用都会失败（fail closed），杜绝锁定后仍可解密注入。
    let encrypted_clone = encrypted.clone();
    let app_handle = app.clone();
    let decrypt_fn: crate::pending::DecryptFn = std::sync::Arc::new(move |_entry_id: &str| -> Result<String, String> {
        use tauri::Manager;
        let state = app_handle.state::<AppState>();
        if !state.is_unlocked() {
            log::warn!("[DECRYPT] 应用已锁定，拒绝解密");
            return Err("应用已锁定，无法解密".to_string());
        }
        let aes_key = state.get_aes_key().ok_or("密钥不存在")?;
        log::info!("[DECRYPT] 调用解密闭包，解密密文");
        let plaintext = decrypt_password(&aes_key, &encrypted_clone)?;
        log::info!("[DECRYPT] 解密成功，长度={}", plaintext.len());
        Ok(plaintext)
    });

    // 登记待粘贴密码（带解密闭包）
    let seq = crate::pending::register_with_decrypt(&id, clear_time, decrypt_fn);

    // 激活 Ctrl+V 钩子（传入句柄，钩子检测到 Ctrl+C / Esc 时销毁粘贴状态）
    crate::hotkey::activate(app);

    log::info!("[CMD] copy_password_to_clipboard 完成: id={}, seq={}, ttl={}s", id, seq, clear_time);

    Ok(clear_time)
}
