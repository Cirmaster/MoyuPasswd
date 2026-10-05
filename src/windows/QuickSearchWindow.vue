<!--
  QuickSearchWindow.vue - 独立的快速搜索窗口

  全局快捷键呼出的独立小窗口，只显示搜索功能。
  类似 Raycast、Alfred 的效果。

  安全设计：
  - 每次显示时实时检查锁定状态
  - 不缓存密码数据，每次从后端实时获取
  - 锁定后立即清除本地数据

  功能：
  - 搜索标题/用户名/URL/自定义字段（明文字段）
  - ↵ 复制密码（注入式，密码不进剪贴板）
  - →/Tab 或「详情」按钮展开字段面板，按字段复制（含敏感字段，按需解密）
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useTheme } from '@/composables/useTheme'
import type { CustomField } from '@/stores/password'
import { disableContextMenu } from '@/lib/windowSetup'
import { IconLock, IconSearch, IconSpinner } from '@/components/icons'
import Toast from '@/components/Toast.vue'

/** 主题管理 */
const { isDark, initTheme } = useTheme()

/** 搜索关键词 */
const searchQuery = ref('')

/** 当前选中的结果索引 */
const selectedIndex = ref(0)

/** 加载状态 */
const loading = ref(false)

/** 是否已解锁 */
const isUnlocked = ref(false)

/** 主密码输入 */
const masterPassword = ref('')

/** 解锁错误信息 */
const unlockError = ref('')

/** 解锁加载状态 */
const unlockLoading = ref(false)

/** 系统认证是否可用且已启用 */
const systemAuthReady = ref(false)

/** 是否正在尝试系统认证 */
const attemptingSystemAuth = ref(false)

/** 是否显示主密码输入（系统认证失败后的备用） */
const showPasswordInput = ref(false)

/** 剪贴板清除时间（秒），从设置中读取 */
const clipboardClearTime = ref(30)

/** Toast 提示状态 */
const toast = ref({
  show: false,
  type: 'success' as 'success' | 'error' | 'info',
  message: '',
})

/** 搜索结果（不缓存，每次实时查询） */
const results = ref<Array<{
  id: string
  title: string
  username: string
  password: string
  url?: string
  extra_fields?: CustomField[]
  category: string
  is_favorite: boolean
}>>([])

/** 当前展开详情面板的条目 ID */
const expandedId = ref<string | null>(null)

/** 详情字段缓存（id → 字段列表；敏感值经 get_password_detail 解密，锁定时清空） */
const detailFields = ref<Record<string, CustomField[]>>({})

/** 详情加载中 */
const detailLoading = ref(false)

/**
 * 加载设置（剪贴板清除时间等）
 */
const loadSettings = async () => {
  try {
    const savedSettings = await invoke<{
      clipboard_clear_time: number
    }>('get_settings')
    clipboardClearTime.value = savedSettings.clipboard_clear_time || 30
  } catch (e) {
    console.warn('加载设置失败:', e)
  }
}

/**
 * 实时查询后端（不缓存数据）
 * 每次都直接查询后端，由后端判断是否允许返回密码
 */
const searchPasswords = async (query: string) => {
  loading.value = true
  try {
    // 每次都直接查后端，不缓存
    const result = await invoke<Array<{
      id: string
      title: string
      username: string
      password: string
      url?: string
      extra_fields?: CustomField[]
      category: string
      is_favorite: boolean
    }>>('get_passwords', {
      search: query || null,
      category: null,
    })

    results.value = result.slice(0, 8)
    // 后端返回成功，说明已解锁
    isUnlocked.value = true
  } catch (e) {
    console.error('查询密码失败:', e)
    // 后端返回错误，检查是否是未解锁导致的
    const errorMsg = String(e)
    if (errorMsg.includes('未解锁') || errorMsg.includes('not unlocked')) {
      // 后端未解锁，显示解锁界面
      isUnlocked.value = false
      results.value = []
      searchQuery.value = ''
    }
  } finally {
    loading.value = false
  }
}

/**
 * 检查解锁状态并查询密码
 * 直接调用 searchPasswords，由后端判断是否允许
 */
const checkUnlockStatus = async () => {
  await searchPasswords(searchQuery.value)
}

/**
 * 检查系统认证状态
 */
const checkSystemAuth = async () => {
  try {
    const [available, enabled] = await Promise.all([
      invoke<boolean>('is_system_auth_available'),
      invoke<boolean>('is_system_auth_enabled'),
    ])
    systemAuthReady.value = available && enabled
    // 系统认证启用时不显示密码输入
    showPasswordInput.value = !systemAuthReady.value
  } catch {
    systemAuthReady.value = false
    showPasswordInput.value = true
  }
}

/**
 * 尝试系统认证解锁
 */
const trySystemAuth = async () => {
  attemptingSystemAuth.value = true
  unlockError.value = ''

  try {
    const success = await invoke<boolean>('unlock_with_system_auth')
    if (success) {
      isUnlocked.value = true
      await searchPasswords('')
      setTimeout(() => {
        const searchInput = document.querySelector('input[placeholder*="搜索"]') as HTMLInputElement
        if (searchInput) searchInput.focus()
      }, 100)
    }
  } catch (e) {
    console.warn('系统认证失败:', e)
    unlockError.value = String(e)
  } finally {
    attemptingSystemAuth.value = false
  }
}

/**
 * 解锁应用（主密码）
 */
const handleUnlock = async () => {
  if (!masterPassword.value) {
    unlockError.value = '请输入主密码'
    return
  }

  unlockLoading.value = true
  unlockError.value = ''

  try {
    const isValid = await invoke<boolean>('verify_master_password', {
      password: masterPassword.value,
    })

    // 清空密码输入（无论成功失败）
    masterPassword.value = ''

    if (isValid) {
      isUnlocked.value = true
      // 解锁后实时查询
      await searchPasswords('')

      // 解锁后聚焦搜索框
      setTimeout(() => {
        const searchInput = document.querySelector('input[placeholder*="搜索"]') as HTMLInputElement
        if (searchInput) {
          searchInput.focus()
        }
      }, 100)
    } else {
      unlockError.value = '密码错误'
    }
  } catch (e) {
    // 清空密码输入
    masterPassword.value = ''
    unlockError.value = String(e)
  } finally {
    unlockLoading.value = false
  }
}

/**
 * 复制密码并关闭窗口
 */
const copyAndClose = async (text: string) => {
  try {
    await invoke('copy_text_to_clipboard', { text, clearAfter: clipboardClearTime.value })
    toast.value = { show: true, type: 'success', message: '已复制到剪贴板' }

    // 启动全局倒计时窗口（显示在屏幕中央），使用设置中的清除时间
    const x = window.screen.width / 2
    const y = window.screen.height / 2
    await invoke('show_countdown', { seconds: clipboardClearTime.value, x, y })

    // 启动光标跟随
    await invoke('start_follow_cursor')

    // 延迟关闭窗口
    setTimeout(async () => {
      const window = getCurrentWindow()
      await window.hide()
    }, 300)
  } catch {
    toast.value = { show: true, type: 'error', message: '复制失败' }
  }
}

/**
 * 登记待粘贴密码后关闭窗口（前端不接触明文，密码不进系统剪贴板）
 * 倒计时内在目标窗口按 Ctrl+V，由后端解密并注入
 * @param id - 密码 ID
 */
const copyPasswordByIdAndClose = async (id: string) => {
  try {
    const clearTime = await invoke<number>('copy_password_to_clipboard', { id })
    toast.value = { show: true, type: 'success', message: '密码就绪，倒计时内按 Ctrl+V 粘贴' }

    // 启动全局倒计时窗口
    const x = window.screen.width / 2
    const y = window.screen.height / 2
    await invoke('show_countdown', { seconds: clearTime, x, y })
    await invoke('start_follow_cursor')

    // 延迟关闭窗口
    setTimeout(async () => {
      const window = getCurrentWindow()
      await window.hide()
    }, 300)
  } catch {
    toast.value = { show: true, type: 'error', message: '复制密码失败' }
  }
}

/**
 * 关闭窗口
 */
const closeWindow = async () => {
  const window = getCurrentWindow()
  await window.hide()
}

/**
 * 展开/收起条目详情面板
 *
 * 展开时按需调用 get_password_detail 解密敏感字段值（列表零解密），
 * 结果只缓存在内存中，锁定/搜索词变化即清。
 * @param item - 搜索结果项
 */
const toggleDetail = async (item: { id: string }) => {
  // 已展开则收起
  if (expandedId.value === item.id) {
    expandedId.value = null
    return
  }
  expandedId.value = item.id

  // 已缓存过直接展示
  if (detailFields.value[item.id]) return

  detailLoading.value = true
  try {
    const detail = await invoke<{ extra_fields?: CustomField[] }>('get_password_detail', {
      id: item.id,
    })
    detailFields.value = { ...detailFields.value, [item.id]: detail.extra_fields || [] }
  } catch (e) {
    console.error('加载详情失败:', e)
    toast.value = { show: true, type: 'error', message: '加载详情失败' }
  } finally {
    detailLoading.value = false
  }
}

/**
 * 复制单个字段值并关闭窗口（与复制用户名/密码行为一致）
 * @param text - 字段值文本
 */
const copyFieldText = async (text: string) => {
  await copyAndClose(text)
}

/**
 * 键盘事件处理
 * 注意：Esc 关闭由后端全局快捷键处理
 */
const handleKeydown = (e: KeyboardEvent) => {
  // Esc 关闭窗口（窗口级处理，替代原全局 Escape 快捷键——全局裸 Esc 会吞掉系统级 Esc）
  if (e.key === 'Escape') {
    e.preventDefault()
    getCurrentWindow().hide()
    return
  }

  // 如果未解锁，按 Enter 时触发解锁
  if (!isUnlocked.value) {
    if (e.key === 'Enter') {
      e.preventDefault()
      handleUnlock()
    }
    return
  }

  // 上下导航
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    selectedIndex.value = Math.min(selectedIndex.value + 1, results.value.length - 1)
    scrollToSelected()
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    selectedIndex.value = Math.max(selectedIndex.value - 1, 0)
    scrollToSelected()
  } else if (e.key === 'ArrowRight' || e.key === 'Tab') {
    // →/Tab 展开选中条目的字段详情（再按一次收起）
    e.preventDefault()
    const selectedItem = results.value[selectedIndex.value]
    if (selectedItem) {
      toggleDetail(selectedItem)
    }
  } else if (e.key === 'ArrowLeft') {
    // ← 收起详情面板
    e.preventDefault()
    expandedId.value = null
  } else if (e.key === 'Enter') {
    e.preventDefault()
    const selectedItem = results.value[selectedIndex.value]
    if (selectedItem) {
      copyPasswordByIdAndClose(selectedItem.id)
    }
  }
}

/**
 * 滚动到选中的项
 */
const scrollToSelected = () => {
  const selectedElement = document.querySelector(`[data-index="${selectedIndex.value}"]`)
  if (selectedElement) {
    selectedElement.scrollIntoView({ block: 'nearest' })
  }
}

/**
 * 监听搜索词变化，实时查询后端（不缓存）
 */
watch(searchQuery, (newQuery) => {
  selectedIndex.value = 0
  expandedId.value = null
  // 搜索词变化时丢弃已解密的字段缓存
  detailFields.value = {}
  // 搜索词变化时实时查询后端
  searchPasswords(newQuery)
})

/**
 * 初始化
 */
onMounted(async () => {
  // 初始化主题
  initTheme()
  // 禁用浏览器默认右键菜单（入口已拦一道；组件内再拦，保证热替换后也生效）
  disableContextMenu()

  // 加载设置
  await loadSettings()

  // 检查系统认证状态
  await checkSystemAuth()

  // 实时检查解锁状态
  await checkUnlockStatus()

  // 监听锁定事件（来自主应用）
  await listen('app-locked', () => {
    // 收到锁定事件，立即清除数据（含已解密的字段缓存）
    isUnlocked.value = false
    results.value = []
    searchQuery.value = ''
    masterPassword.value = ''
    expandedId.value = null
    detailFields.value = {}
  })

  // 监听窗口显示事件（每次窗口从隐藏变为显示时触发）
  await listen('window-shown', async () => {
    // 重新检查系统认证状态（可能在设置中刚启用）
    await checkSystemAuth()
    await checkUnlockStatus()
    // 不自动尝试系统认证，让用户主动选择
  })

  // 窗口获得焦点时，实时检查状态（确保显示最新数据）
  const currentWindow = getCurrentWindow()
  currentWindow.onFocusChanged(({ payload: focused }) => {
    if (focused) {
      // 窗口获得焦点时，重新检查解锁状态
      checkUnlockStatus()
    }
  })

  // 添加全局键盘事件监听
  document.addEventListener('keydown', handleKeydown)

  // 自动聚焦输入框（多次尝试确保聚焦成功）
  const focusInput = () => {
    const inputs = document.querySelectorAll('input')
    inputs.forEach(input => {
      if (input.hasAttribute('autofocus') || input.placeholder?.includes('搜索') || input.placeholder?.includes('主密码')) {
        input.focus()
      }
    })
  }

  // 立即尝试聚焦
  focusInput()

  // 延迟再次尝试（等待窗口完全渲染）
  setTimeout(focusInput, 50)
  setTimeout(focusInput, 150)
  setTimeout(focusInput, 300)

  // 组件卸载时取消监听
  onUnmounted(() => {
    document.removeEventListener('keydown', handleKeydown)
  })
})
</script>

<template>
  <!-- overflow-hidden + min-h-0 链条：列表高度收敛到窗口内，滚动条长在结果区而不是整个窗口 -->
  <div class="h-screen overflow-hidden bg-transparent text-foreground flex flex-col">
    <!-- 未解锁 -->
    <div v-if="!isUnlocked" class="flex-1 flex items-center justify-center bg-card">
      <div class="w-full max-w-sm bg-card border shadow-lg">
        <!-- 系统认证进行中 -->
        <div v-if="attemptingSystemAuth" class="p-6 text-center">
          <div class="inline-flex items-center justify-center w-14 h-14 rounded-xl bg-primary/10 mb-4">
            <IconSpinner class="h-7 w-7 animate-spin text-primary" />
          </div>
          <h2 class="text-lg font-semibold mb-1">正在认证</h2>
          <p class="text-sm text-muted-foreground mb-4">请在弹出的窗口中选择解锁方式</p>
          <Button variant="outline" size="sm" @click="attemptingSystemAuth = false; showPasswordInput = true">
            使用主密码解锁
          </Button>
        </div>

        <!-- 系统认证已启用：只显示认证按钮 -->
        <div v-else-if="systemAuthReady && !showPasswordInput" class="p-6 text-center">
          <div class="inline-flex items-center justify-center w-14 h-14 rounded-xl bg-primary/10 mb-4">
            <IconLock class="h-7 w-7 text-primary" />
          </div>
          <h2 class="text-lg font-semibold mb-1">密码库已锁定</h2>
          <p class="text-sm text-muted-foreground mb-5">点击下方按钮解锁</p>
          <div class="space-y-3">
            <Button class="w-full h-10" @click="trySystemAuth">
              解锁
            </Button>
            <Button variant="link" size="sm" @click="showPasswordInput = true">
              使用主密码解锁
            </Button>
          </div>
        </div>

        <!-- 主密码输入（系统认证未启用或用户选择主密码） -->
        <div v-else class="p-6 text-center">
          <div class="inline-flex items-center justify-center w-14 h-14 rounded-xl bg-primary/10 mb-4">
            <IconLock class="h-7 w-7 text-primary" />
          </div>
          <h2 class="text-lg font-semibold mb-1">密码库已锁定</h2>
          <p class="text-sm text-muted-foreground mb-5">输入主密码解锁</p>
          
          <div class="space-y-3">
            <Input
              v-model="masterPassword"
              type="password"
              placeholder="输入主密码..."
              class="h-10"
              autofocus
              @keydown.enter="handleUnlock"
            />
            <p v-if="unlockError" class="text-sm text-destructive">{{ unlockError }}</p>
            <Button class="w-full h-10" :disabled="unlockLoading" @click="handleUnlock">
              {{ unlockLoading ? '验证中...' : '解锁' }}
            </Button>
            <!-- 系统认证可用时，显示切换按钮 -->
            <Button v-if="systemAuthReady" variant="link" size="sm" @click="showPasswordInput = false">
              使用系统认证解锁
            </Button>
          </div>
        </div>
        <div class="px-4 py-2.5 border-t bg-muted/30 text-center">
          <span class="text-xs text-muted-foreground">按 Esc 关闭窗口</span>
        </div>
      </div>
    </div>

    <!-- 已解锁 - 搜索界面 -->
    <div v-else class="flex-1 min-h-0 flex flex-col bg-card backdrop-blur-sm border shadow-lg">
      <!-- 搜索框 -->
      <div class="flex items-center gap-3 px-4 h-12 border-b">
        <IconSearch class="h-4 w-4 text-muted-foreground" />
        <input
          v-model="searchQuery"
          type="text"
          placeholder="搜索密码..."
          class="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground/50"
          autofocus
        />
      </div>

      <!-- 结果区域（min-h-0 才能让 overflow-y-auto 在 flex 布局中真正生效） -->
      <div class="flex-1 min-h-0 overflow-y-auto">
        <!-- 加载中 -->
        <div v-if="loading" class="h-full flex items-center justify-center text-sm text-muted-foreground">
          搜索中...
        </div>

        <!-- 空状态 -->
        <div v-else-if="results.length === 0" class="h-full flex items-center justify-center text-sm text-muted-foreground">
          未找到匹配的密码
        </div>

        <!-- 结果列表 -->
        <template v-else>
          <div
            v-for="(item, index) in results"
            :key="item.id"
            :data-index="index"
            class="mb-1"
          >
            <!-- 结果行（选中高亮 = action 色，与复制密码按钮严格同色；暗色主题为深板岩蓝而非近白） -->
            <div
              class="flex items-stretch cursor-pointer transition-colors"
              :class="{
                'bg-action text-action-foreground': index === selectedIndex,
                'hover:bg-action/10': index !== selectedIndex,
              }"
              @click="copyPasswordByIdAndClose(item.id)"
              @mouseenter="selectedIndex = index"
            >
              <!-- 左侧：标题/用户名 -->
              <div class="flex-1 min-w-0 flex flex-col justify-center px-4 py-2.5">
                <div class="text-sm font-medium truncate">{{ item.title }}</div>
                <div class="text-xs truncate mt-0.5" :class="index === selectedIndex ? 'text-action-foreground/70' : 'text-muted-foreground'">
                  {{ item.username }}
                </div>
              </div>
              <!-- 右侧操作：三档强调梯度（安静→中→主操作），选中行自动切换为反相芯片 -->
              <div class="flex items-stretch shrink-0">
                <!-- 详情（安静档）：展开/收起字段面板 -->
                <button
                  v-if="item.extra_fields && item.extra_fields.length > 0"
                  class="w-[56px] flex items-center justify-center text-xs font-medium transition-colors"
                  :class="index === selectedIndex
                    ? 'bg-action-foreground/10 text-action-foreground/90 hover:bg-action-foreground/25'
                    : 'bg-muted text-muted-foreground hover:bg-muted/70'"
                  @click.stop="toggleDetail(item)"
                >
                  {{ expandedId === item.id ? '收起' : '详情' }}
                </button>
                <!-- 复制用户（中等档） -->
                <button
                  class="w-[76px] flex items-center justify-center text-xs font-medium transition-colors"
                  :class="index === selectedIndex
                    ? 'bg-action-foreground/20 text-action-foreground hover:bg-action-foreground/35'
                    : 'bg-primary/10 text-primary hover:bg-primary/25'"
                  @click.stop="copyAndClose(item.username)"
                >
                  复制用户
                </button>
                <!-- 复制密码（主操作档）：底色与行高亮严格同色（bg-action），hover 用提亮表达高亮态（不动行高亮色） -->
                <button
                  class="w-[76px] flex items-center justify-center text-xs font-medium transition bg-action text-action-foreground hover:brightness-125 active:brightness-110"
                  @click.stop="copyPasswordByIdAndClose(item.id)"
                >
                  复制密码
                </button>
              </div>
            </div>

            <!-- 字段详情面板（→/Tab 或「详情」按钮展开） -->
            <div v-if="expandedId === item.id" class="px-4 py-2 bg-muted/40 border-t space-y-1">
              <div v-if="detailLoading && !detailFields[item.id]" class="text-xs text-muted-foreground">
                加载字段中...
              </div>
              <template v-else>
                <div
                  v-for="(field, fi) in detailFields[item.id] || []"
                  :key="fi"
                  class="flex items-center gap-2 text-xs"
                >
                  <span class="w-20 shrink-0 text-muted-foreground truncate" :title="field.label">
                    {{ field.label }}
                  </span>
                  <span class="flex-1 min-w-0 truncate font-mono" :title="field.value">
                    {{ field.value || (field.sensitive ? '••••••' : '') }}
                  </span>
                  <IconLock
                    v-if="field.sensitive"
                    class="shrink-0 size-3.5"
                    title="敏感字段，加密存储"
                  />
                  <span
                    v-if="field.value"
                    class="shrink-0 px-1.5 py-0.5 rounded cursor-pointer transition-colors hover:bg-muted text-muted-foreground hover:text-foreground"
                    @click.stop="copyFieldText(field.value)"
                  >
                    复制
                  </span>
                </div>
                <div
                  v-if="(detailFields[item.id] || []).length === 0"
                  class="text-xs text-muted-foreground"
                >
                  无附加字段
                </div>
              </template>
            </div>
          </div>
        </template>
      </div>

      <!-- 底部快捷键 -->
      <div class="flex items-center justify-between px-4 py-2 border-t bg-muted/30 text-xs text-muted-foreground">
        <div class="flex gap-3">
          <span>↑↓ 导航</span>
          <span>↵ 复制</span>
          <span>→/Tab 详情</span>
          <span>Esc 关闭</span>
        </div>
        <span>MoyuPasswd</span>
      </div>
    </div>

    <!-- Toast -->
    <Toast v-model:show="toast.show" :type="toast.type" :message="toast.message" />
  </div>
</template>
