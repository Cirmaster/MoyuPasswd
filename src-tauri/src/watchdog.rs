//! webview 心跳看门狗
//!
//! 自绘标题栏模式下，网页崩溃/卡死后界面上的按钮全部失效，用户只能靠托盘/紧急热键逃生。
//! 本模块用「前端心跳 + 超时自动重载」把崩溃消化掉，多数渲染层崩溃用户根本察觉不到：
//!
//! - 心跳停止超过 15 秒 → 自动 reload 主窗口网页（带重载基线重置，给前端完整启动窗口）
//! - 5 分钟内最多 3 次自动重载（防崩溃循环）；重载失败或超限则标记 webview 异常，
//!   此后「关闭到托盘」放行真正退出（见 lib.rs 的 CloseRequested 分支）
//! - 窗口隐藏（托盘）/最小化时不开门：WebView2 对后台页定时器节流，心跳变慢属正常
//!
//! 逃生热键 Ctrl+Alt+Shift+R（重载）/ Ctrl+Alt+Shift+E（强退）见
//! `commands::shortcuts::register_all`，同样不依赖网页存活。

use std::time::{Duration, Instant};

use tauri::Manager;

use crate::state::AppState;

/// 心跳判定阈值：超过该时长没有心跳视为网页异常（崩溃/卡死）
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(15);

/// 看门狗巡检间隔
const CHECK_INTERVAL: Duration = Duration::from_secs(5);

/// 自动重载预算：窗口期内最多 MAX_RELOADS 次
const RELOAD_WINDOW: Duration = Duration::from_secs(300);
const MAX_RELOADS: usize = 3;

/// 启动看门狗线程（setup 中调用一次；线程随进程退出）
pub fn spawn(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        // 近期自动重载时间点（滚动预算用）
        let mut reloads: Vec<Instant> = Vec::new();
        // 上一巡检是否因窗口隐藏/最小化而暂停检查。
        // 启动时窗口尚未显示，视为暂停
        let mut was_paused = true;

        loop {
            std::thread::sleep(CHECK_INTERVAL);

            let state = app.state::<AppState>();
            // 已判定异常就不再自动干预：等用户热键重载/退出，避免反复 reload 刷屏
            if state.is_webview_unhealthy() {
                continue;
            }

            let Some(window) = app.get_webview_window("main") else {
                continue;
            };
            // 隐藏/最小化时跳过：WebView2 会节流后台页的定时器，心跳变慢是正常现象
            if !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false) {
                was_paused = true;
                continue;
            }

            // 从托盘/最小化恢复可见时：被节流的心跳可能几十秒才补发一发，
            // 直接按旧基线判定会「刚恢复就误重载」——重置基线给完整观察窗口。
            // 若网页真的已死，下一个心跳周期（15s）照样能抓住
            if was_paused {
                was_paused = false;
                state.mark_webview_alive();
                continue;
            }

            if state.heartbeat_age() < HEARTBEAT_TIMEOUT {
                continue;
            }

            // 自动重载预算：RELOAD_WINDOW 内最多 MAX_RELOADS 次
            reloads.retain(|t| t.elapsed() < RELOAD_WINDOW);
            if reloads.len() >= MAX_RELOADS {
                state.set_webview_unhealthy(true);
                log::error!(
                    "webview 心跳停止且自动重载已达上限（{MAX_RELOADS} 次/{RELOAD_WINDOW:?}），标记异常：\
                     之后关闭窗口将直接退出；可按 Ctrl+Alt+Shift+R 手动重载"
                );
                continue;
            }

            log::warn!(
                "webview 心跳停止（{} 秒无响应），自动重载主窗口网页",
                state.heartbeat_age().as_secs()
            );
            match window.reload() {
                Ok(()) => {
                    reloads.push(Instant::now());
                    // 重载即视为重新开始：重置心跳基线，给前端完整启动窗口，
                    // 否则下次巡检会因 age 仍超时而立刻再次重载
                    state.mark_webview_alive();
                }
                Err(e) => {
                    // 重载都失败（如浏览器进程已死）→ 直接判异常，走退出通道
                    log::error!("webview 自动重载失败，标记异常: {e}");
                    state.set_webview_unhealthy(true);
                }
            }
        }
    });
}
