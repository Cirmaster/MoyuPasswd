//! 应用状态模块 - 管理全局状态
//!
//! 使用 Tauri 的状态管理功能存储全局状态。
//!
//! # 安全设计
//!
//! - AES 密钥仅存储在内存中，锁定时清零
//! - 数据库在解锁后才初始化，锁定后清除引用
//! - 使用 parking_lot::Mutex 保证线程安全（支持 MutexGuard::map）
//! - 暴力破解防护：连续失败后渐进锁定

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use parking_lot::Mutex;
use zeroize::Zeroize;
use crate::db::Database;

/// 锁定计数持久化文件名（存 app_dir，不能存加密库——被锁定时库打不开）
const LOCKOUT_FILENAME: &str = "lockout.json";

/// 锁定计数持久化文件格式
#[derive(serde::Serialize, serde::Deserialize)]
struct LockoutFile {
    /// 连续验证失败次数（跨重启保留，保证指数退避持续累积）
    failed_attempts: u32,
    /// 锁定截止时间（Unix 毫秒；过期的截止不恢复为「锁定中」）
    #[serde(default)]
    locked_until_ms: Option<u64>,
}

/// 当前时间（Unix 毫秒）
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 从磁盘载入锁定计数（文件缺失/损坏时从零开始，失败计数不恢复）
fn load_lockout(app_dir: &std::path::Path) -> (u32, Option<u64>) {
    let path = app_dir.join(LOCKOUT_FILENAME);
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<LockoutFile>(&content) {
            Ok(data) => {
                let until = data.locked_until_ms.filter(|&u| u > now_ms());
                (data.failed_attempts, until)
            }
            Err(e) => {
                log::warn!("lockout 文件损坏，忽略: {e}");
                (0, None)
            }
        },
        Err(_) => (0, None),
    }
}

/// 暴力破解防护：连续失败 5 次后锁定
const MAX_FAILED_ATTEMPTS: u32 = 5;
/// 锁定基础时长（秒）
const LOCK_BASE_SECONDS: u64 = 30;
/// 锁定时长上限（秒）
const LOCK_MAX_SECONDS: u64 = 900; // 15 分钟

/// 应用状态
///
/// 存储应用运行时的全局状态。
/// 数据库在认证成功后才可用，锁定后清除。
pub struct AppState {
    /// 应用数据目录（用于后续创建数据库）
    pub app_dir: PathBuf,

    /// 数据库连接（解锁后可用，锁定后为 None）
    db: Mutex<Option<Database>>,

    /// 当前的 AES-256 密钥（解锁后存储，锁定时为 None）
    pub aes_key: Mutex<Option<[u8; 32]>>,

    /// 数据库加密密钥（解锁后存储，锁定时为 None）
    pub db_key: Mutex<Option<[u8; 32]>>,

    /// 是否已解锁
    pub is_unlocked: Mutex<bool>,

    /// 连续密码验证失败次数（用于暴力破解防护；持久化，跨重启累积）
    failed_attempts: Mutex<u32>,

    /// 锁定截止时间（Unix 毫秒；达到失败阈值后设置，到期前拒绝验证）
    locked_until_ms: Mutex<Option<u64>>,

    /// 最近一次 webview 心跳（前端主线程周期性上报；watchdog 据此判断网页是否存活）
    last_heartbeat: Mutex<std::time::Instant>,

    /// 网页是否被判定异常（心跳停止且自动重载失败/超限）。
    /// 为 true 时「关闭到托盘」失效——网页已无法交互，藏进托盘也点不出主窗口，
    /// 关闭必须真正退出
    webview_unhealthy: std::sync::atomic::AtomicBool,
}

impl AppState {
    /// 创建应用状态（首次运行，数据库已用默认密钥创建）
    pub fn new(app_dir: PathBuf, db: Database) -> Self {
        let (failed_attempts, locked_until_ms) = load_lockout(&app_dir);
        Self {
            app_dir,
            db: Mutex::new(Some(db)),
            aes_key: Mutex::new(None),
            db_key: Mutex::new(None),
            is_unlocked: Mutex::new(false),
            failed_attempts: Mutex::new(failed_attempts),
            locked_until_ms: Mutex::new(locked_until_ms),
            last_heartbeat: Mutex::new(std::time::Instant::now()),
            webview_unhealthy: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 创建应用状态（后续运行，数据库待认证后创建）
    pub fn new_without_db(app_dir: PathBuf) -> Self {
        let (failed_attempts, locked_until_ms) = load_lockout(&app_dir);
        Self {
            app_dir,
            db: Mutex::new(None),
            aes_key: Mutex::new(None),
            db_key: Mutex::new(None),
            is_unlocked: Mutex::new(false),
            failed_attempts: Mutex::new(failed_attempts),
            locked_until_ms: Mutex::new(locked_until_ms),
            last_heartbeat: Mutex::new(std::time::Instant::now()),
            webview_unhealthy: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 设置数据库（认证成功后调用）
    pub fn set_database(&self, database: Database) {
        let mut db = self.db.lock();
        *db = Some(database);
    }

    /// 记录 webview 心跳：网页脚本仍存活的证明，同时清除异常标记
    pub fn mark_webview_alive(&self) {
        *self.last_heartbeat.lock() = std::time::Instant::now();
        self.webview_unhealthy
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// 距最近一次心跳的时长（启动后从未心跳则为进程启动以来）
    pub fn heartbeat_age(&self) -> std::time::Duration {
        self.last_heartbeat.lock().elapsed()
    }

    /// 标记网页异常状态（watchdog 自动恢复失败时置 true）
    pub fn set_webview_unhealthy(&self, value: bool) {
        self.webview_unhealthy
            .store(value, std::sync::atomic::Ordering::Relaxed);
    }

    /// 网页是否被判定异常
    pub fn is_webview_unhealthy(&self) -> bool {
        self.webview_unhealthy
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 获取数据库引用
    ///
    /// 如果数据库未初始化（未解锁），返回错误。
    pub fn get_db(&self) -> Result<parking_lot::lock_api::MappedMutexGuard<'_, parking_lot::RawMutex, Database>, String> {
        let guard = self.db.lock();
        if guard.is_some() {
            Ok(parking_lot::MutexGuard::map(guard, |opt| opt.as_mut().unwrap()))
        } else {
            Err("应用未解锁，请先输入主密码".to_string())
        }
    }
    /// 检查数据库是否已初始化
    pub fn has_database(&self) -> bool {
        let db = self.db.lock();
        db.is_some()
    }

    // ==================== AES 密钥管理 ====================

    /// 设置 AES 密钥
    pub fn set_aes_key(&self, key: [u8; 32]) {
        let mut aes_key = self.aes_key.lock();
        if let Some(ref mut old_key) = *aes_key {
            old_key.zeroize();
        }
        *aes_key = Some(key);

        let mut is_unlocked = self.is_unlocked.lock();
        *is_unlocked = true;
    }

    /// 获取 AES 密钥
    ///
    /// 返回 `Zeroizing` 包装：调用方持有的副本在作用域结束自动清零，
    /// 不再以裸 `[u8;32]` 散落各命令栈帧（锁定只清 state 里的那一份）。
    pub fn get_aes_key(&self) -> Option<zeroize::Zeroizing<[u8; 32]>> {
        let aes_key = self.aes_key.lock();
        (*aes_key).map(zeroize::Zeroizing::new)
    }

    /// 清除 AES 密钥（锁定时调用，清零所有密钥和数据库引用）
    ///
    /// 顺序约定：**先翻 `is_unlocked` 标志，再清密钥/连接**。
    /// 反过来时，长命令（如导出全库）持有 db guard 会让本函数卡在清连接那一步，
    /// 此窗口内 `is_unlocked` 仍为 true，新命令仍可通过门禁拿到密钥副本。
    pub fn clear_aes_key(&self) {
        // 第一步：翻掉解锁标志——后续命令立刻被 require_unlocked 拒绝
        {
            let mut is_unlocked = self.is_unlocked.lock();
            *is_unlocked = false;
        }

        let mut aes_key = self.aes_key.lock();
        if let Some(ref mut key) = *aes_key {
            key.zeroize();
        }
        *aes_key = None;

        let mut db_key = self.db_key.lock();
        if let Some(ref mut key) = *db_key {
            key.zeroize();
        }
        *db_key = None;

        let mut db = self.db.lock();
        *db = None;
    }

    /// 设置数据库加密密钥
    pub fn set_db_key(&self, key: [u8; 32]) {
        let mut db_key = self.db_key.lock();
        if let Some(ref mut old_key) = *db_key {
            old_key.zeroize();
        }
        *db_key = Some(key);
    }

    /// 获取数据库加密密钥
    ///
    /// 同 `get_aes_key`：返回 `Zeroizing` 包装，副本自动清零。
    pub fn get_db_key(&self) -> Option<zeroize::Zeroizing<[u8; 32]>> {
        let db_key = self.db_key.lock();
        (*db_key).map(zeroize::Zeroizing::new)
    }

    /// 检查是否已解锁
    pub fn is_unlocked(&self) -> bool {
        let is_unlocked = self.is_unlocked.lock();
        *is_unlocked
    }

    // ==================== 暴力破解防护 ====================

    /// 检查当前是否处于锁定状态
    pub fn check_lockout(&self) -> Result<(), u64> {
        let locked_until = self.locked_until_ms.lock();
        if let Some(until) = *locked_until {
            let now = now_ms();
            if now < until {
                let remaining = (until - now) / 1000;
                return Err(remaining.max(1));
            }
        }
        Ok(())
    }

    /// 记录一次验证失败，必要时触发锁定
    ///
    /// 指数退避：第 5 次失败锁 30s，之后每多失败一次翻倍（60s、120s……），上限 15 分钟。
    /// 失败计数只在验证成功时清零；锁定到期**不清零**，否则退避无法累积。
    pub fn record_failure(&self) -> Option<String> {
        let result = {
            let mut attempts = self.failed_attempts.lock();
            *attempts += 1;

            if *attempts >= MAX_FAILED_ATTEMPTS {
                // 指数必须夹取上限：`30 * 2^63` 会溢出 u64，release 构建（未开
                // overflow-checks）按 wrapping 语义得到 0 —— 结果是「连续失败 68 次后
                // 锁定时长归零、限速彻底失效」，debug 构建则直接 panic。
                // 锁定上限 900s / 基础 30s = 30，翻倍 5 次即 960s 已超上限，再翻无意义。
                let lock_exponent = (*attempts - MAX_FAILED_ATTEMPTS).min(5);
                let lock_seconds = LOCK_BASE_SECONDS
                    .saturating_mul(1u64 << lock_exponent)
                    .min(LOCK_MAX_SECONDS);

                let until = now_ms() + lock_seconds * 1000;
                let mut locked_until = self.locked_until_ms.lock();
                *locked_until = Some(until);

                Some(format!("密码错误次数过多，请等待 {} 秒后重试", lock_seconds))
            } else {
                None
            }
        };
        self.persist_lockout();
        result
    }

    /// 清除锁定状态（验证成功时调用）
    pub fn reset_lockout(&self) {
        {
            let mut attempts = self.failed_attempts.lock();
            *attempts = 0;

            let mut locked_until = self.locked_until_ms.lock();
            *locked_until = None;
        }
        self.persist_lockout();
    }

    /// 清除过期的锁定状态
    ///
    /// 只解除「锁定中」，**保留失败计数**——指数退避依赖计数跨锁定周期累积。
    pub fn clear_expired_lockout(&self) {
        let mut dirty = false;
        {
            let mut locked_until = self.locked_until_ms.lock();
            if let Some(until) = *locked_until {
                if now_ms() >= until {
                    *locked_until = None;
                    dirty = true;
                }
            }
        }
        if dirty {
            self.persist_lockout();
        }
    }

    /// 把锁定计数写回磁盘（防重启清零；写失败仅记日志，不影响主流程）
    fn persist_lockout(&self) {
        let data = LockoutFile {
            failed_attempts: *self.failed_attempts.lock(),
            locked_until_ms: *self.locked_until_ms.lock(),
        };
        let path = self.app_dir.join(LOCKOUT_FILENAME);
        match serde_json::to_string(&data) {
            Ok(content) => {
                if let Err(e) = std::fs::write(&path, content) {
                    log::warn!("保存锁定计数失败: {e}");
                    return;
                }
                if let Err(e) = crate::acl::harden_file_acl(&path) {
                    log::warn!("lockout 文件 ACL 加固失败: {e}");
                }
            }
            Err(e) => log::warn!("序列化锁定计数失败: {e}"),
        }
    }
}
