//! 分类命令模块
//!
//! 处理密码分类的增删改查操作。
//!
//! # 命令列表
//!
//! - `get_categories`: 获取分类列表
//! - `add_category`: 添加分类
//! - `update_category`: 更新分类
//! - `delete_category`: 删除分类

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

/// 分类项结构体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    /// 分类 ID
    pub id: String,
    /// 分类名称
    pub name: String,
    /// 分类图标
    pub icon: Option<String>,
    /// 排序顺序
    pub sort_order: i32,
    /// 创建时间
    pub created_at: i64,
}

/// 获取分类列表
///
/// 获取所有分类，包括每个分类下的密码数量。
///
/// # Arguments
///
/// * `state` - 应用状态
///
/// # Returns
///
/// 分类列表
///
/// # 前端调用
///
/// ```typescript
/// const categories = await invoke('get_categories');
/// ```
#[tauri::command]
pub async fn get_categories(state: State<'_, AppState>) -> Result<Vec<Category>, String> {
    let db = state.get_db()?;
    let conn = db.conn();

    // 查询分类列表
    let mut stmt = conn
        .prepare("SELECT id, name, icon, sort_order, created_at FROM categories ORDER BY sort_order")
        .map_err(|e| e.to_string())?;

    let categories = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let icon: Option<String> = row.get(2)?;
            let sort_order: i32 = row.get(3)?;
            let created_at: i64 = row.get(4)?;

            Ok(Category {
                id,
                name,
                icon,
                sort_order,
                created_at,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(categories)
}

/// 添加分类
///
/// 添加一个新的密码分类。
///
/// # Arguments
///
/// * `name` - 分类名称
/// * `state` - 应用状态
///
/// # Returns
///
/// 新添加的分类
///
/// # 前端调用
///
/// ```typescript
/// const category = await invoke('add_category', { name: '新分类' });
/// ```
#[tauri::command]
pub async fn add_category(
    name: String,
    state: State<'_, AppState>,
) -> Result<Category, String> {
    let db = state.get_db()?;
    let conn = db.conn();

    // 生成唯一 ID
    let id = uuid::Uuid::new_v4().to_string();

    // 获取当前时间戳（毫秒）
    let now = chrono::Utc::now().timestamp_millis();

    // 插入数据库
    conn.execute(
        "INSERT INTO categories (id, name, sort_order, created_at) VALUES (?1, ?2, 0, ?3)",
        params![id, name, now],
    ).map_err(|e| e.to_string())?;

    // 返回新添加的分类
    Ok(Category {
        id,
        name,
        icon: None,
        sort_order: 0,
        created_at: now,
    })
}

/// 更新分类
///
/// 更新分类的名称或图标。
///
/// # Arguments
///
/// * `id` - 分类 ID
/// * `name` - 新分类名称（可选）
/// * `icon` - 新分类图标（可选）
/// * `state` - 应用状态
///
/// # Returns
///
/// 更新后的分类
///
/// # 前端调用
///
/// ```typescript
/// const category = await invoke('update_category', {
///   id: '123',
///   name: '新名称'
/// });
/// ```
#[tauri::command]
pub async fn update_category(
    id: String,
    name: Option<String>,
    icon: Option<String>,
    state: State<'_, AppState>,
) -> Result<Category, String> {
    let db = state.get_db()?;
    let conn = db.conn();

    // 获取当前分类
    let current = conn.query_row(
        "SELECT id, name, icon, sort_order, created_at FROM categories WHERE id = ?1",
        params![id],
        |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let icon: Option<String> = row.get(2)?;
            let sort_order: i32 = row.get(3)?;
            let created_at: i64 = row.get(4)?;
            Ok((id, name, icon, sort_order, created_at))
        },
    );

    match current {
        Ok((current_id, current_name, current_icon, sort_order, created_at)) => {
            // 使用新值或当前值
            let new_icon = icon.or(current_icon);
            let new_name = name.unwrap_or(current_name);

            // 名称去空白，为空则报错
            let trimmed = new_name.trim();
            if trimmed.is_empty() {
                return Err("分类名称不能为空".to_string());
            }
            let new_name = trimmed.to_string();

            // 同名校验（排除自身）
            let dup: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM categories WHERE name = ?1 AND id != ?2",
                    params![new_name, current_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if dup > 0 {
                return Err("已存在同名分类".to_string());
            }

            // 更新数据库
            conn.execute(
                "UPDATE categories SET name = ?1, icon = ?2 WHERE id = ?3",
                params![new_name, new_icon, current_id],
            ).map_err(|e| e.to_string())?;

            // 返回更新后的分类
            Ok(Category {
                id: current_id,
                name: new_name,
                icon: new_icon,
                sort_order,
                created_at,
            })
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Err("未找到分类".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// 删除分类
///
/// 删除指定的分类。删除前需要将该分类下的密码移到其他分类。
///
/// # Arguments
///
/// * `id` - 分类 ID
/// * `state` - 应用状态
///
/// # Returns
///
/// 成功返回 Ok(())，失败返回错误信息
///
/// # 前端调用
///
/// ```typescript
/// await invoke('delete_category', { id: '123' });
/// ```
#[tauri::command]
pub async fn delete_category(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.get_db()?;
    let conn = db.conn();

    // 将该分类下的密码移到"其他"分类
    conn.execute(
        "UPDATE passwords SET category_id = 'other' WHERE category_id = ?1",
        params![id],
    ).map_err(|e| e.to_string())?;

    // 删除分类
    conn.execute(
        "DELETE FROM categories WHERE id = ?1",
        params![id],
    ).map_err(|e| e.to_string())?;

    Ok(())
}
