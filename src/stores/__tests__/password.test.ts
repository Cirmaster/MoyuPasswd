/**
 * password Store v2 单测（后端搜索 / 筛选 / 排序 / 分页驱动）
 *
 * 覆盖契约 §11.2 / §11.3：
 * - 初始默认值
 * - loadPasswords 按当前筛选/排序/分页组装 camelCase 参数，写入 items/total/currentPage
 * - 筛选 / 排序 / 每页条数变化重置第 1 页并重新请求
 * - 页码变化请求对应页、setPage 越界钳制
 * - searchQuery 200ms 防抖（防抖期间不重复请求、相同搜索词不重复请求）
 * - 后端返回 page 回写 currentPage 且不触发二次请求
 * - pageCount = max(1, ceil(total/pageSize))
 * - get_password_stats 写入 stats / allCount / favoriteCount / recentCount
 * - categoryTree 仅结构展示，directCount/totalCount 直接取后端 password_count/total_count
 * - updateCategory parentId 三种参数形状（undefined / clearParent / parentId）
 * - reorderCategories 乐观更新与失败回滚
 * - toggleFavorite / deletePassword / renameTag / deleteTag 成功后刷新
 * - touchUsage 乐观更新、按需刷新、失败回滚
 * - 请求序号防止旧响应覆盖新响应
 *
 * 运行环境：jsdom；`@tauri-apps/api/core` 的 `invoke` 被 mock；使用 fake timers 控制搜索防抖。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

import { invoke } from '@tauri-apps/api/core'
import {
  DEFAULT_PAGE_SIZE,
  SEARCH_DEBOUNCE_MS,
  usePasswordStore,
  type Category,
  type PasswordItem,
  type PasswordPage,
} from '../password'

// ==================== mock 辅助 ====================

type InvokeArgs = Record<string, unknown> | undefined

/** 只声明本测试用到的 mock 能力，避免依赖 vitest `Mock` 的泛型细节 */
interface InvokeMock {
  mockReset: () => void
  mockClear: () => void
  mockImplementation: (impl: (command: string, args?: InvokeArgs) => unknown) => void
  mock: { calls: unknown[][] }
}

const invokeMock = invoke as unknown as InvokeMock

/** 按命令名分发 mock 返回值；未登记的命令返回 undefined（store 内部均有兜底） */
function mockCommands(handlers: Record<string, (args?: InvokeArgs) => unknown>): void {
  invokeMock.mockImplementation((command: string, args?: InvokeArgs) => {
    const handler = handlers[command]
    if (!handler) return Promise.resolve(undefined)
    try {
      return Promise.resolve(handler(args))
    } catch (e) {
      return Promise.reject(e)
    }
  })
}

/** 某个命令的全部调用参数 */
function callsOf(command: string): InvokeArgs[] {
  const result: InvokeArgs[] = []
  for (const call of invokeMock.mock.calls) {
    if (String(call[0]) === command) result.push(call[1] as InvokeArgs)
  }
  return result
}

/** 某个命令的调用次数 */
function callCount(command: string): number {
  return callsOf(command).length
}

/** 某个命令最近一次调用的参数（未调用则抛错） */
function lastCallOf(command: string): InvokeArgs {
  const calls = callsOf(command)
  if (calls.length === 0) throw new Error(`invoke 未被调用：${command}`)
  return calls[calls.length - 1]
}

/** 构造 get_passwords 默认分页返回（page / page_size 回显请求值） */
function pageOf(args: InvokeArgs, total = 0): PasswordPage {
  return {
    items: [],
    total,
    page: Number(args?.page ?? 1),
    page_size: Number(args?.pageSize ?? DEFAULT_PAGE_SIZE),
  }
}

/** 常见后端命令的默认响应 */
function defaultHandlers(): Record<string, (args?: InvokeArgs) => unknown> {
  return {
    get_passwords: (args) => pageOf(args),
    get_password_stats: () => ({ all: 0, favorite: 0, recent: 0 }),
    get_categories: () => [],
    get_tags: () => [],
  }
}

/** 设置 mock 后端（默认响应 + 用例覆盖） */
function mockBackend(overrides: Record<string, (args?: InvokeArgs) => unknown> = {}): void {
  mockCommands({ ...defaultHandlers(), ...overrides })
}

/** 连续执行若干微任务，等待 store 内部的异步加载写入状态（fake timers 下同样有效） */
async function flushPromises(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await Promise.resolve()
}

// ==================== 测试数据工厂 ====================

/** 构造一条完整密码记录 */
function makePassword(overrides: Partial<PasswordItem> = {}): PasswordItem {
  return {
    id: 'p1',
    title: 'GitHub',
    username: 'octocat',
    password: '',
    extra_fields: [],
    category: 'work',
    is_favorite: false,
    created_at: 1_000,
    updated_at: 2_000,
    tags: [],
    last_used_at: null,
    use_count: 0,
    ...overrides,
  }
}

/** 虚拟分类（与 Store 初始值一致） */
const VIRTUAL_CATEGORIES: Category[] = [
  { id: 'all', name: '全部' },
  { id: 'favorite', name: '收藏' },
]

// ==================== 测试 ====================

describe('password store v2（后端驱动）', () => {
  let store: ReturnType<typeof usePasswordStore>

  /** 直接设置本地分类（用于分类树 / 拖拽排序用例） */
  function setLocalCategories(backend: Category[]): void {
    store.categories = [...VIRTUAL_CATEGORIES, ...backend]
  }

  beforeEach(() => {
    vi.useFakeTimers()
    setActivePinia(createPinia())
    invokeMock.mockReset()
    store = usePasswordStore()
  })

  afterEach(() => {
    vi.clearAllTimers()
    vi.useRealTimers()
  })

  // ==================== 初始状态 ====================

  it('初始状态符合 v2 契约默认值', () => {
    expect(store.categories.map((category) => category.id)).toEqual(['all', 'favorite'])
    expect(store.passwords).toEqual([])
    expect(store.pagedPasswords).toEqual([])
    expect(store.totalFiltered).toBe(0)
    expect(store.pageCount).toBe(1)
    expect(store.stats).toEqual({ all: 0, favorite: 0, recent: 0 })
    expect(store.allCount).toBe(0)
    expect(store.favoriteCount).toBe(0)
    expect(store.recentCount).toBe(0)
    expect(store.activeCategory).toBe('all')
    expect(store.activeTags).toEqual([])
    expect(store.searchQuery).toBe('')
    expect(store.sortBy).toBe('updated_at')
    expect(store.sortOrder).toBe('desc')
    expect(store.pageSize).toBe(DEFAULT_PAGE_SIZE)
    expect(store.currentPage).toBe(1)
    expect(store.loading).toBe(false)
    expect(store.error).toBeNull()
    expect(store.hasActiveFilters).toBe(false)
    expect(store.activeCategoryName).toBe('全部')
    // 创建 store 不应自动发请求
    expect(invokeMock.mock.calls.length).toBe(0)
  })

  // ==================== 加载当前页 ====================

  it('loadPasswords 使用默认查询参数并写入 items/total/currentPage', async () => {
    const items = [makePassword({ id: 'p1' }), makePassword({ id: 'p2', title: 'GitLab 工作' })]
    mockBackend({
      get_passwords: (args) => ({ ...pageOf(args), items, total: 42 }),
    })

    await store.loadPasswords()

    expect(lastCallOf('get_passwords')).toEqual({
      search: null,
      category: null,
      tags: null,
      sortBy: 'updated_at',
      sortOrder: 'desc',
      page: 1,
      pageSize: 50,
    })
    expect(store.passwords.map((item) => item.id)).toEqual(['p1', 'p2'])
    expect(store.pagedPasswords).toEqual(store.passwords)
    expect(store.totalFiltered).toBe(42)
    expect(store.pageCount).toBe(1)
    expect(callCount('get_passwords')).toBe(1)
  })

  it('采纳后端返回页码且不触发二次请求', async () => {
    mockBackend({
      get_passwords: () => ({ items: [], total: 120, page: 2, page_size: 50 }),
    })

    await store.loadPasswords()
    await flushPromises()

    expect(store.currentPage).toBe(2)
    expect(callCount('get_passwords')).toBe(1)
  })

  it('筛选/排序参数正确传递（camelCase，空值传 null）', async () => {
    mockBackend()
    await store.loadPasswords()
    await flushPromises()

    store.setActiveCategory('work')
    expect(lastCallOf('get_passwords')).toMatchObject({
      category: 'work',
      search: null,
      tags: null,
      page: 1,
    })
    await flushPromises()

    store.setActiveTags(['DB', 'prod'])
    expect(lastCallOf('get_passwords')).toMatchObject({
      tags: ['DB', 'prod'],
      category: 'work',
    })
    await flushPromises()

    store.clearTags()
    expect(lastCallOf('get_passwords')).toMatchObject({ tags: null })
    await flushPromises()

    store.setActiveCategory('all')
    expect(lastCallOf('get_passwords')).toMatchObject({ category: null })
    await flushPromises()

    store.setActiveCategory('recent')
    expect(lastCallOf('get_passwords')).toMatchObject({ category: 'recent' })
    await flushPromises()

    store.setSort('title')
    expect(lastCallOf('get_passwords')).toMatchObject({
      sortBy: 'title',
      sortOrder: 'asc',
      page: 1,
    })
    await flushPromises()

    store.setSort('title')
    expect(lastCallOf('get_passwords')).toMatchObject({ sortBy: 'title', sortOrder: 'desc' })
    await flushPromises()

    store.setSearchQuery('github 工作')
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS)
    expect(lastCallOf('get_passwords')).toMatchObject({ search: 'github 工作', page: 1 })

    // 仅空白搜索词 → null
    store.setSearchQuery('   ')
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS)
    expect(lastCallOf('get_passwords')).toMatchObject({ search: null })
  })

  // ==================== 变化触发请求 ====================

  it('筛选/排序/每页条数变化重置到第 1 页并请求', async () => {
    mockBackend({
      get_passwords: (args) => ({ ...pageOf(args), total: 120 }),
    })

    await store.loadPasswords()
    await flushPromises()

    store.setPage(2)
    expect(store.currentPage).toBe(2)
    expect(lastCallOf('get_passwords')).toMatchObject({ page: 2 })
    await flushPromises()

    store.setActiveCategory('work')
    expect(store.currentPage).toBe(1)
    expect(lastCallOf('get_passwords')).toMatchObject({ category: 'work', page: 1 })
    await flushPromises()

    store.setPage(2)
    await flushPromises()
    store.setSort('use_count')
    expect(store.currentPage).toBe(1)
    expect(lastCallOf('get_passwords')).toMatchObject({ sortBy: 'use_count', page: 1 })
    await flushPromises()

    store.setPage(2)
    await flushPromises()
    store.setPageSize(20)
    expect(store.currentPage).toBe(1)
    expect(lastCallOf('get_passwords')).toMatchObject({ pageSize: 20, page: 1 })
  })

  it('页码变化请求对应页，setPage 越界钳制', async () => {
    mockBackend({
      get_passwords: (args) => ({ ...pageOf(args), total: 120 }),
    })
    await store.loadPasswords()
    await flushPromises()

    const beforePageChange = callCount('get_passwords')
    store.setPage(3)
    expect(store.currentPage).toBe(3)
    expect(lastCallOf('get_passwords')).toMatchObject({ page: 3 })
    expect(callCount('get_passwords')).toBe(beforePageChange + 1)
    await flushPromises()

    // 超出 pageCount（=3）：钳制到 3，页码未变化则不再请求
    const beforeClamp = callCount('get_passwords')
    store.setPage(99)
    expect(store.currentPage).toBe(3)
    expect(callCount('get_passwords')).toBe(beforeClamp)

    // 小于 1：钳制到 1 并请求
    store.setPage(0)
    expect(store.currentPage).toBe(1)
    expect(lastCallOf('get_passwords')).toMatchObject({ page: 1 })
  })

  it('searchQuery 200ms 防抖，仅最后一次输入触发一次请求', async () => {
    mockBackend({
      get_passwords: (args) => ({ ...pageOf(args), total: 120 }),
    })
    await store.loadPasswords()
    await flushPromises()

    store.setPage(2)
    await flushPromises()

    const beforeSearch = callCount('get_passwords')
    store.setSearchQuery('g')
    store.setSearchQuery('gi')
    store.setSearchQuery('github 工作')

    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS - 1)
    expect(callCount('get_passwords')).toBe(beforeSearch)

    await vi.advanceTimersByTimeAsync(1)
    expect(callCount('get_passwords')).toBe(beforeSearch + 1)
    expect(lastCallOf('get_passwords')).toMatchObject({
      search: 'github 工作',
      page: 1,
    })
  })

  it('相同搜索词已请求过时不重复请求（防抖去重）', async () => {
    mockBackend()
    await store.loadPasswords()
    await flushPromises()

    const beforeSearch = callCount('get_passwords')
    // 空搜索已请求过，仅空白输入等价于空搜索，不应重复请求
    store.setSearchQuery('   ')
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS)

    expect(callCount('get_passwords')).toBe(beforeSearch)
  })

  // ==================== 分页 / 统计 ====================

  it('pageCount 根据后端 total 与 pageSize 计算（至少 1）', async () => {
    mockBackend({
      get_passwords: (args) => ({ ...pageOf(args), total: 51 }),
    })
    await store.loadPasswords()
    await flushPromises()

    expect(store.totalFiltered).toBe(51)
    expect(store.pageCount).toBe(2)

    store.setPageSize(100)
    await flushPromises()
    expect(store.pageCount).toBe(1)
  })

  it('loadStats 写入 stats 并映射 allCount/favoriteCount/recentCount', async () => {
    mockBackend({
      get_password_stats: () => ({ all: 10, favorite: 3, recent: 4 }),
    })

    await store.loadStats()

    expect(lastCallOf('get_password_stats')).toBeUndefined()
    expect(store.stats).toEqual({ all: 10, favorite: 3, recent: 4 })
    expect(store.allCount).toBe(10)
    expect(store.favoriteCount).toBe(3)
    expect(store.recentCount).toBe(4)
  })

  // ==================== 分类树 ====================

  it('categoryTree 使用后端 password_count/total_count（仅结构展示）', async () => {
    mockBackend({
      get_categories: () => [
        {
          id: 'work',
          name: '工作',
          parent_id: null,
          sort_order: 1,
          password_count: 2,
          total_count: 5,
        },
        {
          id: 'db',
          name: '数据库',
          parent_id: 'work',
          sort_order: 1,
          password_count: 1,
          total_count: 2,
        },
        {
          id: 'social',
          name: '社交',
          parent_id: null,
          sort_order: 2,
          password_count: 3,
          total_count: 3,
        },
      ],
    })

    await store.loadCategories()

    expect(store.categories.map((category) => category.id)).toEqual([
      'all',
      'favorite',
      'work',
      'db',
      'social',
    ])

    const tree = store.categoryTree
    expect(tree.map((node) => node.id)).toEqual(['work', 'db', 'social'])
    expect(tree.map((node) => node.depth)).toEqual([0, 1, 0])

    const work = tree.find((node) => node.id === 'work')
    expect(work).toMatchObject({
      parentId: null,
      hasChildren: true,
      directCount: 2,
      totalCount: 5,
    })

    const db = tree.find((node) => node.id === 'db')
    expect(db).toMatchObject({
      parentId: 'work',
      depth: 1,
      hasChildren: false,
      directCount: 1,
      totalCount: 2,
    })

    const social = tree.find((node) => node.id === 'social')
    expect(social).toMatchObject({ depth: 0, directCount: 3, totalCount: 3 })
  })

  // ==================== 分类写操作 ====================

  it('updateCategory 的 parentId 三种参数形状', async () => {
    mockBackend({
      update_category: () => ({
        id: 'c1',
        name: '新名',
        parent_id: null,
        password_count: 0,
        total_count: 0,
      }),
    })

    // 1. parentId === undefined：参数中不含 parentId / clearParent
    await store.updateCategory('c1', '新名')
    expect(lastCallOf('update_category')).toEqual({ id: 'c1', name: '新名' })
    // 成功后刷新当前页/统计/分类
    expect(callCount('get_passwords')).toBeGreaterThan(0)
    expect(callCount('get_password_stats')).toBeGreaterThan(0)
    expect(callCount('get_categories')).toBeGreaterThan(0)

    // 2. parentId === null：传 clearParent: true（置顶级）
    invokeMock.mockClear()
    await store.updateCategory('c1', undefined, null)
    expect(lastCallOf('update_category')).toEqual({ id: 'c1', clearParent: true })

    // 3. parentId 为字符串：传 parentId
    invokeMock.mockClear()
    await store.updateCategory('c1', undefined, 'parent-1')
    expect(lastCallOf('update_category')).toEqual({ id: 'c1', parentId: 'parent-1' })
  })

  it('reorderCategories 先乐观更新 sort_order，成功后刷新分类', async () => {
    const reorderControl: { resolve: (() => void) | null } = { resolve: null }
    mockBackend({
      reorder_categories: () =>
        new Promise<void>((resolve) => {
          reorderControl.resolve = () => resolve()
        }),
      // 后端以新的 sort_order 返回
      get_categories: () => [
        { id: 'b', name: 'B', parent_id: null, sort_order: 0 },
        { id: 'a', name: 'A', parent_id: null, sort_order: 1 },
      ],
    })
    setLocalCategories([
      { id: 'a', name: 'A', parent_id: null, sort_order: 0 },
      { id: 'b', name: 'B', parent_id: null, sort_order: 1 },
    ])

    const beforeReload = callCount('get_categories')
    const pending = store.reorderCategories(['b', 'a'])

    // 乐观更新：本地 sort_order 立即变化
    expect(store.categories.find((category) => category.id === 'b')?.sort_order).toBe(0)
    expect(store.categories.find((category) => category.id === 'a')?.sort_order).toBe(1)
    expect(lastCallOf('reorder_categories')).toEqual({ orderedIds: ['b', 'a'] })

    reorderControl.resolve?.()
    await pending

    expect(callCount('get_categories')).toBeGreaterThan(beforeReload)
    // 分类树按新的 sort_order 展示
    expect(store.categoryTree.map((node) => node.id)).toEqual(['b', 'a'])
  })

  it('reorderCategories 失败时回滚 sort_order 并抛出错误', async () => {
    mockBackend({
      reorder_categories: () => {
        throw new Error('调整分类顺序失败')
      },
    })
    setLocalCategories([
      { id: 'a', name: 'A', parent_id: null, sort_order: 0 },
      { id: 'b', name: 'B', parent_id: null, sort_order: 1 },
    ])

    await expect(store.reorderCategories(['b', 'a'])).rejects.toThrow('调整分类顺序失败')

    expect(store.categories.find((category) => category.id === 'a')?.sort_order).toBe(0)
    expect(store.categories.find((category) => category.id === 'b')?.sort_order).toBe(1)
  })

  // ==================== 写操作后刷新 ====================

  it('toggleFavorite 成功后刷新当前页与统计', async () => {
    mockBackend({ toggle_favorite: () => true })
    const beforePage = callCount('get_passwords')
    const beforeStats = callCount('get_password_stats')

    await store.toggleFavorite('p1')

    expect(lastCallOf('toggle_favorite')).toEqual({ id: 'p1' })
    expect(callCount('get_passwords')).toBe(beforePage + 1)
    expect(callCount('get_password_stats')).toBe(beforeStats + 1)
  })

  it('deletePassword 成功后刷新当前页/统计/分类/标签', async () => {
    mockBackend({ delete_password: () => undefined })
    const beforePage = callCount('get_passwords')
    const beforeStats = callCount('get_password_stats')
    const beforeCategories = callCount('get_categories')
    const beforeTags = callCount('get_tags')

    await store.deletePassword('p1')

    expect(lastCallOf('delete_password')).toEqual({ id: 'p1' })
    expect(callCount('get_passwords')).toBe(beforePage + 1)
    expect(callCount('get_password_stats')).toBe(beforeStats + 1)
    expect(callCount('get_categories')).toBe(beforeCategories + 1)
    expect(callCount('get_tags')).toBe(beforeTags + 1)
  })

  it('renameTag 调后端、同步已选标签并刷新列表/统计/标签', async () => {
    mockBackend({
      rename_tag: () => undefined,
      get_tags: () => [{ name: 'New', count: 1 }],
    })
    store.setActiveTags(['Old'])
    await flushPromises()

    const beforePage = callCount('get_passwords')
    const beforeStats = callCount('get_password_stats')
    const beforeTags = callCount('get_tags')

    await store.renameTag('Old', 'New')

    expect(lastCallOf('rename_tag')).toEqual({ oldName: 'Old', newName: 'New' })
    expect(store.activeTags).toEqual(['New'])
    expect(store.tags).toEqual([{ name: 'New', count: 1 }])
    expect(callCount('get_passwords')).toBe(beforePage + 1)
    expect(callCount('get_password_stats')).toBe(beforeStats + 1)
    expect(callCount('get_tags')).toBe(beforeTags + 1)
  })

  it('deleteTag 调后端、移除已选标签并刷新列表/统计/标签', async () => {
    mockBackend({
      delete_tag: () => undefined,
      get_tags: () => [{ name: 'Keep', count: 1 }],
    })
    store.setActiveTags(['Old', 'Keep'])
    await flushPromises()

    const beforePage = callCount('get_passwords')
    const beforeStats = callCount('get_password_stats')
    const beforeTags = callCount('get_tags')

    await store.deleteTag('Old')

    expect(lastCallOf('delete_tag')).toEqual({ name: 'Old' })
    expect(store.activeTags).toEqual(['Keep'])
    expect(store.tags).toEqual([{ name: 'Keep', count: 1 }])
    expect(callCount('get_passwords')).toBe(beforePage + 1)
    expect(callCount('get_password_stats')).toBe(beforeStats + 1)
    expect(callCount('get_tags')).toBe(beforeTags + 1)
  })

  // ==================== 使用记录 ====================

  it('touchUsage 乐观更新；默认排序下不额外刷新', async () => {
    mockBackend({ touch_password_usage: () => undefined })
    store.passwords = [makePassword({ id: 'p1', use_count: 2, last_used_at: null })]

    const beforePage = callCount('get_passwords')
    const pending = store.touchUsage('p1')

    // await 之前即已乐观更新
    expect(store.passwords[0]?.use_count).toBe(3)
    expect(store.passwords[0]?.last_used_at).not.toBeNull()

    await pending

    expect(lastCallOf('touch_password_usage')).toEqual({ id: 'p1' })
    expect(callCount('get_passwords')).toBe(beforePage)
    expect(store.passwords[0]?.use_count).toBe(3)
    expect(store.recentCount).toBe(0)
  })

  it('touchUsage 在使用频率排序下成功后刷新当前页与统计', async () => {
    const item = makePassword({ id: 'p1', use_count: 2, last_used_at: null })
    mockBackend({
      touch_password_usage: () => undefined,
      get_passwords: (args) => ({ ...pageOf(args), items: [item], total: 1 }),
    })
    store.setSort('use_count')
    await flushPromises()

    const beforePage = callCount('get_passwords')
    const beforeStats = callCount('get_password_stats')

    await store.touchUsage('p1')

    expect(callCount('get_passwords')).toBe(beforePage + 1)
    expect(callCount('get_password_stats')).toBe(beforeStats + 1)
  })

  it('touchUsage 失败时静默回滚且不抛错', async () => {
    mockBackend({
      touch_password_usage: () => {
        throw new Error('boom')
      },
    })
    store.passwords = [makePassword({ id: 'p1', use_count: 2, last_used_at: 1_000 })]

    await expect(store.touchUsage('p1')).resolves.toBeUndefined()

    expect(store.passwords[0]?.use_count).toBe(2)
    expect(store.passwords[0]?.last_used_at).toBe(1_000)
  })

  // ==================== 竞态 ====================

  it('请求序号防止慢的旧响应覆盖新响应', async () => {
    const resolvers: Array<(value: PasswordPage) => void> = []
    mockBackend({
      get_passwords: () =>
        new Promise<PasswordPage>((resolve) => {
          resolvers.push(resolve)
        }),
    })

    const first = store.loadPasswords()
    const second = store.loadPasswords()
    expect(resolvers.length).toBe(2)

    // 新请求先响应
    resolvers[1]?.({
      items: [makePassword({ id: 'new' })],
      total: 1,
      page: 1,
      page_size: 50,
    })
    await flushPromises()

    // 旧请求后响应：应被丢弃
    resolvers[0]?.({
      items: [makePassword({ id: 'old' })],
      total: 9,
      page: 1,
      page_size: 50,
    })
    await flushPromises()

    expect(store.passwords.map((item) => item.id)).toEqual(['new'])
    expect(store.totalFiltered).toBe(1)

    await first
    await second
    expect(store.loading).toBe(false)
  })
})
