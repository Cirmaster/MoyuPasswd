//! 数据管理命令模块
//!
//! 处理数据的导入导出操作。
//! 导出使用导出密码加密（AES-256-GCM，密钥由 Argon2id 派生）；
//! 导入自动识别加密格式与历史明文格式。
//!
//! 导出/导入包含标签与分类层级；旧导出文件缺字段时按默认值（空标签/顶级分类）处理，
//! 保持 `moyu-passwd-encrypted-v1` 格式向后兼容。

use crate::response::{respond, ApiResult};
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
    /// 密码强度（旧导出文件缺失视为未计算；导入时按明文重算，此字段仅作留档）
    #[serde(default)]
    pub password_strength: Option<i32>,
    /// 最近使用时间（旧导出文件缺失视为从未使用）
    #[serde(default)]
    pub last_used_at: Option<i64>,
    /// 使用次数（旧导出文件缺失视为 0）
    #[serde(default)]
    pub use_count: i64,
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

// ==================== 导入规模与字段上限（防御超大/垃圾数据） ====================

/// 导入 JSON 上限（50 MB）：超大文件会撑爆内存并在解密/Argon2 上耗尽 CPU
const MAX_IMPORT_JSON_BYTES: usize = 50 * 1024 * 1024;
/// 单次导入密码条目上限
const MAX_IMPORT_PASSWORDS: usize = 10_000;
/// 单次导入分类条目上限
const MAX_IMPORT_CATEGORIES: usize = 1_000;

/// 校验导入数据的规模与字段上限
///
/// 导入文件是唯一的外部数据入口，必须当成不可信输入：
/// id 要能安全作主键、文本要有长度上限、时间戳要有合理范围，
/// 否则垃圾数据会永久污染数据库（列表/搜索/导出全被拖垮）。
fn validate_import(data: &ImportData) -> Result<(), String> {
    if data.passwords.len() > MAX_IMPORT_PASSWORDS {
        return Err(format!(
            "导入条目过多（{} 条，上限 {} 条）",
            data.passwords.len(),
            MAX_IMPORT_PASSWORDS
        ));
    }
    if data.categories.len() > MAX_IMPORT_CATEGORIES {
        return Err(format!(
            "导入分类过多（{} 个，上限 {} 个）",
            data.categories.len(),
            MAX_IMPORT_CATEGORIES
        ));
    }

    for category in &data.categories {
        validate_import_id(&category.id, "分类 id")?;
        if category.name.trim().is_empty() {
            return Err("分类名不能为空".to_string());
        }
        if category.name.chars().count() > 100 {
            return Err(format!("分类名过长（最长 100 字符）: {:?}", category.name));
        }
    }

    // 时间戳合理范围：1970-01-01 ~ 2100-01-01（毫秒）
    const MIN_TS: i64 = 0;
    const MAX_TS: i64 = 4_102_444_800_000;

    for p in &data.passwords {
        validate_import_id(&p.id, "密码 id")?;
        if p.title.trim().is_empty() {
            return Err("密码标题不能为空".to_string());
        }
        if p.title.chars().count() > 200 {
            return Err(format!("密码标题过长（最长 200 字符）: {:?}", p.title));
        }
        if p.username.chars().count() > 200 {
            return Err("用户名过长（最长 200 字符）".to_string());
        }
        if p.password.chars().count() > 4096 {
            return Err("密码过长（最长 4096 字符）".to_string());
        }
        if p.url.as_ref().map(|u| u.chars().count() > 2048).unwrap_or(false) {
            return Err("URL 过长（最长 2048 字符）".to_string());
        }
        if p.notes.as_ref().map(|n| n.chars().count() > 65536).unwrap_or(false) {
            return Err("备注过长（最长 65536 字符）".to_string());
        }
        if p.extra_fields.len() > 50 {
            return Err(format!("自定义字段过多（{} 个，上限 50 个）", p.extra_fields.len()));
        }
        for f in &p.extra_fields {
            if f.label.trim().is_empty() || f.label.chars().count() > 64 {
                return Err("自定义字段名非法（非空且最长 64 字符）".to_string());
            }
            if f.value.chars().count() > 4096 {
                return Err("自定义字段值过长（最长 4096 字符）".to_string());
            }
        }
        if p.tags.len() > 20 {
            return Err(format!("标签过多（{} 个，上限 20 个）", p.tags.len()));
        }
        if p.created_at < MIN_TS || p.created_at > MAX_TS {
            return Err(format!("创建时间戳非法: {}", p.created_at));
        }
        if p.updated_at < MIN_TS || p.updated_at > MAX_TS {
            return Err(format!("更新时间戳非法: {}", p.updated_at));
        }
    }
    Ok(())
}

/// 校验导入 id：非空、≤64 字符、仅字母数字与 `-`/`_`（可安全作主键与文件内引用）
fn validate_import_id(id: &str, what: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("{what} 非法（应为 ≤64 位字母/数字/-/_）: {id:?}"))
    }
}

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
    window: tauri::WebviewWindow,
) -> ApiResult<String> {
    respond(|| -> Result<String, String> {
        // 仅主窗口可调用：导出是全量明文数据的出口，小窗（快速搜索/倒计时）不得触达
        if window.label() != "main" {
            return Err("导出数据仅允许在主窗口执行".to_string());
        }

        if export_password.is_empty() {
            return Err("请输入导出密码".to_string());
        }

        // 检查是否已解锁
        if !state.is_unlocked() {
            return Err("应用未解锁".to_string());
        }

        // 导出文件是全量明文（密码/备注/敏感字段全部解密后打包），是整个系统泄露后果
        // 最重的产物。此前只查非空，用户用 "1" 也能导出——Argon2 对 1 位口令的暴破
        // 成本几乎为零。复用主密码同一套强度校验（含弱口令黑名单），
        // 并把错误文案里的「主密码」换成「导出密码」，避免提示误导。
        crate::commands::auth::validate_password_strength(&export_password)
            .map_err(|e| e.replace("主密码", "导出密码"))?;

        // 获取 AES 密钥
        let aes_key = state.get_aes_key().ok_or("密钥不存在")?;

        let db = state.get_db()?;
        let conn = db.conn();

        // 查询所有密码（含加密备注/自定义字段/使用统计列）
        let passwords = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, username, password_encrypted, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, created_at, updated_at, strength, last_used_at, use_count
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
                        row.get::<_, Option<i32>>(12)?,
                        row.get::<_, Option<i64>>(13)?,
                        row.get::<_, i64>(14)?,
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

        // 解密并转换。
        // 单行解密失败不挡全量备份：跳过该行并记日志——此前一条损坏行
        // 会让整个导出失败，用户连其余好数据都备份不出来。
        let mut export_passwords = Vec::new();
        let mut skipped_rows = 0usize;
        for (id, title, username, password_encrypted, url, notes_encrypted, extra_plain, extra_enc, category_id, is_favorite, created_at, updated_at, strength, last_used_at, use_count) in passwords {
            let log_id = id.clone();
            let decrypted = (|| -> Result<ExportPasswordItem, String> {
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

                Ok(ExportPasswordItem {
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
                    password_strength: strength,
                    last_used_at,
                    use_count,
                })
            })();
            match decrypted {
                Ok(item) => export_passwords.push(item),
                Err(e) => {
                    skipped_rows += 1;
                    log::warn!("导出跳过损坏条目 {log_id}: {e}");
                }
            }
        }
        if skipped_rows > 0 {
            log::warn!("导出完成，但有 {skipped_rows} 条数据因解密失败被跳过");
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

        // 明文 JSON 用 Zeroizing 承载：加密完成后立即随作用域清零，不在内存里留全量明文
        let plaintext_json =
            zeroize::Zeroizing::new(serde_json::to_string(&export_data).map_err(|e| e.to_string())?);

        encrypt_export(&plaintext_json, &export_password)

    })}

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
    window: tauri::WebviewWindow,
) -> ApiResult<ImportResult> {
    respond(|| -> Result<ImportResult, String> {
        // 仅主窗口可调用：导入会批量改写库数据，小窗不得触达
        if window.label() != "main" {
            return Err("导入数据仅允许在主窗口执行".to_string());
        }

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

        // 规模上限：超大文件直接拒绝（防内存/CPU 撑爆）
        if json.len() > MAX_IMPORT_JSON_BYTES {
            return Err(format!(
                "导入文件过大（{} 字节，上限 {} 字节）",
                json.len(),
                MAX_IMPORT_JSON_BYTES
            ));
        }

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

        // 导入文件是唯一外部数据入口：解析后先做规模/字段校验，垃圾数据不得入库
        validate_import(&import_data)?;

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
            // 判重含软删除行（同 id 即占用主键）：
            // - skip：保留现状（含保留其删除状态），与「绝不静默覆盖」一致
            // - replace：用导入内容覆盖并复活（备份里有此条目 = 用户想恢复它），
            //   但保留本地使用统计——last_used_at/use_count 是本地元数据、不是导出内容，
            //   此前 INSERT OR REPLACE 会把它们连同 strength/删除状态一起静默清掉
            let existing: Option<(Option<i64>, i64)> = tx
                .query_row(
                    "SELECT last_used_at, use_count FROM passwords WHERE id = ?1",
                    params![password.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();
            if existing.is_some() && !replace_on_conflict {
                result.skipped += 1;
                continue;
            }
            let had_existing = existing.is_some();
            let (last_used_at, use_count) = match existing {
                Some((lu, uc)) => (lu, uc),        // 覆盖：保留本地使用统计
                None => (password.last_used_at, password.use_count), // 新增：采用导出值
            };

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

            // 强度按导入的明文重算，不信导出文件里的留档值
            let strength = crypto::password_strength_level(&password.password);

            tx.execute(
                "INSERT OR REPLACE INTO passwords (id, title, username, password_encrypted, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, created_at, updated_at, deleted_at, strength, last_used_at, use_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, ?13, ?14, ?15)",
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
                    strength,
                    last_used_at,
                    use_count,
                ],
            ).map_err(|e| e.to_string())?;

            // 标签：按后端规则规范化后整体重建（replace 冲突时先删旧标签再插新，旧文件无标签则清空）
            let tags = password::normalize_tags(&password.tags);
            password::replace_tags(&tx, &password.id, &tags)?;

            if had_existing {
                result.replaced += 1;
            } else {
                result.imported += 1;
            }
        }

        // 兜底：导入条目引用的分类可能不存在（旧导出文件/手改文件），
        // 统一归入内置「其他」分类，杜绝悬空引用（与 delete_category 的兜底一致）。
        // 「其他」不存在时先重建（防导出文件里恰好没有内置分类）。
        tx.execute(
            "INSERT OR IGNORE INTO categories (id, name, sort_order, created_at) VALUES ('other', '其他', 4, ?1)",
            params![now],
        ).map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE passwords SET category_id = 'other'
             WHERE category_id IS NULL OR category_id NOT IN (SELECT id FROM categories)",
            [],
        ).map_err(|e| e.to_string())?;

        tx.commit().map_err(|e| e.to_string())?;
        log::info!(
            "导入完成: 新增 {}, 跳过 {}, 覆盖 {}",
            result.imported,
            result.skipped,
            result.replaced
        );
        Ok(result)

    })}
