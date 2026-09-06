<!--
  QuickSearchWindow.vue - 独立的快速搜索窗口

  全局快捷键呼出的独立小窗口，只显示搜索功能。
  类似 Raycast、Alfred 的效果。

  安全设计：
  - 每次显示时实时检查锁定状态
  - 不缓存密码数据，每次从后端实时获取
  - 锁定后立即清除本地数据
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
  category: string
  is_favorite: boolean
}>>([])

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
 * 解锁应用
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

    if (isValid) {
      isUnlocked.value = true
      masterPassword.value = ''
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
    await invoke('copy_text_to_clipboard', { text, clearAfter: null })
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
 * 按需解密并复制密码后关闭窗口（前端不接触明文）
 * @param id - 密码 ID
 */
const copyPasswordByIdAndClose = async (id: string) => {
  try {
    const clearTime = await invoke<number>('copy_password_to_clipboard', { id })
    toast.value = { show: true, type: 'success', message: '密码已复制到剪贴板' }

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
 * 键盘事件处理
 * 注意：Esc 关闭由后端全局快捷键处理
 */
const handleKeydown = (e: KeyboardEvent) => {
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
  // 搜索词变化时实时查询后端
  searchPasswords(newQuery)
})

/**
 * 初始化
 */
onMounted(async () => {
  // 初始化主题
  initTheme()

  // 加载设置
  await loadSettings()

  // 实时检查解锁状态
  await checkUnlockStatus()

  // 监听锁定事件（来自主应用）
  await listen('app-locked', () => {
    // 收到锁定事件，立即清除数据
    isUnlocked.value = false
    results.value = []
    searchQuery.value = ''
    masterPassword.value = ''
  })

  // 监听窗口显示事件（每次窗口从隐藏变为显示时触发）
  await listen('window-shown', () => {
    checkUnlockStatus()
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
  <div class="h-screen bg-transparent text-foreground flex flex-col">
    <!-- 未解锁 -->
    <div v-if="!isUnlocked" class="flex-1 flex items-center justify-center bg-card">
      <div class="w-full max-w-sm bg-card border shadow-lg">
        <div class="p-6 text-center">
          <div class="inline-flex items-center justify-center w-14 h-14 rounded-xl bg-primary/10 mb-4">
            <svg xmlns="http://www.w3.org/2000/svg" class="h-7 w-7 text-primary" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
              <path d="M7 11V7a5 5 0 0 1 10 0v4" />
            </svg>
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
          </div>
        </div>
        <div class="px-4 py-2.5 border-t bg-muted/30 text-center">
          <span class="text-xs text-muted-foreground">按 Esc 关闭窗口</span>
        </div>
      </div>
    </div>

    <!-- 已解锁 - 搜索界面 -->
    <div v-else class="flex-1 flex flex-col bg-card backdrop-blur-sm border shadow-lg">
      <!-- 搜索框 -->
      <div class="flex items-center gap-3 px-4 h-12 border-b">
        <svg xmlns="http://www.w3.org/2000/svg" class="h-4 w-4 text-muted-foreground" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="11" cy="11" r="8" />
          <path d="m21 21-4.3-4.3" />
        </svg>
        <input
          v-model="searchQuery"
          type="text"
          placeholder="搜索密码..."
          class="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground/50"
          autofocus
        />
      </div>

      <!-- 结果区域 -->
      <div class="flex-1 overflow-y-auto">
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
            class="flex items-center gap-3 px-4 py-2.5 cursor-pointer transition-colors border-b last:border-b-0"
            :class="{
              'bg-primary text-primary-foreground': index === selectedIndex,
              'hover:bg-muted': index !== selectedIndex,
            }"
            @click="copyPasswordByIdAndClose(item.id)"
            @mouseenter="selectedIndex = index"
          >
            <div class="flex-1 min-w-0">
              <div class="text-sm font-medium truncate">{{ item.title }}</div>
              <div class="text-xs truncate mt-0.5" :class="index === selectedIndex ? 'text-primary-foreground/70' : 'text-muted-foreground'">
                {{ item.username }}
              </div>
            </div>
            <div class="flex gap-1">
              <span
                class="px-2 py-1 text-xs rounded cursor-pointer transition-colors"
                :class="index === selectedIndex ? 'bg-primary-foreground/15 hover:bg-primary-foreground/25 text-primary-foreground' : 'hover:bg-muted text-muted-foreground hover:text-foreground'"
                @click.stop="copyAndClose(item.username)"
              >
                复制用户
              </span>
              <span
                class="px-2 py-1 text-xs rounded cursor-pointer transition-colors"
                :class="index === selectedIndex ? 'bg-primary-foreground/15 hover:bg-primary-foreground/25 text-primary-foreground' : 'hover:bg-muted text-muted-foreground hover:text-foreground'"
                @click.stop="copyPasswordByIdAndClose(item.id)"
              >
                复制密码
              </span>
            </div>
          </div>
        </template>
      </div>

      <!-- 底部快捷键 -->
      <div class="flex items-center justify-between px-4 py-2 border-t bg-muted/30 text-xs text-muted-foreground">
        <div class="flex gap-3">
          <span>↑↓ 导航</span>
          <span>↵ 复制</span>
          <span>Esc 关闭</span>
        </div>
        <span>摸鱼密码</span>
      </div>
    </div>

    <!-- Toast -->
    <Toast v-model:show="toast.show" :type="toast.type" :message="toast.message" />
  </div>
</template>
