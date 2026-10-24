/**
 * password.ts - 密码数据管理 Store（v2：后端搜索 / 筛选 / 排序 / 分页）
 *
 * 使用 Pinia 管理密码相关的数据与操作：
 * 1. 搜索 / 标签 / 分类（含后代）/ 智能视图 / 排序 / 分页 **全部由后端完成**，
 *    前端只提交查询条件并展示后端返回的当前页（`get_passwords` → `PasswordPage`）。
 * 2. 分类树只做结构展示（depth/parentId/hasChildren；directCount 取后端 `password_count`，
 *    totalCount 取后端 `total_count`），不再在前端聚合。
 * 3. 全部 / 收藏 / 最近使用数量来自 `get_password_stats`（`loadStats`）。
 * 4. 筛选 / 排序 / 每页条数变化自动重置第 1 页并重新请求；搜索词 200ms 防抖；
 *    请求序号防止慢的旧响应覆盖新响应。
 * 5. 增删改密码 / 收藏 / 标签 / 分类等写操作成功后刷新当前页 + 统计（必要时 categories/tags）。
 *
 * 契约见：实现方案/密码分类与快速检索优化方案.md（§5 + §11.3）。
 *
 * @example
 * ```ts
 * const store = usePasswordStore()
 * await Promise.all([store.loadPasswords(), store.loadStats(), store.loadCategories(), store.loadTags()])
 *
 * store.setActiveCategory('work') // 后端过滤（含后代分类），自动重置到第 1 页
 * store.setActiveTags(['db'])     // 标签 AND
 * store.setSearchQuery('github 工作') // 200ms 防抖后请求后端
 * store.setSort('use_count', 'desc')
 * store.pagedPasswords            // = passwords（后端返回的当前页）
 * ```
 */

import { defineStore } from 'pinia'
import { ref, computed, watch } from 'vue'
import { invoke } from '@/lib/api'

// ==================== 类型定义 ====================

/**
 * 自定义字段接口定义
 * 一条密码可携带任意多个附加字段（如数据库连接地址/端口/连接命令）
 */
export interface CustomField {
  /** 字段名，如 "连接地址" */
  label: string
  /** 字段值（列表响应中敏感字段为空串，详情解密后回填） */
  value: string
  /** 是否敏感（敏感字段值加密存储，仅详情下发明文） */
  sensitive: boolean
}

/**
 * 密码项接口定义
 * 描述单条密码记录的数据结构（列表/详情均不含明文 password）
 */
export interface PasswordItem {
  /** 唯一标识符 */
  id: string
  /** 网站/应用名称，如 "GitHub" */
  title: string
  /** 登录用户名或邮箱 */
  username: string
  /** 登录密码（列表/详情均不回传明文，保持空串） */
  password: string
  /** 网站 URL（可选） */
  url?: string
  /** 备注明文（仅 get_password_detail 返回；列表不下发） */
  notes?: string
  /** 是否有备注（列表用；备注明文按需通过 get_password_detail 获取） */
  has_notes?: boolean
  /** 自定义字段（列表中敏感字段 value 为空串） */
  extra_fields?: CustomField[]
  /** 所属分类 ID */
  category: string
  /** 是否收藏 */
  is_favorite: boolean
  /** 密码强度等级（后端计算，0-4；未开启强度显示时为空） */
  password_strength?: number
  /** 创建时间（毫秒时间戳） */
  created_at: number
  /** 更新时间（毫秒时间戳） */
  updated_at: number
  /** 标签列表（明文，顺序为录入顺序；大小写不敏感去重由后端保证） */
  tags: string[]
  /** 最近使用时间（毫秒时间戳；从未使用为 null/undefined） */
  last_used_at?: number | null
  /** 使用次数（从未使用为 0） */
  use_count: number
}

/**
 * `get_passwords` 分页返回（契约 §11.2 PasswordPage）
 */
export interface PasswordPage {
  /** 当前页数据（列表零解密） */
  items: PasswordItem[]
  /** 过滤后的总条数 */
  total: number
  /** 实际返回页码（越界已由后端钳制） */
  page: number
  /** 每页条数 */
  page_size: number
}

/**
 * `get_password_stats` 返回（契约 §11.2 PasswordStats）
 */
export interface PasswordStats {
  /** 全部密码数 */
  all: number
  /** 收藏数 */
  favorite: number
  /** 最近使用数（有使用记录） */
  recent: number
}

/**
 * 全局标签信息（`get_tags` 返回）
 */
export interface TagInfo {
  /** 标签展示名 */
  name: string
  /** 含该标签的未删除密码数 */
  count: number
}

/**
 * 分类接口定义
 * `all` / `favorite` 为前端虚拟分类，其余来自后端
 */
export interface Category {
  /** 分类 ID，如 "work", "social" */
  id: string
  /** 分类显示名称，如 "工作", "社交媒体" */
  name: string
  /** 分类图标（可选，预留） */
  icon?: string
  /** 同级排序序号（后端返回，可选；拖拽排序按此展示） */
  sort_order?: number
  /** 创建时间（毫秒时间戳，可选） */
  created_at?: number
  /** 父分类 ID；null/undefined 表示顶级分类 */
  parent_id?: string | null
  /** 直接归属该分类的未删除密码数（后端返回） */
  password_count?: number
  /** 含全部后代分类的未删除密码总数（后端返回，前端直接展示） */
  total_count?: number
}

/** 列表排序字段 */
export type SortKey = 'updated_at' | 'created_at' | 'title' | 'last_used_at' | 'use_count'

/** 列表排序方向 */
export type SortOrder = 'asc' | 'desc'

/**
 * 分类树节点（供侧边栏 / 分类选择器 / 拖拽排序渲染）
 * 由 `categoryTree` 按深度优先顺序扁平输出（父节点后紧跟其子节点）。
 */
export interface CategoryTreeNode {
  /** 分类 ID */
  id: string
  /** 分类显示名 */
  name: string
  /** 父分类 ID；根节点为 null */
  parentId: string | null
  /** 层级深度，根节点为 0 */
  depth: number
  /** 是否有（可见的）子分类 */
  hasChildren: boolean
  /** 直接归属该分类的密码数（后端 password_count） */
  directCount: number
  /** 含所有后代的密码总数（后端 total_count） */
  totalCount: number
}

/**
 * 新增密码请求
 * `tags` 缺省视为空数组（后端契约：NewPassword.tags 可选）
 */
export interface NewPassword {
  title: string
  username: string
  password: string
  url?: string
  notes?: string
  extra_fields?: CustomField[]
  category: string
  is_favorite: boolean
  /** 标签列表（可选，最多 20 个） */
  tags?: string[]
}

/**
 * 更新密码请求
 *
 * url/notes 三层语义：字段缺失=不修改、null=清空、字符串=赋值
 * extra_fields/tags 三层语义：字段缺失=不修改、null/空数组=清空、数组=整体替换
 */
export interface UpdatePassword {
  title?: string
  username?: string
  password?: string
  url?: string | null
  notes?: string | null
  extra_fields?: CustomField[] | null
  category?: string
  is_favorite?: boolean
  /** 标签：缺失=不修改、null/[]=清空、数组=整体替换 */
  tags?: string[] | null
}

/** `get_passwords` 查询参数（invoke 键使用 camelCase） */
type GetPasswordsParams = {
  search: string | null
  category: string | null
  tags: string[] | null
  sortBy: SortKey
  sortOrder: SortOrder
  page: number
  pageSize: number
}

// ==================== 常量与纯函数 ====================

/** 默认每页条数（契约 5.2） */
export const DEFAULT_PAGE_SIZE = 50

/** 可选每页条数（契约 6.2） */
export const PAGE_SIZE_OPTIONS = [20, 50, 100, 200] as const

/** 搜索词防抖时间（毫秒） */
export const SEARCH_DEBOUNCE_MS = 200

/** 前端虚拟分类/视图 ID（不来自后端，也不参与分类树） */
const VIRTUAL_CATEGORY_IDS: ReadonlySet<string> = new Set(['all', 'favorite', 'recent'])

/** 分类名称排序规则 */
const TITLE_COLLATOR_OPTIONS: Intl.CollatorOptions = { numeric: true, sensitivity: 'base' }

/**
 * 创建虚拟分类（固定在最前，非后端数据）
 */
function createVirtualCategories(): Category[] {
  return [
    { id: 'all', name: '全部' },
    { id: 'favorite', name: '收藏' },
  ]
}

/**
 * 把后端计数数值规范化成非负整数（异常值回退 0）
 */
function normalizeCount(value: unknown): number {
  const numeric = Number(value ?? 0)
  return Number.isFinite(numeric) && numeric > 0 ? Math.trunc(numeric) : 0
}

/**
 * 把后端页码规范化为 >= 1 的整数（异常值回退 fallback）
 */
function normalizePageNumber(value: unknown, fallback: number): number {
  const numeric = Number(value ?? fallback)
  return Number.isFinite(numeric) && numeric >= 1 ? Math.trunc(numeric) : fallback
}

/**
 * 规范化后端返回的密码项：
 * 兼容旧后端/旧导出数据缺失 `tags`、`use_count`、`last_used_at` 的情况。
 */
function normalizePassword(item: PasswordItem): PasswordItem {
  return {
    ...item,
    tags: item.tags ?? [],
    use_count: item.use_count ?? 0,
    last_used_at: item.last_used_at ?? null,
    extra_fields: item.extra_fields ?? [],
  }
}

/**
 * 规范化后端返回的分类项：
 * `parent_id` 缺省为 null（顶级分类）；`password_count`/`total_count` 缺省为 0。
 */
function normalizeCategory(category: Category): Category {
  return {
    ...category,
    parent_id: category.parent_id ?? null,
    password_count: normalizeCount(category.password_count),
    total_count: normalizeCount(category.total_count),
  }
}

/**
 * 计算分类的“有效父分类 ID”：
 * - 父分类为空 → 顶级（null）
 * - 父分类指向自身或指向不存在的分类 → 视为顶级（容错脏数据）
 */
function resolveParentId(category: Category, categoryById: Map<string, Category>): string | null {
  const parentId = category.parent_id
  if (!parentId) return null
  if (parentId === category.id) return null
  if (!categoryById.has(parentId)) return null
  return parentId
}

/**
 * 分类排序：sort_order 升序 → 名称（中文规则）→ id（保证稳定）
 * 拖拽排序后本地乐观更新的 sort_order 会立即反映到分类树顺序。
 */
function compareCategoryOrder(a: Category, b: Category): number {
  return (
    (a.sort_order ?? 0) - (b.sort_order ?? 0) ||
    a.name.localeCompare(b.name, 'zh-Hans-CN', TITLE_COLLATOR_OPTIONS) ||
    a.id.localeCompare(b.id)
  )
}

// ==================== Store ====================

/**
 * 密码数据 Store（后端驱动版）
 */
export const usePasswordStore = defineStore('password', () => {
  // ==================== 状态 ====================

  /**
   * 分类列表
   * 前两项固定为虚拟分类：`all`（全部）、`favorite`（收藏）；
   * `recent`（最近使用）仅作为 `activeCategory` 的视图 ID，不在此数组中。
   */
  const categories = ref<Category[]>(createVirtualCategories())

  /** 全局标签列表（`get_tags`） */
  const tags = ref<TagInfo[]>([])

  /** 密码当前页列表（后端 `get_passwords` 返回的 items，前端不再过滤/排序/切片） */
  const passwords = ref<PasswordItem[]>([])

  /** 后端过滤后的总条数（`PasswordPage.total`） */
  const totalFiltered = ref<number>(0)

  /** 全部 / 收藏 / 最近使用数量（`get_password_stats`） */
  const stats = ref<PasswordStats>({ all: 0, favorite: 0, recent: 0 })

  /** 当前选中的分类/视图：'all' | 'favorite' | 'recent' | 后端分类 id */
  const activeCategory = ref<string>('all')

  /** 多选标签（AND 筛选，大小写不敏感） */
  const activeTags = ref<string[]>([])

  /** 搜索关键词（后端分词 AND，含拼音全拼/简拼） */
  const searchQuery = ref<string>('')

  /** 排序字段 */
  const sortBy = ref<SortKey>('updated_at')

  /** 排序方向 */
  const sortOrder = ref<SortOrder>('desc')

  /** 每页条数 */
  const pageSize = ref<number>(DEFAULT_PAGE_SIZE)

  /** 当前页码（从 1 开始，以请求值为准；后端返回的 page 会回写） */
  const currentPage = ref<number>(1)

  /** 加载状态（仅密码列表请求） */
  const loading = ref<boolean>(false)

  /** 错误信息（最近一次密码列表请求失败） */
  const error = ref<string | null>(null)

  // ==================== 请求控制（非响应式内部状态） ====================

  /** 密码列表请求序号：只有最新请求允许写入状态 */
  let requestSeq = 0

  /** 统计请求序号：避免旧响应覆盖新统计 */
  let statsSeq = 0

  /** 后端回写页码时抑制 currentPage watch 的重复请求 */
  let suppressPageFetch = false

  /** 写操作内部批量修改筛选状态时抑制 watch 的自动请求（避免重复请求） */
  let suppressFilterFetch = false

  /** 搜索防抖定时器 */
  let searchTimer: ReturnType<typeof setTimeout> | null = null

  /** 最近一次请求实际发送的搜索词（trim 后；空串表示 null），用于防抖去重 */
  let lastRequestedSearch = ''

  // ==================== 分类树基础计算 ====================

  /** 后端分类（不含虚拟分类/视图） */
  const backendCategories = computed<Category[]>(() =>
    categories.value.filter((category) => !VIRTUAL_CATEGORY_IDS.has(category.id)),
  )

  /** 分类 id → 分类（重复 id 时保留第一个） */
  const categoryById = computed<Map<string, Category>>(() => {
    const map = new Map<string, Category>()
    for (const category of backendCategories.value) {
      if (!map.has(category.id)) map.set(category.id, category)
    }
    return map
  })

  /** 父分类 id → 直接子分类列表（已按 sort_order / 名称排序） */
  const childrenByParent = computed<Map<string, Category[]>>(() => {
    const map = new Map<string, Category[]>()
    const byId = categoryById.value
    for (const category of backendCategories.value) {
      const parentId = resolveParentId(category, byId)
      if (parentId === null) continue
      const siblings = map.get(parentId)
      if (siblings) siblings.push(category)
      else map.set(parentId, [category])
    }
    for (const siblings of map.values()) siblings.sort(compareCategoryOrder)
    return map
  })

  // ==================== 计算属性 ====================

  /** 分类 id → 名称（含虚拟分类；无原型对象避免 id 命中原型链键） */
  const categoryNameById = computed<Record<string, string>>(() => {
    const map = Object.create(null) as Record<string, string>
    for (const category of categories.value) {
      if (map[category.id] === undefined) map[category.id] = category.name
    }
    return map
  })

  /**
   * 分类树（仅供展示 / 选择器 / 拖拽排序，不含 all/favorite/recent）
   *
   * - 任意深度 DFS，父节点后紧跟其子节点；
   * - `directCount` 取后端 `password_count`，`totalCount` 取后端 `total_count`（前端不聚合）；
   * - 使用 visited 防止脏数据（父分类成环）导致死循环。
   */
  const categoryTree = computed<CategoryTreeNode[]>(() => {
    const byId = categoryById.value
    const childrenMap = childrenByParent.value
    const visited = new Set<string>()
    const nodes: CategoryTreeNode[] = []

    /**
     * 深度优先构建节点
     * @param category - 当前分类
     * @param depth - 当前层级
     * @param parentId - 父分类 id（根节点传 null）
     */
    const build = (category: Category, depth: number, parentId: string | null): void => {
      visited.add(category.id)

      // 子分类需排除自身与已访问节点（截断脏数据环）
      const childCategories = (childrenMap.get(category.id) ?? []).filter(
        (child) => child.id !== category.id && !visited.has(child.id),
      )

      nodes.push({
        id: category.id,
        name: category.name,
        parentId,
        depth,
        hasChildren: childCategories.length > 0,
        directCount: category.password_count ?? 0,
        totalCount: category.total_count ?? 0,
      })

      for (const child of childCategories) build(child, depth + 1, category.id)
    }

    // 第一轮：正常根节点（parent_id 为空 / 指向不存在分类 / 指向自身）
    const roots = backendCategories.value
      .filter((category) => resolveParentId(category, byId) === null)
      .sort(compareCategoryOrder)
    for (const root of roots) {
      if (visited.has(root.id)) continue
      build(root, 0, null)
    }

    // 第二轮：脏数据兜底（例如 a→b→a 成环，没有可识别的根）
    const leftovers = backendCategories.value
      .filter((category) => !visited.has(category.id))
      .sort(compareCategoryOrder)
    for (const leftover of leftovers) {
      if (visited.has(leftover.id)) continue
      build(leftover, 0, null)
    }

    return nodes
  })

  /** 当前页数据（后端已分页，前端直接展示） */
  const pagedPasswords = computed<PasswordItem[]>(() => passwords.value)

  /** 总页数（至少 1） */
  const pageCount = computed<number>(() =>
    Math.max(1, Math.ceil(totalFiltered.value / Math.max(1, pageSize.value))),
  )

  /** 全部密码数（来自后端统计） */
  const allCount = computed<number>(() => stats.value.all)

  /** 收藏数（来自后端统计） */
  const favoriteCount = computed<number>(() => stats.value.favorite)

  /** 最近使用数（来自后端统计） */
  const recentCount = computed<number>(() => stats.value.recent)

  /** 是否有激活的筛选条件（搜索词 / 选中标签 / 非默认视图分类） */
  const hasActiveFilters = computed<boolean>(() => {
    if (searchQuery.value.trim().length > 0) return true
    if (activeTags.value.length > 0) return true
    const category = activeCategory.value
    return category !== 'all' && category !== 'favorite' && category !== 'recent'
  })

  /** 当前视图/分类的展示名（空状态与标题用） */
  const activeCategoryName = computed<string>(() => {
    const category = activeCategory.value
    if (category === 'all') return '全部'
    if (category === 'favorite') return '收藏'
    if (category === 'recent') return '最近使用'
    return categoryNameById.value[category] ?? category
  })

  // ==================== 内部工具 ====================

  /**
   * 取消待执行的搜索防抖请求
   */
  function cancelPendingSearch(): void {
    if (searchTimer !== null) {
      clearTimeout(searchTimer)
      searchTimer = null
    }
  }

  /**
   * 组装 `get_passwords` 查询参数（契约 §11.2 / §11.3）
   * - search 为空（含仅空白）传 null
   * - category 为 'all' 传 null
   * - tags 为空传 null
   */
  function buildQueryParams(): GetPasswordsParams {
    const keyword = searchQuery.value.trim()
    const tagsValue = activeTags.value.length > 0 ? [...activeTags.value] : null
    return {
      search: keyword.length > 0 ? keyword : null,
      category: activeCategory.value === 'all' ? null : activeCategory.value,
      tags: tagsValue,
      sortBy: sortBy.value,
      sortOrder: sortOrder.value,
      page: currentPage.value,
      pageSize: pageSize.value,
    }
  }

  /**
   * 从第 1 页重新请求（筛选/排序/每页条数/搜索变化时使用）
   * 页码不是 1 时先回写 1，由 currentPage watch 发起请求。
   */
  function requestFirstPage(): void {
    if (currentPage.value !== 1) {
      currentPage.value = 1
      return
    }
    void loadPasswords()
  }

  // ==================== 数据加载 ====================

  /**
   * 按当前筛选/排序/分页条件从后端加载密码当前页
   *
   * - 请求序号：只有最新请求能写入 passwords/totalFiltered/currentPage，防止旧响应覆盖；
   * - 后端返回的 page 会回写 currentPage（越界钳制由后端完成）；
   * - 发起请求时取消待执行的搜索防抖（本次请求已带最新搜索词）。
   */
  async function loadPasswords(): Promise<void> {
    cancelPendingSearch()

    const seq = ++requestSeq
    const params = buildQueryParams()
    lastRequestedSearch = params.search ?? ''
    loading.value = true
    error.value = null

    try {
      const pageResult = (await invoke<PasswordPage>('get_passwords', { ...params })) ?? null
      // 旧响应：已有更新的请求发出，丢弃本次结果
      if (seq !== requestSeq) return

      if (!pageResult) {
        passwords.value = []
        totalFiltered.value = 0
        return
      }

      passwords.value = (pageResult.items ?? []).map(normalizePassword)
      totalFiltered.value = normalizeCount(pageResult.total)

      const serverPage = normalizePageNumber(pageResult.page, params.page)
      if (serverPage !== currentPage.value) {
        suppressPageFetch = true
        currentPage.value = serverPage
      }
    } catch (e) {
      if (seq !== requestSeq) return
      error.value = String(e)
      console.error('加载密码失败:', e)
    } finally {
      if (seq === requestSeq) loading.value = false
    }
  }

  /**
   * 加载全部 / 收藏 / 最近使用数量（`get_password_stats`）
   */
  async function loadStats(): Promise<void> {
    const seq = ++statsSeq
    try {
      const result = (await invoke<PasswordStats>('get_password_stats')) ?? null
      if (seq !== statsSeq || !result) return
      stats.value = {
        all: normalizeCount(result.all),
        favorite: normalizeCount(result.favorite),
        recent: normalizeCount(result.recent),
      }
    } catch (e) {
      console.error('加载密码统计失败:', e)
    }
  }

  /**
   * 从后端加载分类列表
   * 虚拟分类（全部/收藏）始终固定在最前；分类的 password_count/total_count 由后端返回。
   */
  async function loadCategories(): Promise<void> {
    try {
      const result = await invoke<Category[]>('get_categories')
      const backend = (result ?? [])
        .filter((category) => !VIRTUAL_CATEGORY_IDS.has(category.id))
        .map(normalizeCategory)
      categories.value = [...createVirtualCategories(), ...backend]
    } catch (e) {
      console.error('加载分类失败:', e)
    }
  }

  /**
   * 从后端加载全局标签列表（`get_tags` 已按 count DESC、name 排序）
   */
  async function loadTags(): Promise<void> {
    try {
      const result = await invoke<TagInfo[]>('get_tags')
      tags.value = result ?? []
    } catch (e) {
      console.error('加载标签失败:', e)
    }
  }

  /**
   * 刷新当前页 + 统计（写操作后的通用刷新）
   */
  async function refreshPageAndStats(): Promise<void> {
    await Promise.all([loadPasswords(), loadStats()])
  }

  // ==================== 筛选 / 排序 / 分页状态操作 ====================

  /**
   * 设置当前选中的分类/视图（watch 自动重置页码并请求）
   * @param id - 'all' | 'favorite' | 'recent' | 分类 id
   */
  function setActiveCategory(id: string): void {
    activeCategory.value = id
  }

  /**
   * 设置搜索关键词（watch 内 200ms 防抖，自动重置第 1 页）
   * @param query - 搜索关键词
   */
  function setSearchQuery(query: string): void {
    searchQuery.value = query
  }

  /**
   * 切换一个标签的选中状态（大小写不敏感去重；watch 自动重置页码并请求）
   * @param name - 标签名
   */
  function toggleTag(name: string): void {
    const normalized = name.trim()
    if (!normalized) return
    const lower = normalized.toLowerCase()
    const index = activeTags.value.findIndex((tag) => tag.trim().toLowerCase() === lower)
    if (index === -1) {
      activeTags.value = [...activeTags.value, normalized]
    } else {
      activeTags.value = activeTags.value.filter((_, i) => i !== index)
    }
  }

  /**
   * 整体设置选中的标签（自动 trim、去空、大小写不敏感去重）
   * @param names - 标签名数组
   */
  function setActiveTags(names: string[]): void {
    const seen = new Set<string>()
    const next: string[] = []
    for (const raw of names) {
      const name = raw.trim()
      if (!name) continue
      const lower = name.toLowerCase()
      if (seen.has(lower)) continue
      seen.add(lower)
      next.push(name)
    }
    activeTags.value = next
  }

  /** 清空所有选中标签 */
  function clearTags(): void {
    activeTags.value = []
  }

  /**
   * 设置排序（watch 自动重置第 1 页并请求）
   * @param key - 排序字段
   * @param order - 排序方向；不传时：同一字段切换升降序，新字段默认 title 升序、其余降序
   */
  function setSort(key: SortKey, order?: SortOrder): void {
    // 一次设置两个字段，抑制 watch 中间态触发两次请求
    suppressFilterFetch = true
    try {
      if (order !== undefined) {
        sortBy.value = key
        sortOrder.value = order
      } else if (sortBy.value === key) {
        sortOrder.value = sortOrder.value === 'asc' ? 'desc' : 'asc'
      } else {
        sortBy.value = key
        sortOrder.value = key === 'title' ? 'asc' : 'desc'
      }
    } finally {
      suppressFilterFetch = false
    }
    requestFirstPage()
  }

  /**
   * 设置当前页（越界自动钳制到 [1, pageCount]；watch 自动请求）
   * @param page - 页码（从 1 开始）
   */
  function setPage(page: number): void {
    const next = Math.trunc(Number(page))
    if (!Number.isFinite(next)) return
    currentPage.value = Math.min(Math.max(1, next), pageCount.value)
  }

  /**
   * 设置每页条数（watch 自动重置到第 1 页并请求）
   * @param size - 每页条数（20/50/100/200）
   */
  function setPageSize(size: number): void {
    const next = Math.trunc(Number(size))
    if (!Number.isFinite(next) || next <= 0) return
    // 抑制 pageSize 变化触发的筛选 watch，统一由 requestFirstPage 发起一次请求
    suppressFilterFetch = true
    try {
      pageSize.value = next
    } finally {
      suppressFilterFetch = false
    }
    requestFirstPage()
  }

  // ==================== 使用记录 ====================

  /**
   * 记录密码使用（复制/注入时调用；UI 可 fire-and-forget）
   *
   * 乐观更新：先就地更新本地 `last_used_at`/`use_count`，失败时回滚并静默。
   * 若当前排序依赖使用记录（last_used_at/use_count）或处于「最近使用」视图，
   * 成功后刷新当前页 + 统计，保证列表顺序/计数及时准确。
   * @param id - 密码 ID
   */
  async function touchUsage(id: string): Promise<void> {
    const item = passwords.value.find((password) => password.id === id)
    const previousLastUsedAt = item?.last_used_at ?? null
    const previousUseCount = item?.use_count ?? 0

    if (item) {
      item.last_used_at = Date.now()
      item.use_count = previousUseCount + 1
    }

    try {
      await invoke('touch_password_usage', { id })

      const sortAffected = sortBy.value === 'last_used_at' || sortBy.value === 'use_count'
      if (sortAffected || activeCategory.value === 'recent') {
        await refreshPageAndStats()
      }
    } catch (e) {
      // 失败静默：回滚乐观更新，不打断复制
      if (item) {
        item.last_used_at = previousLastUsedAt
        item.use_count = previousUseCount
      }
      console.debug('记录密码使用失败:', e)
    }
  }

  // ==================== 标签管理 ====================

  /**
   * 重命名标签（后端可能合并同名标签）
   * 成功后刷新当前页 + 统计 + 标签列表；同步更新本地已选标签。
   * @param oldName - 原标签名
   * @param newName - 新标签名
   */
  async function renameTag(oldName: string, newName: string): Promise<void> {
    await invoke('rename_tag', { oldName, newName })

    const oldLower = oldName.trim().toLowerCase()
    const newTrimmed = newName.trim()
    // 已选标签同步改名；抑制 watch 的自动请求，统一在下面刷新一次
    suppressFilterFetch = true
    try {
      activeTags.value = activeTags.value.map((tag) =>
        tag.trim().toLowerCase() === oldLower ? newTrimmed : tag,
      )
    } finally {
      suppressFilterFetch = false
    }

    await Promise.all([loadPasswords(), loadStats(), loadTags()])
  }

  /**
   * 删除标签
   * 成功后刷新当前页 + 统计 + 标签列表，并从已选标签中移除。
   * @param name - 标签名
   */
  async function deleteTag(name: string): Promise<void> {
    await invoke('delete_tag', { name })

    const lower = name.trim().toLowerCase()
    suppressFilterFetch = true
    try {
      activeTags.value = activeTags.value.filter((tag) => tag.trim().toLowerCase() !== lower)
    } finally {
      suppressFilterFetch = false
    }

    await Promise.all([loadPasswords(), loadStats(), loadTags()])
  }

  // ==================== 密码增删改 ====================

  /**
   * 添加新密码
   * 成功后刷新当前页 + 统计 + 分类/标签（计数与标签列表可能变化）。
   * @param item - 密码数据（可含 tags）
   */
  async function addPassword(item: NewPassword): Promise<PasswordItem> {
    loading.value = true
    error.value = null

    try {
      const result = await invoke<PasswordItem>('add_password', { data: item })
      await Promise.all([loadPasswords(), loadStats(), loadCategories(), loadTags()])
      return normalizePassword(result)
    } catch (e) {
      error.value = String(e)
      console.error('添加密码失败:', e)
      throw e
    } finally {
      loading.value = false
    }
  }

  /**
   * 更新密码
   * 成功后刷新当前页 + 统计 + 分类/标签。
   * @param id - 要更新的密码 ID
   * @param data - 要更新的字段（url/notes/tags 缺失=不修改、null=清空）
   */
  async function updatePassword(id: string, data: UpdatePassword): Promise<PasswordItem> {
    loading.value = true
    error.value = null

    try {
      const result = await invoke<PasswordItem>('update_password', { id, data })
      await Promise.all([loadPasswords(), loadStats(), loadCategories(), loadTags()])
      return normalizePassword(result)
    } catch (e) {
      error.value = String(e)
      console.error('更新密码失败:', e)
      throw e
    } finally {
      loading.value = false
    }
  }

  /**
   * 获取单条密码详情（含备注明文，按需调用）
   *
   * 列表不下发备注明文，编辑/查看详情时才调用本方法。
   * @param id - 密码 ID
   */
  async function getPasswordDetail(id: string): Promise<PasswordItem> {
    const result = await invoke<PasswordItem>('get_password_detail', { id })
    return normalizePassword(result)
  }

  /**
   * 删除密码
   * 成功后刷新当前页 + 统计 + 分类/标签。
   * @param id - 要删除的密码 ID
   */
  async function deletePassword(id: string): Promise<void> {
    loading.value = true
    error.value = null

    try {
      await invoke('delete_password', { id })
      await Promise.all([loadPasswords(), loadStats(), loadCategories(), loadTags()])
    } catch (e) {
      error.value = String(e)
      console.error('删除密码失败:', e)
      throw e
    } finally {
      loading.value = false
    }
  }

  /**
   * 切换收藏状态
   * 成功后刷新当前页 + 统计（收藏视图下条目可能移出当前页）。
   * @param id - 要切换收藏的密码 ID
   */
  async function toggleFavorite(id: string): Promise<void> {
    try {
      await invoke<boolean>('toggle_favorite', { id })
      await refreshPageAndStats()
    } catch (e) {
      console.error('切换收藏失败:', e)
    }
  }

  // ==================== 分类管理 ====================

  /**
   * 添加新分类
   * @param name - 分类名称
   * @param parentId - 父分类 ID；不传表示不指定（顶级），null 显式置顶级
   */
  async function addCategory(name: string, parentId?: string | null): Promise<Category> {
    try {
      // parentId 为 undefined 时不传该字段
      const params: { name: string; parentId?: string | null } = { name }
      if (parentId !== undefined) params.parentId = parentId
      const result = await invoke<Category>('add_category', { ...params })

      const normalized = normalizeCategory(result)
      categories.value.push(normalized)
      await Promise.all([loadCategories(), loadPasswords(), loadStats()])
      return normalized
    } catch (e) {
      console.error('添加分类失败:', e)
      throw e
    }
  }

  /**
   * 更新分类（名称 / 父分类）
   *
   * parentId 三层语义（契约 5.4，注意 Tauri 2 顶层 Option 无法区分键缺失与 null）：
   * - undefined：参数对象不含 `parentId`/`clearParent`，表示不修改
   * - null：传 `clearParent: true`，表示置为顶级分类
   * - string：传 `parentId: <id>`，表示设置父分类
   *
   * @param id - 分类 ID
   * @param name - 新分类名称（undefined 时不修改）
   * @param parentId - 父分类 ID（undefined 时不修改）
   */
  async function updateCategory(
    id: string,
    name?: string,
    parentId?: string | null,
  ): Promise<Category> {
    try {
      const params: { id: string; name?: string; parentId?: string; clearParent?: boolean } = {
        id,
      }
      if (name !== undefined) params.name = name
      if (parentId === null) {
        // 置顶级：Tauri 顶层 Option 无法用 null 表达“清空”，改传 clearParent
        params.clearParent = true
      } else if (parentId !== undefined) {
        params.parentId = parentId
      }
      const result = await invoke<Category>('update_category', { ...params })

      const normalized = normalizeCategory(result)
      const index = categories.value.findIndex((category) => category.id === id)
      if (index !== -1) categories.value[index] = normalized

      await Promise.all([loadCategories(), loadPasswords(), loadStats()])
      return normalized
    } catch (e) {
      console.error('更新分类失败:', e)
      throw e
    }
  }

  /**
   * 删除分类
   * 若删除的是当前选中分类则回到「全部」视图；成功后刷新分类/当前页/统计。
   * @param id - 要删除的分类 ID
   */
  async function deleteCategory(id: string): Promise<void> {
    try {
      await invoke('delete_category', { id })

      if (activeCategory.value === id) {
        // 回到「全部」；抑制 watch 自动请求，统一在下面刷新一次
        suppressFilterFetch = true
        try {
          activeCategory.value = 'all'
        } finally {
          suppressFilterFetch = false
        }
      }
      categories.value = categories.value.filter((category) => category.id !== id)

      await Promise.all([loadCategories(), loadPasswords(), loadStats()])
    } catch (e) {
      console.error('删除分类失败:', e)
      throw e
    }
  }

  /**
   * 批量调整分类顺序（拖拽排序）
   *
   * 仅提交同一父分类下的兄弟分类新顺序；先乐观更新本地 sort_order，
   * 后端失败时回滚并抛出错误（由调用方提示）。
   * @param orderedIds - 同一父分类下按新顺序排列的分类 ID
   */
  async function reorderCategories(orderedIds: string[]): Promise<void> {
    if (orderedIds.length === 0) return

    const previous = categories.value
    const orderMap = new Map<string, number>()
    orderedIds.forEach((id, index) => orderMap.set(id, index))
    categories.value = categories.value.map((category) => {
      const order = orderMap.get(category.id)
      return order === undefined ? category : { ...category, sort_order: order }
    })

    try {
      await invoke('reorder_categories', { orderedIds })
      // 以向后端为准的排序/计数重新加载，并刷新当前页与统计
      await Promise.all([loadCategories(), loadPasswords(), loadStats()])
    } catch (e) {
      categories.value = previous
      console.error('调整分类顺序失败:', e)
      throw e
    }
  }

  // ==================== 请求触发（watch） ====================

  // 筛选条件（分类 / 标签 / 排序 / 每页条数）变化：重置第 1 页并请求
  // 使用 sync 触发，保证 setSort / setPageSize 内部批量赋值只产生一次请求。
  watch(
    () =>
      [
        activeCategory.value,
        activeTags.value.join('\u0000'),
        sortBy.value,
        sortOrder.value,
        pageSize.value,
      ].join('\u0001'),
    () => {
      if (suppressFilterFetch) return
      requestFirstPage()
    },
    { flush: 'sync' },
  )

  // 页码变化：请求对应页（后端回写页码时抑制）
  watch(
    currentPage,
    () => {
      if (suppressPageFetch) {
        suppressPageFetch = false
        return
      }
      if (suppressFilterFetch) return
      void loadPasswords()
    },
    { flush: 'sync' },
  )

  // 搜索词变化：200ms 防抖 + 重置第 1 页后请求
  watch(
    searchQuery,
    () => {
      cancelPendingSearch()
      searchTimer = setTimeout(() => {
        searchTimer = null
        // 已按同一搜索词请求过第 1 页（例如同时发生了筛选变化），不重复请求
        if (searchQuery.value.trim() === lastRequestedSearch && currentPage.value === 1) return
        requestFirstPage()
      }, SEARCH_DEBOUNCE_MS)
    },
    { flush: 'sync' },
  )

  // ==================== 返回 ====================

  return {
    // 状态
    categories,
    tags,
    passwords,
    totalFiltered,
    stats,
    activeCategory,
    activeTags,
    searchQuery,
    sortBy,
    sortOrder,
    pageSize,
    currentPage,
    loading,
    error,
    // 计算属性
    pagedPasswords,
    pageCount,
    categoryTree,
    categoryNameById,
    allCount,
    favoriteCount,
    recentCount,
    hasActiveFilters,
    activeCategoryName,
    // 数据加载
    loadPasswords,
    loadStats,
    loadCategories,
    loadTags,
    // 筛选 / 排序 / 分页状态
    setActiveCategory,
    setSearchQuery,
    toggleTag,
    setActiveTags,
    clearTags,
    setSort,
    setPage,
    setPageSize,
    // 密码 / 标签 / 分类操作
    touchUsage,
    renameTag,
    deleteTag,
    addPassword,
    updatePassword,
    getPasswordDetail,
    deletePassword,
    toggleFavorite,
    addCategory,
    updateCategory,
    deleteCategory,
    reorderCategories,
  }
})
