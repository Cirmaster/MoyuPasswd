//! WebView2 浏览器快捷键关闭模块
//!
//! 背景：打包后的窗口跑在 WebView2（Edge 内核）里，内核默认开着「浏览器专属快捷键」——
//! 按 F3 / Ctrl+F 会唤出一个跟应用无关的「网页查找」搜索框，还有 Ctrl+P 打印、
//! F5 / Ctrl+R 刷新、Ctrl+± 缩放、F12 开发者工具、Alt+←/→ 前进后退等。
//! 这些都是浏览器的功能，桌面密码管理器里不该出现，统一在内核层关掉。
//!
//! 实现：`ICoreWebView2Settings3.AreBrowserAcceleratorKeysEnabled = false`。
//! 按微软文档，这个开关只关「浏览器专属」快捷键；文本编辑类快捷键
//! （Ctrl+C/V/X/A/Z、Home/End/PageUp/PageDown）不受影响，应用里复制粘贴照常可用。
//! 应用自己的快捷键（Ctrl+K 快速搜索、Ctrl+V 注入）走系统级全局热键
//! （tauri-plugin-global-shortcut），按键在进入 WebView2 之前就被系统分发给应用，同样不受影响。
//!
//! 开发模式特例（仅 debug 构建）：全关会把 F12 开发者工具也关掉，而前端已禁用右键菜单
//! （没有「检查」入口），开发调试只剩 F12 一条路。所以 debug 构建下额外挂一个内核的
//! AcceleratorKeyPressed 监听，捕获 F12 按下时手动调 `OpenDevToolsWindow()`；
//! release 构建没有这段，F12 保持关闭。
//!
//! Tauri 2.11 没把这个开关暴露进 tauri.conf.json，只能通过 `with_webview` 拿到窗口底层的
//! WebView2 COM 接口直接设置。**每个 webview 窗口都要调一次**，漏掉的窗口仍会响应浏览器快捷键：
//! - 主窗口：lib.rs 的 setup 里遍历启动时已建的窗口统一处理
//! - quick-search / countdown：运行时才建窗，建窗后各自调用（shortcuts.rs / countdown.rs）
//!
//! 幂等：重复调用只是重复写同一个设置，无副作用。

use tauri::Runtime;

#[cfg(windows)]
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
/// cast（QueryInterface）用：webview2-com 的 COM 绑定实现的是 windows-core 0.61 的
/// Interface trait，不能用 windows 0.58 自带的 windows::core::Interface（是另一个 trait）
#[cfg(windows)]
use windows_core::Interface;

#[cfg(all(windows, debug_assertions))]
use webview2_com::AcceleratorKeyPressedEventHandler;
#[cfg(all(windows, debug_assertions))]
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
    COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN, ICoreWebView2, ICoreWebView2AcceleratorKeyPressedEventArgs,
    ICoreWebView2Controller,
};

/// 关闭单个窗口的 WebView2 浏览器专属快捷键（幂等，可重复调用）
///
/// # 参数
/// * `window` - 目标 webview 窗口（主窗口 / quick-search / countdown 都走这里）
pub fn disable_browser_keys<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    #[cfg(windows)]
    {
        let label = window.label().to_string();
        let label_in = label.clone();

        let result = window.with_webview(move |pw| {
            // COM 链路：controller → ICoreWebView2 → ICoreWebView2Settings → Settings3。
            // AreBrowserAcceleratorKeysEnabled 挂在 Settings3 接口上（WebView2 1.0.864 起才有），
            // 所以最后一步要 QueryInterface（cast）
            unsafe {
                let controller = pw.controller();
                let core = match controller.CoreWebView2() {
                    Ok(c) => c,
                    Err(e) => {
                        log::warn!("[BROWSER_KEYS] {label_in}: 获取 ICoreWebView2 失败: {e:?}");
                        return;
                    }
                };
                let settings = match core.Settings() {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("[BROWSER_KEYS] {label_in}: 获取 WebView2 Settings 失败: {e:?}");
                        return;
                    }
                };
                let settings3 = match settings.cast::<ICoreWebView2Settings3>() {
                    Ok(s) => s,
                    Err(e) => {
                        // 运行时太老没有 Settings3 接口：放弃关闭，不影响其他功能
                        log::warn!(
                            "[BROWSER_KEYS] {label_in}: WebView2 运行时无 ICoreWebView2Settings3，跳过: {e:?}"
                        );
                        return;
                    }
                };
                match settings3.SetAreBrowserAcceleratorKeysEnabled(false) {
                    Ok(()) => log::info!(
                        "[BROWSER_KEYS] {label_in}: 浏览器专属快捷键已关闭（F3/Ctrl+F 查找、Ctrl+P 打印、F5/Ctrl+R 刷新等）"
                    ),
                    Err(e) => log::error!("[BROWSER_KEYS] {label_in}: 关闭浏览器快捷键失败: {e:?}"),
                }

                // debug 构建：F12 转发到开发者工具（前端关了右键菜单，给开发留个入口）
                #[cfg(debug_assertions)]
                arm_devtools_f12(&label_in, &controller, &core);
            }
        });

        if let Err(e) = result {
            log::error!("[BROWSER_KEYS] {label}: with_webview 调用失败: {e}");
        }
    }

    // macOS（WKWebView）/ Linux（WebKitGTK）没有 WebView2 这套浏览器快捷键，无需处理
    #[cfg(not(windows))]
    {
        let _ = window;
    }
}

/// 开发模式：把 F12 转发成「打开开发者工具」（仅 debug 构建编译，release 没有这段）
///
/// 浏览器快捷键全关后 F12 也失效，而前端禁用了右键菜单（没有「检查」入口），
/// 开发调试只剩 F12 一条路。这里监听内核的 AcceleratorKeyPressed 事件——
/// 微软文档明确该事件不受 AreBrowserAcceleratorKeysEnabled 影响，所有加速键都会回调——
/// 捕获 F12 按下时手动调 `OpenDevToolsWindow()` 并标记 Handled 吞掉按键。
///
/// 监听器注册后随窗口存活，token 不做保留（不 remove，窗口销毁时一并释放）。
#[cfg(all(windows, debug_assertions))]
fn arm_devtools_f12(label: &str, controller: &ICoreWebView2Controller, core: &ICoreWebView2) {
    /// VK_F12 虚拟键码（Win32 头文件里的值，避免为一个常量引整个 windows crate 的键盘模块）
    const VK_F12: u32 = 0x7B;

    let core = core.clone();
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(
        move |_sender: Option<ICoreWebView2Controller>, args: Option<ICoreWebView2AcceleratorKeyPressedEventArgs>| {
            let Some(args) = args else {
                return Ok(());
            };
            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND(0);
            let mut vk = 0u32;
            unsafe {
                args.KeyEventKind(&mut kind)?;
                args.VirtualKey(&mut vk)?;
            }
            // 只在按下时触发（KEY_DOWN=0 / SYSTEM_KEY_DOWN=2），抬起也触发会弹两个 devtools
            let is_down = kind.0 == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN.0
                || kind.0 == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN.0;
            if is_down && vk == VK_F12 {
                unsafe {
                    // 打开开发者工具并吞掉按键（不再当浏览器快捷键处理，也不进页面）
                    core.OpenDevToolsWindow()?;
                    args.SetHandled(true)?;
                }
            }
            Ok(())
        },
    ));

    let mut token: i64 = 0;
    let registered = unsafe { controller.add_AcceleratorKeyPressed(&handler, &mut token) };
    match registered {
        Ok(()) => log::info!(
            "[BROWSER_KEYS] {label}: 开发模式保留 F12 → 开发者工具（其余浏览器快捷键仍关闭）"
        ),
        Err(e) => log::warn!("[BROWSER_KEYS] {label}: 注册 F12 转发失败: {e:?}"),
    }
}
