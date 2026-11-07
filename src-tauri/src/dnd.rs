//! 拖放交付模块（Wayland）
//!
//! 背景：Wayland 下没有全局按键注入，`pending.rs` 那套「等用户按 Ctrl+V」走不通。
//! 唯一既不碰系统剪贴板、又能把密码交到目标输入框的通道是**拖放**——数据由
//! `wl_data_device.start_drag` 点对点传给目标，与 `set_selection`（剪贴板）是
//! 协议层两条独立路径，因此剪贴板管理器（Klipper 等）看不到。
//!
//! 实测结论（2026-11-07，KDE Plasma 6.6.5，探针见 `scripts/dnd-probe/`）：
//!
//! - HTML5 `dragstart` 会被 WebKitGTK 转成 GTK 原生拖放，Wayland 与 XWayland 目标都能收；
//! - **直接在 `drag-data-get` 里覆盖数据无效**：WebKit 的数据是异步从 web 进程取回的
//!   （信号触发时 `gtk_selection_data_get_length() == -1`），它在信号返回之后才写入，
//!   会把我们写的内容冲掉；
//! - **在 `drag-begin` 时摘掉 WebKit 的 `drag-data-get` 处理函数则有效**，此时目标还没来取
//!   数据，异步回填不会被触发。
//!
//! 因此本模块分三步：
//! 1. 前端在 `pointerdown` 调 `prepare_dnd_payload` 武装（只登记解密闭包，不解密）；
//! 2. JS 的 `dragstart` 只 `setData` 一个空串，占位；
//! 3. GTK 的 `drag-data-get` 由本模块接管，现场解密 → 填入 → 立即清零。
//!    明文全程在 Rust 内，不经 IPC、不进前端。
//!
//! 代价（已写进 UI 文案与 README，不要偷偷当作没有）：
//! - **无归因**：Wayland 的 `wl_data_source` 不告诉源「对方是谁」，所以 `inject.rs`
//!   那套终端黑名单、二次确认、审计目标在拖放路径上全部失效；
//! - **窗口摘除是单向的**：摘掉 WebKit 的处理函数后拿不回来，webview 自身向外拖内容的
//!   能力会永久失效（本项目当前没有这种用法，见 `useCategoryDragSort.ts` 是自研指针排序）。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use zeroize::Zeroizing;

use crate::pending::DecryptFn;

/// 武装有效期（秒）
///
/// 比 `pending` 的 10~300 秒短得多：拖放是即时动作，用户从按下把手到松手不会超过几秒，
/// 窗口开太长只是白白扩大「已武装但没在用」的时间。
pub const TTL_SECS: u64 = 20;

/// 一次待交付的拖放
struct Armed {
    /// 密码条目 ID（只存 ID，不存明文）
    entry_id: String,
    /// 代数，与 `pending` 的 seq 同构，用于日志对照
    seq: u64,
    /// 过期时刻
    deadline: Instant,
    /// 解密闭包（延迟解密：取数据时才调用）
    decrypt_fn: DecryptFn,
}

/// 全局武装状态（同一时刻只允许一次拖放在途）
static ARMED: Mutex<Option<Armed>> = Mutex::new(None);

/// 代数计数器
static SEQ: AtomicU64 = AtomicU64::new(0);

/// 本次拖放是否真的把数据送出去过
///
/// `drag-end` 的 `selected_action` 只能说明「目标那边点了接受」，**不能**说明我们这边
/// 供过数据：如果 GTK 从未发出 `drag-data-get`（例如目标根本没来取），动作位有时仍是
/// COPY。2026-11-07 就踩过这个坑——界面报「已交付」，目标却什么都没收到。
/// 所以交付成功必须同时满足「供过数据」+「目标是 COPY」。
static SERVED: AtomicBool = AtomicBool::new(false);

/// 拖放结果事件载荷（发给前端的 `dnd-result`）
#[derive(Debug, Clone, Serialize)]
pub struct DndResult {
    /// 目标是否接受（动作位含 COPY **且** 本模块确实供过数据）
    pub delivered: bool,
    /// 本模块是否真的在 `drag-data-get` 里填过数据
    ///
    /// `delivered == false && served == false` 意味着「拖放链路没走通」（数据没被取走），
    /// 与「目标明确拒收」是两回事，前端提示语要区分开。
    pub served: bool,
}

/// 武装一次拖放（不解密）
///
/// # 参数
/// * `entry_id` - 密码条目 ID
/// * `decrypt_fn` - 解密闭包，真正取数据时才调用
///
/// # 返回
/// 本次的代数 seq（日志与前端提示用）
pub fn arm(entry_id: &str, decrypt_fn: DecryptFn) -> u64 {
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    SERVED.store(false, Ordering::SeqCst);
    let mut guard = ARMED.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_some() {
        log::info!("[DND] 上一次武装未结束即被覆盖（seq={}）", seq);
    }
    *guard = Some(Armed {
        entry_id: entry_id.to_string(),
        seq,
        deadline: Instant::now() + Duration::from_secs(TTL_SECS),
        decrypt_fn,
    });
    log::info!("[DND] 已武装: id={}, seq={}, ttl={}s", entry_id, seq, TTL_SECS);
    seq
}

/// 当前是否处于武装状态且未过期
///
/// GTK 的 `drag-begin` 用它判断「这次拖放要不要接管数据」——不是本应用的密码拖放
/// 就完全不碰 WebKit 的处理函数，把副作用限制到最小。
pub fn is_armed() -> bool {
    let guard = ARMED.lock().unwrap_or_else(|e| e.into_inner());
    matches!(guard.as_ref(), Some(a) if Instant::now() < a.deadline)
}

/// 现场解密，取回明文（供 GTK 的 `drag-data-get` 调用）
///
/// 一次拖放里目标可能连续索要多个 MIME（`text/plain`、`UTF8_STRING`、`STRING`），
/// 所以**每次调用都重新解密**，不在内存里缓存明文；调用方用完必须立即 `zeroize`。
///
/// # 返回
/// 明文；未武装、已过期或解密失败时为 `None`（fail closed，不降级到任何别的通道）
pub fn take_plaintext() -> Option<String> {
    let pair = {
        let mut guard = ARMED.lock().unwrap_or_else(|e| e.into_inner());
        // 过期即作废，避免「武装后用户放弃拖放、十分钟后又拖别的东西」拿到旧密码
        if matches!(guard.as_ref(), Some(a) if Instant::now() >= a.deadline) {
            log::info!("[DND] 武装已过期，作废");
            *guard = None;
        }
        guard
            .as_ref()
            .map(|a| (a.entry_id.clone(), a.decrypt_fn.clone()))
    }?;

    let (entry_id, decrypt_fn) = pair;
    match decrypt_fn(&entry_id) {
        Ok(plaintext) => {
            log::info!("[DND] 解密成功，长度={}", plaintext.len());
            Some(plaintext)
        }
        Err(e) => {
            // 锁定后迟到的调用会走到这里：fail closed，不交付任何东西
            log::warn!("[DND] 解密失败，拒绝交付: {e}");
            None
        }
    }
}

/// 重置「本次拖放是否供过数据」（每次 drag-begin 调用）
pub fn reset_served() {
    SERVED.store(false, Ordering::SeqCst);
}

/// 交付成功后登记一次使用（`last_used_at` / `use_count`）
///
/// 主窗口行拖拽（GTK 信号）与池窗口行拖拽（GTK 原生拖源）都走这里，
/// 保证两条路径的「使用记录」语义一致。
pub fn record_usage(app: &tauri::AppHandle, entry_id: &str) {
    use tauri::Manager;
    let state = app.state::<crate::state::AppState>();
    let Ok(db) = state.get_db() else {
        return;
    };
    let conn = db.conn();
    let now = chrono::Utc::now().timestamp_millis();
    if let Err(e) = conn.execute(
        "UPDATE passwords SET last_used_at = ?1, use_count = use_count + 1 WHERE id = ?2",
        rusqlite::params![now, entry_id],
    ) {
        log::warn!("[DND] 登记使用记录失败: id={entry_id}, err={e}");
    }
}

/// 记录「本次拖放确实供过数据」
pub fn mark_served() {
    SERVED.store(true, Ordering::SeqCst);
}

/// 本次拖放是否供过数据
pub fn was_served() -> bool {
    SERVED.load(Ordering::SeqCst)
}

/// 作废当前武装
///
/// # 参数
/// * `reason` - 作废原因（写日志用：drag-end / drag-failed / 前端取消）
///
/// # 返回
/// 被作废的条目 ID（没有在途拖放时为 `None`）。
/// 调用方可据此判断「要不要发结果事件」以及「交付成功时该给谁记一次使用」。
pub fn revoke(reason: &str) -> Option<String> {
    let mut guard = ARMED.lock().unwrap_or_else(|e| e.into_inner());
    match guard.take() {
        Some(a) => {
            log::info!("[DND] 已作废: seq={}, reason={}", a.seq, reason);
            Some(a.entry_id)
        }
        None => None,
    }
}

/// 只读当前武装的条目 ID，**不清除**
///
/// 与 [`revoke`] 的区别：拖放结束后**不能**清空武装。清空会让「连续拖同一条」变成
/// 每次都得等 `pointerdown` 重新武装，而那是异步 IPC，手快时来不及 ⇒ 表现为
/// 「有时能拖、有时拖不动」。武装继续靠 TTL 自然过期，锁屏/退出仍是立即清空。
pub fn armed_id() -> Option<String> {
    let guard = ARMED.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(a) if Instant::now() < a.deadline => Some(a.entry_id.clone()),
        _ => None,
    }
}

/// 当前武装的条目 ID（未武装或已过期时为 `None`）
///
/// 拖拽开始时要拿它去冻结池里对应的那条，所以这里也要做过期判定：
/// 一个已经过期的武装不该再冻住任何东西。
pub fn current_armed_id() -> Option<String> {
    let guard = ARMED.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(a) if Instant::now() < a.deadline => Some(a.entry_id.clone()),
        _ => None,
    }
}

// ==================== 拖拽池 ====================
//
// 池只存**元数据**：id / 标题 / 用户名 / 到期时刻 / 冻结标记。
// 不存明文，也不存解密闭包——`pointerdown` 时才现场调 `prepare_dnd_payload` 构造，
// 于是一个进程里密文与闭包的份数始终是 1（与单条武装时完全一致）。
//
// 容量与 TTL 是常量而非设置项：等池子的实际行为稳定下来再考虑暴露给用户，
// 现在就加设置页只会让「改一边莫名其妙影响另一边」的问题提前出现。

/// 池容量上限（超出时淘汰剩余时间最短的一条）
pub const POOL_CAPACITY: usize = 5;

/// 单条在池里的留存秒数
///
/// 比 `TTL_SECS`（武装有效期）长：武装从按下算起，池子从入池算起，
/// 而池存在的意义就是「放一会儿再拖」，30 秒是「够用又不至于长期挂在屏幕上」的折中。
pub const POOL_TTL_SECS: u64 = 30;

/// 池内一条待交付条目
///
/// 不实现 `Debug`：里面存了解密闭包（`Arc<dyn Fn>`），既不实现 `Debug` 也不该被打日志印出来。
#[derive(Clone)]
struct PoolEntry {
    /// 密码条目 ID（池里唯一标识，同一条重复入池时复用）
    id: String,
    /// 标题（池窗口展示用；主界面本来就明文可见，等级一致）
    title: String,
    /// 用户名（同上；`passwords.username` 是非空列）
    username: String,
    /// 入池代数，仅日志对照用
    seq: u64,
    /// 到期时刻（Rust 侧持有，前端只画剩余时间环）
    deadline: Instant,
    /// 拖拽进行中：冻结计时，绝不让数据源在拖拽途中断掉
    frozen: bool,
    /// 数据来源（密码走延迟解密，普通文本直接持有；见 [`PoolPayload`]）
    payload: PoolPayload,
}

/// 池条目的数据来源
///
/// 池里既可能放**密码**（只存密文 + 闭包，拖出去那一刻才解密），
/// 也可能放**普通文本**（用户名、自定义字段——这些本来就不是密文）。
///
/// 之所以让条目自己带来源（而不是靠全局单槽 ARMED）：GTK 的 `drag_data_get`
/// 是按行回调的，闭包/文本能直接捕获该行的 id 去取，不需要「先武装、再拖」的
/// 跨进程时序——那正是之前「有时能拖、有时拖不动」的根源。
#[derive(Clone)]
pub enum PoolPayload {
    /// 密码：延迟解密，闭包不含密钥副本（调用时才从 `AppState` 取）
    Secret(DecryptFn),
    /// 普通文本：本身就是明文，用 `Zeroizing` 包住，随条目一起清零
    Plain(Zeroizing<String>),
}

/// 全局池（最新入池的排在最前）
static POOL: Mutex<Vec<PoolEntry>> = Mutex::new(Vec::new());

/// 池条目的对外视图（浮窗用来渲染行，**不含任何明文**）
#[derive(Debug, Clone, Serialize)]
pub struct PoolView {
    /// 密码条目 ID
    pub id: String,
    /// 标题
    pub title: String,
    /// 用户名
    pub username: String,
    /// 剩余毫秒（前端据此画环并本地自减，避免时钟基准不一致）
    pub remaining_ms: u64,
    /// 单条总时长毫秒（环的分母；由后端给出，前端不重复定义 TTL）
    pub ttl_ms: u64,
    /// 是否正在被拖动（冻结中，不会到期）
    pub frozen: bool,
}

/// 把一个条目放进池（重复入池＝置顶 + 重置计时）
///
/// # 参数
/// * `id` / `title` / `username` - 条目元数据（调用方已查库拿到）
///
/// # 返回
/// 本次入池的代数 seq（日志对照用）
pub fn pool_put(id: &str, title: &str, username: &str, payload: PoolPayload) -> u64 {
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    let deadline = Instant::now() + Duration::from_secs(POOL_TTL_SECS);
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());

    // 同一条已在池里：置顶 + 重置计时 + 解冻，不重复插入
    if let Some(pos) = pool.iter().position(|e| e.id == id) {
        let mut entry = pool.remove(pos);
        entry.deadline = deadline;
        entry.frozen = false;
        entry.seq = seq;
        entry.title = title.to_string();
        entry.username = username.to_string();
        entry.payload = payload;
        pool.insert(0, entry);
        log::info!("[DND] 池：条目置顶 id={id}, 池深={}", pool.len());
        return seq;
    }

    // 新条目：先腾位置。优先淘汰「没在拖、剩余时间最短」的那条；
    // 全都冻结在拖（理论上不可能超过 1 条）时退化为淘汰末尾，绝不 panic。
    while pool.len() >= POOL_CAPACITY {
        let victim = pool
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.frozen)
            .min_by_key(|(_, e)| e.deadline)
            .map(|(i, e)| (i, e.id.clone()));
        match victim {
            Some((i, evicted)) => {
                pool.remove(i);
                log::info!("[DND] 池满，淘汰 id={evicted}");
            }
            None => {
                if let Some(evicted) = pool.pop() {
                    log::warn!("[DND] 池满且无可淘汰条目，强制移除 id={}", evicted.id);
                }
            }
        }
    }

    pool.insert(
        0,
        PoolEntry {
            id: id.to_string(),
            title: title.to_string(),
            username: username.to_string(),
            seq,
            deadline,
            frozen: false,
            payload,
        },
    );
    log::info!("[DND] 池：新条目 id={id}, 池深={}", pool.len());
    seq
}

/// 按条目 ID 现取明文（供 GTK 的 `drag_data_get` 调用）
///
/// # 返回
/// 明文；条目不存在或解密失败时为 `None`（fail closed，不降级到任何别的通道）。
/// **调用方用完必须立刻 zeroize。**
pub fn pool_text(id: &str) -> Option<Zeroizing<String>> {
    // 先把来源 clone 出来再释放锁：解密要查库/读密钥，不该占着池的锁
    let payload = {
        let pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        pool.iter().find(|e| e.id == id).map(|e| e.payload.clone())
    }?;

    match payload {
        PoolPayload::Secret(decrypt) => match decrypt(id) {
            Ok(plaintext) => {
                log::info!("[DND] 池条目解密成功 id={id}, 长度={}", plaintext.len());
                Some(Zeroizing::new(plaintext))
            }
            Err(e) => {
                log::warn!("[DND] 池条目解密失败 id={id}: {e}");
                None
            }
        },
        PoolPayload::Plain(text) => Some(text),
    }
}

/// 由内容生成稳定的条目 ID
///
/// 同一个用户名/字段重复复制时复用同一条，走「置顶 + 重置计时」而不是塞两条，
/// 与密码条目的行为一致。哈希只进 ID，不落日志。
pub fn text_pool_id(title: &str, subtitle: &str, text: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut hasher);
    subtitle.hash(&mut hasher);
    text.hash(&mut hasher);
    format!("text:{:x}", hasher.finish())
}

/// 池快照（按「最近入池在上」的顺序）
///
/// 顺手做过期清理：前端来拿列表时一定是最新的，不依赖维护线程的节拍。
pub fn pool_snapshot() -> Vec<PoolView> {
    pool_prune();
    let pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    pool.iter()
        .map(|e| PoolView {
            id: e.id.clone(),
            title: e.title.clone(),
            username: e.username.clone(),
            remaining_ms: e.deadline.saturating_duration_since(now).as_millis() as u64,
            ttl_ms: POOL_TTL_SECS * 1000,
            frozen: e.frozen,
        })
        .collect()
}

/// 池当前条数
pub fn pool_count() -> usize {
    let pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    pool.len()
}

/// 从池里移除一条
///
/// # 返回
/// 真的移除了为 `true`（调用方据此决定要不要通知窗口刷新）
pub fn pool_remove(id: &str) -> bool {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    let before = pool.len();
    pool.retain(|e| e.id != id);
    let removed = pool.len() != before;
    if removed {
        log::info!("[DND] 池：移除 id={id}, 池深={}", pool.len());
    }
    removed
}

/// 清空整池（锁屏 / 退出 / 手动清）
///
/// # 返回
/// 清空前池非空为 `true`
pub fn pool_clear() -> bool {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    let had = !pool.is_empty();
    pool.clear();
    if had {
        log::info!("[DND] 池：已清空");
    }
    had
}

/// 交付成功后释放一条（解冻，但**不重置倒计时**）
///
/// 交付成功**不等于**用完：同一条密码经常要连着填好几个地方，每拖一次就消失
/// 会逼用户反复回主窗口点复制。所以交付后保留条目，只有用户点 × 才移除。
///
/// 但计时**继续走**（用户明确要求）：池子里的东西不该因为被拖过一次就无限续命，
/// 到期该消失还是消失——留在屏幕上的每一条都是一份可见的账号信息。
///
/// # 返回
/// 条目存在（说明确实需要通知前端刷新）为 `true`
pub fn pool_release(id: &str) -> bool {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    match pool.iter_mut().find(|e| e.id == id) {
        Some(e) => {
            e.frozen = false;
            log::info!("[DND] 池：交付成功，条目保留（计时继续） id={id}");
            true
        }
        None => false,
    }
}

/// 冻结 / 解冻一条（拖拽期间冻结，绝不让它在拖动途中到期）
pub fn pool_set_frozen(id: &str, frozen: bool) {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(e) = pool.iter_mut().find(|e| e.id == id) {
        e.frozen = frozen;
        log::info!("[DND] 池：id={id} frozen={frozen}");
    }
}

/// 清理到期条目（冻结中的跳过）
///
/// # 返回
/// 有条目被移除为 `true`
pub fn pool_prune() -> bool {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let before = pool.len();
    pool.retain(|e| e.frozen || now < e.deadline);
    let removed = pool.len() != before;
    if removed {
        log::info!("[DND] 池：到期清理后池深={}", pool.len());
    }
    removed
}

/// 在窗口上安装拖放处理器（仅 Linux 生效，其他平台是空操作）
///
/// # 参数
/// * `window` - 目标窗口（主窗口与池窗口各装一次；池窗口本身就是拖源）
/// * `app` - 应用句柄，用于把交付结果事件推给前端、并驱动池窗口收尾
///
/// 这里不用泛型 `R`：拖放回调里要调 `pool_window` 的窗口操作（显隐/改尺寸），
/// 那些是具体到 Wry 的接口，泛型参数在那一层用不上，硬撑着只会多一层转换。
pub fn install_gtk_handlers(window: &tauri::WebviewWindow, app: tauri::AppHandle) {
    #[cfg(target_os = "linux")]
    gtk_impl::install(window, app);

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (window, app);
    }
}

#[cfg(target_os = "linux")]
mod gtk_impl {
    //! GTK / WebKitGTK 侧接线
    //!
    //! 拿到底层 `webkit2gtk::WebView`（它同时是一个 `gtk::Widget`），在它上面挂三个信号：
    //! `drag-begin`（摘掉 WebKit 的数据供给）、`drag-data-get`（填明文）、`drag-end`（收尾）。

    use gtk::prelude::*;
    use tauri::Emitter;

    use super::{is_armed, mark_served, take_plaintext, was_served, DndResult};

    /// 安装信号处理器
    pub fn install(window: &tauri::WebviewWindow, app: tauri::AppHandle) {
        let label = window.label().to_string();
        let app_in = app.clone();
        let label_in = label.clone();

        let result = window.with_webview(move |platform_webview| {
            // WebView 继承自 gtk::Widget，upcast 后就能用 WidgetExt 的信号 API
            let widget: gtk::Widget = platform_webview.inner().upcast();

            // GDK 后端决定拖放走哪套协议（wl_data_device 还是 XDND），排障时先看这一行：
            // 若显示 GdkX11Display，说明应用其实跑在 XWayland 上，与探针（显式 Wayland）不同。
            if let Some(display) = gtk::gdk::Display::default() {
                log::info!("[DND] 安装时的 GDK 后端: {}", display.type_().name());
            }

            // 数据供给（第一次拖放时安装，之后每次 drag-begin 会重装一次）
            connect_data_get(&widget);

            // drag-begin：只有本应用的密码拖放才接管数据
            let widget_begin = widget.clone();
            widget.connect_drag_begin(move |_w, ctx| {
                if !is_armed() {
                    return;
                }

                // 每次拖拽都是新的一次「是否供过数据」，必须在这里重置：
                // 武装现在会跨多次拖拽保留（见 armed_id 的说明），
                // 若沿用上一次的 SERVED=true，drag-end 会把失败误判成交付成功。
                super::reset_served();

                // 把池里这一条冻住：拖拽途中到期会让 `wl_data_source` 被撤销，
                // 目标那边表现为「拖了没反应」，所以计时必须在拖动期间停住
                if let Some(id) = super::current_armed_id() {
                    super::pool_set_frozen(&id, true);
                }

                // 把这次拖放实际对外声明的 MIME 全打出来。这一步是排障的关键：
                // 若 target 列表为空，目标根本无从「来取数据」，GTK 也就不会发
                // `drag-data-get`，我们下面所有填充逻辑都不会被调用。
                let targets: Vec<String> = ctx
                    .list_targets()
                    .into_iter()
                    .map(|a| a.name().to_string())
                    .collect();
                log::info!(
                    "[DND] drag-begin: actions={:?}, 对外声明 {} 个 target: {:?}",
                    ctx.actions(),
                    targets.len(),
                    targets
                );

                // WebKit 的异步回填会覆盖我们写的值，必须在目标来取数据之前摘掉它
                kill_webkit_data_handler(&widget_begin);
                // 摘除会连带干掉我们自己的处理器，重新接上
                connect_data_get(&widget_begin);

                // 再补一次：WebKit 有可能在它**自己的** drag-begin 处理函数里才挂上数据供给，
                // 而那个处理函数可能排在本次之后执行。这里等本次信号发完再摘一遍，
                // 把「谁先谁后」这个不确定性彻底消除。
                let widget_idle = widget_begin.clone();
                gtk::glib::idle_add_local_once(move || {
                    if !is_armed() {
                        return;
                    }
                    kill_webkit_data_handler(&widget_idle);
                    connect_data_get(&widget_idle);
                });
            });

            // drag-end：无论成功失败都要作废，并告诉前端结果
            let app_end = app_in.clone();
            widget.connect_drag_end(move |_w, ctx| {
                // 目标可能返回 COPY 也可能返回 MOVE，只要含 COPY 就算目标接受
                let accepted = ctx.selected_action().contains(gtk::gdk::DragAction::COPY);
                // **必须**再叠加「我们确实供过数据」：动作位只反映目标的意愿，
                // 数据链路根本没建立时它也可能给出 COPY（2026-11-07 实测）。
                let served = was_served();
                let delivered = accepted && served;
                log::info!(
                    "[DND] drag-end: selected_action={:?}, 目标接受={accepted}, 实际供数={served}, 判定交付={delivered}",
                    ctx.selected_action()
                );
                // 注意用 armed_id（只读）而不是 revoke：拖完必须保留武装，
                // 否则连续拖同一条时，第二次要等异步 IPC 重新武装，手快就失败
                if let Some(entry_id) = super::armed_id() {
                    if delivered {
                        super::record_usage(&app_end, &entry_id);
                        // 交付成功**不删除**：留着让用户反复拖，只有手动 × 才移除。
                        // 注意是 release 而不是续期：倒计时照常走，到期就消失
                        if super::pool_release(&entry_id) {
                            crate::pool_window::on_pool_changed(&app_end);
                        }
                    } else {
                        // 未交付（目标拒收/松手在空白处）：解冻继续计时，留在池里等重试
                        super::pool_set_frozen(&entry_id, false);
                    }
                    let _ = app_end.emit(
                        "dnd-result",
                        DndResult {
                            delivered,
                            served,
                        },
                    );
                }
            });

            // drag-failed：目标拒收或用户中途放弃
            let app_fail = app_in.clone();
            widget.connect_drag_failed(move |_w, _ctx, reason| {
                log::info!("[DND] drag-failed: {reason:?}");
                let served = was_served();
                if let Some(entry_id) = super::armed_id() {
                    // 拖拽失败也要解冻，否则这条会永远挂在池里不消失
                    super::pool_set_frozen(&entry_id, false);
                    let _ = app_fail.emit(
                        "dnd-result",
                        DndResult {
                            delivered: false,
                            served,
                        },
                    );
                }
                gtk::glib::Propagation::Proceed
            });

            log::info!("[DND] {label_in}: 拖放处理器已安装");
        });

        match result {
            Ok(()) => log::info!("[DND] {label}: 处理器安装请求已提交"),
            Err(e) => log::error!("[DND] {label}: with_webview 失败: {e}"),
        }
    }

    /// 挂上 `drag-data-get` 处理器
    ///
    /// 每次 `drag-begin` 摘除 WebKit 的处理函数后都要重挂一次——摘除是「按信号 ID 全摘」，
    /// 会把自己的也摘掉。
    fn connect_data_get(widget: &gtk::Widget) {
        widget.connect_drag_data_get(|_w, _ctx, data, info, _time| {
            let atom = data.target();
            let target_name = atom.name().to_string();
            // 入口无条件记一行：这是判断「GTK 到底有没有来取数据」的唯一依据
            log::info!(
                "[DND] drag-data-get 触发: info={info}, target={target_name}, 当前长度={}",
                data.length()
            );

            // WebKit 私有的富拖放格式：塞纯文本会破坏其结构，交给 WebKit 自己（它此时没数据，
            // 目标拿到的就是空值，不会泄露任何东西）
            if target_name.starts_with("org.webkitgtk") {
                return;
            }

            // 每次索要都现解密（一次拖放里目标可能连要 text/plain、UTF8_STRING、STRING）。
            // 用 `Zeroizing` 而不是手动 zeroize：提前 return、`set_text` 内 panic
            // 这些路径也会在离开作用域时清零。
            let Some(plaintext) = take_plaintext().map(zeroize::Zeroizing::new) else {
                log::info!("[DND] 无可用密码，数据留空: info={info}, target={target_name}");
                return;
            };

            let len = plaintext.len();
            let written = data.set_text(plaintext.as_str());
            // 立刻丢，不等作用域结束
            drop(plaintext);

            // 只记长度、不记内容
            log::info!("[DND] 已填充: info={info}, target={target_name}, len={len}, ok={written}");
            if written {
                // 只有真写进去才算「供过数据」，drag-end 据此判定交付是否成立
                mark_served();
            }
        });
    }

    /// 摘掉 WebKit 在 webview 上挂的 `drag-data-get` 处理函数
    ///
    /// 不这么做的话：WebKit 会先发 IPC 到 web 进程取数据，等它回来（信号早已返回）再写一次
    /// `GtkSelectionData`，把我们填的明文覆盖成 JS 那边 `setData` 的占位空串。
    ///
    /// 副作用：WebKit 的拖出数据链路就此失效，且**无法恢复**（拿不到它的回调指针）。
    /// 本项目没有任何「从界面把内容拖出去」的功能，所以代价可控；若将来要做，
    /// 必须重新设计（见 docs/wayland-拖放交付设计.md §6）。
    fn kill_webkit_data_handler(widget: &gtk::Widget) {
        let signal_id = unsafe {
            gtk::glib::gobject_ffi::g_signal_lookup(
                c"drag-data-get".as_ptr(),
                gtk::ffi::gtk_widget_get_type(),
            )
        };
        if signal_id == 0 {
            log::warn!("[DND] 找不到 drag-data-get 信号，无法摘除 WebKit 处理器");
            return;
        }

        let removed = unsafe {
            gtk::glib::gobject_ffi::g_signal_handlers_disconnect_matched(
                widget.as_ptr() as *mut gtk::glib::gobject_ffi::GObject,
                gtk::glib::gobject_ffi::G_SIGNAL_MATCH_ID,
                signal_id,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        log::info!("[DND] 已摘除 drag-data-get 处理函数 {removed} 个");
    }
}
