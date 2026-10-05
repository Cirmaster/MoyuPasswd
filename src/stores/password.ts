/**
 * password.ts - 密码数据管理 Store
 *
 * 使用 Pinia 管理所有密码相关的数据和操作，包括：
 * 1. 密码列表的增删改查
 * 2. 分类管理
 * 3. 搜索和筛选
 * 4. 收藏功能
 *
 * @example
 * ```ts
 * const store = usePasswordStore()
 *
 * // 获取过滤后的密码列表
 * store.filteredPasswords
 *
 * // 添加密码
 * store.addPassword({ title: 'GitHub', username: 'user', password: 'xxx', category: 'work' })
 *
 * // 搜索
 * store.setSearchQuery('github')
 * ```
 */

import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'

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
 * 描述单条密码记录的数据结构
 */
export interface PasswordItem {
  /** 唯一标识符 */
  id: string
  /** 网站/应用名称，如 "GitHub" */
  title: string
  /** 登录用户名或邮箱 */
  username: string
  /** 登录密码（解密后的明文） */
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
}

/**
 * 分类接口定义
 * 描述密码分类的数据结构
 */
export interface Category {
  /** 分类 ID，如 "work", "social" */
  id: string
  /** 分类显示名称，如 "工作", "社交媒体" */
  name: string
  /** 分类图标（可选，预留） */
  icon?: string
}

/**
 * 新增密码请求
 */
interface NewPassword {
  title: string
  username: string
  password: string
  url?: string
  notes?: string
  extra_fields?: CustomField[]
  category: string
  is_favorite: boolean
}

/**
 * 更新密码请求
 *
 * url/notes 三层语义：字段缺失=不修改、null=清空、字符串=赋值
 * extra_fields 三层语义：字段缺失=不修改、null/[]=清空、数组=整体替换
 */
interface UpdatePassword {
  title?: string
  username?: string
  password?: string
  url?: string | null
  notes?: string | null
  extra_fields?: CustomField[] | null
  category?: string
  is_favorite?: boolean
}

/**
 * 密码数据 Store
 * 管理所有密码和分类的状态
 */
export const usePasswordStore = defineStore('password', () => {
  // ==================== 状态 ====================

  /**
   * 分类列表
   * 包含预设分类和用户自定义分类
   * "all" 和 "favorite" 是特殊分类，不可删除
   */
  const categories = ref<Category[]>([
    { id: 'all', name: '全部' },
    { id: 'favorite', name: '收藏' },
    { id: 'social', name: '社交媒体' },
    { id: 'work', name: '工作' },
    { id: 'finance', name: '金融' },
    { id: 'other', name: '其他' },
  ])

  /**
   * 密码列表
   * 从 Tauri 后端加载
   */
  const passwords = ref<PasswordItem[]>([])

  /** 当前选中的分类 ID，默认显示全部 */
  const activeCategory = ref('all')

  /** 搜索关键词，用于过滤密码列表 */
  const searchQuery = ref('')

  /** 加载状态 */
  const loading = ref(false)

  /** 错误信息 */
  const error = ref<string | null>(null)

  // ==================== 计算属性 ====================

  /**
   * 过滤后的密码列表
   * 根据当前选中的分类和搜索关键词进行过滤
   *
   * 过滤逻辑：
   * 1. 先按分类筛选（"全部"显示所有，"收藏"显示收藏的，其他显示对应分类）
   * 2. 再按搜索关键词筛选（匹配标题、用户名、URL）
   */
  const filteredPasswords = computed(() => {
    let list = passwords.value

    // 按分类过滤
    if (activeCategory.value === 'favorite') {
      // 收藏分类：只显示收藏的密码
      list = list.filter((p) => p.is_favorite)
    } else if (activeCategory.value !== 'all') {
      // 其他分类：只显示对应分类的密码
      list = list.filter((p) => p.category === activeCategory.value)
    }

    // 按搜索关键词过滤（标题/用户名/URL/自定义字段；敏感字段值不参与——列表里本来就是空串）
    if (searchQuery.value) {
      const query = searchQuery.value.toLowerCase()
      list = list.filter(
        (p) =>
          p.title.toLowerCase().includes(query) ||
          p.username.toLowerCase().includes(query) ||
          p.url?.toLowerCase().includes(query) ||
          p.extra_fields?.some(
            (f) =>
              f.label.toLowerCase().includes(query) || f.value.toLowerCase().includes(query),
          ),
      )
    }

    return list
  })

  // ==================== 操作方法 ====================

  /**
   * 从后端加载密码列表
   */
  async function loadPasswords() {
    loading.value = true
    error.value = null

    try {
      const result = await invoke<PasswordItem[]>('get_passwords', {
        search: null,
        category: null,
      })
      passwords.value = result
    } catch (e) {
      error.value = String(e)
      console.error('加载密码失败:', e)
    } finally {
      loading.value = false
    }
  }

  /**
   * 从后端加载分类列表
   */
  async function loadCategories() {
    try {
      const result = await invoke<Category[]>('get_categories')
      // 合并特殊分类
      categories.value = [
        { id: 'all', name: '全部' },
        { id: 'favorite', name: '收藏' },
        ...result,
      ]
    } catch (e) {
      console.error('加载分类失败:', e)
    }
  }

  /**
   * 设置当前选中的分类
   * @param id - 分类 ID
   */
  function setActiveCategory(id: string) {
    activeCategory.value = id
  }

  /**
   * 设置搜索关键词
   * @param query - 搜索关键词
   */
  function setSearchQuery(query: string) {
    searchQuery.value = query
  }

  /**
   * 添加新密码
   * @param item - 密码数据
   */
  async function addPassword(item: NewPassword) {
    loading.value = true
    error.value = null

    try {
      const result = await invoke<PasswordItem>('add_password', { data: item })
      passwords.value.push(result)
      return result
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
   * @param id - 要更新的密码 ID
   * @param data - 要更新的字段（url/notes 缺失=不修改、null=清空）
   */
  async function updatePassword(id: string, data: UpdatePassword) {
    loading.value = true
    error.value = null

    try {
      const result = await invoke<PasswordItem>('update_password', { id, data })
      const index = passwords.value.findIndex((p) => p.id === id)
      if (index !== -1) {
        passwords.value[index] = result
      }
      return result
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
  async function getPasswordDetail(id: string) {
    return await invoke<PasswordItem>('get_password_detail', { id })
  }

  /**
   * 删除密码
   * @param id - 要删除的密码 ID
   */
  async function deletePassword(id: string) {
    loading.value = true
    error.value = null

    try {
      await invoke('delete_password', { id })
      passwords.value = passwords.value.filter((p) => p.id !== id)
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
   * @param id - 要切换收藏的密码 ID
   */
  async function toggleFavorite(id: string) {
    try {
      const isFavorite = await invoke<boolean>('toggle_favorite', { id })
      const item = passwords.value.find((p) => p.id === id)
      if (item) {
        item.is_favorite = isFavorite
      }
    } catch (e) {
      console.error('切换收藏失败:', e)
    }
  }

  /**
   * 添加新分类
   * @param name - 分类名称
   */
  async function addCategory(name: string) {
    try {
      const result = await invoke<Category>('add_category', { name })
      categories.value.push(result)
      return result
    } catch (e) {
      console.error('添加分类失败:', e)
      throw e
    }
  }

  /**
   * 更新分类名称
   * @param id - 分类 ID
   * @param name - 新分类名称
   */
  async function updateCategory(id: string, name: string) {
    try {
      const result = await invoke<Category>('update_category', { id, name })
      const index = categories.value.findIndex((c) => c.id === id)
      if (index !== -1) {
        categories.value[index] = result
      }
      return result
    } catch (e) {
      console.error('更新分类失败:', e)
      throw e
    }
  }

  /**
   * 删除分类
   * @param id - 要删除的分类 ID
   */
  async function deleteCategory(id: string) {
    try {
      await invoke('delete_category', { id })
      categories.value = categories.value.filter((c) => c.id !== id)
    } catch (e) {
      console.error('删除分类失败:', e)
      throw e
    }
  }

  // ==================== 返回 ====================

  return {
    // 状态
    categories,
    passwords,
    activeCategory,
    searchQuery,
    filteredPasswords,
    loading,
    error,
    // 方法
    loadPasswords,
    loadCategories,
    setActiveCategory,
    setSearchQuery,
    addPassword,
    updatePassword,
    getPasswordDetail,
    deletePassword,
    toggleFavorite,
    addCategory,
    updateCategory,
    deleteCategory,
  }
})
