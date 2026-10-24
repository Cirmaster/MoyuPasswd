<!--
  QuickSearch.vue - 快速搜索组件

  全局快速搜索弹窗，用于快速查找和复制密码。
  类似于 VS Code 的 Ctrl+K 搜索功能。

  功能：
  1. 全局快捷键 Ctrl+K 呼出
  2. 搜索全部由后端完成（get_passwords）：空格分词 AND，匹配标题/用户名/URL/标签/分类名/自定义字段，
     并支持中文拼音全拼/简拼（后端 pinyin）；前端只负责展示返回的当前页
  3. 无搜索词时后端按最近使用（last_used_at desc）返回前 10 条
  4. 结果行展示标签小徽章（最多 3 个 +N）与分类名
  5. 键盘导航（上下箭头选择，回车复制）
  6. 最多显示 10 条结果
  7. 支持复制用户名和密码：复制密码后刷新 Store 列表（使用记录由后端登记）；
     复制用户名后调用 store.touchUsage 记录使用（失败静默）
  8. →/Tab 或「详情」按钮展开字段面板，按字段复制

  @example
  ```vue
  <QuickSearch v-model:open="showQuickSearch" />
  ```
-->

<script setup lang="ts">
import { ref, watch, onUnmounted } from 'vue'
import { storeToRefs } from 'pinia'
import { invoke } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Dialog,
  DialogContent,
} from '@/components/ui/dialog'
import { usePasswordStore, type CustomField, type PasswordItem } from '@/stores/password'
import { IconSearch, IconLock } from '@/components/icons'
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

/** 每次查询的结果条数（后端分页 pageSize） */
const PAGE_SIZE = 10

/** 搜索输入防抖时间（毫秒）：避免每次按键都打后端 */
const SEARCH_DEBOUNCE_MS = 150

/**
 * 后端 get_passwords 的分页返回（契约 §11.2 PasswordPage）
 * 前端只使用 items / total，其余字段保留以贴合后端结构。
 */
interface PasswordPage {
  /** 当前页数据（列表零解密：password 为空串） */
  items: PasswordItem[]
  /** 过滤后的总条数 */
  total: number
  /** 实际返回页码（越界已由后端钳制） */
  page: number
  /** 每页条数 */
  page_size: number
}

/** 密码 Store（分类名映射用于展示；复制密码后刷新列表） */
const passwordStore = usePasswordStore()
const { categoryNameById } = storeToRefs(passwordStore)
const { getPasswordDetail } = passwordStore

/** 搜索关键词 */
const searchQuery = ref('')

/** 当前结果（后端返回的当前页，前端只展示，不做本地过滤） */
const results = ref<PasswordItem[]>([])

/** 结果加载中 */
const resultsLoading = ref(false)

/** 请求序号：防止慢的旧响应覆盖新响应 */
let requestSeq = 0

/** 搜索防抖定时器 */
let debounceTimer: ReturnType<typeof setTimeout> | null = null

/** 打开弹窗重置搜索词时，跳过 searchQuery watch 触发的重复请求 */
let skipNextQuerySearch = false

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
 * 标签兜底读取
 *
 * 契约中 PasswordItem.tags 为必有字段；这里仍兜底空数组，
 * 兼容标签迁移尚未落地时后端返回的旧数据。
 * @param item - 结果条目
 */
const tagsOf = (item: { tags?: string[] }): string[] => item.tags ?? []

/**
 * 分类展示名（分类 id → 名称；映射缺失时退回分类 id，避免空白）
 * @param category - 分类 ID
 */
const categoryLabel = (category: string): string => categoryNameById.value[category] ?? category

/**
 * 从后端拉取搜索结果（前端只展示，不做本地过滤/排序）
 *
 * 请求参数（契约 §11.3）：
 * - search：搜索词，空搜索传 null（后端按最近使用倒序返回）
 * - category/tags：快速搜索不带筛选条件
 * - sortBy：有搜索词按更新时间倒序，空搜索按最近使用（last_used_at）倒序
 * - page/pageSize：固定第 1 页、10 条
 *
 * @param query - 搜索关键词（空串表示「最近使用」）
 */
const searchPasswords = async (query: string) => {
  const keyword = query.trim()
  const seq = ++requestSeq
  resultsLoading.value = true

  try {
    const page = await invoke<PasswordPage>('get_passwords', {
      search: keyword || null,
      category: null,
      tags: null,
      sortBy: keyword ? 'updated_at' : 'last_used_at',
      sortOrder: 'desc',
      page: 1,
      pageSize: PAGE_SIZE,
    })

    // 旧请求已被更新的请求取代，丢弃其响应
    if (seq !== requestSeq) return

    results.value = page?.items ?? []
    // 结果条数变化后把选中索引钳制到有效范围
    const maxIndex = Math.max(0, results.value.length - 1)
    if (selectedIndex.value > maxIndex) selectedIndex.value = maxIndex
  } catch (e) {
    if (seq !== requestSeq) return
    console.error('快速搜索查询失败:', e)
    results.value = []
  } finally {
    // 只有最新一次请求负责结束 loading
    if (seq === requestSeq) resultsLoading.value = false
  }
}

/**
 * 防抖调度搜索（输入停止 150ms 后才请求后端）
 * @param query - 搜索关键词
 */
const scheduleSearch = (query: string) => {
  if (debounceTimer !== null) {
    clearTimeout(debounceTimer)
  }
  debounceTimer = setTimeout(() => {
    debounceTimer = null
    void searchPasswords(query)
  }, SEARCH_DEBOUNCE_MS)
}

/** 取消未执行的防抖请求 */
const cancelPendingSearch = () => {
  if (debounceTimer !== null) {
    clearTimeout(debounceTimer)
    debounceTimer = null
  }
}

/**
 * 记录使用（fire-and-forget）
 *
 * 不等待后端返回、不阻塞复制交互；失败静默（使用记录不是关键路径）。
 * @param id - 密码 ID
 */
const touchUsageSilently = (id: string) => {
  void passwordStore.touchUsage(id).catch(() => {})
}

/**
 * 复制文本到剪贴板
 * @param text - 要复制的文本
 *
 * 复制成功后会关闭弹窗（用于详情面板中单个字段值的复制）
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
 * 复制用户名（成功后静默记录一次使用并关闭弹窗）
 * @param item - 结果条目
 */
const copyUsername = async (item: PasswordItem) => {
  try {
    await invoke('copy_text_to_clipboard', { text: item.username, clearAfter: null })
    toast.value = { show: true, type: 'success', message: '已复制到剪贴板' }
    // 复制属于一次「使用」，异步记录，不阻塞关闭
    touchUsageSilently(item.id)
    emit('update:open', false)
  } catch {
    toast.value = { show: true, type: 'error', message: '复制失败' }
  }
}

/**
 * 通过后端复制密码（前端不接触明文）
 *
 * 使用记录由后端 `copy_password_to_clipboard` 统一更新（契约 4.3），
 * 前端不再重复调用 touchUsage，避免同一动作计数 +2；
 * 复制成功后刷新 Store 列表，让主界面「最近使用」/使用频率排序即时生效。
 * @param id - 密码 ID
 */
const copyPasswordById = async (id: string) => {
  try {
    await invoke('copy_password_to_clipboard', { id })
    toast.value = { show: true, type: 'success', message: '密码已复制到剪贴板' }
    // 后端已记录使用记录；刷新主列表当前页与统计（最近使用计数/排序即时生效）
    void passwordStore.loadPasswords()
    void passwordStore.loadStats()
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
 * - ArrowRight/Tab: 展开/收起选中结果的字段详情
 * - ArrowLeft: 收起详情面板
 * - Escape: 关闭弹窗
 *
 * @param e - 键盘事件
 */
const handleKeydown = (e: KeyboardEvent) => {
  if (e.key === 'ArrowDown') {
    // 向下选择，不超过最后一条（无结果时保持 0，避免选中项悬空）
    e.preventDefault()
    selectedIndex.value = Math.min(selectedIndex.value + 1, Math.max(0, results.value.length - 1))
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
 * 每次打开时重置搜索词/选中索引，并立即拉取「最近使用」前 10 条
 */
watch(
  () => props.open,
  (val) => {
    if (val) {
      // 打开前先取消可能残留的防抖请求，保证本次打开只按「最近使用」请求一次
      cancelPendingSearch()
      const hadQuery = searchQuery.value !== ''
      // 重置搜索词；若有旧词，本次 watch 会触发 searchQuery 的 watch，用标记跳过重复请求
      searchQuery.value = ''
      skipNextQuerySearch = hadQuery
      selectedIndex.value = 0
      expandedId.value = null
      // 每次打开丢弃已解密的字段缓存
      detailFields.value = {}
      // 立即请求（打开弹窗即展示最近使用）
      void searchPasswords('')
    } else {
      // 关闭时取消待执行防抖，并让在途请求的响应失效（避免关闭后写回结果）
      cancelPendingSearch()
      requestSeq++
      resultsLoading.value = false
    }
  },
)

/**
 * 搜索词变化时：收起详情、清空字段缓存、回到第一条，并防抖请求后端
 */
watch(searchQuery, () => {
  selectedIndex.value = 0
  expandedId.value = null
  detailFields.value = {}

  // 弹窗未打开时不发请求
  if (!props.open) return
  // 打开弹窗时重置搜索词引起的重复触发，跳过（打开时已立即请求）
  if (skipNextQuerySearch) {
    skipNextQuerySearch = false
    return
  }
  scheduleSearch(searchQuery.value)
})

/**
 * 结果条数变化时钳制选中索引，避免「选中项悬空」导致键盘操作无响应
 */
watch(
  () => results.value.length,
  (length) => {
    const maxIndex = Math.max(0, length - 1)
    if (selectedIndex.value > maxIndex) selectedIndex.value = maxIndex
  },
)

/**
 * 卸载清理
 *
 * 注：Ctrl+K 呼出的是独立快速搜索小窗（后端全局快捷键），
 * 本内嵌弹窗不再拦截 Ctrl+K——旧写法与全局快捷键同时触发，会双弹窗；
 * 内嵌弹窗仅由主界面的搜索入口打开。
 */
onUnmounted(() => {
  // 卸载时清理防抖定时器
  cancelPendingSearch()
  requestSeq++
})
</script>

<template>
  <!-- 快速搜索弹窗 -->
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <DialogContent class="sm:max-w-lg p-0 gap-0">
      <!-- 搜索输入框区域 -->
      <div class="flex items-center border-b px-4">
        <!-- 搜索图标 -->
        <IconSearch class="h-5 w-5 text-muted-foreground shrink-0" />
        <!-- 搜索输入框（后端搜索：分词 AND + 标签/分类名 + 拼音全拼/简拼） -->
        <Input
          v-model="searchQuery"
          placeholder="搜索名称 / 用户名 / 网址 / 标签 / 拼音… (Ctrl+K)"
          class="border-0 focus-visible:ring-0 shadow-none"
          autofocus
          @keydown="handleKeydown"
        />
      </div>

      <!-- 搜索结果列表 -->
      <div class="max-h-[300px] overflow-y-auto p-2">
        <!-- 加载中（仅首次/无旧结果时展示，避免结果区闪烁） -->
        <div
          v-if="resultsLoading && results.length === 0"
          class="py-6 text-center text-muted-foreground"
        >
          搜索中...
        </div>
        <!-- 空状态提示 -->
        <div v-else-if="results.length === 0" class="py-6 text-center text-muted-foreground">
          未找到匹配的密码
        </div>
        <!-- 结果列表 -->
        <div v-for="(item, index) in results" :key="item.id" class="mb-1">
          <!-- 结果行（选中高亮 = action 色，与复制密码按钮严格同色；暗色主题为深板岩蓝而非近白） -->
          <div
            class="flex items-stretch cursor-pointer transition-colors"
            :class="{
              'bg-action text-action-foreground': index === selectedIndex,
              'hover:bg-action/10': index !== selectedIndex,
            }"
            @click="copyPasswordById(item.id)"
            @mouseenter="selectedIndex = index"
          >
            <!-- 左侧：密码信息 + 标签徽章 + 分类名 -->
            <div class="flex-1 min-w-0 flex flex-col justify-center p-3">
              <div class="font-medium truncate">{{ item.title }}</div>
              <div class="text-sm truncate" :class="index === selectedIndex ? 'text-action-foreground/70' : 'text-muted-foreground'">
                {{ item.username }}
              </div>
              <!-- 分类名 + 标签小徽章（最多 3 个，超出显示 +N，title 展示全部标签） -->
              <div class="mt-1 flex items-center gap-1 min-w-0 overflow-hidden">
                <span
                  v-if="categoryLabel(item.category)"
                  class="shrink-0 rounded px-1 py-px text-[10px] leading-4"
                  :class="index === selectedIndex
                    ? 'bg-action-foreground/15 text-action-foreground/90'
                    : 'bg-primary-ink/10 text-primary-ink'"
                >
                  {{ categoryLabel(item.category) }}
                </span>
                <span
                  v-for="tag in tagsOf(item).slice(0, 3)"
                  :key="tag"
                  class="shrink-0 max-w-[96px] truncate rounded px-1 py-px text-[10px] leading-4"
                  :class="index === selectedIndex
                    ? 'bg-action-foreground/10 text-action-foreground/80'
                    : 'bg-muted text-muted-foreground'"
                  :title="tag"
                >
                  #{{ tag }}
                </span>
                <span
                  v-if="tagsOf(item).length > 3"
                  class="shrink-0 text-[10px] leading-4"
                  :class="index === selectedIndex ? 'text-action-foreground/70' : 'text-muted-foreground'"
                  :title="tagsOf(item).join('、')"
                >
                  +{{ tagsOf(item).length - 3 }}
                </span>
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
              <!-- 复制用户（中等档；成功后静默记录使用） -->
              <button
                class="w-[76px] flex items-center justify-center text-xs font-medium transition-colors"
                :class="index === selectedIndex
                  ? 'bg-action-foreground/20 text-action-foreground hover:bg-action-foreground/35'
                  : 'bg-primary-ink/10 text-primary-ink hover:bg-primary-ink/25'"
                @click.stop="copyUsername(item)"
              >
                复制用户
              </button>
              <!-- 复制密码（主操作档）：走 keycap 键帽令牌，各主题可给反差色（经典=同行动作色、暮光=点睛金），hover 用提亮表达 -->
              <button
                class="w-[76px] flex items-center justify-center text-xs font-medium transition bg-keycap text-keycap-foreground hover:brightness-125 active:brightness-110"
                @click.stop="copyPasswordById(item.id)"
              >
                复制密码
              </button>
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
                <IconLock
                  v-if="field.sensitive"
                  class="shrink-0 size-3.5"
                  title="敏感字段，加密存储"
                />
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
        <span>← 收起</span>
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
