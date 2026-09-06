//! 数据库模块 - 管理 SQLite 数据库
//!
//! 提供数据库初始化、迁移和基本操作。
//! 使用 rusqlite + SQLCipher 加密数据库。
//!
//! # 加密流程
//!
//! - **首次运行**：使用默认密钥创建数据库，设置主密码后用 `rekey` 切换为派生密钥
//! - **后续运行**：使用派生密钥打开数据库（密钥从主密码 + db_salt 派生）
//!
//! # 数据库表
//!
//! - `master_password`: 预留（凭证已迁移至独立元数据文件）
//! - `passwords`: 存储加密的密码数据
//! - `categories`: 存储密码分类
//! - `settings`: 存储应用设置

use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;
use parking_lot::Mutex;

/// 数据库文件名
pub const DB_NAME: &str = "moyu_passwd.db";

/// 默认数据库加密密钥
///
/// 首次创建数据库时使用，设置主密码后通过 PRAGMA rekey 切换为派生密钥。
/// 32 字节，硬编码在代码中（不保密，仅用于初始创建阶段）。
pub const DEFAULT_DB_KEY: &[u8; 32] = b"moyu-passwd-default-key-00000000";

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

        // 如果是用默认 key 创建（首次运行），删除可能存在的旧数据库
        // 旧数据库可能是明文 SQLite 或使用不同 key 加密的
        if key == DEFAULT_DB_KEY && db_path.exists() {
            log::info!("删除旧数据库文件: {:?}", db_path);
            fs::remove_file(&db_path).map_err(|e| format!("删除旧数据库失败: {e}"))?;
            // 同时删除 WAL 和 SHM 文件
            let _ = fs::remove_file(app_dir.join(format!("{}-wal", DB_NAME)));
            let _ = fs::remove_file(app_dir.join(format!("{}-shm", DB_NAME)));
        }

        // 打开数据库连接
        let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;

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

        // 获取当前版本
        let current_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _migrations",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // 执行迁移
        let migrations = get_migrations();
        for migration in migrations {
            if migration.version > current_version {
                conn.execute_batch(migration.sql).map_err(|e| format!("迁移 {} 失败: {}", migration.version, e))?;
                conn.execute(
                    "INSERT INTO _migrations (version, description) VALUES (?1, ?2)",
                    rusqlite::params![migration.version, migration.description],
                ).map_err(|e| e.to_string())?;
            }
        }

        Ok(())
    }

    /// 获取数据库连接
    pub fn conn(&self) -> parking_lot::MutexGuard<'_, Connection> {
        self.conn.lock()
    }
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
                INSERT OR IGNORE INTO settings (key, value) VALUES ('theme', 'system');
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
    ]
}
