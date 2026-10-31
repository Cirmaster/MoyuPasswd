//! X11 平台工具模块（仅 Linux）
//!
//! 注入链路 X11 实现的共享助手：会话类型判定、EWMH 属性查询（进程归因）、
//! 键盘映射查询（字符 → keycode）。Ctrl+V 抓取与文本注入分别在
//! hotkey.rs / inject.rs 的 `x11_impl` 模块里，Windows 路径零改动。
//!
//! 全程用户态、无需提权（X11 协议本身没有权限门槛），能力与 Windows 版
//! WH_KEYBOARD_LL / SendInput / GetForegroundWindow 对齐。
//! 背景与取舍见 docs/linux-平台开发.md。

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

/// 是否 X11 会话
///
/// Wayland（含 XWayland）下必须整链路 fail closed：EWMH 拿到的「前台窗口」
/// 可能是过期的 XWayland 窗口，注入会打进错误的目标。
/// 判定：XDG_SESSION_TYPE 显式为 x11 即可；该变量缺失时退回看 WAYLAND_DISPLAY。
pub fn is_x11_session() -> bool {
    match std::env::var("XDG_SESSION_TYPE") {
        Ok(t) => t == "x11",
        Err(_) => std::env::var("WAYLAND_DISPLAY").is_err(),
    }
}

/// 建立 X 连接（失败记日志返回 None）
pub fn connect() -> Option<(RustConnection, usize)> {
    match x11rb::connect(None) {
        Ok(c) => Some(c),
        Err(e) => {
            log::error!("[X11] 连接失败: {e}");
            None
        }
    }
}

/// intern_atom 助手
pub fn intern(conn: &impl Connection, name: &str) -> Option<Atom> {
    Some(conn.intern_atom(false, name.as_bytes()).ok()?.reply().ok()?.atom)
}

/// 查询窗口上的 32 位属性（CARDINAL / WINDOW 均为 32 位，用 ANY 类型过滤）
pub fn get_cardinal(conn: &impl Connection, window: u32, prop: &str) -> Option<u32> {
    let atom = intern(conn, prop)?;
    let reply = conn
        .get_property(false, window, atom, AtomEnum::ANY, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let mut values = reply.value32()?;
    values.next()
}

/// 在当前键盘映射中查 keysym 对应的 (keycode, 是否需 Shift)
///
/// 只认普通级 / Shift 级（keysym 表的前两列）；AltGr 级（第 3/4 列）需要
/// 额外修饰键、布局差异大，宁可拒绝也不打错字符（fail closed）。
/// XTest 只能发 keycode，字符 → 按键必须过这张表——「语义长在桌面层」的体现。
pub fn key_for_keysym(conn: &impl Connection, keysym: u32) -> Option<(u8, bool)> {
    let setup = conn.setup();
    let min = setup.min_keycode;
    let count = setup.max_keycode - min + 1; // 协议里 count 是 CARD8
    let map = conn.get_keyboard_mapping(min, count).ok()?.reply().ok()?;
    let per = map.keysyms_per_keycode as usize;
    if per == 0 {
        return None;
    }
    for (i, chunk) in map.keysyms.chunks(per).enumerate() {
        for (level, &ks) in chunk.iter().enumerate().take(2) {
            if ks != 0 && ks == keysym {
                return Some((min + i as u8, level == 1));
            }
        }
    }
    None
}

/// 字符 → X11 keysym
///
/// ASCII 可见字符的 keysym 即码点；其余按 Unicode keysym 约定
///（0x01000000 + 码点）编码，通常不在键盘映射里，由 key_for_keysym 拒绝
///（非 ASCII 密码字符注入失败，走审计，回退剪贴板粘贴）。
pub fn char_to_keysym(ch: char) -> u32 {
    let cp = ch as u32;
    if (0x20..=0x7E).contains(&cp) {
        cp
    } else {
        0x0100_0000 + cp
    }
}
