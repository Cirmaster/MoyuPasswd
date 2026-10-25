//! 数据库模块 - 管理 SQLite 数据库
//!
//! 提供数据库初始化、迁移和基本操作。
//! 使用 rusqlite + SQLCipher 加密数据库。
//!
//! # 加密流程
//!
//! - **首次运行**：使用默认密钥创建数据库，设置主密码后用 `rekey` 切换为派生密钥；
//!   若目录中已存在无法用默认密钥打开的旧库文件，改名隔离保留，绝不删除
//! - **后续运行**：使用派生密钥打开数据库（密钥从主密码 + db_salt 派生）
//!
//! # 数据库表
//!
//! - `master_password`: 预留（凭证已迁移至独立元数据文件）
//! - `passwords`: 存储加密的密码数据
//! - `password_tags`: 存储密码标签（明文，含排序号）
//! - `categories`: 存储密码分类（`parent_id` 支持分类树）
//! - `settings`: 存储应用设置

use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use parking_lot::Mutex;

/// 数据库文件名
pub const DB_NAME: &str = "moyu_passwd.db";

/// 默认数据库加密密钥
///
/// 首次创建数据库时使用，设置主密码后通过 PRAGMA rekey 切换为派生密钥。
/// 32 字节，硬编码在代码中（不保密，仅用于初始创建阶段）。
pub const DEFAULT_DB_KEY: &[u8; 32] = b"moyu-passwd-default-key-00000000";

/// 忙等待超时（秒）
///
/// 跨进程/跨连接锁竞争时让 SQLite 自动重试，而不是立刻报 "database is locked"。
/// rusqlite 默认 0 = 不等待，任何瞬时锁冲突都会变成硬错误。
const BUSY_TIMEOUT_SECS: u64 = 5;

/// 尝试用指定密钥打开数据库并触发一次实际读取
///
/// 密钥错误时 SQLCipher 要到第一次读取才报错，`PRAGMA user_version` 会触发读取。
/// 用于首跑时判断「已存在的数据库文件」能否用默认密钥复用。
fn can_open_with_key(db_path: &Path, key: &[u8; 32]) -> bool {
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("探测旧数据库打开失败: {e}");
            return false;
        }
    };
    // 忙等待超时：避免锁竞争把「能打开」误判成「打不开」（误判会导致旧库被隔离）
    if let Err(e) = conn.busy_timeout(std::time::Duration::from_secs(BUSY_TIMEOUT_SECS)) {
        log::warn!("探测旧数据库设置忙等待超时失败: {e}");
    }
    let key_hex = hex::encode(key);
    if let Err(e) = conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", key_hex)) {
        log::warn!("探测旧数据库设置密钥失败: {e}");
        return false;
    }
    match conn.query_row("PRAGMA user_version;", [], |row| row.get::<_, i32>(0)) {
        Ok(_) => true,
        Err(e) => {
            log::warn!("探测旧数据库密钥不匹配或文件损坏: {e}");
            false
        }
    }
}

/// 探测密钥是否与现有数据库匹配（不迁移、不改动任何数据）
///
/// SQLCipher 的密钥校验发生在第一次实际读取时，因此用 `sqlite_master` 触发一次读取。
/// 用于区分「密钥真的不匹配」与「临时故障」，调用方**绝不能**把临时故障当成密钥失效：
///
/// - `Ok(true)`：密钥匹配
/// - `Ok(false)`：密钥不匹配（SQLCipher 读取报 NOTADB）
/// - `Err(_)`：无法判定（文件被锁、读取瞬时失败等），调用方应原样上抛、不做破坏性动作
pub fn verify_key(app_dir: &Path, key: &[u8; 32]) -> Result<bool, String> {
    let db_path = app_dir.join(DB_NAME);
    if !db_path.exists() {
        return Err("数据库文件不存在，无法判定密钥".to_string());
    }

    let conn = Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_secs(BUSY_TIMEOUT_SECS))
        .map_err(|e| format!("设置忙等待超时失败: {e}"))?;

    let key_hex = hex::encode(key);
    conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", key_hex))
        .map_err(|e| format!("设置数据库密钥失败: {e}"))?;

    // 触发第一次读取：密钥错误时此处报 NOTADB（file is not a database）
    match conn.query_row("SELECT count(*) FROM sqlite_master;", [], |row| {
        row.get::<_, i32>(0)
    }) {
        Ok(_) => Ok(true),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::NotADatabase =>
        {
            Ok(false)
        }
        Err(e) => Err(format!("探测密钥失败: {e}")),
    }
}

/// 隔离旧库时随库保存的凭据快照后缀
///
/// 只隔离数据文件是不够的：没有对应的主密码哈希与盐值，用户即使记得当时的主密码
/// 也派生不出解密密钥。快照与隔离库同名同批次，恢复时一并还原。
const META_SNAPSHOT_SUFFIX: &str = ".meta.json";

/// 隔离已存在的旧数据库文件（改名保留，绝不删除用户数据）
///
/// 旧数据库可能是明文 SQLite、使用不同密钥加密或已损坏；
/// 一律改名为 `moyu_passwd.db.invalid-<时间戳>` 保留备查，由用户决定去留。
///
/// 同时把当前凭据（主密码哈希/盐值）快照到 `<隔离库>.meta.json`，
/// 使「设置 → 备份」里的恢复功能能真正把库还原到可用状态。
fn quarantine_existing_db(app_dir: &Path) -> Result<(), String> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // 先快照凭据：必须在改动任何文件之前读，此时 meta 还是与旧库配套的那一份
    if let Ok(Some(json)) = crate::db_meta::read_meta_json(app_dir) {
        let snapshot = app_dir.join(format!(
            "{}.invalid-{}{}",
            DB_NAME, ts, META_SNAPSHOT_SUFFIX
        ));
        match fs::write(&snapshot, json) {
            Ok(()) => {
                crate::acl::harden_backup_acl(&snapshot);
                log::warn!("已随隔离库保存凭据快照: {:?}", snapshot);
            }
            Err(e) => log::warn!("保存隔离库凭据快照失败（该库将无法直接恢复解锁）: {e}"),
        }
    } else {
        log::warn!("隔离旧库时没有可用的凭据元数据，恢复后需手工定位对应主密码");
    }

    for suffix in ["", "-wal", "-shm"] {
        let src = app_dir.join(format!("{}{}", DB_NAME, suffix));
        if src.exists() {
            let dst = app_dir.join(format!("{}.invalid-{}{}", DB_NAME, ts, suffix));
            fs::rename(&src, &dst).map_err(|e| format!("隔离旧数据库失败: {e}"))?;
            // 隔离文件同样是可解密的历史库，ACL 必须与主库一致
            crate::acl::harden_backup_acl(&dst);
            log::warn!("已隔离旧数据库文件: {:?} -> {:?}", src, dst);
        }
    }
    Ok(())
}

/// 数据库连接管理器
pub struct Database {
    /// 数据库连接
    conn: Mutex<Connection>,
}

impl Database {
    /// 创建新的数据库连接
    ///
    /// 使用指定密钥打开（或创建）SQLCipher 数据库。
    ///
    /// # Arguments
    ///
    /// * `app_dir` - 应用数据目录
    /// * `key` - 32 字节的加密密钥
    ///
    /// # Returns
    ///
    /// 数据库实例
    pub fn new(app_dir: &PathBuf, key: &[u8; 32]) -> Result<Self, String> {
        // 确保目录存在
        fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;

        // 数据库文件路径
        let db_path = app_dir.join(DB_NAME);

        // 如果是用默认 key 创建（首次运行）且已存在数据库文件：
        // 能用默认密钥打开就直接复用；打不开（明文旧库/异密钥/损坏）则改名隔离。
        // 注意：绝不删除用户数据——「读取失败」也绝不能走到这里（见 db_meta 三态）。
        if key == DEFAULT_DB_KEY && db_path.exists() && !can_open_with_key(&db_path, DEFAULT_DB_KEY) {
            quarantine_existing_db(app_dir)?;
        }

        // 打开数据库连接
        let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;

        // 忙等待超时：跨进程锁竞争（如另一实例残留连接/杀软扫描）时自动重试，
        // 避免任何瞬时锁冲突直接变成 "database is locked" 硬错误
        conn.busy_timeout(std::time::Duration::from_secs(BUSY_TIMEOUT_SECS))
            .map_err(|e| format!("设置忙等待超时失败: {e}"))?;

        // 设置加密密钥
        let key_hex = hex::encode(key);
        conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", key_hex))
            .map_err(|e| format!("设置数据库密钥失败: {e}"))?;

        // 启用外键约束
        conn.execute_batch("PRAGMA foreign_keys = ON;").map_err(|e| e.to_string())?;

        let db = Self {
            conn: Mutex::new(conn),
        };

        // 执行迁移
        db.migrate()?;

        // 收紧数据库文件 ACL（Windows；失败不致命，仅记录日志）
        if let Err(e) = crate::acl::harden_file_acl(&db_path) {
            log::warn!("数据库文件 ACL 加固失败: {e}");
        }

        Ok(db)
    }

    /// 使用新密钥重新加密数据库
    ///
    /// 在设置或修改主密码后调用，将数据库从当前密钥切换为新密钥。
    ///
    /// # Arguments
    ///
    /// * `new_key` - 新的 32 字节加密密钥
    pub fn rekey(&self, new_key: &[u8; 32]) -> Result<(), String> {
        let conn = self.conn.lock();
        let key_hex = hex::encode(new_key);
        conn.execute_batch(&format!("PRAGMA rekey = \"x'{}'\";", key_hex))
            .map_err(|e| format!("重新加密数据库失败: {e}"))?;
        log::info!("数据库已使用新密钥重新加密");
        Ok(())
    }

    /// 执行数据库迁移
    fn migrate(&self) -> Result<(), String> {
        let conn = self.conn.lock();

        // 创建迁移版本表
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _migrations (
                version INTEGER PRIMARY KEY,
                description TEXT NOT NULL,
                applied_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000)
            );"
        ).map_err(|e| e.to_string())?;

        // 获取当前版本。
        // 注意：读取失败必须上抛，绝不能吞掉错误当「全新库」处理——
        // 否则瞬时锁冲突会让迁移序列整体重跑，撞上 ALTER TABLE 直接失败。
        let current_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _migrations",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("读取迁移版本失败: {e}"))?;

        // 执行迁移
        // 每条 migration 的 SQL 与其版本记录包进同一事务：
        // execute_batch 内多条语句若中途失败（磁盘满/锁冲突），不在事务里
        // 就会留下半执行状态且版本不记录 → 下次启动重跑整条 migration，
        // 撞上已执行的 ALTER TABLE 报 duplicate column，应用永久无法启动。
        // 事务化后：SQL 与版本记录要么同时生效、要么同时回滚，可安全重试。
        let migrations = get_migrations();
        for migration in migrations {
            if migration.version > current_version {
                let tx = conn
                    .unchecked_transaction()
                    .map_err(|e| format!("迁移 {} 开启事务失败: {}", migration.version, e))?;
                tx.execute_batch(migration.sql)
                    .map_err(|e| format!("迁移 {} 失败: {}", migration.version, e))?;
                tx.execute(
                    "INSERT INTO _migrations (version, description) VALUES (?1, ?2)",
                    rusqlite::params![migration.version, migration.description],
                )
                .map_err(|e| format!("迁移 {} 记录版本失败: {}", migration.version, e))?;
                // 提交点：整条迁移原子生效
                tx.commit()
                    .map_err(|e| format!("迁移 {} 提交失败: {}", migration.version, e))?;
            }
        }

        Ok(())
    }

    /// 获取数据库连接
    pub fn conn(&self) -> parking_lot::MutexGuard<'_, Connection> {
        self.conn.lock()
    }
}

// ==================== 备份与恢复 ====================
//
// 库里可能出现两类历史副本，都允许用户在「设置 → 备份」里看到并恢复：
// - `moyu_passwd.db.invalid-<秒>`：启动时无法用当前密钥打开的旧库被隔离（含凭据快照）
// - `moyu_passwd.db.current-<毫秒>`：恢复备份前被挪开的当时库（用于回退误操作）
// - `backup/moyu_passwd.db.bak`：修改主密码前的整库备份
//
// 设计约束：
// - 只按**白名单文件名**解析，绝不接受任意路径（杜绝目录穿越）
// - 恢复采用「复制」而不是「移动」：备份条目始终保留，可反复回退
// - 被替换掉的当前库改名保留为 `current-*`，误恢复也能拽回来

/// 修改主密码前备份的固定标识（相对 app_dir）
const CHANGE_PASSWORD_BACKUP_NAME: &str = "backup/moyu_passwd.db.bak";

/// 可恢复的备份条目
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackupEntry {
    /// 恢复时使用的标识（受控文件名，已通过白名单校验）
    pub file_name: String,
    /// 类别：quarantine=旧库隔离，current=恢复前的当前库，change_password=改密前备份
    pub kind: String,
    /// 生成时间（Unix 毫秒）
    pub created_at_ms: u64,
    /// 文件大小（字节）
    pub size_bytes: u64,
    /// 是否带凭据快照（带则恢复后可直接用当时的主密码解锁）
    pub has_meta_snapshot: bool,
}

/// 校验备份文件标识是否合法（纯字符串判断，无 IO，供命令层前置校验）
pub fn validate_backup_name(file_name: &str) -> Result<(), String> {
    if file_name == CHANGE_PASSWORD_BACKUP_NAME {
        return Ok(());
    }
    for prefix in [
        format!("{}.invalid-", DB_NAME),
        format!("{}.current-", DB_NAME),
    ] {
        if let Some(ts) = file_name.strip_prefix(&prefix) {
            if !ts.is_empty() && ts.chars().all(|c| c.is_ascii_digit()) {
                return Ok(());
            }
        }
    }
    Err("备份文件名非法".to_string())
}

/// 把文件名里的时间戳统一成毫秒
///
/// 历史隔离文件用的是秒（10 位），新生成的 current 用毫秒（13 位），按量级区分。
fn normalize_ts(digits: &str) -> Option<u64> {
    let value: u64 = digits.parse().ok()?;
    Some(if value < 10_000_000_000 {
        value * 1000
    } else {
        value
    })
}

/// 文件的修改时间（Unix 毫秒；取不到返回 0）
fn modified_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 列出所有可恢复的历史副本（按时间倒序）
pub fn list_backups(app_dir: &Path) -> Vec<BackupEntry> {
    let mut entries: Vec<BackupEntry> = Vec::new();

    let invalid_prefix = format!("{}.invalid-", DB_NAME);
    let current_prefix = format!("{}.current-", DB_NAME);

    if let Ok(dir) = std::fs::read_dir(app_dir) {
        for item in dir.flatten() {
            let file_name = item.file_name().to_string_lossy().to_string();
            let (kind, prefix) = if file_name.starts_with(&invalid_prefix) {
                ("quarantine", invalid_prefix.as_str())
            } else if file_name.starts_with(&current_prefix) {
                ("current", current_prefix.as_str())
            } else {
                continue;
            };

            // 跳过附属文件：-wal / -shm / .meta.json 不是独立的库
            if file_name.ends_with(META_SNAPSHOT_SUFFIX)
                || file_name.ends_with("-wal")
                || file_name.ends_with("-shm")
            {
                continue;
            }

            let ts = file_name.strip_prefix(prefix).unwrap_or("");
            if ts.is_empty() || !ts.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }

            let path = app_dir.join(&file_name);
            let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size_bytes == 0 {
                // 0 字节残留没有恢复价值，列出来只会误导用户
                continue;
            }
            let snapshot = app_dir.join(format!(
                "{}.invalid-{}{}",
                DB_NAME, ts, META_SNAPSHOT_SUFFIX
            ));

            entries.push(BackupEntry {
                file_name: file_name.clone(),
                kind: kind.to_string(),
                created_at_ms: normalize_ts(ts).unwrap_or_else(|| modified_ms(&path)),
                size_bytes,
                has_meta_snapshot: kind == "quarantine" && snapshot.exists(),
            });
        }
    }

    // 改密前备份（固定文件名）
    let bak = app_dir.join("backup").join(format!("{}.bak", DB_NAME));
    if bak.exists() {
        let size_bytes = std::fs::metadata(&bak).map(|m| m.len()).unwrap_or(0);
        if size_bytes > 0 {
            entries.push(BackupEntry {
                file_name: CHANGE_PASSWORD_BACKUP_NAME.to_string(),
                kind: "change_password".to_string(),
                created_at_ms: modified_ms(&bak),
                size_bytes,
                has_meta_snapshot: false,
            });
        }
    }

    entries.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    entries
}

/// 从备份恢复数据库
///
/// 调用前提：**数据库连接已关闭**（Windows 下打开的文件无法替换），
/// 由命令层先 `clear_aes_key` 再调用。
///
/// 流程：当前库改名保留 → 复制备份回主库位置 → 还原凭据快照（若有）→ 清理事务日志。
/// 恢复后密钥与 meta 可能已变，调用方必须锁定应用，让用户用该库当时的主密码解锁。
pub fn restore_backup(app_dir: &Path, file_name: &str) -> Result<(), String> {
    validate_backup_name(file_name)?;

    let (src, snapshot) = if file_name == CHANGE_PASSWORD_BACKUP_NAME {
        (app_dir.join("backup").join(format!("{}.bak", DB_NAME)), None)
    } else {
        let ts = file_name
            .strip_prefix(&format!("{}.invalid-", DB_NAME))
            .or_else(|| file_name.strip_prefix(&format!("{}.current-", DB_NAME)))
            .ok_or("备份文件名非法")?;
        let snap = app_dir.join(format!(
            "{}.invalid-{}{}",
            DB_NAME, ts, META_SNAPSHOT_SUFFIX
        ));
        (
            app_dir.join(file_name),
            if file_name.contains(".invalid-") && snap.exists() {
                Some(snap)
            } else {
                None
            },
        )
    };

    if !src.exists() {
        return Err("备份文件不存在（可能已被移动或删除）".to_string());
    }

    // 1. 当前库先改名保留，恢复错了还能从列表里再恢复回来
    let current = app_dir.join(DB_NAME);
    if current.exists() {
        let ts_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let keep = app_dir.join(format!("{}.current-{}", DB_NAME, ts_now));
        std::fs::rename(&current, &keep).map_err(|e| format!("暂存当前库失败: {e}"))?;
        crate::acl::harden_backup_acl(&keep);
        log::warn!("恢复前的当前库已另存: {:?}", keep);
    }
    // 旧的 WAL/SHM 与新库不配套，必须清掉，否则 SQLite 会尝试回放脏日志
    for suffix in ["-wal", "-shm"] {
        let side = app_dir.join(format!("{}{}", DB_NAME, suffix));
        if side.exists() {
            let _ = std::fs::remove_file(&side);
        }
    }

    // 2. 复制（不移动）备份到主库位置：备份条目保留，支持反复回退
    std::fs::copy(&src, &current).map_err(|e| format!("还原备份文件失败: {e}"))?;
    if let Err(e) = crate::acl::harden_file_acl(&current) {
        log::warn!("恢复后的主库 ACL 加固失败: {e}");
    }

    // 3. 有凭据快照则一并还原：没有它，恢复出来的库无法用任何密码解锁
    match snapshot {
        Some(snap) => {
            let json = std::fs::read_to_string(&snap)
                .map_err(|e| format!("读取凭据快照失败: {e}"))?;
            crate::db_meta::write_meta_json(app_dir, &json)?;
            log::warn!("已随备份还原凭据快照: {:?}", snap);
        }
        None => log::warn!("该备份没有凭据快照，恢复后需用与它配套的主密码解锁"),
    }

    // 4. 清掉可能残留的改密事务日志，否则下次启动会按旧日志把刚恢复的库再度回滚
    let _ = crate::db_meta::delete_meta_journal(app_dir);

    log::warn!("已从备份恢复数据库: {} -> {}", file_name, DB_NAME);
    Ok(())
}

/// 迁移结构体
struct Migration {
    /// 版本号
    version: i32,
    /// 描述
    description: &'static str,
    /// SQL 语句
    sql: &'static str,
}

/// 获取数据库迁移列表
fn get_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "create_initial_tables",
            sql: "
                -- 主密码表：预留（凭证已迁移至独立元数据文件，表保留以兼容迁移序列）
                CREATE TABLE IF NOT EXISTS master_password (
                    id INTEGER PRIMARY KEY DEFAULT 1,
                    hash TEXT NOT NULL DEFAULT '',
                    salt TEXT NOT NULL DEFAULT '',
                    created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
                    updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000)
                );

                -- 密码表：存储加密的密码数据
                CREATE TABLE IF NOT EXISTS passwords (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    username TEXT NOT NULL,
                    password_encrypted TEXT NOT NULL,
                    url TEXT,
                    notes TEXT,
                    category_id TEXT DEFAULT 'other',
                    is_favorite INTEGER DEFAULT 0,
                    created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
                    updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
                    deleted_at INTEGER
                );

                -- 分类表：存储密码分类
                CREATE TABLE IF NOT EXISTS categories (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    icon TEXT,
                    sort_order INTEGER DEFAULT 0,
                    created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000)
                );

                -- 设置表：存储应用设置
                CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                -- 创建索引
                CREATE INDEX IF NOT EXISTS idx_passwords_category ON passwords(category_id);
                CREATE INDEX IF NOT EXISTS idx_passwords_deleted ON passwords(deleted_at);
                CREATE INDEX IF NOT EXISTS idx_passwords_favorite ON passwords(is_favorite);
            ",
        },
        Migration {
            version: 2,
            description: "insert_default_categories",
            sql: "
                -- 插入默认分类
                INSERT OR IGNORE INTO categories (id, name, sort_order) VALUES ('social', '社交媒体', 1);
                INSERT OR IGNORE INTO categories (id, name, sort_order) VALUES ('work', '工作', 2);
                INSERT OR IGNORE INTO categories (id, name, sort_order) VALUES ('finance', '金融', 3);
                INSERT OR IGNORE INTO categories (id, name, sort_order) VALUES ('other', '其他', 4);

                -- 插入默认设置
                INSERT OR IGNORE INTO settings (key, value) VALUES ('auto_lock_time', '5');
                INSERT OR IGNORE INTO settings (key, value) VALUES ('clipboard_clear_time', '30');
                -- 明暗模式默认深色：与前端 useTheme / index.html 防闪脚本的默认值保持一致。
                -- 这里不能写 'system'——设置页的兜底逻辑会把本行当作用户偏好读走，
                -- 导致首次安装后一进设置页主题就跳成「跟随系统」
                INSERT OR IGNORE INTO settings (key, value) VALUES ('theme', 'dark');
                INSERT OR IGNORE INTO settings (key, value) VALUES ('language', 'zh-CN');
            ",
        },
        Migration {
            version: 3,
            description: "add_aes_salt_to_master_password",
            sql: "
                -- 新增 AES 密钥盐列（预留，实际盐值已迁移至元数据文件）
                ALTER TABLE master_password ADD COLUMN aes_salt TEXT;
            ",
        },
        Migration {
            version: 4,
            description: "add_notes_encrypted_to_passwords",
            sql: "
                -- 新增加密备注列（备注与密码同样使用 AES-256-GCM 加密）
                ALTER TABLE passwords ADD COLUMN notes_encrypted TEXT;
            ",
        },
        Migration {
            version: 5,
            description: "add_db_salt_to_master_password",
            sql: "
                -- 新增数据库加密密钥盐列（预留，实际盐值已迁移至元数据文件）
                ALTER TABLE master_password ADD COLUMN db_salt TEXT;
            ",
        },
        Migration {
            version: 6,
            description: "add_strength_to_passwords",
            sql: "
                -- 新增密码强度列（写入时计算并存储，列表加载不再全量解密）
                ALTER TABLE passwords ADD COLUMN strength INTEGER;
            ",
        },
        Migration {
            version: 7,
            description: "add_extra_fields_to_passwords",
            sql: "
                -- 新增自定义字段两列：
                -- extra_fields_plain: 明文 JSON（全部字段的 label/order + 非敏感字段的 value，可被搜索）
                -- extra_fields_encrypted: 敏感字段 value 的 AES-256-GCM 密文（JSON，仅含 order + value）
                ALTER TABLE passwords ADD COLUMN extra_fields_plain TEXT;
                ALTER TABLE passwords ADD COLUMN extra_fields_encrypted TEXT;
            ",
        },
        Migration {
            version: 8,
            description: "add_tags_and_usage",
            sql: "
                -- 标签表：标签为明文（非机密，仅作分类元数据），随 password_id 关联；
                -- sort_order 保留用户录入顺序，软删除密码不级联删除标签（保证可恢复性一致）
                CREATE TABLE IF NOT EXISTS password_tags (
                    password_id TEXT NOT NULL,
                    tag TEXT NOT NULL,
                    sort_order INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (password_id, tag)
                );
                -- 标签按大小写不敏感全局分组/查询
                CREATE INDEX IF NOT EXISTS idx_password_tags_tag ON password_tags(tag COLLATE NOCASE);
                CREATE INDEX IF NOT EXISTS idx_password_tags_password ON password_tags(password_id);

                -- 使用记录：复制/注入密码时更新，支撑「最近使用」与「使用频率」
                ALTER TABLE passwords ADD COLUMN last_used_at INTEGER;
                ALTER TABLE passwords ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0;
            ",
        },
        Migration {
            version: 9,
            description: "add_category_tree",
            sql: "
                -- 分类树：parent_id 为 NULL 表示顶级分类；
                -- 不建外键（删除/防环逻辑在命令层兜底，避免影响已有迁移序列）
                ALTER TABLE categories ADD COLUMN parent_id TEXT;
                CREATE INDEX IF NOT EXISTS idx_categories_parent ON categories(parent_id);
            ",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个干净的临时目录（带进程号，避免并发测试互相踩）
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "moyu-passwd-test-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 备份文件名白名单：只接受本模块自己生成的三类名字，拒绝路径穿越
    #[test]
    fn validate_backup_name_rejects_path_traversal() {
        assert!(validate_backup_name("backup/moyu_passwd.db.bak").is_ok());
        assert!(validate_backup_name("moyu_passwd.db.invalid-1690000000").is_ok());
        assert!(validate_backup_name("moyu_passwd.db.current-1690000000000").is_ok());

        assert!(validate_backup_name("../evil.db").is_err());
        assert!(validate_backup_name("moyu_passwd.db.invalid-../../x").is_err());
        assert!(validate_backup_name("moyu_passwd.db.invalid-").is_err());
        assert!(validate_backup_name("moyu_passwd.db").is_err());
        assert!(validate_backup_name("backup/other.db.bak").is_err());
    }

    /// 列表要跳过 -wal/-shm/.meta.json 和 0 字节残留，只给出真正可恢复的库
    #[test]
    fn list_backups_skips_sidecar_and_empty_files() {
        let dir = temp_dir("list");
        fs::write(dir.join(format!("{}.invalid-1700000000", DB_NAME)), b"old").unwrap();
        fs::write(
            dir.join(format!("{}.invalid-1700000000.meta.json", DB_NAME)),
            b"{}",
        )
        .unwrap();
        fs::write(
            dir.join(format!("{}.invalid-1700000000-wal", DB_NAME)),
            b"wal",
        )
        .unwrap();
        fs::write(
            dir.join(format!("{}.current-1700000000000", DB_NAME)),
            b"cur",
        )
        .unwrap();
        // 0 字节残留没有恢复价值
        fs::write(dir.join(format!("{}.invalid-1700000001", DB_NAME)), b"").unwrap();

        let list = list_backups(&dir);
        assert_eq!(list.len(), 2, "应为 1 个隔离库 + 1 个 current");
        assert!(list
            .iter()
            .any(|e| e.kind == "quarantine" && e.has_meta_snapshot));
        assert!(list
            .iter()
            .any(|e| e.kind == "current" && !e.has_meta_snapshot));
        assert!(list[0].created_at_ms >= list[1].created_at_ms, "应按时间倒序");

        let _ = fs::remove_dir_all(&dir);
    }

    /// 恢复：主库换成备份内容，备份条目保留，原当前库另存可回退
    ///
    /// 注意：这里**故意不建 .meta.json 快照**——有快照时 restore 会写系统凭据管理器，
    /// 测试环境绝不能碰用户真实的 meta。
    #[test]
    fn restore_backup_keeps_current_and_preserves_backup_entry() {
        let dir = temp_dir("restore");
        let current = dir.join(DB_NAME);
        fs::write(&current, b"current-db").unwrap();
        let backup_name = format!("{}.invalid-1700000000", DB_NAME);
        let backup = dir.join(&backup_name);
        fs::write(&backup, b"old-db").unwrap();

        restore_backup(&dir, &backup_name).unwrap();

        // 1. 主库已换成备份内容
        assert_eq!(fs::read(&current).unwrap(), b"old-db");
        // 2. 备份条目仍在（支持反复回退）
        assert!(backup.exists());
        // 3. 原当前库被另存为 current-*，内容完好
        let kept: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{}.current-", DB_NAME))
            })
            .collect();
        assert_eq!(kept.len(), 1, "原当前库应被另存一份");
        assert_eq!(fs::read(kept[0].path()).unwrap(), b"current-db");

        let _ = fs::remove_dir_all(&dir);
    }
}
