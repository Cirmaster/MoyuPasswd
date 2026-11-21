//! 技术日志（区别于 `audit` 的审计日志）
//!
//! 审计日志回答「谁在什么时候把密码交给了谁」，技术日志回答「为什么没交出去」——
//! 两条线缺一不可。此前 release 构建根本没有安装 logger，所有 `log::xxx!` 全是空转，
//! 现场出问题只能靠猜（B2 的「按键被吞、屏幕无反应」就是典型）。
//!
//! # 级别策略
//!
//! - release 默认只放行 **Warn 及以上**：失败与异常必定留痕，又不把每次粘贴的
//!   流程细节写进磁盘；
//! - 用户在设置里打开「诊断日志」后放行 Info（注入流程细节），**运行期即时生效**；
//! - debug 构建恒全开：开发时不希望日志行为随设置变化而漂移。
//!
//! 过滤用 `filter(allow)` 而不是「dispatch 级别 + `log::set_max_level`」：
//! 插件在 setup 时会把 `log` 的全局 max_level 改成它自己的 dispatch 级别，
//! 初始化顺序稍有变化（插件 setup 早于/晚于我们的调用）就会让 release 全量写盘。
//! 过滤函数只读一个原子开关，顺序无关。
//!
//! # 隐私
//!
//! 日志里**永远不写密码明文**（见 `inject.rs` 注入函数的注释）。
//! 会出现的敏感信息只有条目 ID 与目标进程名/路径，文件落在应用日志目录，
//! 初始化后按审计日志同样的方式收紧 ACL。

use log::LevelFilter;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
use tauri_plugin_log::{Target, TargetKind};

/// 日志文件名（落在应用日志目录；轮转后为 `moyu-passwd.log.1`）
const FILE_NAME: &str = "moyu-passwd";

/// 单个日志文件上限（超过后轮转，只保留一代）
///
/// 插件接口要求 u128，这里跟着它的类型走，避免每次调用都强转
const MAX_FILE_BYTES: u128 = 1024 * 1024;

/// 「诊断日志」开关（运行期可改）
static DIAGNOSTIC: AtomicBool = AtomicBool::new(false);

/// 记录是否放行
///
/// - debug 构建：全放行
/// - release：Warn/Error 恒放行，其余看诊断开关
fn allow(metadata: &log::Metadata) -> bool {
    if cfg!(debug_assertions) {
        return true;
    }
    matches!(metadata.level(), log::Level::Error | log::Level::Warn)
        || DIAGNOSTIC.load(Ordering::Relaxed)
}

/// 安装日志插件（release 构建同样安装）
///
/// 必须在 setup 里、其他模块开始写日志之前调用。
pub fn init(app: &tauri::AppHandle) -> tauri::Result<()> {
    let mut targets = vec![Target::new(TargetKind::LogDir {
        file_name: Some(FILE_NAME.to_string()),
    })];
    // stdout 只在 debug 构建保留：release 是 GUI 程序，没有控制台可看
    #[cfg(debug_assertions)]
    targets.push(Target::new(TargetKind::Stdout));

    app.plugin(
        tauri_plugin_log::Builder::default()
            // 级别不设阈值，交给下面的过滤函数（见模块注释）
            .level(LevelFilter::Trace)
            .filter(allow)
            .max_file_size(MAX_FILE_BYTES)
            .targets(targets)
            .build(),
    )?;

    log::info!(
        "日志已初始化: 文件名={FILE_NAME}.log, 诊断日志={}",
        diagnostic_enabled()
    );

    // 日志文件由插件在 init 时创建，这里顺手收紧 ACL（失败不影响启动）
    if let Ok(dir) = app.path().app_log_dir() {
        let file = dir.join(format!("{FILE_NAME}.log"));
        if let Err(e) = crate::acl::harden_file_acl(&file) {
            log::warn!("日志文件 ACL 加固失败（可能尚未创建）: {e}");
        }
    }

    Ok(())
}

/// 当前是否开着诊断日志
pub fn diagnostic_enabled() -> bool {
    DIAGNOSTIC.load(Ordering::Relaxed)
}

/// 切换「诊断日志」开关（运行期生效，不需要重启）
///
/// debug 构建恒为 Info，不需要也不允许通过开关降级——否则开发环境里
/// 「开不开设置」会改变日志行为，反而干扰排查。
pub fn apply_level(diagnostic: bool) {
    DIAGNOSTIC.store(diagnostic, Ordering::Relaxed);
    log::info!("诊断日志已{}", if diagnostic { "开启" } else { "关闭" });
}
