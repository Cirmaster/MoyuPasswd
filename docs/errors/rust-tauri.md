# Rust / Tauri 常见问题与解决方案

## tauri-plugin-sql

### PluginBuilder / TauriPlugin 类型不匹配

**报错信息：**
```
error[E0603]: struct `PluginBuilder` is private
// 或
类型不匹配：应为 `TauriPlugin<R>`，但实际为 `TauriPlugin<R, Option<PluginConfig>>`
```

**原因：**
`tauri_plugin_sql` 的内部类型是私有的，不能直接用于函数返回值。

**解决方案：**
不封装函数，直接在 `lib.rs` 中初始化插件。

```rust
// ❌ 错误写法 - 封装函数
pub fn init_db_plugin(db_name: &str) -> TauriPlugin<R> {
    tauri_plugin_sql::Builder::default()
        .add_migrations(&db_path, migrations)
        .build()
}

// ✅ 正确写法 - 直接初始化
// db/mod.rs
pub const DB_NAME: &str = "moyu_passwd.db";
pub fn get_db_path() -> String {
    format!("sqlite:{}", DB_NAME)
}
pub fn get_migrations() -> Vec<Migration> { ... }

// lib.rs
let db_path = db::get_db_path();
let migrations = db::get_migrations();
tauri::Builder::default()
    .plugin(
        tauri_plugin_sql::Builder::default()
            .add_migrations(&db_path, migrations)
            .build(),
    )
```

---

## Tauri 命令

### 命令未注册

**报错信息：**
```
Unhandled Promise Rejection: command not found
```

**原因：**
在 `lib.rs` 的 `invoke_handler` 中忘记注册命令。

**解决方案：**
在 `generate_handler!` 宏中添加命令名。

```rust
// lib.rs
.invoke_handler(tauri::generate_handler![
    commands::my_command,  // 确保这里包含所有命令
])
```

---

## Cargo

### 网络错误 SSL connect error

**报错信息：**
```
warning: spurious network error: SSL connect error
error: failed to get `xxx` as a dependency
```

**原因：**
网络问题或 SSL 证书问题，常见于国内网络环境。

**解决方案：**
1. 配置 cargo 镜像源（推荐）
2. 使用 VPN
3. 多次重试

```toml
# ~/.cargo/config.toml
[source.crates-io]
replace-with = 'ustc'

[source.ustc]
registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"
```

---

## thiserror

### ? 操作符无法转换错误类型

**报错信息：**
```
`?` couldn't convert the error to `std::string::String`
Note: `AuthError` needs to implement `Into<std::string::String>`
```

**原因：**
函数返回 `Result<T, String>`，但使用 `?` 操作符时，错误类型没有实现 `Into<String>`。

**解决方案：**
使用 `.map_err(|e| e.to_string())` 将错误转换为字符串。

```rust
// ❌ 错误写法
#[tauri::command]
pub async fn my_command() -> Result<(), String> {
    let result = some_function()?;  // some_function 返回自定义错误类型
    Ok(())
}

// ✅ 正确写法
#[tauri::command]
pub async fn my_command() -> Result<(), String> {
    let result = some_function()
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ✅ 嵌套错误处理
#[tauri::command]
pub async fn my_command() -> Result<(), String> {
    let result = crypto::hash_password(&password)
        .map_err(|e| AuthError::Crypto(e.to_string()).to_string())?;
    Ok(())
}
```

---

## RwLock

### lock 方法不存在

**报错信息：**
```
error[E0599]: no method named `lock` found for struct `tauri::async_runtime::RwLock<T>`
```

**原因：**
Tauri 2.0 使用异步 `RwLock`，不支持同步的 `.lock()` 方法。

**解决方案：**
使用 `.read().await` 或 `.write().await` 替代。

```rust
// ❌ 错误写法
let instances = db.0.lock().map_err(|e| e.to_string())?;

// ✅ 正确写法 - 读操作
let instances = db.0.read().await;

// ✅ 正确写法 - 写操作
let instances = db.0.write().await;
```

---

## rusqlite

### tauri-plugin-sql 的 DbPool 方法是私有的

**报错信息：**
```
error[E0624]: method `select` is private
error[E0624]: method `execute` is private
```

**原因：**
`tauri-plugin-sql` 的 `DbPool` 的 `select` 和 `execute` 方法是 `pub(crate)`，只能在 crate 内部使用。

**解决方案：**
使用 `rusqlite` 直接操作数据库，不依赖 `tauri-plugin-sql`。

```toml
# Cargo.toml
[dependencies]
rusqlite = { version = "0.31", features = ["bundled"] }
# 移除 tauri-plugin-sql
```

```rust
// 使用 rusqlite 直接操作
use rusqlite::Connection;

let conn = Connection::open("database.db")?;
conn.execute("INSERT INTO table VALUES (?1)", params![value])?;
```

---

## Tauri Manager trait

### 方法未找到

**报错信息：**
```
error[E0599]: no method named `path` found for mutable reference `&mut tauri::App`
error[E0599]: no method named `manage` found for mutable reference `&mut tauri::App`
```

**原因：**
`path()` 和 `manage()` 方法来自 `Manager` trait，需要显式导入。

**解决方案：**
在 lib.rs 中添加 `use tauri::Manager;`。

```rust
use tauri::Manager;

// 现在可以使用
app.path().app_data_dir()
app.manage(MyState::new())
```

---

## dev 启动后窗口全白（2026-11-21，排障过程中自己引入的）

**现象：**
`cargo tauri dev` 窗口能出来，里面全白；webview 心跳一次都没上报，
看门狗每 15 秒判定「心跳停止」自动 reload，3 次后标记异常，之后一直白。

**排障顺序（按这个从外往里查，别跳）：**

1. **先看 dev server 有没有被连上**，不要先怀疑前端代码：
   ```powershell
   Get-NetTCPConnection -LocalPort 5173 | Format-Table State,LocalAddress,OwnerProcess
   ```
   Vite 加载成功时一定会有 HMR 的 `Established` 连接（页面开了 WebSocket）。
   **只有 Listen、没有 Established → 页面压根没加载**，问题在网络/导航层，不在 JS。
2. **看窗口实际停在哪一页**：项目里平时不打印这个（诊断日志已清掉），需要时临时加
   `log::info!("{:?}", window.url())`，或直接用下面的调试口读 target 的 `url`。
   日志在 `%LOCALAPPDATA%\com.moyu.passwd\logs\moyu-passwd.log`（**时间戳是 UTC，本地 +8**）。
3. 需要看页面内部时，开 Chromium 调试口，比反复重启快得多：
   ```powershell
   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333 --enable-logging=stderr --v=1"
   cargo tauri dev
   # 然后 curl http://127.0.0.1:9333/json/list 看 target 的 url / title
   ```
   再用 CDP 读 `document.readyState`、`#app` 子节点数、`performance.getEntriesByType('resource')`，
   就能分清「没加载」「加载了但没挂载」「挂载了但没渲染」。

**坑 0 是本次真因；坑 1~3 是同一现象下顺手撞到的次要坑（坑 3 的临时修法已回滚，未留在代码里）。**

### 坑 0（真因）：排障过程中自己引入的两个变量

这次白屏不是项目原有代码的问题，是排障过程中加进去的两个东西。两个都去掉后恢复正常：

**变量 1：`bundle.windows.webviewInstallMode: fixedRuntime` 指向项目内那份 WebView2**

`src-tauri/webview2-runtime`（156.0.4314.42）不是用微软 cab 解出来的，是把某个已安装目录
整个拷过来的（里面混着 `EBWebView`、`Installer`、`Extensions`），而且被命令行沙箱打上了异常 ACL：

```
Everyone:(I)(CI)(DENY)(DC)
S-1-4-404179122-295122383:(I)(OI)(CI)(W,D,DC)      ← 非标准 SID
LAPTOP-XXX\CodexSandboxUsers:(I)(OI)(CI)(RX)       ← 命令行沙箱建的本地组
（缺 ALL APPLICATION PACKAGES / ALL RESTRICTED APPLICATION PACKAGES）
```

对照实验：**同一份二进制**拷到 `%TEMP%`（普通 ACL）→ `url()` 1.5 秒内变成 devUrl、正常；
留在项目目录 → 永远 `about:blank`。二进制没变，只有 ACL 变了。

**变量 2：在 `setup()` 里用 `WebviewWindowBuilder` 建主窗口，并"补发"导航**

`tauri.conf.json` 的 `windows` 数组清空、改成代码建窗，还加过一个 1.5 秒后
"发现 `url()` 是 `about:blank` 就再 `navigate` 一次"的兜底。两条都是坑：

- Tauri 官方文档明确写了 `WebviewWindowBuilder` 在 Windows 的某些上下文（同步命令/事件回调）
  会 deadlock、出空白窗口（tauri#13092）；
- `url()` 在首个页面 commit 前恒返回 `about:blank`（tauri#9027），据此补发导航会**打断**
  正在进行的那次加载（Cap 的实测：`about:blank` 会持续到首帧 commit，约 2.1 秒）。

**最终处理：** 两条全部撤掉——`webviewInstallMode` 删掉（dev 与安装包都用系统 WebView2），
窗口回到 `tauri.conf.json` 的 `windows` 数组创建、代码里不再建窗、不再补导航。

**判定入口（以后遇到白屏先按这个走，不要先改代码）：**

1. dev 用的是哪份运行时：
   ```powershell
   Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" |
     Select-Object ProcessId, @{n='Exe'; e={$_.ExecutablePath}}
   ```
   指向 `target\debug\webview2-runtime\` 就是项目内置版，指向 `Program Files` 才是系统运行时；
2. dev server 有没有被连上：`Get-NetTCPConnection -LocalPort 5173`，**没有 `Established` 就是页面没加载**；
3. 日志：`%LOCALAPPDATA%\com.moyu.passwd\logs\moyu-passwd.log`（**时间戳是 UTC，本地 +8**）；
4. 要看页面内部就开 Chromium 调试口，比反复重启快：
   `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`;

**教训：**
1. **先做"只换一个变量"的对照实验，再改代码。** 这次先连改了五六个文件，绕了两小时；
2. 不要在窗口创建、导航这种框架自己管的事情上加"智能兜底"——它比原来的 bug 更难查；
3. 排障期的诊断代码要当场记下来、事后清掉，不要留在仓库里。

### 坑 1（次要，非本次根因）：Vite 8 默认只监听 `[::1]`，WebView2 连不上
```
curl -4 http://127.0.0.1:5173   -> 连接失败
curl -6 http://[::1]:5173       -> 200
```
Vite 默认 `host: 'localhost'`，Node 17+ 按 verbatim 解析先得 `::1`，于是只绑 IPv6；
WebView2 走 Happy Eyeballs 先试 `127.0.0.1` → 拒连 → `about:blank`，**且不报任何错**。

```ts
// vite.config.ts
server: { host: '127.0.0.1', port: 5173, strictPort: true }
```
`tauri.conf.json` 的 `devUrl` 同步改成 `http://127.0.0.1:5173`，两端都用字面 IPv4。
`strictPort` 必加：端口漂移后 devUrl 失配，现象同样是白屏。

### 坑 2：用 `window.url()` 判断「导航没发生」，然后补 `navigate()` —— 越补越白
Tauri 在 Windows 上**导航还没 commit 时 `url()` 一直返回 `about:blank`**（tauri#9027）。
按它判断就会反复调 `navigate()`，而**每次导航都会中断上一次正在进行的加载**。
本项目 dev 依赖有 10MB 级（reka-ui 5.5MB / vue 1.2MB / vueuse 1MB），
秒级重试直接把页面永远卡在半途 —— 比原本的白屏更难查。

**规矩：不要用 `url()` 决定是否补导航。** 需要时只在调试端口手动 `Page.navigate` 验证，不要写进产品代码。

### 坑 3（临时修法已回滚）：看门狗 15 秒判定可能打断冷启动
首屏没心跳就 reload，reload 又掐掉正在下载的模块 → 越 reload 越白。
排查期曾在 `watchdog.rs` 加过 `STARTUP_GRACE = 90s`（本次进程没收到过心跳时放宽判定），
但那次白屏与它无关，**该改动已回滚、未保留在代码里**。若以后确实遇到慢机器冷启动，
再按同样思路单独加。

### 坑 4：想靠「管理员终端」验证提权行为，验证不了
- debug 构建里 `set_run_as_admin` **故意不写注册表**（会让 cargo 拉起 app.exe 报 os error 740），
  所以设置页那个开关在 dev 下勾了也没有 UAC —— 见 `elevate.rs:79`；
- 官方文档的建议是宿主进程以**标准（非提权）完整性**运行（提权后的 Chromium 子进程行为有额外风险），
  但本次白屏经对照实验排除，**不是提权导致的**；

要验证提权行为，用打包后的安装版 + 设置开关，不要在 dev 里提权。

---

*最后更新: 2026-11-21*
