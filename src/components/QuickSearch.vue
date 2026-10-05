<!--
  QuickSearch.vue - 快速搜索组件

  全局快速搜索弹窗，用于快速查找和复制密码。
  类似于 VS Code 的 Ctrl+K 搜索功能。

  功能：
  1. 全局快捷键 Ctrl+K 呼出
  2. 实时搜索密码标题、用户名、URL、自定义字段
  3. 键盘导航（上下箭头选择，回车复制）
  4. 最多显示 10 条结果
  5. 支持复制用户名和密码
  6. →/Tab 或「详情」按钮展开字段面板，按字段复制

  @example
  ```vue
  <QuickSearch v-model:open="showQuickSearch" />
  ```
-->

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { storeToRefs } from 'pinia'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Dialog,
  DialogContent,
} from '@/components/ui/dialog'
import { usePasswordStore, type CustomField } from '@/stores/password'
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
const { getPasswordDetail } = passwordStore

/** 搜索关键词 */
const searchQuery = ref('')

/** 当前选中的结果索引（用于键盘导航） */
const selectedIndex = ref(0)

/** 当前展开详情面板的条目 ID */
const expandedId = ref<string | null>(null)

/** 详情字段缓存（id → 字段列表；敏感值经 get_password_detail 解密） */
const detailFields = ref<Record<string, CustomField[]>>({})

/** 详情加载中 */
const detailLoading = ref(false)

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
    await invoke('copy_text_to_clipboard', { text, clearAfter: null })
    toast.value = { show: true, type: 'success', message: '已复制到剪贴板' }
    // 复制成功后关闭弹窗
    emit('update:open', false)
  } catch {
    toast.value = { show: true, type: 'error', message: '复制失败' }
  }
}

/**
 * 通过后端复制密码（前端不接触明文）
 * @param id - 密码 ID
 */
const copyPasswordById = async (id: string) => {
  try {
    await invoke('copy_password_to_clipboard', { id })
    toast.value = { show: true, type: 'success', message: '密码已复制到剪贴板' }
    emit('update:open', false)
  } catch {
    toast.value = { show: true, type: 'error', message: '复制密码失败' }
  }
}

/**
 * 展开/收起条目详情面板
 *
 * 展开时按需调用 get_password_detail 解密敏感字段值（列表零解密）。
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
    const detail = await getPasswordDetail(item.id)
    detailFields.value = { ...detailFields.value, [item.id]: detail.extra_fields || [] }
  } catch (e) {
    console.error('加载详情失败:', e)
    toast.value = { show: true, type: 'error', message: '加载详情失败' }
  } finally {
    detailLoading.value = false
  }
}

/**
 * 复制单个字段值并关闭弹窗（与复制用户名/密码行为一致）
 * @param text - 字段值文本
 */
const copyFieldText = async (text: string) => {
  await copyToClipboard(text)
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
    // 回车复制当前选中项的密码
    e.preventDefault()
    const selectedItem = results.value[selectedIndex.value]
    if (selectedItem) {
      copyPasswordById(selectedItem.id)
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
      expandedId.value = null
      // 每次打开丢弃已解密的字段缓存
      detailFields.value = {}
    }
  },
)

/**
 * 搜索词变化时收起详情并清空字段缓存
 */
watch(searchQuery, () => {
  expandedId.value = null
  detailFields.value = {}
})

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
        <div v-for="(item, index) in results" :key="item.id" class="rounded-md overflow-hidden">
          <!-- 结果行 -->
          <div
            class="flex items-center justify-between p-3 cursor-pointer transition-colors"
            :class="{
              'bg-muted': index === selectedIndex,        // 选中状态
              'hover:bg-muted/50': index !== selectedIndex, // 悬停状态
            }"
            @click="copyPasswordById(item.id)"
            @mouseenter="selectedIndex = index"
          >
            <!-- 左侧：密码信息 -->
            <div class="flex-1 min-w-0">
              <div class="font-medium truncate">{{ item.title }}</div>
              <div class="text-sm text-muted-foreground truncate">{{ item.username }}</div>
            </div>
            <!-- 右侧：操作按钮 -->
            <div class="flex items-center gap-2 ml-4">
              <!-- 详情按钮：展开/收起字段面板 -->
              <Button
                v-if="item.extra_fields && item.extra_fields.length > 0"
                variant="ghost"
                size="sm"
                class="h-8 px-2 text-xs"
                @click.stop="toggleDetail(item)"
              >
                {{ expandedId === item.id ? '收起' : '详情' }}
              </Button>
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
                @click.stop="copyPasswordById(item.id)"
              >
                复制密码
              </Button>
            </div>
          </div>

          <!-- 字段详情面板（→/Tab 或「详情」按钮展开） -->
          <div v-if="expandedId === item.id" class="px-3 pb-2 bg-muted/40 space-y-1">
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
                <span v-if="field.sensitive" class="shrink-0" title="敏感字段，加密存储">🔒</span>
                <Button
                  v-if="field.value"
                  variant="ghost"
                  size="sm"
                  class="h-6 px-1.5 text-xs shrink-0"
                  @click.stop="copyFieldText(field.value)"
                >
                  复制
                </Button>
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
      </div>

      <!-- 底部键盘快捷键提示 -->
      <div class="border-t px-4 py-2 text-xs text-muted-foreground flex gap-4">
        <span>↑↓ 导航</span>
        <span>↵ 复制密码</span>
        <span>→/Tab 详情</span>
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
