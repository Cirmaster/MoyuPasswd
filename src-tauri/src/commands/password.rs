//! 密码命令模块
//!
//! 处理密码的增删改查操作。
//! 所有密码数据都使用 AES-256-GCM 加密后存储。
//!
//! # 命令列表
//!
//! - `get_passwords`: 获取密码列表（后端完成分类/标签/搜索过滤 + 排序 + 分页；
//!   零解密：不含备注明文，强度读预存列；自定义字段仅明文列；搜索支持中文拼音全拼/简拼）
//! - `get_password_stats`: 智能视图计数（全部/收藏/最近使用）
//! - `get_password_detail`: 获取单条详情（按需解密备注与敏感自定义字段）
//! - `add_password`: 添加密码（密码与标签同事务写入）
//! - `update_password`: 更新密码（url/notes/extra_fields/tags 支持「不修改/清空/赋值」三层语义）
//! - `delete_password`: 删除密码（软删除）
//! - `toggle_favorite`: 切换收藏状态
//! - `touch_password_usage`: 登记一次使用（更新最近使用时间与使用次数）
//! - `get_tags`: 获取全局标签列表（含未删除密码的使用计数）
//! - `rename_tag`: 全局重命名标签（冲突即合并）
//! - `delete_tag`: 从所有密码移除某标签

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use parking_lot::Mutex;
use pinyin::ToPinyin;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::crypto;
use crate::state::AppState;

/// 列表默认每页条数
const DEFAULT_PAGE_SIZE: u32 = 50;

/// 列表每页条数上限（防止一次拉取过多）
const MAX_PAGE_SIZE: u32 = 500;

/// 拼音结果缓存上限（超出后整体清空，避免无上限增长）
const PINYIN_CACHE_LIMIT: usize = 5000;

/// 单条密码允许的最大标签数（后端唯一权威规则）
pub(crate) const MAX_TAGS_PER_PASSWORD: usize = 20;

/// 单个标签的最大字符数（按 Unicode 字符计，不按字节）
pub(crate) const MAX_TAG_CHARS: usize = 32;

/// 标签串分隔符（GROUP_CONCAT 聚合/解析用；char(31) 为控制字符，正常标签不会出现）
const TAG_SEPARATOR: char = '\u{1f}';

/// 列表查询中聚合标签列的子查询
///
/// 用子查询 + GROUP_CONCAT(tag, char(31)) 在单条 SQL 内取标签；
/// 子查询内按 sort_order、rowid 排序以保留用户录入顺序（软删除密码读取时已由
/// 外层 `deleted_at IS NULL` 过滤，无需再 join）。详情接口用 `fetch_tags` 单独读取。
const TAGS_SUBQUERY: &str = "(SELECT GROUP_CONCAT(tag, char(31)) FROM (SELECT tag FROM password_tags WHERE password_id = passwords.id ORDER BY sort_order, rowid))";

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
    /// 标签（明文，按录入顺序；标签不是机密）
    pub tags: Vec<String>,
    /// 最近使用时间（毫秒时间戳；从未使用为 None）
    pub last_used_at: Option<i64>,
    /// 累计使用次数
    pub use_count: i64,
}

/// 分页密码列表（`get_passwords` 返回）
///
/// `items` 为当前页数据（仍零解密：password=""、notes=None、敏感字段空串）；
/// `total` 为过滤后的总条数；`page` 为实际返回页码（越界已钳制）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordPage {
    /// 当前页密码项
    pub items: Vec<PasswordItem>,
    /// 过滤后的总条数
    pub total: i64,
    /// 实际返回页码（已钳制到有效范围）
    pub page: u32,
    /// 每页条数
    pub page_size: u32,
}

/// 智能视图统计（`get_password_stats` 返回）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordStats {
    /// 全部未删除密码数
    pub all: i64,
    /// 收藏数
    pub favorite: i64,
    /// 最近使用（有使用记录）数量
    pub recent: i64,
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
    /// 标签（可选，缺省视为空）
    #[serde(default)]
    pub tags: Option<Vec<String>>,
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
    /// 标签（三层语义同 url：缺失=不修改、null/空数组=清空、数组=整体替换）
    #[serde(default, deserialize_with = "de_double_option")]
    pub tags: Option<Option<Vec<String>>>,
    /// 所属分类 ID
    pub category: Option<String>,
    /// 是否收藏
    pub is_favorite: Option<bool>,
}

/// 双层 Option 反序列化：区分「字段缺失（不修改）」与「显式 null（清空）」
///
/// 供本模块的 url/notes/extra_fields/tags 等三层语义字段复用。
pub(crate) fn de_double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let value: Option<T> = Deserialize::deserialize(deserializer)?;
    Ok(Some(value))
}

/// 规范化标签列表（后端唯一权威规则）
///
/// - 逐个 trim，丢弃空标签；
/// - 单标签按 Unicode 字符截断到 32 个字符（不按字节）；
/// - 同一密码内大小写不敏感去重，保留首次出现的写法；
/// - 保留录入顺序，最多保留 20 个。
pub(crate) fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for raw in tags {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 按字符截断（中文标签 32 字符 = 96 字节，不能按字节切）
        let truncated: String = trimmed.chars().take(MAX_TAG_CHARS).collect();
        // 大小写不敏感去重（Unicode 感知），保留首次出现的写法
        let key = truncated.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        normalized.push(truncated);
        if normalized.len() >= MAX_TAGS_PER_PASSWORD {
            break;
        }
    }

    normalized
}

/// 读取单条密码的标签（按 sort_order、rowid 保持录入顺序）
fn fetch_tags(conn: &rusqlite::Connection, password_id: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT tag FROM password_tags WHERE password_id = ?1 ORDER BY sort_order, rowid")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![password_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;

    let mut tags = Vec::new();
    for row in rows {
        tags.push(row.map_err(|e| e.to_string())?);
    }
    Ok(tags)
}

/// 覆盖写入某条密码的标签（调用方负责事务，保证与 passwords 写入原子）
pub(crate) fn replace_tags(
    conn: &rusqlite::Connection,
    password_id: &str,
    tags: &[String],
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM password_tags WHERE password_id = ?1",
        params![password_id],
    )
    .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("INSERT INTO password_tags (password_id, tag, sort_order) VALUES (?1, ?2, ?3)")
        .map_err(|e| e.to_string())?;
    for (sort_order, tag) in tags.iter().enumerate() {
        stmt.execute(params![password_id, tag, sort_order as i64])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 解析 GROUP_CONCAT 聚合出的标签串（char(31) 分隔）
fn parse_tags(concat: Option<String>) -> Vec<String> {
    concat
        .map(|value| {
            value
                .split(TAG_SEPARATOR)
                .filter(|tag| !tag.is_empty())
                .map(|tag| tag.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// 拼音结果进程内缓存：原文 -> (全拼, 简拼)
fn pinyin_cache() -> &'static Mutex<HashMap<String, (String, String)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (String, String)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 计算文本的拼音全拼与简拼
///
/// 汉字取 `pinyin` crate 的 `plain()`（无音调、小写）；非汉字字符原样小写保留。
/// 全拼为逐字拼音拼接；简拼为每个汉字的拼音首字母（非汉字字符本身也计入）。
/// 结果做进程内缓存（上限 5000 条，超出时整体清空重来）。
pub(crate) fn pinyin_variants(text: &str) -> (String, String) {
    // 命中缓存直接返回（克隆一份，避免持锁返回引用）
    {
        let cache = pinyin_cache().lock();
        if let Some(cached) = cache.get(text) {
            return cached.clone();
        }
    }

    let mut full = String::with_capacity(text.len());
    let mut initials = String::with_capacity(text.len());
    for ch in text.chars() {
        if let Some(pinyin) = ch.to_pinyin() {
            // 汉字：全拼追加无音调拼音，简拼追加首字母
            let plain = pinyin.plain();
            full.push_str(plain);
            if let Some(first) = plain.chars().next() {
                initials.push(first);
            }
        } else {
            // 非汉字（ASCII 字母/数字/符号等）：原样小写保留
            for lower in ch.to_lowercase() {
                full.push(lower);
                initials.push(lower);
            }
        }
    }

    let variants = (full, initials);
    let mut cache = pinyin_cache().lock();
    if cache.len() >= PINYIN_CACHE_LIMIT {
        // 简单上限控制：满了整体清空（列表场景下缓存命中率足够高，够用即可）
        cache.clear();
    }
    cache.insert(text.to_string(), variants.clone());
    variants
}

/// 展开单段搜索文本：原文小写 + 全拼 + 简拼（空格分隔）
pub(crate) fn expand_search_text(text: &str) -> String {
    let lower = text.to_lowercase();
    let (full, initials) = pinyin_variants(text);
    format!("{lower} {full} {initials}")
}

/// 构建搜索 haystack：各字段展开后按行拼接
pub(crate) fn build_search_haystack(fields: &[&str]) -> String {
    let mut haystack = String::new();
    for field in fields {
        haystack.push_str(&expand_search_text(field));
        haystack.push('\n');
    }
    haystack
}

/// 搜索关键词分词：空白分词 + 转小写
pub(crate) fn tokenize_search(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|token| token.to_lowercase())
        .collect()
}

/// 搜索匹配：各关键词之间为 AND（每个词都必须出现在 haystack 中）
pub(crate) fn matches_search(haystack: &str, tokens: &[String]) -> bool {
    tokens.iter().all(|token| haystack.contains(token.as_str()))
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

/// 后端加载的分类树索引（列表过滤用：分类名 + 父子关系）
struct CategoryIndex {
    /// 分类 id -> 名称
    names: HashMap<String, String>,
    /// 父分类 id -> 子分类 id 列表
    children: HashMap<String, Vec<String>>,
}

impl CategoryIndex {
    /// 从数据库加载分类树（只读 id/name/parent_id）
    fn load(conn: &rusqlite::Connection) -> Result<Self, String> {
        let mut stmt = conn
            .prepare("SELECT id, name, parent_id FROM categories")
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
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        let mut names = HashMap::with_capacity(rows.len());
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        for (id, name, parent_id) in rows {
            if let Some(parent) = parent_id {
                children.entry(parent).or_default().push(id.clone());
            }
            names.insert(id, name);
        }

        Ok(Self { names, children })
    }

    /// 分类名（分类已被删除/悬空时返回空串）
    fn name_of(&self, id: &str) -> &str {
        self.names.get(id).map(String::as_str).unwrap_or("")
    }

    /// 收集自身及全部后代分类 ID（visited 集合防历史脏数据成环）
    fn collect_descendants(&self, root_id: &str) -> HashSet<String> {
        let mut result: HashSet<String> = HashSet::new();
        let mut stack = vec![root_id.to_string()];
        while let Some(id) = stack.pop() {
            if !result.insert(id.clone()) {
                continue;
            }
            if let Some(children) = self.children.get(&id) {
                for child in children {
                    if !result.contains(child) {
                        stack.push(child.clone());
                    }
                }
            }
        }
        result
    }
}

/// 列表候选行（后端过滤/排序用；仍零解密）
struct ListRow {
    /// 密码项（已构造为接口结构）
    item: PasswordItem,
    /// 标题拼音全拼（仅按标题排序时计算）
    title_sort: String,
}

/// 获取密码列表（后端完成筛选/搜索/排序/分页）
///
/// 数据流：SQL 取全部未删除行（零解密）→ 后端加载分类树 → Rust 过滤
/// （分类含全部后代 / 收藏 / 最近使用 / 标签 AND / 搜索分词 AND + 拼音）→
/// 排序 → 分页。前端只展示返回的当前页。
///
/// # Arguments
///
/// * `search` - 搜索关键词（可选，空格分词 AND；匹配原文 + 拼音全拼/简拼）
/// * `category` - 分类 ID（可选，"all"/None 表示全部，"favorite" 收藏，"recent" 最近使用，普通分类含全部后代）
/// * `tags` - 标签筛选（可选，所选标签之间为 AND，大小写不敏感）
/// * `sort_by` - 排序字段（updated_at/created_at/title/last_used_at/use_count，默认 updated_at）
/// * `sort_order` - 排序方向（asc/desc，默认 desc）
/// * `page` - 页码（默认 1，越界钳制）
/// * `page_size` - 每页条数（默认 50，钳制 1..=500）
/// * `state` - 应用状态
///
/// # 前端调用
///
/// ```typescript
/// const page = await invoke('get_passwords', {
///   search: 'gongzuo',
///   category: 'work',
///   tags: ['数据库'],
///   sortBy: 'updated_at',
///   sortOrder: 'desc',
///   page: 1,
///   pageSize: 50
/// });
/// ```
#[tauri::command]
pub async fn get_passwords(
    search: Option<String>,
    category: Option<String>,
    tags: Option<Vec<String>>,
    sort_by: Option<String>,
    sort_order: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
    state: State<'_, AppState>,
) -> Result<PasswordPage, String> {
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

    // 参数规范化：排序默认 updated_at DESC，分页默认 1 / 50（page_size 钳制 1..=500）
    let sort_by = sort_by.as_deref().unwrap_or("updated_at");
    let ascending = matches!(sort_order.as_deref(), Some("asc"));
    let page_size = page_size
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let page = page.unwrap_or(1).max(1);

    // 读取全部未删除行（列表零解密：备注只给 has_notes，强度读预存列，
    // 自定义字段读明文列、敏感值只给空串占位；标签聚合子查询见 TAGS_SUBQUERY）
    let sql = format!(
        "SELECT id, title, username, url, notes_encrypted, extra_fields_plain, category_id, is_favorite, strength, created_at, updated_at, last_used_at, use_count, {} AS tags
         FROM passwords
         WHERE deleted_at IS NULL",
        TAGS_SUBQUERY
    );
    let rows = {
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let collected = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,          // id
                    row.get::<_, String>(1)?,          // title
                    row.get::<_, String>(2)?,          // username
                    row.get::<_, Option<String>>(3)?,  // url
                    row.get::<_, Option<String>>(4)?,  // notes_encrypted
                    row.get::<_, Option<String>>(5)?,  // extra_fields_plain
                    row.get::<_, String>(6)?,          // category_id
                    row.get::<_, i32>(7)?,             // is_favorite
                    row.get::<_, Option<i32>>(8)?,     // strength
                    row.get::<_, i64>(9)?,             // created_at
                    row.get::<_, i64>(10)?,            // updated_at
                    row.get::<_, Option<i64>>(11)?,    // last_used_at
                    row.get::<_, i64>(12)?,            // use_count
                    row.get::<_, Option<String>>(13)?, // tags
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        collected
    };

    // 后端加载分类树（用于「普通分类含全部后代」过滤与分类名搜索）
    let category_index = CategoryIndex::load(&conn)?;

    // 分类视图判定
    let category_value = category.as_deref().unwrap_or("all");
    let favorite_only = category_value == "favorite";
    let recent_only = category_value == "recent";
    let category_scope: Option<HashSet<String>> = if category_value.is_empty()
        || category_value == "all"
        || favorite_only
        || recent_only
    {
        None
    } else {
        Some(category_index.collect_descendants(category_value))
    };

    // 标签筛选：trim + 小写（AND，大小写不敏感）
    let tag_filters: Vec<String> = tags
        .unwrap_or_default()
        .into_iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();

    // 搜索分词（AND；空搜索词不参与过滤）
    let tokens = tokenize_search(search.as_deref().unwrap_or(""));

    // 过滤：智能视图 → 分类（含后代）→ 标签 AND → 搜索（每个词都需命中 haystack）
    let mut filtered: Vec<ListRow> = Vec::new();
    for (
        id,
        title,
        username,
        url,
        notes_encrypted,
        extra_fields_plain,
        category_id,
        is_favorite,
        strength,
        created_at,
        updated_at,
        last_used_at,
        use_count,
        tags_concat,
    ) in rows
    {
        // 零解密构造列表项（与详情接口不同：不下发备注明文/明文密码）
        let extra_fields = merge_extra_fields(extra_fields_plain.as_deref(), None, None)
            .unwrap_or_default();
        let item = PasswordItem {
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
            tags: parse_tags(tags_concat),
            last_used_at,
            use_count,
        };

        // 智能视图 / 分类（含全部后代）过滤
        if favorite_only && !item.is_favorite {
            continue;
        }
        if recent_only && item.last_used_at.is_none() {
            continue;
        }
        if let Some(scope) = &category_scope {
            if !scope.contains(&item.category) {
                continue;
            }
        }

        // 标签 AND 过滤（大小写不敏感）
        if !tag_filters.is_empty() {
            let item_tags: Vec<String> = item.tags.iter().map(|tag| tag.to_lowercase()).collect();
            if !tag_filters
                .iter()
                .all(|filter| item_tags.iter().any(|tag| tag == filter))
            {
                continue;
            }
        }

        // 搜索：每个词都必须命中 haystack（原文/全拼/简拼）
        if !tokens.is_empty() {
            let category_name = category_index.name_of(&item.category);
            let mut fields: Vec<&str> =
                Vec::with_capacity(4 + item.tags.len() + item.extra_fields.len() * 2);
            fields.push(item.title.as_str());
            fields.push(item.username.as_str());
            if let Some(url) = item.url.as_deref() {
                fields.push(url);
            }
            fields.push(category_name);
            for tag in &item.tags {
                fields.push(tag.as_str());
            }
            for field in &item.extra_fields {
                fields.push(field.label.as_str());
                // 敏感字段在列表中 value 为空串，不参与搜索
                if !field.sensitive {
                    fields.push(field.value.as_str());
                }
            }
            let haystack = build_search_haystack(&fields);
            if !matches_search(&haystack, &tokens) {
                continue;
            }
        }

        // 仅按标题排序时才计算拼音全拼，避免无谓开销
        let title_sort = if sort_by == "title" {
            pinyin_variants(&item.title).0
        } else {
            String::new()
        };

        filtered.push(ListRow { item, title_sort });
    }

    // 排序：先按 updated_at DESC 稳定预排序，作为所有排序的次级顺序（tie-breaker）；
    // 稳定排序保证同一主键值的行保持该次级顺序。
    filtered.sort_by(|a, b| b.item.updated_at.cmp(&a.item.updated_at));

    match sort_by {
        "created_at" => filtered.sort_by(|a, b| {
            if ascending {
                a.item.created_at.cmp(&b.item.created_at)
            } else {
                b.item.created_at.cmp(&a.item.created_at)
            }
        }),
        "title" => filtered.sort_by(|a, b| {
            // 标题按拼音全拼比较（中文按拼音序，英文按字母序）
            if ascending {
                a.title_sort.cmp(&b.title_sort)
            } else {
                b.title_sort.cmp(&a.title_sort)
            }
        }),
        "last_used_at" => filtered.sort_by(|a, b| {
            // 缺失使用记录按 0 处理
            let left = a.item.last_used_at.unwrap_or(0);
            let right = b.item.last_used_at.unwrap_or(0);
            if ascending {
                left.cmp(&right)
            } else {
                right.cmp(&left)
            }
        }),
        "use_count" => filtered.sort_by(|a, b| {
            if ascending {
                a.item.use_count.cmp(&b.item.use_count)
            } else {
                b.item.use_count.cmp(&a.item.use_count)
            }
        }),
        // 默认 updated_at（未知排序字段也回退到 updated_at）
        _ => filtered.sort_by(|a, b| {
            if ascending {
                a.item.updated_at.cmp(&b.item.updated_at)
            } else {
                b.item.updated_at.cmp(&a.item.updated_at)
            }
        }),
    }

    // 分页：total = 过滤后总条数；页码钳制到 [1, page_count]（空结果时 page=1）
    let total = filtered.len() as i64;
    let page_count = ((total + page_size as i64 - 1) / page_size as i64).max(1) as u32;
    let page = page.min(page_count);
    let offset = ((page - 1) as usize) * (page_size as usize);
    let items: Vec<PasswordItem> = filtered
        .into_iter()
        .skip(offset)
        .take(page_size as usize)
        .map(|row| row.item)
        .collect();

    Ok(PasswordPage {
        items,
        total,
        page,
        page_size,
    })
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

    // 查询数据库（列表接口不可见的备注/敏感字段密文在此按需解密）
    let result = conn.query_row(
        "SELECT id, title, username, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, strength, created_at, updated_at, last_used_at, use_count
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
            let last_used_at: Option<i64> = row.get(12)?;
            let use_count: i64 = row.get(13)?;

            Ok((id, title, username, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, strength, created_at, updated_at, last_used_at, use_count))
        },
    );

    match result {
        Ok((id, title, username, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, strength, created_at, updated_at, last_used_at, use_count)) => {
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
            let tags = fetch_tags(&conn, &id)?;
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
                tags,
                last_used_at,
                use_count,
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

    // 标签按后端规则规范化（缺省视为空）
    let tags = normalize_tags(data.tags.as_deref().unwrap_or_default());

    // 密码与标签同事务写入，保证原子性
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute(
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
    replace_tags(&tx, &id, &tags)?;
    tx.commit().map_err(|e| e.to_string())?;

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
        tags,
        last_used_at: None,
        use_count: 0,
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
        last_used_at,
        use_count,
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
        Option<i64>,
        i64,
    ) = conn
        .query_row(
            "SELECT title, username, password_encrypted, url, notes_encrypted, extra_fields_plain, extra_fields_encrypted, category_id, is_favorite, strength, created_at, last_used_at, use_count
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
                    row.get(11)?,
                    row.get(12)?,
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

    // 标签三层语义：缺失=不修改、null/空数组=清空、数组=整体替换（按后端规则规范化）
    let tags_change = match &data.tags {
        None => None,
        Some(None) => Some(Vec::new()),
        Some(Some(list)) => Some(normalize_tags(list)),
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

    // 密码与标签同事务写入（标签未变更时不触碰 password_tags），保证原子性
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute(
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
    if let Some(new_tags) = &tags_change {
        replace_tags(&tx, &id, new_tags)?;
    }
    let tags = fetch_tags(&tx, &id)?;
    tx.commit().map_err(|e| e.to_string())?;

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
        tags,
        last_used_at,
        use_count,
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

/// 标签统计项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagInfo {
    /// 标签展示名（大小写不敏感分组的组内原写法）
    pub name: String,
    /// 使用该标签的未删除密码数
    pub count: i64,
}

/// 登记一次密码使用（复制/注入时更新「最近使用」与「使用次数」）
///
/// # Arguments
///
/// * `id` - 密码 ID
/// * `state` - 应用状态
///
/// # 前端调用
///
/// ```typescript
/// await invoke('touch_password_usage', { id: '123' });
/// ```
#[tauri::command]
pub async fn touch_password_usage(
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

    // 只更新时间/次数，不改 updated_at——使用行为不应打乱「按更新时间」排序
    let affected = conn
        .execute(
            "UPDATE passwords SET last_used_at = ?1, use_count = use_count + 1
             WHERE id = ?2 AND deleted_at IS NULL",
            params![now, id],
        )
        .map_err(|e| e.to_string())?;
    if affected == 0 {
        return Err("未找到密码".to_string());
    }

    Ok(())
}

/// 获取全局标签列表
///
/// 按大小写不敏感分组（COLLATE NOCASE），展示名取组内任一原写法（MIN(tag)），
/// count 只统计未删除密码；结果按 count DESC、name 排序。
#[tauri::command]
pub async fn get_tags(state: State<'_, AppState>) -> Result<Vec<TagInfo>, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    let db = state.get_db()?;
    let conn = db.conn();

    let mut stmt = conn
        .prepare(
            "SELECT MIN(pt.tag) AS name, COUNT(DISTINCT pt.password_id) AS count
             FROM password_tags pt
             JOIN passwords p ON p.id = pt.password_id
             WHERE p.deleted_at IS NULL
             GROUP BY pt.tag COLLATE NOCASE
             ORDER BY count DESC, name",
        )
        .map_err(|e| e.to_string())?;

    let tags = stmt
        .query_map([], |row| {
            Ok(TagInfo {
                name: row.get(0)?,
                count: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(tags)
}

/// 全局重命名标签
///
/// 重命名到已存在的标签即合并（同一密码去重、保留原 sort_order 位置）。
///
/// # Arguments
///
/// * `old_name` - 原标签名（大小写不敏感匹配）
/// * `new_name` - 新标签名（trim 后 1~32 个字符）
/// * `state` - 应用状态
#[tauri::command]
pub async fn rename_tag(
    old_name: String,
    new_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    let old_name = old_name.trim();
    if old_name.is_empty() {
        return Err("标签名称不能为空".to_string());
    }

    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("标签名称不能为空".to_string());
    }
    if new_name.chars().count() > MAX_TAG_CHARS {
        return Err(format!("标签名称不能超过 {MAX_TAG_CHARS} 个字符"));
    }

    let db = state.get_db()?;
    let conn = db.conn();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

    // 收集原标签所在密码及其最小 sort_order（用于保留排序位置）
    let rows: Vec<(String, i64)> = {
        let mut stmt = tx
            .prepare(
                "SELECT password_id, MIN(sort_order) FROM password_tags
                 WHERE tag = ?1 COLLATE NOCASE
                 GROUP BY password_id",
            )
            .map_err(|e| e.to_string())?;
        let collected = stmt
            .query_map(params![old_name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        collected
    };

    if rows.is_empty() {
        return Err("标签不存在".to_string());
    }

    for (password_id, min_sort_order) in rows {
        // 先删旧写法（大小写不敏感，含同组脏数据），再插新写法：
        // 若该密码已存在新标签，INSERT OR IGNORE 保持原行，实现合并去重
        tx.execute(
            "DELETE FROM password_tags WHERE password_id = ?1 AND tag = ?2 COLLATE NOCASE",
            params![password_id, old_name],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT OR IGNORE INTO password_tags (password_id, tag, sort_order) VALUES (?1, ?2, ?3)",
            params![password_id, new_name, min_sort_order],
        )
        .map_err(|e| e.to_string())?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    log::info!("标签重命名完成: {} -> {}", old_name, new_name);
    Ok(())
}

/// 删除标签（从所有密码移除该标签）
///
/// 标签只是分类元数据，删除不影响密码本身；软删除密码的标签一并清理。
#[tauri::command]
pub async fn delete_tag(name: String, state: State<'_, AppState>) -> Result<(), String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    let name = name.trim();
    if name.is_empty() {
        return Err("标签名称不能为空".to_string());
    }

    let db = state.get_db()?;
    let conn = db.conn();

    conn.execute(
        "DELETE FROM password_tags WHERE tag = ?1 COLLATE NOCASE",
        params![name],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// 获取智能视图统计（全部 / 收藏 / 最近使用）
///
/// 只统计未删除密码；「最近使用」= 存在 last_used_at 记录。
///
/// # 前端调用
///
/// ```typescript
/// const stats = await invoke('get_password_stats');
/// // { all: 12, favorite: 3, recent: 5 }
/// ```
#[tauri::command]
pub async fn get_password_stats(state: State<'_, AppState>) -> Result<PasswordStats, String> {
    // 检查是否已解锁
    if !state.is_unlocked() {
        return Err("应用未解锁".to_string());
    }

    let db = state.get_db()?;
    let conn = db.conn();

    let all: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM passwords WHERE deleted_at IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let favorite: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM passwords WHERE deleted_at IS NULL AND is_favorite = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let recent: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM passwords WHERE deleted_at IS NULL AND last_used_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    Ok(PasswordStats {
        all,
        favorite,
        recent,
    })
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

    // 成功登记待粘贴后顺带登记使用记录（注入成功/失败不影响登记结果，故失败只记日志）
    let now = chrono::Utc::now().timestamp_millis();
    if let Err(e) = conn.execute(
        "UPDATE passwords SET last_used_at = ?1, use_count = use_count + 1 WHERE id = ?2",
        params![now, id],
    ) {
        log::warn!("[CMD] 登记使用记录失败: id={}, err={}", id, e);
    }

    log::info!("[CMD] copy_password_to_clipboard 完成: id={}, seq={}, ttl={}s", id, seq, clear_time);

    Ok(clear_time)
}

#[cfg(test)]
mod tests {
    use super::{
        build_search_haystack, matches_search, normalize_tags, pinyin_variants, tokenize_search,
    };

    /// 空串与纯空白标签被丢弃，其余标签保留
    #[test]
    fn normalize_tags_drops_blank_tags() {
        let tags = vec![
            "  工作 ".to_string(),
            "".to_string(),
            "   ".to_string(),
            "数据库".to_string(),
        ];
        assert_eq!(
            normalize_tags(&tags),
            vec!["工作".to_string(), "数据库".to_string()]
        );
    }

    /// 大小写不敏感去重，保留首次出现的写法
    #[test]
    fn normalize_tags_dedupes_case_insensitively() {
        let tags = vec![
            "GitHub".to_string(),
            "github".to_string(),
            "GITHUB".to_string(),
            "Work".to_string(),
        ];
        assert_eq!(
            normalize_tags(&tags),
            vec!["GitHub".to_string(), "Work".to_string()]
        );
    }

    /// 超过 32 个字符按 Unicode 字符截断（不是按字节）
    #[test]
    fn normalize_tags_truncates_by_chars_not_bytes() {
        // 40 个汉字 = 40 字符 / 120 字节
        let tags = vec!["密".repeat(40)];
        let normalized = normalize_tags(&tags);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].chars().count(), 32);
        assert_eq!(normalized[0], "密".repeat(32));
    }

    /// 最多保留前 20 个标签
    #[test]
    fn normalize_tags_limits_to_twenty() {
        let tags: Vec<String> = (0..25).map(|i| format!("tag{i}")).collect();
        let normalized = normalize_tags(&tags);
        assert_eq!(normalized.len(), 20);
        assert_eq!(normalized[0], "tag0");
        assert_eq!(normalized[19], "tag19");
    }

    /// 保持录入顺序（不排序）
    #[test]
    fn normalize_tags_keeps_input_order() {
        let tags = vec!["z".to_string(), "a".to_string(), "m".to_string()];
        assert_eq!(
            normalize_tags(&tags),
            vec!["z".to_string(), "a".to_string(), "m".to_string()]
        );
    }

    /// 拼音全拼与简拼（中文）
    #[test]
    fn pinyin_variants_chinese_full_and_initials() {
        let (full, initials) = pinyin_variants("工作");
        assert_eq!(full, "gongzuo");
        assert_eq!(initials, "gz");
    }

    /// 非汉字原样小写保留（全拼与简拼一致）
    #[test]
    fn pinyin_variants_keeps_ascii_lowercase() {
        let (full, initials) = pinyin_variants("GitHub");
        assert_eq!(full, "github");
        assert_eq!(initials, "github");
    }

    /// 中英混合：汉字转拼音，英文原样保留；简拼含汉字首字母
    #[test]
    fn pinyin_variants_handles_mixed_text() {
        let (full, initials) = pinyin_variants("工作GitHub");
        assert_eq!(full, "gongzuogithub");
        assert_eq!(initials, "gzgithub");
    }

    /// 拼音结果进程内缓存：重复调用结果一致
    #[test]
    fn pinyin_variants_cache_is_consistent() {
        assert_eq!(pinyin_variants("工作"), pinyin_variants("工作"));
    }

    /// 搜索分词：空白分词 + 转小写
    #[test]
    fn tokenize_search_splits_and_lowercases() {
        assert_eq!(
            tokenize_search("  GitHub   工作  "),
            vec!["github".to_string(), "工作".to_string()]
        );
        assert!(tokenize_search("   ").is_empty());
    }

    /// 搜索分词 AND：全拼/简拼/原文均可命中，缺词即不命中
    #[test]
    fn matches_search_requires_all_tokens() {
        let haystack = build_search_haystack(&["工作", "GitHub"]);
        assert!(matches_search(&haystack, &tokenize_search("gongzuo github")));
        assert!(matches_search(&haystack, &tokenize_search("gz 工作")));
        assert!(!matches_search(
            &haystack,
            &tokenize_search("gongzuo 数据库")
        ));
    }

    /// 原始文本（不走拼音）也能命中，且大小写不敏感
    #[test]
    fn matches_search_matches_original_text() {
        let haystack = build_search_haystack(&["数据库"]);
        assert!(matches_search(&haystack, &tokenize_search("数据库")));
        assert!(matches_search(&haystack, &tokenize_search("SHUJUKU")));
        assert!(!matches_search(&haystack, &tokenize_search("mysql")));
    }
}
