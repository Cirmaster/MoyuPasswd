//! 拖拽池窗口（Wayland 交付的载体）—— **纯 GTK 原生实现**
//!
//! # 为什么不用 webview
//!
//! 最初池窗口是一个 Tauri 窗口（WebKitGTK），拖拽走 HTML5 `dragstart`。实测下来这条路
//! 在 layer surface 上不可靠：**失败时连 GTK 的 `drag-begin` 都收不到**，说明 WebKit
//! 压根没把那次鼠标拖动识别成拖拽（它依赖窗口激活状态与命中缓存），而那一层我们无法干预。
//!
//! 所以改成纯 GTK：窗口、列表、拖拽全部由 GTK 处理。拖拽用
//! `gtk_drag_source_set` + `drag_data_get`——这套信号本项目早就在接管（见 `dnd.rs`），
//! 现在是直接用它，不再经过 WebKit。顺带把尺寸问题也解决了：GTK 布局的
//! `set_size_request` + `resize(1,1)` 正是 layer-shell 官方要求的两步，不再有
//! 「size request 记进去了但 surface 不变」的怪事。
//!
//! # 明文边界（与之前完全一致）
//!
//! 只在 `drag_data_get` 里解密，填完立即 `zeroize`。池里存的是**解密闭包**（含密文，
//! 不含明文、不含密钥副本），明文份数任何时刻 ≤ 1。
//!
//! # 位置
//!
//! layer surface 不归窗口管理器管，`begin_move_drag` 对它无效，位置只能靠 anchor + margin。
//! 而连续改 margin 会踩 gtk-layer-shell 的已知缺陷（surface 收不到 `.frame` 事件、
//! GTK 不 commit ⇒ 满屏闪），所以位置调整做成「整块跳角」：右下 → 左下 → 左上 → 右上。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use gtk::prelude::*;
use tauri::Emitter;

use crate::dnd;

// ==================== 版式常量（设计 px） ====================

/// 展开态宽度：只放「环 + 标题/用户名 + 秒数 + 两个小按钮」
const POOL_WIDTH: i32 = 180;

/// 单行高度
const ROW_HEIGHT: i32 = 36;

/// 列表上下内边距合计
const BODY_PADDING: i32 = 10;

/// 倒计时刷新间隔
const REFRESH_MS: u64 = 500;

/// 贴底时让开底部面板（实测 54）+ 8
const POOL_MARGIN_BOTTOM: i32 = 62;
/// 贴顶时让开顶部面板（实测 24）+ 8
const POOL_MARGIN_TOP: i32 = 32;
/// 左右留白
const POOL_MARGIN_SIDE: i32 = 8;

/// 拖拽结果事件（主窗口据此弹 toast）
const EVENT_RESULT: &str = "dnd-result";

// ==================== 状态 ====================

/// layer-shell 是否生效
static LAYER_MODE: AtomicBool = AtomicBool::new(false);

/// 当前停靠角：0=右下 1=左下 2=左上 3=右上
static CORNER: AtomicUsize = AtomicUsize::new(0);

/// 用户是否主动收起过（收起后新条目入池要重新亮出来）
static SUPPRESSED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// 池窗口 UI（GTK 对象只能在主线程持有，故放 thread_local）
    static UI: RefCell<Option<PoolUi>> = const { RefCell::new(None) };
}

/// 一行的可变部件（刷新倒计时只动这几个，不重建整列）
struct RowUi {
    id: String,
    seconds: gtk::Label,
    ring: gtk::DrawingArea,
    /// 剩余毫秒（draw 回调读它）
    remaining: Rc<Cell<f64>>,
}

/// 池窗口整体
struct PoolUi {
    window: gtk::Window,
    list: gtk::Box,
    rows: Vec<RowUi>,
}

// ==================== 公开 API ====================

/// layer-shell 是否生效
pub fn layer_active() -> bool {
    LAYER_MODE.load(Ordering::SeqCst)
}

/// 角的可读名字
pub fn corner_name(corner: usize) -> &'static str {
    match corner {
        0 => "右下",
        1 => "左下",
        2 => "左上",
        _ => "右上",
    }
}

/// 池状态变化后的统一入口（入池 / 移除 / 到期 / 清空）
pub fn on_pool_changed(app: &tauri::AppHandle) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || refresh_now(&app));
}

/// 有新条目入池：清掉「用户主动收起」标记并亮出来
pub fn reveal(app: &tauri::AppHandle) {
    SUPPRESSED.store(false, Ordering::SeqCst);
    on_pool_changed(app);
}

/// 用户主动收起
pub fn hide_manual(app: &tauri::AppHandle) {
    SUPPRESSED.store(true, Ordering::SeqCst);
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        UI.with(|u| {
            if let Some(ui) = u.borrow().as_ref() {
                ui.window.hide();
            }
        });
    });
}

/// 托盘菜单的显示/隐藏
pub fn toggle(app: &tauri::AppHandle) {
    if dnd::pool_count() == 0 {
        log::info!("[POOL] 池为空，托盘切换忽略");
        return;
    }
    let visible = UI.with(|u| {
        u.borrow()
            .as_ref()
            .map(|ui| ui.window.is_visible())
            .unwrap_or(false)
    });
    if visible {
        hide_manual(app);
    } else {
        reveal(app);
    }
}

/// 锁屏 / 退出：清池并收窗
pub fn shutdown(app: &tauri::AppHandle) {
    dnd::pool_clear();
    SUPPRESSED.store(false, Ordering::SeqCst);
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        UI.with(|u| {
            if let Some(ui) = u.borrow().as_ref() {
                ui.window.hide();
            }
        });
    });
}

/// 切换停靠角（右下 → 左下 → 左上 → 右上）
pub fn cycle_corner(app: &tauri::AppHandle) {
    if !layer_active() {
        return;
    }
    let next = (CORNER.load(Ordering::SeqCst) + 1) % 4;
    CORNER.store(next, Ordering::SeqCst);

    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        UI.with(|u| {
            let b = u.borrow();
            let Some(ui) = b.as_ref() else { return };
            apply_corner(&ui.window, next);
        });
        log::info!("[POOL] 停靠角切换为 {}", corner_name(next));
    });
}

// ==================== 主线程实现 ====================

/// 一次完整刷新：按池内容重建或更新行、调整尺寸、显隐
fn refresh_now(app: &tauri::AppHandle) {
    let entries = dnd::pool_snapshot();

    if entries.is_empty() {
        UI.with(|u| {
            if let Some(ui) = u.borrow().as_ref() {
                ui.window.hide();
            }
        });
        return;
    }

    ensure_ui(app);

    UI.with(|u| {
        let mut b = u.borrow_mut();
        let Some(ui) = b.as_mut() else { return };

        let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
        let same =
            ui.rows.len() == ids.len() && ui.rows.iter().zip(ids.iter()).all(|(r, id)| &r.id == id);

        if same {
            for (row, e) in ui.rows.iter().zip(entries.iter()) {
                row.remaining.set(e.remaining_ms as f64);
                row.seconds
                    .set_text(&format!("{}s", (e.remaining_ms + 999) / 1000));
                row.ring.queue_draw();
            }
        } else {
            rebuild_rows(app, ui, &entries);
        }

        resize_window(&ui.window, entries.len());

        if !SUPPRESSED.load(Ordering::SeqCst) {
            ui.window.show_all();
        }
    });
}

/// 建窗口（只建一次）
fn ensure_ui(app: &tauri::AppHandle) {
    if UI.with(|u| u.borrow().is_some()) {
        return;
    }

    install_css();

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_decorated(false);
    window.set_keep_above(true);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_resizable(false);
    window.set_app_paintable(true);

    // 透明背景（圆角卡片需要 RGBA visual）
    if let Some(screen) = gdk_screen() {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
    }

    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.set_widget_name("pool-card");

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.pack_start(&list, true, true, 0);
    window.add(&card);

    // 尺寸必须在 layer 初始化之前压进去：surface 的尺寸在 map 那一刻定型
    window.set_size_request(POOL_WIDTH, BODY_PADDING + ROW_HEIGHT);

    let layer = enable_layer_shell(&window);
    LAYER_MODE.store(layer, Ordering::SeqCst);
    if layer {
        apply_corner(&window, CORNER.load(Ordering::SeqCst));
        log::info!("[POOL] 已启用 layer-shell（overlay 层，置顶）");
    } else {
        log::warn!("[POOL] 未启用 layer-shell：普通窗口（不置顶）");
    }

    window.connect_destroy(|_| {
        UI.with(|u| *u.borrow_mut() = None);
    });

    UI.with(|u| {
        *u.borrow_mut() = Some(PoolUi {
            window: window.clone(),
            list,
            rows: Vec::new(),
        });
    });

    // 倒计时与到期清理：500ms 一次，跑在主线程（GTK 对象不能跨线程）
    let app_tick = app.clone();
    gtk::glib::timeout_add_local(Duration::from_millis(REFRESH_MS), move || {
        refresh_now(&app_tick);
        gtk::glib::ControlFlow::Continue
    });

    log::info!("[POOL] 池窗口已创建（GTK 原生）");
}

/// 重建所有行
fn rebuild_rows(app: &tauri::AppHandle, ui: &mut PoolUi, entries: &[dnd::PoolView]) {
    for child in ui.list.children() {
        ui.list.remove(&child);
    }
    ui.rows.clear();

    for (index, entry) in entries.iter().enumerate() {
        let (widget, row) = make_row(app, entry, index);
        ui.list.pack_start(&widget, false, false, 0);
        ui.rows.push(row);
    }
    ui.list.show_all();
}

/// 量出光标在指针右下方**实际**占用的尺寸（逻辑 px）
///
/// 为什么不用 `gtk-cursor-theme-size`：那只是标称尺寸，不同主题的箭头长宽比差别很大
/// （同样标 36，有的箭头宽 22、有的宽 30），用户还可能用自定义光标。
/// 所以这里直接读光标图像，扫 alpha 求非透明像素的右下边界——热点在左上角，
/// 这块边界就是它会盖住的区域。
///
/// 三级回退：alpha 边界 → 图像整体尺寸 → 24×24 的经验值。
fn cursor_footprint() -> (i32, i32) {
    // 拿不到图像时用 GTK 设置里的光标主题尺寸（KDE 会通过 XSettings 同步过来）
    let theme_size = || {
        gtk::Settings::default()
            .map(|s| s.gtk_cursor_theme_size())
            .filter(|v| *v >= 16)
            .unwrap_or(24)
    };

    // 默认箭头（GDK_LEFT_PTR）。拖动时的光标虽然是 dnd 主题光标，
    // 但尺寸与默认箭头一致，用它量就够
    let cursor = gtk::gdk::Display::default()
        .and_then(|d| gtk::gdk::Cursor::for_display(&d, gtk::gdk::CursorType::LeftPtr));

    let Some(pb) = cursor.and_then(|c| c.image()) else {
        log::info!("[POOL] 读不到光标图像，按主题尺寸估算");
        let s = theme_size();
        return (s, s);
    };

    let (w, h) = (pb.width(), pb.height());
    let n = pb.n_channels() as usize;
    let stride = pb.rowstride() as usize;
    let bytes = pb.read_pixel_bytes();
    let data: &[u8] = bytes.as_ref();

    let (mut max_x, mut max_y) = (0, 0);
    for y in 0..h {
        for x in 0..w {
            let idx = y as usize * stride + x as usize * n;
            if idx + n > data.len() {
                continue;
            }
            // 无 alpha 通道（RGB）就当全不透明
            let a = if n == 4 { data[idx + 3] } else { 255 };
            if a > 32 {
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    if max_x == 0 && max_y == 0 {
        // 图像全透明或读不出内容：退回整体尺寸
        return (to_logical(w), to_logical(h));
    }
    (to_logical(max_x + 1), to_logical(max_y + 1))
}

/// 物理像素 → 逻辑 px（HiDPI 下光标图像是 2x 加载的）
fn to_logical(v: i32) -> i32 {
    let r = render_scale();
    ((v as f64) / r).round().max(1.0) as i32
}

/// 幽灵相对指针的偏移（逻辑 px），按光标实际大小算
///
/// 横向必须完全越过箭头——那是用户最容易被看到的遮挡；纵向只要越过箭头斜边即可，
/// 所以只取高度的六成（卡片左上角落到斜边右侧就不会被压）。
/// 下限 20 是为了挡住"读不到光标"的异常情况（比如 Wayland 下 `image()` 返回空）。
fn ghost_offset() -> (i32, i32) {
    let (cw, ch) = cursor_footprint();
    let x = (cw + 2).clamp(20, 128);
    let y = ((ch as f64 * 0.62).round() as i32 + 2).clamp(20, 128);
    // 排障锚点：偏移看着不对时，先看这两个数是不是用户光标该有的尺寸
    log::info!("[POOL] 光标占用 {cw}x{ch}，幽灵偏移 {x}x{y}");
    (x, y)
}

/// 落点状态
#[derive(Clone, Copy, PartialEq, Eq)]
enum GhostState {
    /// 还没经过任何目标（刚起拖）
    Idle,
    /// 当前目标接受这次拖放
    Ok,
    /// 当前目标不接受
    No,
}

/// 拖拽幽灵（drag icon）
///
/// 为什么自己造：GTK3 的拖拽源默认什么都不显示，用户看不到自己在拖哪一条、
/// 也看不到落点到底能不能放。`gtk_drag_set_icon_widget` 允许塞任意 widget，
/// 于是「条目名 + 落点状态」可以一起画在指针旁边。
///
/// 避开光标用的是 margin 而不是负 hotspot：负的 hot_x/hot_y 在部分合成器会被
/// 夹到 0（图标又压回指针上）。给 root 加 margin 后，可见卡片从指针右下
/// [`ghost_offset`] 处开始，margin 区域在 icon surface 里是透明的。
struct Ghost {
    root: gtk::EventBox,
    card: gtk::EventBox,
    state: gtk::Label,
}

impl Clone for Ghost {
    // gtk 对象是引用计数的，clone 只是加引用（glib 的 ObjectExt::clone）
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            card: self.card.clone(),
            state: self.state.clone(),
        }
    }
}

impl Ghost {
    fn new(title: &str) -> Self {
        let root = gtk::EventBox::new();
        root.set_widget_name("ghost-root");
        // 偏移按光标实际大小算：用户的光标可能是 48px 的自定义主题，
        // 任何固定值都会被盖住一角（见 ghost_offset）
        let (gap_x, gap_y) = ghost_offset();
        root.set_margin_start(gap_x);
        root.set_margin_top(gap_y);

        let card = gtk::EventBox::new();
        card.set_widget_name("ghost-card");

        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        // GTK3 里 CSS 的 `padding` 对 GtkEventBox 不生效（字会紧贴边框），
        // 所以用内层 box 的 margin 来当 padding：卡片背景跟着内容一起放大，
        // 视觉上就是内边距。
        line.set_margin_start(9);
        line.set_margin_end(9);
        line.set_margin_top(5);
        line.set_margin_bottom(5);

        let label = gtk::Label::new(Some(&shorten(title, 14)));
        label.set_widget_name("ghost-title");
        line.pack_start(&label, false, false, 0);

        let state = gtk::Label::new(Some("拖到目标输入框"));
        state.set_widget_name("ghost-state");
        line.pack_start(&state, false, false, 0);

        card.add(&line);
        root.add(&card);

        Self { root, card, state }
    }

    /// 切换落点状态：只改文案与颜色，不动结构
    fn set_state(&self, state: GhostState) {
        let ctx = self.card.style_context();
        ctx.remove_class("ghost-ok");
        ctx.remove_class("ghost-no");

        let text = match state {
            GhostState::Idle => "拖到目标输入框",
            GhostState::Ok => {
                ctx.add_class("ghost-ok");
                "✓ 可放置"
            }
            GhostState::No => {
                ctx.add_class("ghost-no");
                "✕ 不接受拖放"
            }
        };
        self.state.set_text(text);
        // drag icon 在 Wayland 下是独立的 wl_surface，内容变了要主动重绘才会更新
        self.root.queue_draw();
    }
}

/// 截断过长标题：幽灵太宽会盖住目标输入框
fn shorten(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// 造一行：环 + 标题/用户名 + 秒数 + ⇱（仅首行）+ ×
fn make_row(app: &tauri::AppHandle, entry: &dnd::PoolView, index: usize) -> (gtk::EventBox, RowUi) {
    let ebox = gtk::EventBox::new();
    ebox.set_widget_name("pool-row");
    ebox.set_size_request(POOL_WIDTH - 12, ROW_HEIGHT);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);

    // 剩余时间环（自绘：GTK 没有现成的环形进度）
    let ring = gtk::DrawingArea::new();
    ring.set_size_request(18, 18);
    let remaining = Rc::new(Cell::new(entry.remaining_ms as f64));
    let ttl = if entry.ttl_ms > 0 {
        entry.ttl_ms as f64
    } else {
        1.0
    };
    {
        let rem = remaining.clone();
        ring.connect_draw(move |_, cr| {
            draw_ring(cr, rem.get(), ttl);
            gtk::glib::Propagation::Proceed
        });
    }
    row.pack_start(&ring, false, false, 0);

    // 标题 / 用户名
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let title = gtk::Label::new(Some(&entry.title));
    title.set_widget_name("pool-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(12);

    let user = gtk::Label::new(Some(&entry.username));
    user.set_widget_name("pool-user");
    user.set_xalign(0.0);
    user.set_ellipsize(gtk::pango::EllipsizeMode::End);
    user.set_max_width_chars(14);

    text.pack_start(&title, false, false, 0);
    text.pack_start(&user, false, false, 0);
    row.pack_start(&text, true, true, 0);

    // 秒数
    let seconds = gtk::Label::new(Some(&format!("{}s", (entry.remaining_ms + 999) / 1000)));
    seconds.set_widget_name("pool-sec");
    row.pack_start(&seconds, false, false, 0);

    // ⇱ 换停靠角：只放第一行，每行都放太吵
    if index == 0 && layer_active() {
        let corner_btn = gtk::Button::with_label("⇱");
        corner_btn.set_relief(gtk::ReliefStyle::None);
        corner_btn.set_tooltip_text(Some("切换停靠角（右下 → 左下 → 左上 → 右上）"));
        let app_c = app.clone();
        corner_btn.connect_clicked(move |_| cycle_corner(&app_c));
        row.pack_start(&corner_btn, false, false, 0);
    }

    // × 移除（这是唯一的移除入口：池空则窗口自动隐藏）
    let remove_btn = gtk::Button::with_label("×");
    remove_btn.set_relief(gtk::ReliefStyle::None);
    remove_btn.set_tooltip_text(Some("从池里移除"));
    {
        let id = entry.id.clone();
        let app_r = app.clone();
        remove_btn.connect_clicked(move |_| {
            if dnd::pool_remove(&id) {
                on_pool_changed(&app_r);
            }
        });
    }
    row.pack_start(&remove_btn, false, false, 0);

    ebox.add(&row);

    // 浮窗没有标题栏，光看一行文字无从知道"这条可以拖"。抓手光标 + 悬停提示
    // 是最轻的两个暗示：不占空间、不改变尺寸，鼠标一停就明白。
    ebox.set_tooltip_text(Some("按住这条拖到目标输入框"));
    {
        let eb = ebox.clone();
        ebox.connect_realize(move |_| {
            // EventBox 的 GdkWindow 要 realize 之后才存在，光标只能这时候设
            if let (Some(win), Some(display)) = (eb.window(), gtk::gdk::Display::default()) {
                if let Some(cursor) = gtk::gdk::Cursor::from_name(&display, "grab") {
                    win.set_cursor(Some(&cursor));
                }
            }
        });
    }

    // ===== 拖拽源：GTK 原生，完全不经过 WebKit =====
    let targets = [
        gtk::TargetEntry::new("text/plain;charset=utf-8", gtk::TargetFlags::empty(), 0),
        gtk::TargetEntry::new("UTF8_STRING", gtk::TargetFlags::empty(), 1),
        gtk::TargetEntry::new("text/plain", gtk::TargetFlags::empty(), 2),
        gtk::TargetEntry::new("STRING", gtk::TargetFlags::empty(), 3),
    ];
    ebox.drag_source_set(
        gtk::gdk::ModifierType::BUTTON1_MASK,
        &targets,
        gtk::gdk::DragAction::COPY,
    );

    // 本次拖动的幽灵：drag-begin 建、drag-end/drag-failed 收。
    // 必须自己收——GTK 文档明确「will not destroy the widget」，
    // 不收的话每拖一次就漏一个 widget（连同它的 CSS/布局）。
    let ghost_slot: Rc<RefCell<Option<Ghost>>> = Rc::new(RefCell::new(None));

    // drag-data-get：**唯一**产生明文的地方
    {
        let id = entry.id.clone();
        ebox.connect_drag_data_get(move |_, _, data, _info, _time| {
            // 每次索要都现解密：池里存的是密文 + 闭包，没有任何明文缓存。
            // 用 `Zeroizing` 而不是手动 zeroize —— 提前 return、`set_text` 内 panic
            // 这些路径也会在离开作用域时清零，不会漏。
            let Some(plaintext) = dnd::pool_text(&id) else {
                log::info!("[POOL] 无可用明文，数据留空 id={id}");
                return;
            };

            let len = plaintext.len();
            let ok = data.set_text(plaintext.as_str());
            // 立刻丢，不等作用域结束：这份明文在内存里只活到写进选择数据为止
            drop(plaintext);
            log::info!("[POOL] 已填充拖拽数据 id={id}, len={len}, ok={ok}");
        });
    }

    // drag-begin：冻结倒计时 + 装拖拽幽灵 + 监听落点是否可接受
    {
        let id = entry.id.clone();
        let ghost_title = entry.title.clone();
        let slot = ghost_slot.clone();
        ebox.connect_drag_begin(move |_, ctx| {
            dnd::pool_set_frozen(&id, true);
            log::info!("[POOL] drag-begin id={id}");

            // GTK3 的拖拽源默认**不画任何图标**，得自己塞一个 widget 给 context。
            // 它是独立的 offscreen window，CSS 继承 screen 级 provider（见 install_css），
            // 所以能复用同一套深色卡片样式。
            let ghost = Ghost::new(&ghost_title);
            ghost.root.show_all();
            ghost.set_state(GhostState::Idle);
            ctx.drag_set_icon_widget(&ghost.root, 0, 0);
            *slot.borrow_mut() = Some(ghost.clone());

            // 落点是否可接受：目标调用 `gtk_drag_status()` 时 GDK 会发 action-changed。
            // 这是**事件驱动**的，不用轮询；action 为空 = 当前目标不接受这次拖放。
            //
            // 注意它反映的是「目标声明的动作」，不是「目标真的粘进去了」——
            // 有些应用会在松手后丢掉数据，那种情况只能靠 drag-end 的 accepted 判定。
            let g = ghost.clone();
            // 去重：拖动过程中 action 会反复变化，同一状态不重复打日志
            let last = Cell::new(GhostState::Idle);
            ctx.connect_action_changed(move |_, action| {
                let ok = action.contains(gtk::gdk::DragAction::COPY);
                let next = if ok { GhostState::Ok } else { GhostState::No };
                if last.replace(next) != next {
                    log::info!(
                        "[POOL] 落点状态: {}",
                        if ok { "可放置" } else { "不接受拖放" }
                    );
                }
                g.set_state(next);
            });
        });
    }

    // drag-end：解冻；目标接受则记一次使用（条目**保留**，可反复拖）
    {
        let id = entry.id.clone();
        let app_end = app.clone();
        let slot = ghost_slot.clone();
        ebox.connect_drag_end(move |_, ctx| {
            dnd::pool_set_frozen(&id, false);
            // 幽灵由 GTK 保留但不销毁，得自己收（见 ghost_slot 的说明）。
            // 不用 `destroy()`：gtk-rs 0.18 把它标成 unsafe（要求调用方保证
            // 不在信号处理中销毁仍在使用的 widget）。安全等价做法是解绑父容器
            // 再丢掉最后引用 —— GObject 引用计数归零后自然回收。
            if let Some(g) = slot.borrow_mut().take() {
                if g.root.parent().is_some() {
                    g.root.unparent();
                }
            }
            let accepted = ctx.selected_action().contains(gtk::gdk::DragAction::COPY);
            log::info!("[POOL] drag-end id={id}, accepted={accepted}");
            if accepted {
                dnd::record_usage(&app_end, &id);
            }
            let _ = app_end.emit(
                EVENT_RESULT,
                serde_json::json!({ "delivered": accepted, "served": accepted }),
            );
            on_pool_changed(&app_end);
        });
    }

    // drag-failed：同样要解冻，否则这条会永远挂着不消失
    {
        let id = entry.id.clone();
        let app_fail = app.clone();
        let slot = ghost_slot.clone();
        ebox.connect_drag_failed(move |_, _, _| {
            dnd::pool_set_frozen(&id, false);
            if let Some(g) = slot.borrow_mut().take() {
                if g.root.parent().is_some() {
                    g.root.unparent();
                }
            }
            let _ = app_fail.emit(
                EVENT_RESULT,
                serde_json::json!({ "delivered": false, "served": false }),
            );
            gtk::glib::Propagation::Proceed
        });
    }

    if entry.frozen {
        ebox.set_opacity(0.6);
    }

    (
        ebox,
        RowUi {
            id: entry.id.clone(),
            seconds,
            ring,
            remaining,
        },
    )
}

/// 画剩余时间环
fn draw_ring(cr: &gtk::cairo::Context, remaining_ms: f64, ttl_ms: f64) {
    use std::f64::consts::{FRAC_PI_2, TAU};

    let ratio = (remaining_ms / ttl_ms).clamp(0.0, 1.0);
    let (cx, cy, r) = (9.0, 9.0, 7.0);

    cr.set_line_width(2.4);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.16);
    cr.arc(cx, cy, r, 0.0, TAU);
    let _ = cr.stroke();

    // 剩余不足 10 秒转成警示红
    if remaining_ms <= 10_000.0 {
        cr.set_source_rgb(0.96, 0.42, 0.42);
    } else {
        cr.set_source_rgb(0.42, 0.78, 1.0);
    }
    cr.arc(cx, cy, r, -FRAC_PI_2, -FRAC_PI_2 + TAU * ratio);
    let _ = cr.stroke();
}

/// 调整窗口尺寸
///
/// layer-shell 官方 "Forcing Window Size" 要求两步：`set_size_request` + `resize(1,1)`。
/// 少了第二步，size request 记进去了但 surface 尺寸不会变。
fn resize_window(window: &gtk::Window, rows: usize) {
    let n = rows.max(1) as i32;
    let h = BODY_PADDING + ROW_HEIGHT * n;
    window.set_size_request(POOL_WIDTH, h);
    if LAYER_MODE.load(Ordering::SeqCst) {
        window.resize(1, 1);
    }
}

/// 把窗口贴到指定角（先清空四条边，避免两个对边同锚被拉伸成跨屏）
fn apply_corner(window: &gtk::Window, corner: usize) {
    let Some(api) = layer_shell_api() else {
        return;
    };
    let r = render_scale();
    let side = (POOL_MARGIN_SIDE as f64 * r).round() as i32;
    let v_bottom = (POOL_MARGIN_BOTTOM as f64 * r).round() as i32;
    let v_top = (POOL_MARGIN_TOP as f64 * r).round() as i32;

    // SAFETY: 主线程调用，且窗口已初始化成 layer surface
    unsafe {
        let w = window.as_ptr() as *mut std::ffi::c_void;
        for edge in [EDGE_LEFT, EDGE_RIGHT, EDGE_TOP, EDGE_BOTTOM] {
            (api.set_anchor)(w, edge, 0);
            (api.set_margin)(w, edge, 0);
        }
        let (h_edge, h_margin, v_edge, v_margin) = match corner {
            1 => (EDGE_LEFT, side, EDGE_BOTTOM, v_bottom),
            2 => (EDGE_LEFT, side, EDGE_TOP, v_top),
            3 => (EDGE_RIGHT, side, EDGE_TOP, v_top),
            _ => (EDGE_RIGHT, side, EDGE_BOTTOM, v_bottom),
        };
        (api.set_anchor)(w, h_edge, 1);
        (api.set_margin)(w, h_edge, h_margin);
        (api.set_anchor)(w, v_edge, 1);
        (api.set_margin)(w, v_edge, v_margin);
    }
}

/// 把窗口放到指定的左上角（物理像素），尺寸交给 size request
///
/// 只锚左边和上边：这样窗口不会被拉伸，位置完全由 margin 决定。
/// 与 [`apply_corner`] 的区别是它贴角，这个可以放到任意坐标。
fn apply_position(window: &gtk::Window, left: i32, top: i32) {
    let Some(api) = layer_shell_api() else {
        return;
    };

    // SAFETY: 主线程调用，且窗口已初始化成 layer surface
    unsafe {
        let w = window.as_ptr() as *mut std::ffi::c_void;
        for edge in [EDGE_LEFT, EDGE_RIGHT, EDGE_TOP, EDGE_BOTTOM] {
            (api.set_anchor)(w, edge, 0);
            (api.set_margin)(w, edge, 0);
        }
        (api.set_anchor)(w, EDGE_LEFT, 1);
        (api.set_margin)(w, EDGE_LEFT, left);
        (api.set_anchor)(w, EDGE_TOP, 1);
        (api.set_margin)(w, EDGE_TOP, top);
    }
}

/// 当前缩放（设计 px → 物理 px；gtk-layer-shell 的 margin 是物理像素，不做换算）
fn render_scale() -> f64 {
    gdk_screen()
        .map(|s| s.resolution() as f64 / 96.0)
        .filter(|v| *v > 0.5 && *v < 4.0)
        .unwrap_or(1.0)
}

fn gdk_screen() -> Option<gtk::gdk::Screen> {
    gtk::gdk::Display::default().map(|d| d.default_screen())
}

/// 注入样式（深色圆角卡片，观感与之前一致）
fn install_css() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let Some(screen) = gdk_screen() else { return };
        let provider = gtk::CssProvider::new();
        let _ = provider.load_from_data(CSS.as_bytes());
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}

const CSS: &str = r#"
window { background-color: transparent; }
#pool-card {
  background-color: rgba(24, 24, 28, 0.96);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 10px;
  padding: 5px 6px;
}
#pool-row { border-radius: 6px; }
#pool-row:hover { background-color: rgba(255, 255, 255, 0.07); }
#pool-title { color: #f2f2f5; font-size: 12px; font-weight: 600; }
#pool-user { color: #9aa0b4; font-size: 10px; }
#pool-sec { color: #9aa0b4; font-size: 10px; }
button {
  padding: 0;
  min-width: 16px;
  min-height: 16px;
  background: transparent;
  border: none;
  box-shadow: none;
  color: #9aa0b4;
  font-size: 12px;
}
button:hover { color: #f2f2f5; }

/* ===== 拖拽幽灵 ===== */
/* 外层必须有窗口才画得出 margin 的透明区（见 Ghost 的说明），
   不显式声明会被主题填成不透明底色，变成一块挡光标的方块 */
#ghost-root { background-color: transparent; }
#ghost-card {
  background-color: rgba(24, 24, 28, 0.97);
  border: 1px solid rgba(255, 255, 255, 0.16);
  border-radius: 8px;
  /* 内边距不写在这里：GTK3 的 CSS padding 对 GtkEventBox 不生效，
     改由内层 box 的 margin 承担（见 Ghost::new） */
}
#ghost-title { color: #f2f2f5; font-size: 12px; font-weight: 600; }
#ghost-state { color: #9aa0b4; font-size: 10px; }
#ghost-card.ghost-ok { border-color: rgba(90, 220, 140, 0.9); }
#ghost-card.ghost-ok #ghost-state { color: #5adc8c; font-weight: 600; }
#ghost-card.ghost-no { border-color: rgba(240, 110, 110, 0.85); }
#ghost-card.ghost-no #ghost-state { color: #f06e6e; font-weight: 600; }
"#;

// ==================== layer-shell（dlopen 软依赖） ====================

const EDGE_LEFT: i32 = 0;
const EDGE_RIGHT: i32 = 1;
const EDGE_TOP: i32 = 2;
const EDGE_BOTTOM: i32 = 3;
const LAYER_OVERLAY: i32 = 3;
/// GtkLayerShellKeyboardMode::ON_DEMAND
const KEYBOARD_ON_DEMAND: i32 = 2;

type LsApi = &'static LsFns;

struct LsFns {
    is_supported: unsafe extern "C" fn() -> i32,
    init_for_window: unsafe extern "C" fn(*mut std::ffi::c_void),
    set_layer: unsafe extern "C" fn(*mut std::ffi::c_void, i32),
    set_anchor: unsafe extern "C" fn(*mut std::ffi::c_void, i32, i32),
    set_margin: unsafe extern "C" fn(*mut std::ffi::c_void, i32, i32),
    set_exclusive_zone: unsafe extern "C" fn(*mut std::ffi::c_void, i32),
    set_keyboard_mode: Option<unsafe extern "C" fn(*mut std::ffi::c_void, i32)>,
    set_keyboard_interactivity: Option<unsafe extern "C" fn(*mut std::ffi::c_void, i32)>,
    set_namespace: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_char)>,
}

static LS: std::sync::OnceLock<Option<LsFns>> = std::sync::OnceLock::new();

fn layer_shell_api() -> Option<LsApi> {
    LS.get_or_init(load_layer_shell).as_ref()
}

/// 启用 layer-shell：overlay 层、不占可用区、点击时才给键盘焦点
fn enable_layer_shell(window: &gtk::Window) -> bool {
    let Some(api) = layer_shell_api() else {
        return false;
    };

    // SAFETY: 主线程、realize 之前调用；符号已解析
    unsafe {
        if (api.is_supported)() == 0 {
            return false;
        }
        let w = window.as_ptr() as *mut std::ffi::c_void;
        (api.init_for_window)(w);
        (api.set_layer)(w, LAYER_OVERLAY);
        (api.set_exclusive_zone)(w, 0);

        // ON_DEMAND：点击时才给键盘焦点，平时不抢。
        // NONE 会让窗口永远处于非激活态，输入处理会变得不可靠。
        if let Some(f) = api.set_keyboard_mode {
            f(w, KEYBOARD_ON_DEMAND);
        } else if let Some(f) = api.set_keyboard_interactivity {
            f(w, 1);
        }
        if let Some(f) = api.set_namespace {
            if let Ok(ns) = std::ffi::CString::new("moyu-passwd-pool") {
                f(w, ns.as_ptr());
            }
        }
    }
    true
}

fn load_layer_shell() -> Option<LsFns> {
    use std::ffi::{c_char, c_int, c_void, CString};

    extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    const RTLD_NOW: c_int = 2;

    /// 取符号并转成函数指针（`transmute_copy` 避免泛型推断不出大小）
    unsafe fn sym<T>(handle: *mut c_void, name: &str) -> Option<T> {
        let c = CString::new(name).ok()?;
        let p = dlsym(handle, c.as_ptr());
        if p.is_null() {
            None
        } else {
            Some(std::mem::transmute_copy(&p))
        }
    }
    unsafe fn need<T>(handle: *mut c_void, name: &str) -> Option<T> {
        match sym(handle, name) {
            Some(f) => Some(f),
            None => {
                log::warn!("[POOL] libgtk-layer-shell 缺符号 {name}，放弃置顶");
                None
            }
        }
    }

    // SAFETY: 进程内只调一次；handle 故意不关闭，函数指针要活到进程结束
    unsafe {
        let name = CString::new("libgtk-layer-shell.so.0").ok()?;
        let handle = dlopen(name.as_ptr(), RTLD_NOW);
        if handle.is_null() {
            log::info!("[POOL] 未安装 libgtk-layer-shell，退化为普通窗口（不置顶）");
            return None;
        }
        Some(LsFns {
            is_supported: need(handle, "gtk_layer_is_supported")?,
            init_for_window: need(handle, "gtk_layer_init_for_window")?,
            set_layer: need(handle, "gtk_layer_set_layer")?,
            set_anchor: need(handle, "gtk_layer_set_anchor")?,
            set_margin: need(handle, "gtk_layer_set_margin")?,
            set_exclusive_zone: need(handle, "gtk_layer_set_exclusive_zone")?,
            set_keyboard_mode: sym(handle, "gtk_layer_set_keyboard_mode"),
            set_keyboard_interactivity: sym(handle, "gtk_layer_set_keyboard_interactivity"),
            set_namespace: sym(handle, "gtk_layer_set_namespace"),
        })
    }
}

// ==================== 入池过渡动画（卡片从浮窗外侧滑入） ====================
//
// # 为什么不是"移动窗口"
//
// Wayland 下窗口位置不可设置（探针 P-C 实测：`gtk_window_move(200,140)` 之后
// 读回的位置是 `(0,0)`），layer surface 也不能连续改 margin——改完 surface 收不到
// `.frame` 事件、GTK 不 commit，实测在 KWin 下就是**满屏乱闪**。
//
// 所以这里不移动任何窗口，而是开一块**铺满整屏、位置固定 (0,0)** 的画布
// （overlay 层、透明、输入穿透），在画布内部用 cairo 把一张小卡片从浮窗外侧
// 画进浮窗。画布自身位置和尺寸全程不变 ⇒ 不触发 margin 修改 ⇒ 不会闪。
//
// # 为什么起点是浮窗外侧而不是光标位置
//
// Wayland 下客户端拿不到光标的屏幕坐标：GDK 的 `gdk_device_get_position` 在
// Wayland 后端返回的是相对「指针焦点窗口」的坐标（见 gdk/wayland/gdkdevice-wayland.c
// 的 get_coordinates()），实测是常量。起点终点全部由浮窗位置推导，才能保证准确。

/// 动画代数：连点同一个密码时，旧回调看到代数变了就自己退出，
/// 这样新动画可以立刻接上（不用等旧的播完），也不会两块画布打架。
static FLY_GEN: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// 当前正在播飞入动画的画布（新动画启动时先把旧的收掉）
    static FLY_CANVAS: RefCell<Option<gtk::Window>> = RefCell::new(None);
}

/// 动画总时长（毫秒）
///
/// 太短会像"闪一下"，240 帧率下 300ms 约 18 帧，能看清轨迹又不拖沓。
const FLY_MS: f64 = 300.0;

/// 屏幕逻辑尺寸 + 来源说明
///
/// **逻辑像素**：`gdk_monitor_get_geometry` 返回的就是应用像素。
///
/// 不能只信 `primary_monitor()`——KWin 下它可能是 NULL，一旦回退到硬编码的
/// 1920×1080，浮窗在更大屏幕上就会算偏，动画方向整个出错。
fn screen_size() -> (i32, i32, &'static str) {
    let Some(display) = gtk::gdk::Display::default() else {
        return (1920, 1080, "fallback");
    };

    if let Some(m) = display.primary_monitor() {
        let g = m.geometry();
        if g.width() > 0 && g.height() > 0 {
            return (g.width(), g.height(), "primary");
        }
    }

    // 没有 primary 就取所有显示器的包围盒
    let n = display.n_monitors();
    if n > 0 {
        let (mut w, mut h) = (0, 0);
        for i in 0..n {
            if let Some(m) = display.monitor(i) {
                let g = m.geometry();
                w = w.max(g.x() + g.width());
                h = h.max(g.y() + g.height());
            }
        }
        if w > 0 && h > 0 {
            return (w, h, "monitors");
        }
    }

    (1920, 1080, "fallback")
}

/// 浮窗当前停靠位置的左上角与尺寸（物理像素）
///
/// 高度按**实际条数**算：浮窗是贴底锚定的，条数越多窗口越高、左上角越靠上，
/// 终点位置算错的话卡片会飞歪。
fn pool_rect() -> ((i32, i32), (i32, i32)) {
    let (sw, sh, _) = screen_size();
    let rows = dnd::pool_count().max(1) as i32;
    // +2 是 `#pool-card` 左右/上下各 1px 的边框：`set_size_request` 只是**最小**尺寸，
    // 内容（168 宽的行 + 12 padding + 2 border = 182）更大，窗口会撑到 182。
    // 少了这 2px，卡片落点就比浮窗第一条偏右下，落地时会"往回顿一下"。
    let w = POOL_WIDTH + 2;
    let h = BODY_PADDING + ROW_HEIGHT * rows + 2;
    let side = POOL_MARGIN_SIDE;
    let bottom = POOL_MARGIN_BOTTOM;
    let top = POOL_MARGIN_TOP;

    let (x, y) = match CORNER.load(Ordering::SeqCst) {
        1 => (side, sh - bottom - h),
        2 => (side, top),
        3 => (sw - side - w, top),
        _ => (sw - side - w, sh - bottom - h),
    };
    ((x, y), (w, h))
}

/// 显示浮窗，并尽量先播一段「卡片从浮窗外侧滑入」的动画
///
/// 失败（没有 layer-shell、建窗出错）时**直接显示**浮窗，
/// 不会让用户看不到东西。调用方只需调这一个函数，不要再自己调 [`reveal`]。
pub fn reveal_with_fly_in(app: &tauri::AppHandle) {
    let app_main = app.clone();
    let handed = app.run_on_main_thread(move || {
        if !start_fly_in(&app_main) {
            reveal(&app_main);
        }
    });

    // 投递失败（主线程已退出等）：至少把浮窗亮出来
    if handed.is_err() {
        reveal(app);
    }
}

/// 在主线程上真正启动动画；返回是否启动成功
fn start_fly_in(app: &tauri::AppHandle) -> bool {
    // 没有 layer-shell 就没有可靠的定位手段，直接显示浮窗
    if !layer_active() {
        return false;
    }
    let (sw, sh, size_source) = screen_size();

    // 坐标系约定：**全程逻辑像素**。画布铺满整屏、固定贴在 (0,0)，
    // 所以卡片直接按屏幕逻辑坐标绘制，中途**没有任何缩放换算**——
    // 之前反复对不准光标，根源就是拿不可靠的 r 去换 margin。
    let card_w = POOL_WIDTH - 12;
    let card_h = ROW_HEIGHT;
    if card_w <= 0 || card_h <= 0 {
        return false;
    }

    // 终点：浮窗里第一条的位置（卡片内边距 6/5 + 边框 1）
    let ((pool_x, pool_y), _) = pool_rect();
    let end = (pool_x + 7, pool_y + 6);

    // 起点：浮窗**左上方外侧**一段距离，卡片从那里滑进浮窗。
    //
    // **为什么不用光标**：Wayland 下 GDK 的 `gdk_device_get_position` 返回的是相对
    // "指针焦点窗口"的坐标（`gdk/wayland/gdkdevice-wayland.c` 的 get_coordinates()
    // 直接取 wl_pointer 的 surface-local 坐标），客户端不知道自己的窗口在屏幕上
    // 的位置，所以它是个常量、不跟随鼠标。KDE 能用 KWin 脚本 + DBus 读到
    // `workspace.cursorPos`，但要每次加载 JS、结果还得从 journal 里捞，为 300ms
    // 的过渡不值得。
    //
    // 从浮窗外侧滑入则完全不需要坐标查询：起点终点都由浮窗位置推出来，绝对准确。
    let start = ((pool_x - 100).max(0), (pool_y - 48).max(0));

    // 画布 = **整块屏幕**，位置固定 (0,0)。
    //
    // 不按包围盒定位：那需要把逻辑坐标换算成 layer-shell 的 margin，而 margin 的单位
    // 在不同后端上不一致（且 r 在 Wayland 下拿不到真实缩放），换算一错画布就整体偏移。
    // 整屏 + (0,0) 之后**没有任何相对换算**，卡片直接按屏幕逻辑坐标绘制。
    let (bw, bh) = (sw.max(1), sh.max(1));

    let canvas = gtk::Window::new(gtk::WindowType::Toplevel);
    canvas.set_decorated(false);
    canvas.set_app_paintable(true);
    canvas.set_skip_taskbar_hint(true);
    canvas.set_skip_pager_hint(true);
    canvas.set_accept_focus(false);
    canvas.set_size_request(bw, bh);
    // 透明合成必须有 RGBA visual，否则画布会是一块实心黑
    if let Some(screen) = gdk_screen() {
        if let Some(visual) = screen.rgba_visual() {
            canvas.set_visual(Some(&visual));
        }
    }

    let area = gtk::DrawingArea::new();
    area.set_size_request(bw, bh);
    canvas.add(&area);

    // 必须在 show 之前初始化 layer-shell，否则它直接失效
    if !enable_layer_shell(&canvas) {
        return false;
    }
    // 画布贴屏幕原点：唯一的取值，且 0 在任何单位下都是 0，不需要换算
    apply_position(&canvas, 0, 0);

    // 画布要完全"透传"鼠标：否则动画那 300ms 里它会吃掉全屏点击。
    // 输入区域只能在 realize 之后设置，所以绑在 realize 上——必须赶在 show 之前连。
    {
        let c = canvas.clone();
        canvas.connect_realize(move |_| {
            if let Some(gw) = c.window() {
                let empty = gtk::cairo::Region::create();
                gw.input_shape_combine_region(&empty, 0, 0);
            }
        });
    }

    canvas.show_all();

    // 诊断锚点：画布到底有没有上屏。没有这行就说明 layer surface 没 map，
    // 动画再怎么写也不会被看见。
    canvas.connect_map(|_| log::info!("[POOL] 飞入画布已上屏"));

    // 双保险：realize 若在 show_all 内同步发生，上面的回调可能赶不上
    if let Some(gw) = canvas.window() {
        let empty = gtk::cairo::Region::create();
        gw.input_shape_combine_region(&empty, 0, 0);
    }

    let progress = Rc::new(Cell::new(0.0f64));
    {
        let p = progress.clone();
        let (sx, sy) = start;
        let (ex, ey) = end;
        area.connect_draw(move |_, cr| {
            // p 存的是**位置进度**（0..1），不是时间。
            // 画布就是整屏，所以这里可以直接用屏幕坐标画，不做任何偏移。
            let pos = p.get();
            let cx = sx as f64 + (ex - sx) as f64 * pos;
            let cy = sy as f64 + (ey - sy) as f64 * pos;

            // 离浮窗还有一大截就开始收：位置到 60% 起淡出、92% 完全消失。
            // 不能飞到落点再切——那样像"撞上去"，而且落点的几像素误差会露出来。
            // 淡出跨 32% 的行程（约 100ms），不会显得突然没掉。
            let alpha = if pos > 0.60 {
                ((0.92 - pos) / 0.32).clamp(0.0, 1.0)
            } else {
                1.0
            };
            draw_fly_card(cr, cx, cy, card_w as f64, card_h as f64, alpha);
            gtk::glib::Propagation::Proceed
        });
    }

    // 连点同一个密码：立刻收掉上一块画布，让新动画从头播，而不是被丢掉
    FLY_CANVAS.with(|c| {
        if let Some(old) = c.borrow_mut().take() {
            old.close();
        }
    });
    FLY_CANVAS.with(|c| *c.borrow_mut() = Some(canvas.clone()));

    let generation = FLY_GEN.fetch_add(1, Ordering::SeqCst) + 1;

    // 定时器驱动（帧时钟在这块 surface 上不触发，见 spawn_fly_tick 的说明）
    spawn_fly_tick(&area, &canvas, app, generation, progress.clone());

    // 兜底：万一窗口没能 map、定时器收不了尾，到点强制关画布。
    // 这块画布覆盖大半屏幕，绝不能留在桌面上。
    {
        let c = canvas.clone();
        let a = app.clone();
        gtk::glib::timeout_add_local_once(
            Duration::from_millis(FLY_MS as u64 + 1200),
            move || {
                if FLY_GEN.load(Ordering::SeqCst) != generation {
                    return;
                }
                log::warn!("[POOL] 飞入动画超时未收尾，强制关闭画布");
                SUPPRESSED.store(false, Ordering::SeqCst);
                refresh_now(&a);
                c.close();
            },
        );
    }

    // 自检：终点跑到起点左上方说明坐标算错了，日志里留个记号
    if end.0 < start.0 || end.1 < start.1 {
        log::warn!(
            "[POOL] 飞入方向异常：起点={:?} 终点={:?}（CORNER={}）",
            start,
            end,
            CORNER.load(Ordering::SeqCst)
        );
    }

    log::info!(
        "[POOL] 飞入: 屏幕={sw}x{sh}({size_source}), 起点=({},{}), 终点=({},{}), 画布=整屏 {}x{}",
        start.0,
        start.1,
        end.0,
        end.1,
        bw,
        bh
    );
    true
}

/// 挂上定时器回调推进动画
///
/// 试过 `add_tick_callback`（帧时钟），它在这块 layer surface 上一次都不触发
/// （日志里只有超时兜底生效），动画完全不推进。改回 `timeout(8ms)`：
/// 不与 vsync 对齐，但**一定会跑**，可靠性优先。
fn spawn_fly_tick(
    area: &gtk::DrawingArea,
    canvas: &gtk::Window,
    app: &tauri::AppHandle,
    generation: u64,
    progress: Rc<Cell<f64>>,
) {
    let app_done = app.clone();
    let canvas_done = canvas.clone();
    let area_tick = area.clone();
    let canvas_map = canvas.clone();
    // 起点时刻（None = 还没开始计时：窗口尚未 map）
    let started: Rc<Cell<Option<std::time::Instant>>> = Rc::new(Cell::new(None));

    // 8ms 采样 + GTK 的绘制合并，观感与帧时钟差别很小（理由见函数文档）
    gtk::glib::timeout_add_local(Duration::from_millis(8), move || {
        // 已被新动画取代：安静退出，别去关新画布
        if FLY_GEN.load(Ordering::SeqCst) != generation {
            return gtk::glib::ControlFlow::Break;
        }

        // 窗口还没 map 就不开始计时，否则第一帧就跳过一段（起步一顿）
        if let Some(start) = started.get() {
            let t = start.elapsed().as_secs_f64() / (FLY_MS / 1000.0);
            let pos = t.clamp(0.0, 1.0);

            // 走到 92% 就收尾：此时卡片已经淡到看不见，浮窗紧接着出现在落点
            if pos >= 0.92 {
                let a = app_done.clone();
                gtk::glib::idle_add_local_once(move || {
                    SUPPRESSED.store(false, Ordering::SeqCst);
                    refresh_now(&a);
                });
                canvas_done.close();
                return gtk::glib::ControlFlow::Break;
            }

            progress.set(pos);
            area_tick.queue_draw();
        } else if canvas_map.is_mapped() {
            started.set(Some(std::time::Instant::now()));
        }

        gtk::glib::ControlFlow::Continue
    });
}

/// 画飞行中的小卡片
///
/// 只画形状、不画文字：cairo 的 toy 文本 API 对中文不可靠（没有 pango 就选不到
/// 中文字体），而这里只是一闪而过的过渡，形状足够表达"这一条飞进去了"。
fn draw_fly_card(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, alpha: f64) {
    use std::f64::consts::TAU;

    rounded_rect(cr, x, y, w, h, 8.0);
    cr.set_source_rgba(0.094, 0.094, 0.11, 0.97 * alpha);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.16 * alpha);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    // 左侧倒计时环（占位）
    let cx = x + 14.0;
    let cy = y + h / 2.0;
    cr.set_source_rgba(0.42, 0.78, 1.0, 0.9 * alpha);
    cr.set_line_width(2.2);
    cr.arc(cx, cy, 6.0, 0.0, TAU);
    let _ = cr.stroke();

    // 两条内容占位（标题 / 用户名）
    cr.set_source_rgba(0.95, 0.95, 0.96, 0.55 * alpha);
    rounded_rect(cr, x + 26.0, cy - 7.0, (w * 0.34).max(8.0), 4.0, 2.0);
    let _ = cr.fill();
    cr.set_source_rgba(0.60, 0.63, 0.70, 0.40 * alpha);
    rounded_rect(cr, x + 26.0, cy + 3.0, (w * 0.24).max(6.0), 3.0, 1.5);
    let _ = cr.fill();
}

/// 圆角矩形路径（不填充，调用方决定 fill / stroke）
fn rounded_rect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};

    let r = radius.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}
