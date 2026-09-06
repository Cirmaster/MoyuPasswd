//! 空闲检测模块
//!
//! 提供系统级空闲时间检测（Windows）和应用内活动上报机制。
//! 用于实现后端自动锁定，不依赖前端 JavaScript 事件。
//!
//! # 工作原理
//!
//! - Windows: 使用 `GetLastInputInfo` API 获取系统空闲时间
//! - 非 Windows: 依赖前端 `report_activity` 上报
//! - 后台线程每 30 秒检查一次，超时触发锁定

use std::sync::atomic::{AtomicU64, Ordering};

/// 应用内最后活动时间（Unix 毫秒时间戳）
/// 前端通过 `report_activity` 命令更新
static LAST_ACTIVITY: AtomicU64 = AtomicU64::new(0);

/// 初始化空闲检测模块
///
/// 在应用启动时调用，设置初始活动时间
pub fn init() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    LAST_ACTIVITY.store(now, Ordering::SeqCst);
}

/// 报告用户活动
///
/// 前端每次用户操作时调用此命令
#[tauri::command]
pub async fn report_activity() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    LAST_ACTIVITY.store(now, Ordering::SeqCst);
}

/// 获取自上次活动以来的空闲时间（秒）
///
/// 综合系统空闲时间和应用内活动时间，取较小值
pub fn get_idle_seconds() -> u64 {
    let system_idle = get_system_idle_seconds();
    let app_idle = get_app_idle_seconds();

    system_idle.min(app_idle)
}

/// 获取应用内空闲时间（秒）
fn get_app_idle_seconds() -> u64 {
    let last_activity = LAST_ACTIVITY.load(Ordering::SeqCst);
    if last_activity == 0 {
        return u64::MAX; // 未初始化
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    now.saturating_sub(last_activity) / 1000
}

/// 获取系统空闲时间（秒）
///
/// Windows: 使用 GetLastInputInfo API
/// 其他平台: 返回 u64::MAX（不检测系统空闲）
#[cfg(windows)]
fn get_system_idle_seconds() -> u64 {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    unsafe {
        let mut lii = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };

        if GetLastInputInfo(&mut lii).as_bool() {
            let current_tick = windows::Win32::System::SystemInformation::GetTickCount();
            let idle_ticks = current_tick.saturating_sub(lii.dwTime);
            // GetTickCount 返回毫秒，转换为秒（u32 -> u64）
            (idle_ticks / 1000) as u64
        } else {
            u64::MAX // 获取失败，不检测系统空闲
        }
    }
}

#[cfg(not(windows))]
fn get_system_idle_seconds() -> u64 {
    u64::MAX // 非 Windows 平台不检测系统空闲
}

/// 启动空闲检测后台线程
///
/// 每 30 秒检查一次空闲时间，超时后调用锁定回调。
/// 由 `lib.rs` 在应用启动时调用。
///
/// # Arguments
///
/// * `auto_lock_minutes` - 自动锁定时间（分钟），从设置读取
/// * `lock_callback` - 锁定回调函数
pub fn start_idle_watcher<F>(lock_callback: F)
where
    F: Fn() + Send + 'static,
{
    std::thread::spawn(move || {
        log::info!("空闲检测线程已启动");

        loop {
            // 每 30 秒检查一次
            std::thread::sleep(std::time::Duration::from_secs(30));

            // 从全局获取自动锁定时间设置（分钟）
            let auto_lock_minutes = get_auto_lock_minutes();
            if auto_lock_minutes == 0 {
                continue; // 0 表示禁用自动锁定
            }

            let idle_seconds = get_idle_seconds();
            let lock_seconds = auto_lock_minutes * 60;

            if idle_seconds >= lock_seconds {
                log::info!(
                    "空闲时间 {} 秒，超过自动锁定阈值 {} 秒，执行锁定",
                    idle_seconds,
                    lock_seconds
                );
                lock_callback();
            }
        }
    });
}

/// 获取自动锁定时间设置（分钟）
///
/// 从数据库 settings 表读取，默认 5 分钟
fn get_auto_lock_minutes() -> u64 {
    // 这里无法直接访问 AppState，使用一个简单的全局变量
    // 在 lib.rs 中设置
    AUTO_LOCK_MINUTES.load(Ordering::SeqCst)
}

/// 全局自动锁定时间（分钟）
static AUTO_LOCK_MINUTES: AtomicU64 = AtomicU64::new(5);

/// 更新自动锁定时间设置
///
/// 由设置命令调用
pub fn set_auto_lock_minutes(minutes: u64) {
    AUTO_LOCK_MINUTES.store(minutes, Ordering::SeqCst);
}
