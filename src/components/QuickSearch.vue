<!--
  QuickSearch.vue - 快速搜索组件

  全局快速搜索弹窗，用于快速查找和复制密码。
  类似于 VS Code 的 Ctrl+K 搜索功能。

  功能：
  1. 全局快捷键 Ctrl+K 呼出
  2. 实时搜索密码标题、用户名、URL
  3. 键盘导航（上下箭头选择，回车复制）
  4. 最多显示 10 条结果
  5. 支持复制用户名和密码

  @example
  ```vue
  <QuickSearch v-model:open="showQuickSearch" />
  ```
-->

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { storeToRefs } from 'pinia'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Dialog,
  DialogContent,
} from '@/components/ui/dialog'
import { usePasswordStore } from '@/stores/password'
import Toast from './Toast.vue'

/**
 * 组件 Props
 */
interface Props {
  /** 控制弹窗是否打开 */
  open: boolean
}

const props = defineProps<Props>()

/**
 * 组件事件
 * - update:open: 更新弹窗打开状态
 */
const emit = defineEmits<{
  'update:open': [value: boolean]
}>()

/** 密码 Store */
const passwordStore = usePasswordStore()
const { passwords } = storeToRefs(passwordStore)

/** 搜索关键词 */
const searchQuery = ref('')

/** 当前选中的结果索引（用于键盘导航） */
const selectedIndex = ref(0)

/** Toast 提示状态 */
const toast = ref({
  show: false,
  type: 'success' as 'success' | 'error' | 'info',
  message: '',
})

/**
 * 搜索结果（计算属性）
 *
 * 搜索逻辑：
 * - 如果搜索词为空，显示前 10 条密码
 * - 否则按标题、用户名、URL 过滤，最多显示 10 条
 */
const results = computed(() => {
  // 空搜索时显示前 10 条
  if (!searchQuery.value) return passwords.value.slice(0, 10)

  // 按关键词过滤
  const query = searchQuery.value.toLowerCase()
  return passwords.value
    .filter(
      (p) =>
        p.title.toLowerCase().includes(query) ||
        p.username.toLowerCase().includes(query) ||
        p.url?.toLowerCase().includes(query),
    )
    .slice(0, 10) // 最多显示 10 条
})

/**
 * 复制文本到剪贴板
 * @param text - 要复制的文本
 *
 * 复制成功后会关闭弹窗
 */
const copyToClipboard = async (text: string) => {
  try {
    await navigator.clipboard.writeText(text)
    toast.value = { show: true, type: 'success', message: '已复制到剪贴板' }
    // 复制成功后关闭弹窗
    emit('update:open', false)
  } catch {
    toast.value = { show: true, type: 'error', message: '复制失败' }
  }
}

/**
 * 处理键盘事件
 * 支持的按键：
 * - ArrowDown: 选择下一条结果
 * - ArrowUp: 选择上一条结果
 * - Enter: 复制当前选中结果的密码
 * - Escape: 关闭弹窗
 *
 * @param e - 键盘事件
 */
const handleKeydown = (e: KeyboardEvent) => {
  if (e.key === 'ArrowDown') {
    // 向下选择，不超过最后一条
    e.preventDefault()
    selectedIndex.value = Math.min(selectedIndex.value + 1, results.value.length - 1)
  } else if (e.key === 'ArrowUp') {
    // 向上选择，不超过第一条
    e.preventDefault()
    selectedIndex.value = Math.max(selectedIndex.value - 1, 0)
  } else if (e.key === 'Enter') {
    // 回车复制当前选中项的密码
    e.preventDefault()
    const selectedItem = results.value[selectedIndex.value]
    if (selectedItem) {
      copyToClipboard(selectedItem.password)
    }
  } else if (e.key === 'Escape') {
    // Esc 关闭弹窗
    emit('update:open', false)
  }
}

/**
 * 监听弹窗打开状态
 * 每次打开时重置搜索词和选中索引
 */
watch(
  () => props.open,
  (val) => {
    if (val) {
      searchQuery.value = ''
      selectedIndex.value = 0
    }
  },
)

/**
 * 注册全局快捷键
 * Ctrl+K (Windows/Linux) 或 Cmd+K (Mac) 呼出搜索
 *
 * 组件挂载时注册，卸载时自动移除
 */
onMounted(() => {
  const handler = (e: KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
      e.preventDefault()
      emit('update:open', true)
    }
  }
  window.addEventListener('keydown', handler)
  // 组件卸载时移除监听
  onUnmounted(() => window.removeEventListener('keydown', handler))
})
</script>

<template>
  <!-- 快速搜索弹窗 -->
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <DialogContent class="sm:max-w-lg p-0 gap-0">
      <!-- 搜索输入框区域 -->
      <div class="flex items-center border-b px-4">
        <!-- 搜索图标 -->
        <svg
          xmlns="http://www.w3.org/2000/svg"
          class="h-5 w-5 text-muted-foreground shrink-0"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <circle cx="11" cy="11" r="8" />
          <path d="m21 21-4.3-4.3" />
        </svg>
        <!-- 搜索输入框 -->
        <Input
          v-model="searchQuery"
          placeholder="搜索密码... (Ctrl+K)"
          class="border-0 focus-visible:ring-0 shadow-none"
          autofocus
          @keydown="handleKeydown"
        />
      </div>

      <!-- 搜索结果列表 -->
      <div class="max-h-[300px] overflow-y-auto p-2">
        <!-- 空状态提示 -->
        <div v-if="results.length === 0" class="py-6 text-center text-muted-foreground">
          未找到匹配的密码
        </div>
        <!-- 结果列表 -->
        <div
          v-for="(item, index) in results"
          :key="item.id"
          class="flex items-center justify-between p-3 rounded-md cursor-pointer transition-colors"
          :class="{
            'bg-muted': index === selectedIndex,        // 选中状态
            'hover:bg-muted/50': index !== selectedIndex, // 悬停状态
          }"
          @click="copyToClipboard(item.password)"
          @mouseenter="selectedIndex = index"
        >
          <!-- 左侧：密码信息 -->
          <div class="flex-1 min-w-0">
            <div class="font-medium truncate">{{ item.title }}</div>
            <div class="text-sm text-muted-foreground truncate">{{ item.username }}</div>
          </div>
          <!-- 右侧：复制按钮 -->
          <div class="flex items-center gap-2 ml-4">
            <!-- 复制用户名按钮 -->
            <Button
              variant="ghost"
              size="sm"
              class="h-8 px-2 text-xs"
              @click.stop="copyToClipboard(item.username)"
            >
              复制用户
            </Button>
            <!-- 复制密码按钮 -->
            <Button
              variant="ghost"
              size="sm"
              class="h-8 px-2 text-xs"
              @click.stop="copyToClipboard(item.password)"
            >
              复制密码
            </Button>
          </div>
        </div>
      </div>

      <!-- 底部键盘快捷键提示 -->
      <div class="border-t px-4 py-2 text-xs text-muted-foreground flex gap-4">
        <span>↑↓ 导航</span>
        <span>↵ 复制密码</span>
        <span>Esc 关闭</span>
      </div>
    </DialogContent>
  </Dialog>

  <!-- Toast 提示 -->
  <Toast
    v-model:show="toast.show"
    :type="toast.type"
    :message="toast.message"
  />
</template>
