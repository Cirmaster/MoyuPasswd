//! Tauri 命令模块
//!
//! 包含所有前端可调用的 Tauri 命令。
//! 命令按功能分组：
//! - `auth`: 认证相关（主密码设置、验证、锁定）
//! - `password`: 密码 CRUD 操作
//! - `category`: 分类管理
//! - `settings`: 设置管理

pub mod auth;
pub mod password;
pub mod category;
pub mod settings;
pub mod data;
pub mod shortcuts;
pub mod countdown;

// 重新导出所有命令
pub use auth::*;
pub use password::*;
pub use category::*;
pub use settings::*;
pub use data::*;
pub use shortcuts::*;
pub use countdown::*;
