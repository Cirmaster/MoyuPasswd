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

*最后更新: 2024*
