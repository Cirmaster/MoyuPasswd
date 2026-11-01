# Linux 平台开发

> 本文梳理 MoyuPasswd 在 Linux 平台上的功能现状：哪些是 Windows 专属、哪些降级可用、哪些正常工作。
> 项目以 Windows 为主要发布平台（NSIS/WiX 安装包），Linux 上以开发/调试为主要目的。
> 最近更新：2026-10-31

## 开发环境

Linux 上编译运行需要系统库（Debian/Ubuntu 系）：

```bash
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev
```

依赖装好后：

```bash
pnpm install
pnpm tauri dev
```

## 问题清单

### ❌ 完全不可用（Windows 专属）

#### 1. Ctrl+V 密码注入【X11 已实现（2026-10-31）；Wayland 不支持】

X11 会话下整条链路已实现（`x11.rs` + `inject.rs::x11_impl` + `hotkey.rs::x11_impl`，x11rb 用户态免提权）：

| 环节 | Windows 实现 | X11 实现 |
|------|--------------|----------|
| Ctrl+V 触发 | WH_KEYBOARD_LL 钩子 | XGrabKey 会话期抓取（无待注入密码时 XTest 回放、把粘贴还给应用） |
| 进程归因 | GetForegroundWindow + QueryFullProcessImageNameW | EWMH `_NET_ACTIVE_WINDOW` + `_NET_WM_PID` + `/proc` |
| 注入 | SendInput（KEYEVENTF_UNICODE） | XTest 按 keycode 合成（字符按当前键盘映射折算） |
| 剪贴板哨兵 | 剪贴板序号 | CLIPBOARD selection 属主变化 |
| 复核 | 前台窗口 + PID 双复核 | 同语义双复核 |
| Esc / Ctrl+C 取消 | 钩子监听 | **不支持**（监听需 XRecord；Ctrl+C 场景由哨兵兜底，Esc 只能等 TTL/关倒计时） |

已知限制（fail closed，回退剪贴板粘贴，不会打错目标）：
- **Wayland 会话整链路禁用**：无全局窗口查询/输入合成接口（安全设计），
  `is_x11_session()` 判定后归因/注入/抓取全部跳过；
- **非 ASCII 密码字符**（及 AltGr 级布局字符）注入失败：XTest 无 Unicode 直注，
  字符映射不到当前键盘布局时整次放弃（绝不打半截密码）；
- **剪贴板管理器**（Klipper 等）接管 CLIPBOARD 属主可能提前作废会话——误判方向安全
  （退化为剪贴板粘贴，不误注入）；
- macOS 仍是桩（Quartz 事件待实现）。

#### 2. 管理员启动开关：静默无效，且 UI 误导【已修复：按平台隐藏】

- `src-tauri/src/elevate.rs:143`（`non_windows::win_impl`）：
  - `set_run_as_admin` 空操作但返回 `Ok` —— 设置看起来保存成功，实际什么都不做；
  - `is_elevated` 恒返回 `false`。
- ~~`src/views/SettingsView.vue` 设置页照常显示开关，文案是「弹 UAC」（Windows 概念）。~~

**影响**：用户可以打开开关并得到"已保存"的假象。
**已修复**（2026-10-31）：设置页以 `isWindows`（`navigator.userAgent` 判断）`v-if` 隐藏该项，
非 Windows 不再显示，也不再调用 `is_elevated`。后端空操作实现保留，兜住直接调用的场景。

#### 3. 生物识别解锁不可用

- `src-tauri/src/system_auth/mod.rs`：Linux 平台运行时报 `Biometry is not supported on this platform`。
- 影响：只能用主密码解锁。`UnlockView` 已做优雅回退（系统认证失败 → 显示主密码输入框），流程不坏，只是体验打折。

### ⚠️ 降级可用（有行为差异/小隐患）

#### 4. 倒计时气泡【已修复（2026-10-31）】

- **光标跟随**：X11 下已实现（`commands/countdown.rs` 的 `get_cursor_position` 走
  `query_pointer`，线程内缓存 X 连接，60fps 轮询）；Wayland 无全局光标查询接口，
  启动前探测失败即跳过跟随（气泡固定在初始位置，不空转）。
- **焦点**：气泡 `set_focusable(false)`（GTK `set_accept_focus(false)`），WM 在窗口
  map 时不再抢走目标应用焦点。此前「`show()` 不抢焦点」的判断不完整——tao/GTK 层
  确实只 map 不请求焦点（源码核实过），但 WM 的焦点策略会给新映射窗口焦点，
  必须显式声明不可聚焦才稳（对齐 Windows `SW_SHOWNA` 语义）。
  这也是「复制密码后要先点回目标窗口才能注入」的根因，已随修复消失。

#### 5. 剪贴板哨兵空操作（隐患比预期小）

- `src-tauri/src/hotkey.rs:370`（`rebase_clipboard_sentinel`）空操作：检测不到「用户复制了新内容」。
- 但 `schedule_clear` 在定时清除前会**读回剪贴板比对内容**（`src-tauri/src/clipboard.rs:22`），
  内容不是本应用写入的文本就跳过清除 —— 不会误删用户的新内容。
- **实际差异**：倒计时会话不会像 Windows 那样在外部复制后立即作废，仅剩比对兜底一层。

#### 6. 自动锁定缺少系统空闲这一层

- `src-tauri/src/idle.rs:132`：非 Windows 无系统空闲 API，返回 `u64::MAX`（不检测）。
- 自动锁定只靠前端应用内活动计时（`useAutoLock` 心跳），基本够用，少一层系统级保障。

#### 7. 安全加固类空缺（影响小）

| 项 | 位置 | Windows | Linux |
|----|------|---------|-------|
| 元数据存储（主密码验证数据等） | `src-tauri/src/db_meta.rs:213` | 凭据管理器（Credential Manager） | 明文文件（app 目录下） |
| DB 文件 ACL 收紧 | `src-tauri/src/acl.rs` | 收紧为当前用户 | 【已修复】chmod 0600 |
| 启动期致命错误提示 | `src-tauri/src/lib.rs:89` | MessageBoxW 弹窗 | 【已修复】rfd 原生弹窗 + stderr 兜底 |

**已修复**（2026-10-31）：
- `acl.rs` 的 `harden_file_acl` 在 Unix 上实现为 chmod 0600，一处修复覆盖全部调用点
  （数据库、审计日志、元数据、备份/隔离文件）。
- 非 Windows 的 `show_fatal_error` 改用 rfd 原生错误对话框（Linux GTK3 / macOS NSAlert，
  `Cargo.toml` 的 `[target.'cfg(not(windows))'.dependencies]`），保留 stderr 兜底；
  Windows 的 MessageBoxW 路径不变。

### ✅ 正常工作

- 全局快捷键（Ctrl+K 快速搜索 / Ctrl+Shift+N 快速添加 / Ctrl+Shift+G 密码生成器）
- 托盘图标（`libayatana-appindicator` 有弃用警告，无碍）
- 开机自启（`plugin-autostart`，写 `~/.config/autostart/*.desktop`）
- 剪贴板读写 / 定时清除（`plugin-clipboard-manager` 回退路径）
- 解锁、密码库增删改查、分类、标签
- 备份与恢复、导入导出
- 快速搜索小窗、倒计时小窗（除跟随/抢焦点问题）
- 主题切换（四套色系 × 明暗）

### 不算缺陷

- `src-tauri/src/browser_keys.rs`：关闭 WebView2 浏览器专属快捷键（F3/Ctrl+F/Ctrl+P 等）是
  WebView2（Edge 内核）专属问题域，WebKitGTK 无此类行为，非 Windows 为空操作即可。

## 已处理（Linux 驱动的修复，留档）

- **`inject.rs` 非 Windows 桩签名与 Windows 实现漂移**导致编译失败 —— 已对齐签名
  （`TargetProcess` 补 `hwnd` 占位字段、`inject_text` 对齐三参数），仅恢复可编译，注入功能本身仍不可用（见问题 1）。
- **无边框窗口在 X11 下无法缩放** —— 新增 `src/components/WindowResizeHandles.vue`（自绘边缘/角落热区 +
  `startResizeDragging`），并在 `src-tauri/capabilities/default.json` 补
  `core:window:allow-start-resize-dragging` 权限。
- **原生 `<select>` 弹层不跟主题**（深色主题弹白色列表）—— 自研
  `src/components/ui/select/Select.vue`（Teleport 自绘弹层 + 主题令牌），替换全部 8 处原生 select。
  原因：WebKitGTK 基本管不到原生弹层的样式，`color-scheme` 只能覆盖一部分。
- `docs/linux-平台开发.md` 之前，`src/assets/main.css` 保留了 `color-scheme` 声明与
  `select option` 兜底样式，对残留原生控件（如滚动条）仍然有效。
- **Linux 高分屏下窗口内容拥挤** —— 窗口缩放模型统一为「物理 = 设计 × 渲染缩放」，
  新增 `scale.rs`/`window_sizes.rs`，Windows 行为零变化（见「窗口缩放模型」章节）。
- **GTK3 `resizable(false)` 窗口尺寸不可靠**（初始尺寸/程序化 resize 被冻结在 200px）——
  倒计时气泡、快速搜索改 `resizable(true)` + min/max 尺寸提示锁定（见「窗口缩放模型」章节）。

## 窗口缩放模型（高分屏/DPI，2026-11-01）

**症状**：Windows（3200×2000 @ 175% 缩放）开发的界面，在 Linux（同分辨率、无几何缩放、
Xft.dpi=144）下窗口内的内容特别拥挤。

**根因**：窗口几何层（tao/GTK）与渲染层（WebKitGTK）的缩放不同步。
Windows 上 WebView2 与窗口几何共用系统缩放（1.75），两层永远同步，CSS 视口恒等于设计尺寸；
Linux 高分屏上 tao/GTK 的 scale factor 是 1（窗口不放大），WebKitGTK 却按 `gtk-xft-dpi`
把 devicePixelRatio 抬到 1.5（实测定律：`dPR = GDK 几何缩放 × gtk-xft-dpi ÷ 1024 ÷ 96`，
WebKitGTK 2.54），900×650 的窗口只剩 600×433 的 CSS 布局视口 → 拥挤。

**模型**（`src-tauri/src/scale.rs` 是唯一换算入口）：

```text
窗口物理尺寸 = 设计尺寸 × 渲染缩放 R（R = webview devicePixelRatio）
CSS 布局视口 = 物理 ÷ R ≡ 设计尺寸        （不变量，任何平台恒成立）
```

平台矩阵（主窗口，设计尺寸 900×650）：

| 平台 | R | 窗口物理 | CSS 视口 |
|---|---|---|---|
| Windows 175% | 1.75 | 1575×1137 | 900×650 |
| Linux 高分屏（Xft.dpi=144） | 1.5 | 1350×975 | 900×650 |
| Linux 96dpi | 1.0 | 900×650 | 900×650 |
| macOS Retina | 2.0 | 1800×1300 | 900×650 |

**维护约定**：

1. 设计尺寸常量只定义在 `src-tauri/src/window_sizes.rs`，禁止在别处散落裸数值；
2. 窗口尺寸/定位换算只走 `scale.rs`：建窗 builder 用 `tao_logical_size/pos`，
   运行时 `set_size`/`set_position` 用 `PhysicalSize`/`PhysicalPosition`；
3. **前端 CSS 写死 px 是允许且推荐的**——CSS px 即设计尺寸，视口恒等于设计尺寸，写死即安全；
4. 屏幕坐标约定：光标查询（GetCursorPos / X11 query_pointer）返回物理坐标，直接用；
   前端 `event.screenX/screenY` 是 CSS 逻辑坐标，传入命令后由后端换算；
5. `tauri.conf.json` 的 width/height/minWidth/minHeight 只是启动占位初值，
   setup 阶段以 `window_sizes.rs` 常量 × R 覆盖（conf 格式无法表达换算）；
6. 前端启动时经 `sync_render_scale` 上报 `devicePixelRatio` 实测值自检（安全网，
   覆盖 Wayland 分数缩放等推断可能失准的环境），三个窗口入口都接了
   `src/lib/renderScale.ts`；换显示器/改系统缩放由 `ScaleFactorChanged` 与
   dpr 监听兜住，按登记设计尺寸重排全部窗口。

**实测踩坑（GTK3，必读）**：`resizable(false)` 的窗口初始尺寸与程序化 resize 都不可靠
（实测高度被冻结在 200px）。倒计时气泡/快速搜索一律 `resizable(true)` + min/max 尺寸提示
锁定（无边框无缩放控件，用户无从手动缩放，效果等同固定尺寸但 resize 正常）。

## 跨平台开发约定

结构结论（2026-10-31 评估）：**不重排目录结构**。现行「共享逻辑 + 文件内 cfg 平台模块 + 统一签名」
是 Rust 标准库同款做法：平台差异留在同一文件内对照可见，Windows 生产路径零挪动。
已做的 Linux 适配全部位于 `not(windows)`/`unix` 分支或新增文件，Windows 行为零变化。

新增涉及平台差异的代码时遵循：

1. **统一 API + 平台模块**：对外只暴露一个签名，内部 `#[cfg(windows)] mod win_impl` /
   `#[cfg(unix)] mod unix_impl`（重场景可学 `db_meta.rs` 的双定义 `mod platform`，调用方完全无感）。
   业务代码只调统一 API，不散落 `#[cfg]`。
2. **桩的行为规范**：非目标平台的桩必须可预期——记日志、返回安全默认值；
   禁止像 `elevate.rs` 那样静默返回 `Ok` 谎报成功（该处已用设置页按平台隐藏兜住）。
3. **Windows 路径零改动原则**：Linux 适配一律进 `not(windows)`/`unix` 分支或新增文件；
   可以把 Windows 代码整体移进 `#[cfg(windows)]` 块，但语句顺序与语义不变。
4. **前端平台判断**统一从 `src/lib/platform.ts` 取（`isWindows` / `isLinux` / `isMac`），
   不在组件里散写 `navigator.userAgent`。
5. **能力声明**：新窗口能力在 `src-tauri/capabilities/*.json` 按最小化原则显式添加
   （参考 default.json 的注释）；声明全平台共用，平台差异在业务侧门控。
6. **平台专属依赖**放 `Cargo.toml` 的 `[target.'cfg(...)'.dependencies]`（现有先例：
   `cfg(windows)` 的 windows/webview2-com、`cfg(not(windows))` 的 rfd），避免污染其他平台的构建。

## 修复优先级建议

1. ~~**设置页按平台隐藏「管理员启动」开关**（问题 2）~~ ✅ 已完成（2026-10-31）。
2. ~~**倒计时：非 Windows 不启动跟随线程**（问题 4）~~ ✅ 已完成（2026-10-31）；「抢焦点」经源码核实为误判，无需修改。
3. ~~**Linux 文件权限收紧 0600**（问题 7）~~ ✅ 已完成（2026-10-31），覆盖 db/审计/元数据/备份。
4. ~~**致命错误改用 GUI 弹窗**（问题 7）~~ ✅ 已完成（2026-10-31）。
5. ~~**注入链路的 Linux 支持**（问题 1）~~ ✅ X11 已完成（2026-10-31）；Wayland 按设计不支持，待生态给出合规接口再评估。
