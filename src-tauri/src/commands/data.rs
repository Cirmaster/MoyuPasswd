//! 数据管理命令模块
//!
//! 处理数据的导入导出操作。
//! 导出使用导出密码加密（AES-256-GCM，密钥由 Argon2id 派生）；
//! 导入自动识别加密格式与历史明文格式。
//!
//! 导出/导入包含标签与分类层级；旧导出文件缺字段时按默认值（空标签/顶级分类）处理，
//! 保持 `moyu-passwd-encrypted-v1` 格式向后兼容。

use std::collections::HashMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::password::{self, CustomField};
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
///
/// 导出文件是解密后的全量内容（密码/备注/敏感字段都已解密），
/// `extra_fields`/`tags` 对旧版本导出文件向后兼容（缺失视为空）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportPasswordItem {
    pub id: String,
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: Option<String>,
    pub notes: Option<String>,
    #[serde(default)]
    pub extra_fields: Vec<CustomField>,
    /// 标签（明文，按录入顺序；旧导出文件缺失视为空）
    #[serde(default)]
    pub tags: Vec<String>,
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
    /// 父分类 ID（旧导出文件缺失视为顶级分类）
    #[serde(default)]
    pub parent_id: Option<String>,
}

/// 导入数据请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportData {
    pub passwords: Vec<ExportPasswordItem>,
    pub categories: Vec<ExportCategory>,
}

/// 加密导出包裹的格式标识
const ENCRYPTED_FORMAT: &str = "moyu-passwd-encrypted-v1";

/// 加密导出包裹结构
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedExport {
    format: String,
    salt: String,
    nonce: String,
    data: String,
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

    Ok(format!(
        "{}:{}",
        hex::encode(&encrypted.nonce),
        hex::encode(&encrypted.ciphertext)
    ))
}

/// 用导出密码加密导出内容
fn encrypt_export(plaintext: &str, export_password: &str) -> Result<String, String> {
    let salt = crypto::generate_aes_salt();
    let key = crypto::derive_aes_key(export_password, &salt).map_err(|e| e.to_string())?;
    let encrypted = crypto::encrypt_aes256gcm(&key, plaintext.as_bytes())
        .map_err(|e| e.to_string())?;

    let wrapper = EncryptedExport {
        format: ENCRYPTED_FORMAT.to_string(),
        salt,
        nonce: hex::encode(&encrypted.nonce),
        data: hex::encode(&encrypted.ciphertext),
    };

    serde_json::to_string(&wrapper).map_err(|e| e.to_string())
}

/// 用导出密码解密导出内容
fn decrypt_export(wrapper: &EncryptedExport, export_password: &str) -> Result<String, String> {
    let key = crypto::derive_aes_key(export_password, &wrapper.salt).map_err(|e| e.to_string())?;
    let nonce = hex::decode(&wrapper.nonce).map_err(|e| e.to_string())?;
    let ciphertext = hex::decode(&wrapper.data).map_err(|e| e.to_string())?;

    let encrypted_data = crypto::EncryptedData { ciphertext, nonce };
    let decrypted = crypto::decrypt_aes256gcm(&key, &encrypted_data)
        .map_err(|e| e.to_string())?;

    String::from_utf8(decrypted).map_err(|e| e.to_string())
}

/// 导出所有密码数据（加密）
///
/// 将所有密码和分类导出为 JSON，并使用导出密码加密。
///
/// # Arguments
///
/// * `export_password` - 导出密码（用于加密导出文件）
/// * `state` - 应用状态
///
/// # Returns
///
/// 加密后的导出数据 JSON 字符串
#[tauri::command]
pub async fn export_data(
    export_password: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    if export_password.is_empty() {
        return Err("请输入导出密码".to_string());
    }

    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    // 获取 AES 密钥
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    let db = state.get_db()?;
    let conn = db.conn();

    // 查询所有密码（含加密备注与自定义字段两列）
    let passwords = {
        let mut stmt = conn
            .prepare(
                "SELECT id, title, username, password_encrypted, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, created_at, updated_at
                 FROM passwords WHERE deleted_at IS NULL",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, i32>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        rows
    };

    // 预取所有标签（按 sort_order、rowid 保序），避免逐条查询
    let mut tags_map: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT password_id, tag FROM password_tags ORDER BY password_id, sort_order, rowid")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let password_id: String = row.get(0).map_err(|e| e.to_string())?;
            let tag: String = row.get(1).map_err(|e| e.to_string())?;
            tags_map.entry(password_id).or_default().push(tag);
        }
    }

    // 解密并转换
    let mut export_passwords = Vec::new();
    for (id, title, username, password_encrypted, url, notes_encrypted, extra_plain, extra_enc, category_id, is_favorite, created_at, updated_at) in passwords {
        let password = decrypt_password(&aes_key, &password_encrypted)?;
        let notes = match notes_encrypted {
            Some(enc) => Some(decrypt_password(&aes_key, &enc)?),
            None => None,
        };
        // 自定义字段：敏感值随导出一并解密（导出文件整体由导出密码加密保护）
        let extra_fields = password::merge_extra_fields(
            extra_plain.as_deref(),
            extra_enc.as_deref(),
            Some(&aes_key),
        )?;
        // 标签：明文随导出（按录入顺序）
        let tags = tags_map.remove(&id).unwrap_or_default();

        export_passwords.push(ExportPasswordItem {
            id,
            title,
            username,
            password,
            url,
            notes,
            extra_fields,
            tags,
            category: category_id,
            is_favorite: is_favorite != 0,
            created_at,
            updated_at,
        });
    }

    // 查询所有分类（含分类树父节点）
    let categories = {
        let mut stmt = conn
            .prepare("SELECT id, name, icon, sort_order, parent_id FROM categories ORDER BY sort_order")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ExportCategory {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    icon: row.get(2)?,
                    sort_order: row.get(3)?,
                    parent_id: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        rows
    };

    // 构建导出数据
    let export_data = ExportData {
        exported_at: chrono::Utc::now().timestamp_millis(),
        passwords: export_passwords,
        categories,
    };

    let plaintext_json = serde_json::to_string(&export_data).map_err(|e| e.to_string())?;

    encrypt_export(&plaintext_json, &export_password)
}

/// 导入结果统计
#[derive(Debug, serde::Serialize)]
pub struct ImportResult {
    /// 新增条目数
    pub imported: usize,
    /// 跳过（id 已存在且保留现有）条目数
    pub skipped: usize,
    /// 覆盖（id 已存在且被导入数据替换）条目数
    pub replaced: usize,
}

/// 导入密码数据
///
/// 从 JSON 格式导入密码和分类。自动识别加密导出与历史明文格式。
/// 整包在单个事务中执行：任一条失败整体回滚，不留半截导入。
///
/// # Arguments
///
/// * `json` - JSON 数据字符串
/// * `import_password` - 导入密码（加密导出时使用）
/// * `on_conflict` - id 冲突策略：`"skip"`（默认，保留现有条目）或 `"replace"`（覆盖）
/// * `state` - 应用状态
///
/// # Returns
///
/// 导入统计（新增/跳过/覆盖）
#[tauri::command]
pub async fn import_data(
    json: String,
    import_password: String,
    on_conflict: Option<String>,
    state: State<'_, AppState>,
) -> Result<ImportResult, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    // 冲突策略：默认 skip——绝不静默覆盖现有数据
    let replace_on_conflict = match on_conflict.as_deref() {
        None | Some("skip") => false,
        Some("replace") => true,
        Some(other) => return Err(format!("未知的冲突策略: {other}")),
    };

    // 获取 AES 密钥
    let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

    // 识别格式：仅接受加密导出格式，拒绝明文导入
    let import_data: ImportData = {
        let value: serde_json::Value =
            serde_json::from_str(&json).map_err(|e| format!("解析数据失败: {e}"))?;
        if value.get("format").and_then(|f| f.as_str()) == Some(ENCRYPTED_FORMAT) {
            if import_password.is_empty() {
                return Err("该文件已加密，请输入导入密码".to_string());
            }
            let wrapper: EncryptedExport = serde_json::from_value(value)
                .map_err(|e| format!("解析加密数据失败: {e}"))?;
            let plaintext = decrypt_export(&wrapper, &import_password)?;
            serde_json::from_str(&plaintext).map_err(|e| format!("解析数据失败: {e}"))?
        } else {
            return Err("不支持明文导入，请使用加密导出文件".to_string());
        }
    };

    let db = state.get_db()?;
    let conn = db.conn();
    // 整包事务：中途任何失败回滚，不留半截导入
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp_millis();
    let mut result = ImportResult {
        imported: 0,
        skipped: 0,
        replaced: 0,
    };

    // 导入分类（id 已存在则跳过；parent_id 先按导出值写入，全部插入后统一修正悬空引用）
    for category in &import_data.categories {
        tx.execute(
            "INSERT OR IGNORE INTO categories (id, name, icon, sort_order, created_at, parent_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![category.id, category.name, category.icon, category.sort_order, now, category.parent_id],
        ).map_err(|e| e.to_string())?;
    }

    // 分类全部插入后，把指向不存在分类的 parent_id 置为 NULL（旧导出文件无该字段时天然为 NULL）
    tx.execute(
        "UPDATE categories SET parent_id = NULL
         WHERE parent_id IS NOT NULL AND parent_id NOT IN (SELECT id FROM categories)",
        [],
    ).map_err(|e| e.to_string())?;

    // 导入密码（按 id 判重，冲突策略决定跳过或覆盖）
    for password in &import_data.passwords {
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM passwords WHERE id = ?1",
                params![password.id],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if exists && !replace_on_conflict {
            result.skipped += 1;
            continue;
        }

        // 加密密码
        let encrypted_password = encrypt_password(&aes_key, &password.password)?;

        // 加密备注（若有）
        let encrypted_notes = match &password.notes {
            Some(n) if !n.is_empty() => Some(encrypt_password(&aes_key, n)?),
            _ => None,
        };

        // 自定义字段：拆分明文列 + 敏感值密文列（旧导出文件无该字段时视为空）
        let (extra_fields_plain, extra_fields_encrypted) =
            password::split_extra_fields(&aes_key, &password.extra_fields)?;

        tx.execute(
            "INSERT OR REPLACE INTO passwords (id, title, username, password_encrypted, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                password.id,
                password.title,
                password.username,
                encrypted_password,
                password.url,
                encrypted_notes,
                extra_fields_plain,
                extra_fields_encrypted,
                password.category,
                password.is_favorite as i32,
                password.created_at,
                password.updated_at,
            ],
        ).map_err(|e| e.to_string())?;

        // 标签：按后端规则规范化后整体重建（replace 冲突时先删旧标签再插新，旧文件无标签则清空）
        let tags = password::normalize_tags(&password.tags);
        password::replace_tags(&tx, &password.id, &tags)?;

        if exists {
            result.replaced += 1;
        } else {
            result.imported += 1;
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    log::info!(
        "导入完成: 新增 {}, 跳过 {}, 覆盖 {}",
        result.imported,
        result.skipped,
        result.replaced
    );
    Ok(result)
}
