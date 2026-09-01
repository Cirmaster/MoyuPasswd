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
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import Toast from '@/components/Toast.vue'

/** 搜索关键词 */
const searchQuery = ref('')

/** 当前选中的结果索引 */
const selectedIndex = ref(0)

/** 加载状态 */
const loading = ref(false)

/** 主题：light 或 dark */
const theme = ref<'light' | 'dark'>('dark')

/** 是否已解锁 */
const isUnlocked = ref(false)

/** 主密码输入 */
const masterPassword = ref('')

/** 解锁错误信息 */
const unlockError = ref('')

/** 解锁加载状态 */
const unlockLoading = ref(false)

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
 * 实时查询后端（不缓存数据）
 */
const searchPasswords = async (query: string) => {
  if (!isUnlocked.value) {
    results.value = []
    return
  }

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
  } catch (e) {
    console.error('查询密码失败:', e)
    results.value = []
  } finally {
    loading.value = false
  }
}

/**
 * 检查解锁状态（实时查询后端）
 */
const checkUnlockStatus = async () => {
  try {
    isUnlocked.value = await invoke<boolean>('is_unlocked')

    // 如果已解锁，初始化查询
    if (isUnlocked.value) {
      await searchPasswords('')
    } else {
      // 未解锁，清除数据
      results.value = []
      searchQuery.value = ''
    }
  } catch (e) {
    console.error('检查解锁状态失败:', e)
    isUnlocked.value = false
    results.value = []
  }
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
    await navigator.clipboard.writeText(text)
    toast.value = { show: true, type: 'success', message: '已复制到剪贴板' }

    // 启动全局倒计时窗口（显示在屏幕中央）
    const x = window.screen.width / 2
    const y = window.screen.height / 2
    await invoke('show_countdown', { seconds: 10, x, y })

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
  // 如果未解锁，不处理导航键
  if (!isUnlocked.value) {
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
      copyAndClose(selectedItem.password)
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
  // 读取系统主题
  const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches
  theme.value = isDark ? 'dark' : 'light'

  // 监听系统主题变化
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (e) => {
    theme.value = e.matches ? 'dark' : 'light'
  })

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

  // 窗口获得焦点时，实时查询一次（确保显示最新数据）
  const currentWindow = getCurrentWindow()
  currentWindow.onFocusChanged(({ payload: focused }) => {
    if (focused && isUnlocked.value) {
      // 窗口获得焦点时，实时查询
      searchPasswords(searchQuery.value)
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
  <div
    class="min-h-screen p-4 transition-colors duration-200"
    :class="theme === 'dark' ? 'bg-gray-900/95 text-gray-100' : 'bg-white/95 text-gray-900'"
  >
    <!-- 未解锁：显示密码输入 -->
    <div v-if="!isUnlocked" class="flex flex-col items-center justify-center min-h-[300px]">
      <svg
        xmlns="http://www.w3.org/2000/svg"
        class="h-12 w-12 mb-4"
        :class="theme === 'dark' ? 'text-gray-400' : 'text-gray-500'"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
      >
        <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
        <path d="M7 11V7a5 5 0 0 1 10 0v4" />
      </svg>
      <h2 class="text-lg font-medium mb-4">密码库已锁定</h2>
      <div class="w-full max-w-xs space-y-3">
        <div>
          <Label for="master-password" class="sr-only">主密码</Label>
          <Input
            id="master-password"
            v-model="masterPassword"
            type="password"
            placeholder="输入主密码解锁"
            :class="theme === 'dark' ? 'bg-gray-800 border-gray-700' : 'bg-gray-100 border-gray-200'"
            autofocus
            @keydown.enter="handleUnlock"
          />
          <p v-if="unlockError" class="text-sm text-red-500 mt-1">{{ unlockError }}</p>
        </div>
        <Button
          class="w-full"
          :disabled="unlockLoading"
          @click="handleUnlock"
        >
          {{ unlockLoading ? '解锁中...' : '解锁' }}
        </Button>
      </div>
      <p class="text-xs mt-4" :class="theme === 'dark' ? 'text-gray-500' : 'text-gray-400'">
        按 Esc 关闭窗口
      </p>
    </div>

    <!-- 已解锁：显示搜索 -->
    <template v-else>
      <!-- 搜索框 -->
      <div class="relative mb-4">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          class="absolute left-3 top-1/2 -translate-y-1/2 h-5 w-5"
          :class="theme === 'dark' ? 'text-gray-400' : 'text-gray-500'"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <circle cx="11" cy="11" r="8" />
          <path d="m21 21-4.3-4.3" />
        </svg>
        <Input
          v-model="searchQuery"
          placeholder="搜索密码..."
          class="pl-10 text-lg h-12"
          :class="theme === 'dark' ? 'bg-gray-800 border-gray-700' : 'bg-gray-100 border-gray-200'"
          autofocus
        />
      </div>

      <!-- 加载状态 -->
      <div v-if="loading" class="text-center py-8" :class="theme === 'dark' ? 'text-gray-400' : 'text-gray-500'">
        加载中...
      </div>

      <!-- 搜索结果 -->
      <div v-else class="space-y-1 max-h-[350px] overflow-y-auto">
        <div v-if="results.length === 0" class="text-center py-8" :class="theme === 'dark' ? 'text-gray-400' : 'text-gray-500'">
          未找到匹配的密码
        </div>

        <div
          v-for="(item, index) in results"
          :key="item.id"
          :data-index="index"
          class="flex items-center justify-between p-3 rounded-lg cursor-pointer transition-colors"
          :class="[
            index === selectedIndex ? 'bg-blue-600 text-white' : '',
            index !== selectedIndex ? (theme === 'dark' ? 'hover:bg-gray-800' : 'hover:bg-gray-100') : '',
          ]"
          @click="copyAndClose(item.password)"
          @mouseenter="selectedIndex = index"
        >
          <div class="flex-1 min-w-0">
            <div class="font-medium truncate">{{ item.title }}</div>
            <div
              class="text-sm truncate"
              :class="index === selectedIndex ? 'text-blue-100' : (theme === 'dark' ? 'text-gray-400' : 'text-gray-500')"
            >
              {{ item.username }}
            </div>
          </div>
          <div class="flex items-center gap-2 ml-4">
            <Button
              variant="ghost"
              size="sm"
              class="h-8 px-2 text-xs"
              :class="index === selectedIndex ? 'hover:bg-blue-500 text-white' : ''"
              @click.stop="copyAndClose(item.username)"
            >
              复制用户
            </Button>
            <Button
              variant="ghost"
              size="sm"
              class="h-8 px-2 text-xs"
              :class="index === selectedIndex ? 'hover:bg-blue-500 text-white' : ''"
              @click.stop="copyAndClose(item.password)"
            >
              复制密码
            </Button>
          </div>
        </div>
      </div>

      <!-- 底部提示 -->
      <div
        class="fixed bottom-4 left-4 right-4 flex justify-between text-xs"
        :class="theme === 'dark' ? 'text-gray-500' : 'text-gray-400'"
      >
        <div class="flex gap-4">
          <span>↑↓ 导航</span>
          <span>↵ 复制密码</span>
          <span>Esc 关闭</span>
        </div>
        <span>摸鱼密码</span>
      </div>
    </template>

    <!-- Toast 提示 -->
    <Toast
      v-model:show="toast.show"
      :type="toast.type"
      :message="toast.message"
    />
  </div>
</template>
