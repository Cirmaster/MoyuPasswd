//! 分类命令模块
//!
//! 处理密码分类的增删改查操作（扁平分类升级为分类树）。
//!
//! # 命令列表
//!
//! - `get_categories`: 获取分类列表（含父分类、直接密码数与子树总数）
//! - `add_category`: 添加分类（可指定父分类）
//! - `update_category`: 更新分类（名称/图标/父分类；防环；clearParent 置顶级）
//! - `delete_category`: 删除分类（子分类上移一级，密码移到「其他」）
//! - `reorder_categories`: 批量调整分类排序（拖拽排序）

use crate::response::{respond, ApiResult};
use std::collections::{HashMap, HashSet};

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
    /// 父分类 ID（None 表示顶级分类）
    pub parent_id: Option<String>,
    /// 该分类直接归属的未删除密码数（不含后代分类）
    pub password_count: i64,
    /// 该分类及其全部后代分类直接归属的未删除密码数（含自身；后端汇总，前端直接展示）
    pub total_count: i64,
}

/// 规范化父分类 ID：trim 后为空视为顶级分类（None）
fn normalize_parent_id(parent_id: Option<String>) -> Option<String> {
    parent_id.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// 校验父分类存在（parent_id 为 None/空串表示顶级分类，无需校验）
fn ensure_parent_exists(conn: &rusqlite::Connection, parent_id: &str) -> Result<(), String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM categories WHERE id = ?1",
            params![parent_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists == 0 {
        return Err("父分类不存在".to_string());
    }
    Ok(())
}

/// 防环校验：把 `id` 的父分类设为 `parent_id` 时，沿目标父分类的 parent 链上溯
///
/// 遇到自身即报错（不能是自身或后代）。层级深度不设限；
/// 用 visited 集合防御历史脏数据本身存在环时造成的死循环。
fn ensure_no_cycle(conn: &rusqlite::Connection, id: &str, parent_id: &str) -> Result<(), String> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut current = Some(parent_id.to_string());

    while let Some(node) = current {
        if node == id {
            return Err("分类层级不能形成环".to_string());
        }
        // 已访问过说明历史数据本身有环，停止上溯（命令层保证本次修改不新增环）
        if !visited.insert(node.clone()) {
            break;
        }
        current = match conn.query_row(
            "SELECT parent_id FROM categories WHERE id = ?1",
            params![node],
            |row| row.get::<_, Option<String>>(0),
        ) {
            Ok(parent) => parent,
            // 上级分类缺失（悬空脏数据）视为已到顶，不阻断本次修改
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.to_string()),
        };
    }

    Ok(())
}

/// 分类父子关系索引：父分类 id -> 子分类 id 列表
type CategoryChildren = HashMap<String, Vec<String>>;

/// 加载全部分类的父子关系（id -> children）
fn load_category_children(conn: &rusqlite::Connection) -> Result<CategoryChildren, String> {
    let mut stmt = conn
        .prepare("SELECT id, parent_id FROM categories")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut children: CategoryChildren = HashMap::new();
    for (id, parent_id) in rows {
        if let Some(parent) = parent_id {
            children.entry(parent).or_default().push(id);
        }
    }
    Ok(children)
}

/// 加载全部分类的「直接归属」未删除密码数（id -> count）
fn load_direct_password_counts(
    conn: &rusqlite::Connection,
) -> Result<HashMap<String, i64>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id,
                    (SELECT COUNT(*) FROM passwords p WHERE p.category_id = c.id AND p.deleted_at IS NULL)
             FROM categories c",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().collect())
}

/// 汇总某分类自身及全部后代的直接密码数（visited 防历史脏数据成环）
fn sum_subtree_password_count(
    children: &CategoryChildren,
    direct_counts: &HashMap<String, i64>,
    root_id: &str,
) -> i64 {
    let mut visited: HashSet<String> = HashSet::new();
    let mut stack = vec![root_id.to_string()];
    let mut total = 0i64;
    while let Some(id) = stack.pop() {
        if !visited.insert(id.clone()) {
            continue;
        }
        total += direct_counts.get(&id).copied().unwrap_or(0);
        if let Some(kids) = children.get(&id) {
            for child in kids {
                if !visited.contains(child) {
                    stack.push(child.clone());
                }
            }
        }
    }
    total
}

/// 获取分类列表
///
/// 获取所有分类，包含父分类、直接归属密码数与「自身 + 全部后代」的汇总数。
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
pub async fn get_categories(state: State<'_, AppState>) -> ApiResult<Vec<Category>> {
    respond(|| -> Result<Vec<Category>, String> {
        // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
        crate::commands::auth::require_unlocked(&state)?;

        let db = state.get_db()?;
        let conn = db.conn();

        // 查询分类列表（password_count 只统计直接归属且未删除的密码；total_count 由下方 Rust 汇总）
        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.name, c.icon, c.sort_order, c.created_at, c.parent_id,
                        (SELECT COUNT(*) FROM passwords p WHERE p.category_id = c.id AND p.deleted_at IS NULL) AS password_count
                 FROM categories c
                 ORDER BY c.sort_order",
            )
            .map_err(|e| e.to_string())?;

        let mut categories = stmt
            .query_map([], |row| {
                Ok(Category {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    icon: row.get(2)?,
                    sort_order: row.get(3)?,
                    created_at: row.get(4)?,
                    parent_id: row.get(5)?,
                    password_count: row.get(6)?,
                    total_count: 0, // 占位，随后按子树汇总
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        // 汇总 total_count：自身 + 全部后代的直接密码数（visited 防历史脏数据成环）
        let mut children: CategoryChildren = HashMap::new();
        for category in &categories {
            if let Some(parent) = &category.parent_id {
                children
                    .entry(parent.clone())
                    .or_default()
                    .push(category.id.clone());
            }
        }
        let direct_counts: HashMap<String, i64> = categories
            .iter()
            .map(|category| (category.id.clone(), category.password_count))
            .collect();
        for category in &mut categories {
            category.total_count = sum_subtree_password_count(&children, &direct_counts, &category.id);
        }

        Ok(categories)

    })}

/// 添加分类
///
/// 添加一个新的密码分类，可指定父分类（父分类必须存在）。
///
/// # Arguments
///
/// * `name` - 分类名称
/// * `parent_id` - 父分类 ID（可选，缺省/null 表示顶级分类）
/// * `state` - 应用状态
///
/// # Returns
///
/// 新添加的分类
///
/// # 前端调用
///
/// ```typescript
/// const category = await invoke('add_category', { name: '新分类', parentId: 'work' });
/// ```
#[tauri::command]
pub async fn add_category(
    name: String,
    parent_id: Option<String>,
    state: State<'_, AppState>,
) -> ApiResult<Category> {
    respond(|| -> Result<Category, String> {
        // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
        crate::commands::auth::require_unlocked(&state)?;

        let db = state.get_db()?;
        let conn = db.conn();

        // 名称去空白，为空则报错（与 update_category 校验一致）
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("分类名称不能为空".to_string());
        }
        let name = trimmed.to_string();

        // 同名校验（大小写不敏感）
        let dup: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM categories WHERE LOWER(name) = LOWER(?1)",
                params![name],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if dup > 0 {
            return Err("已存在同名分类".to_string());
        }

        // 父分类：trim 空串视为顶级；指定时父分类必须存在
        let parent_id = normalize_parent_id(parent_id);
        if let Some(parent) = &parent_id {
            ensure_parent_exists(&conn, parent)?;
        }

        // 生成唯一 ID
        let id = uuid::Uuid::new_v4().to_string();

        // 获取当前时间戳（毫秒）
        let now = chrono::Utc::now().timestamp_millis();

        // 插入数据库
        conn.execute(
            "INSERT INTO categories (id, name, sort_order, created_at, parent_id) VALUES (?1, ?2, 0, ?3, ?4)",
            params![id, name, now, parent_id],
        ).map_err(|e| e.to_string())?;

        // 返回新添加的分类（新分类没有直接归属的密码，子树总数也为 0）
        Ok(Category {
            id,
            name,
            icon: None,
            sort_order: 0,
            created_at: now,
            parent_id,
            password_count: 0,
            total_count: 0,
        })

    })}

/// 更新分类
///
/// 更新分类的名称、图标或父分类。
///
/// # 父分类语义
///
/// Tauri v2 顶层命令参数对 `Option<T>` 无法区分「键缺失」与「显式 null」
/// （两者都会得到 `None`，见 `tauri::ipc::CommandItem::deserialize_option`），
/// 因此用独立的 `clear_parent` 表达「置为顶级」：
/// - `clear_parent = true`：置为顶级分类；
/// - 否则 `parent_id` 非空字符串：设置父分类（校验存在 + 防环）；
/// - 否则（缺失/null/空串）：不修改父分类。
///
/// # Arguments
///
/// * `id` - 分类 ID
/// * `name` - 新分类名称（可选）
/// * `icon` - 新分类图标（可选）
/// * `parent_id` - 新父分类 ID（可选）
/// * `clear_parent` - 是否置为顶级分类（可选）
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
///   name: '新名称',
///   parentId: 'work'
/// });
/// // 置为顶级分类：
/// await invoke('update_category', { id: '123', clearParent: true });
/// ```
#[tauri::command]
pub async fn update_category(
    id: String,
    name: Option<String>,
    icon: Option<String>,
    parent_id: Option<String>,
    clear_parent: Option<bool>,
    state: State<'_, AppState>,
) -> ApiResult<Category> {
    respond(|| -> Result<Category, String> {
        // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
        crate::commands::auth::require_unlocked(&state)?;

        let db = state.get_db()?;
        let conn = db.conn();

        // 获取当前分类
        let current = conn.query_row(
            "SELECT id, name, icon, sort_order, created_at, parent_id FROM categories WHERE id = ?1",
            params![id],
            |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let icon: Option<String> = row.get(2)?;
                let sort_order: i32 = row.get(3)?;
                let created_at: i64 = row.get(4)?;
                let parent_id: Option<String> = row.get(5)?;
                Ok((id, name, icon, sort_order, created_at, parent_id))
            },
        );

        match current {
            Ok((current_id, current_name, current_icon, sort_order, created_at, current_parent_id)) => {
                // 使用新值或当前值
                let new_icon = icon.or(current_icon);
                let new_name = name.unwrap_or(current_name);

                // 名称去空白，为空则报错
                let trimmed = new_name.trim();
                if trimmed.is_empty() {
                    return Err("分类名称不能为空".to_string());
                }
                let new_name = trimmed.to_string();

                // 同名校验（大小写不敏感，排除自身）
                let dup: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM categories WHERE LOWER(name) = LOWER(?1) AND id != ?2",
                        params![new_name, current_id],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if dup > 0 {
                    return Err("已存在同名分类".to_string());
                }

                // 父分类：clearParent=true 置顶级；parentId 非空=设置；否则不修改
                let new_parent_id = if clear_parent.unwrap_or(false) {
                    None
                } else {
                    match normalize_parent_id(parent_id) {
                        // 未提供/空串：不修改父分类
                        None => current_parent_id,
                        Some(target) => {
                            if target == current_id {
                                return Err("分类层级不能形成环".to_string());
                            }
                            ensure_parent_exists(&conn, &target)?;
                            ensure_no_cycle(&conn, &current_id, &target)?;
                            Some(target)
                        }
                    }
                };

                // 更新数据库
                conn.execute(
                    "UPDATE categories SET name = ?1, icon = ?2, parent_id = ?3 WHERE id = ?4",
                    params![new_name, new_icon, new_parent_id, current_id],
                ).map_err(|e| e.to_string())?;

                // 直接归属的未删除密码数（用于返回，与实际查询保持一致）
                let password_count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM passwords WHERE category_id = ?1 AND deleted_at IS NULL",
                        params![current_id],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;

                // 自身 + 全部后代的直接密码数汇总（后端计算，前端直接展示）
                let children = load_category_children(&conn)?;
                let direct_counts = load_direct_password_counts(&conn)?;
                let total_count = sum_subtree_password_count(&children, &direct_counts, &current_id);

                // 返回更新后的分类
                Ok(Category {
                    id: current_id,
                    name: new_name,
                    icon: new_icon,
                    sort_order,
                    created_at,
                    parent_id: new_parent_id,
                    password_count,
                    total_count,
                })
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Err("未找到分类".to_string()),
            Err(e) => Err(e.to_string()),
        }

    })}

/// 删除分类
///
/// 删除指定的分类：子分类上移一级（parent_id 改为被删分类的 parent_id），
/// 该分类下的密码移到「其他」分类；内置「其他」分类不可删除。
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
) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
        crate::commands::auth::require_unlocked(&state)?;

        let db = state.get_db()?;
        let conn = db.conn();

        // 内置兜底分类不可删除：所有被删分类下的密码都重归属到 'other'
        if id == "other" {
            return Err("内置分类不可删除".to_string());
        }

        // 多表写入（分类上移/密码重归属/删除分类）同事务，保证原子性
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        // 读取被删分类的父分类（子分类上移一级后挂到它下面）
        let parent_id: Option<String> = tx
            .query_row(
                "SELECT parent_id FROM categories WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => "未找到分类".to_string(),
                _ => e.to_string(),
            })?;

        // 确保兜底分类存在（历史数据中 'other' 可能已被删除，防止产生悬空引用）
        let now = chrono::Utc::now().timestamp_millis();
        tx.execute(
            "INSERT OR IGNORE INTO categories (id, name, sort_order, created_at) VALUES ('other', '其他', 4, ?1)",
            params![now],
        ).map_err(|e| e.to_string())?;

        // 子分类上移一级：parent_id 改为被删分类的 parent_id
        tx.execute(
            "UPDATE categories SET parent_id = ?1 WHERE parent_id = ?2",
            params![parent_id, id],
        ).map_err(|e| e.to_string())?;

        // 将该分类下的密码移到"其他"分类（软删除密码一并重归属，保持数据一致）
        tx.execute(
            "UPDATE passwords SET category_id = 'other' WHERE category_id = ?1",
            params![id],
        ).map_err(|e| e.to_string())?;

        // 删除分类
        tx.execute("DELETE FROM categories WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;

        tx.commit().map_err(|e| e.to_string())?;

        Ok(())

    })}

/// 批量调整分类排序（前端拖拽排序后提交同一父分类下的新顺序）
///
/// 只调整 `sort_order`（按数组下标从 0 递增写入），不改变父分类关系；
/// 多分类更新在同一事务内完成，任一分类不存在则整体回滚。
///
/// # Arguments
///
/// * `ordered_ids` - 同一父分类下按新顺序排列的分类 ID
/// * `state` - 应用状态
///
/// # 前端调用
///
/// ```typescript
/// await invoke('reorder_categories', { orderedIds: ['c', 'a', 'b'] });
/// ```
#[tauri::command]
pub async fn reorder_categories(
    ordered_ids: Vec<String>,
    state: State<'_, AppState>,
) -> ApiResult<()> {
    respond(|| -> Result<(), String> {
        // 命令门禁：未解锁（含首跑未设主密码）一律拒绝
        crate::commands::auth::require_unlocked(&state)?;

        if ordered_ids.is_empty() {
            return Ok(());
        }

        let db = state.get_db()?;
        let conn = db.conn();
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        for (index, id) in ordered_ids.iter().enumerate() {
            let affected = tx
                .execute(
                    "UPDATE categories SET sort_order = ?1 WHERE id = ?2",
                    params![index as i32, id],
                )
                .map_err(|e| e.to_string())?;
            if affected == 0 {
                // 分类不存在：整体回滚，避免只改了一半的顺序
                return Err("未找到分类".to_string());
            }
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(())

    })}
