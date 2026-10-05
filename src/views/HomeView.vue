<!--
  HomeView.vue - 密码列表主页面

  这是应用的主界面，解锁后进入此页面。
  布局结构：
  - 顶部导航栏：Logo、快捷功能按钮、设置、主题切换、锁定
  - 左侧分类栏：显示所有分类，点击切换分类
  - 右侧内容区：搜索框 + 密码列表表格

  功能：
  1. 密码列表展示（表格形式）
  2. 按分类筛选
  3. 搜索过滤
  4. 收藏/取消收藏
  5. 复制用户名/密码
  6. 显示/隐藏密码
  7. 添加/编辑/删除密码
  8. 密码生成器
  9. 快速搜索（Ctrl+K）

  路由路径：/home
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { storeToRefs } from 'pinia'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { usePasswordStore, type PasswordItem } from '@/stores/password'
import { useTheme } from '@/composables/useTheme'
import { formatTimestamp } from '@/lib/utils'
import PasswordFormDialog from '@/components/PasswordFormDialog.vue'
import PasswordGenerator from '@/components/PasswordGenerator.vue'
import PasswordStrength from '@/components/PasswordStrength.vue'
import {
  IconLock,
  IconGrid,
  IconSettings,
  IconSun,
  IconMoon,
  IconMinus,
  IconSquarePen,
  IconSearch,
  IconPlus,
  IconStar,
  IconCopy,
  IconPencil,
  IconTrash,
  IconEmpty,
} from '@/components/icons'
import QuickSearch from '@/components/QuickSearch.vue'
import QuickAdd from '@/components/QuickAdd.vue'
import Toast from '@/components/Toast.vue'

// ==================== 依赖注入 ====================

/** 路由实例 */
const router = useRouter()

/** 主题管理 */
const { isDark, toggleTheme } = useTheme()

/** 剪贴板清除时间（秒），从设置中读取 */
const clipboardClearTime = ref(30)

/**
 * 复制文本到剪贴板（带自动清除）
 * @param text - 要复制的文本
 * @param event - 鼠标事件（用于获取位置）
 */
const copyToClipboard = async (text: string, event?: MouseEvent) => {
  try {
    await invoke('copy_text_to_clipboard', { text, clearAfter: clipboardClearTime.value })
    showToast('success', '已复制')

    // 获取鼠标位置并显示全局倒计时窗口
    const x = event?.screenX ?? window.screen.width / 2
    const y = event?.screenY ?? window.screen.height / 2
    await invoke('show_countdown', { seconds: clipboardClearTime.value, x, y })

    // 启动光标跟随
    await invoke('start_follow_cursor')
  } catch (e) {
    console.error('Copy failed:', e)
    showToast('error', '复制失败')
  }
}

/** 密码 Store */
const passwordStore = usePasswordStore()
// 从 Store 中解构响应式状态
const { categories, passwords, filteredPasswords, activeCategory, searchQuery, loading } = storeToRefs(passwordStore)
// 从 Store 中解构方法
const { setActiveCategory, setSearchQuery, toggleFavorite, deletePassword, loadPasswords, loadCategories } = passwordStore

// ==================== 生命周期 ====================

/**
 * 页面加载时从后端获取数据
 */
/** 事件监听清理函数（组件卸载时统一取消，防止叠加监听） */
const unlisteners: Array<() => void> = []

onUnmounted(() => {
  unlisteners.forEach((unlisten) => unlisten())
  unlisteners.length = 0
})

onMounted(async () => {
  await Promise.all([
    loadPasswords(),
    loadCategories(),
  ])

  // 加载设置
  try {
    const savedSettings = await invoke<{
      clipboard_clear_time: number
      auto_lock_time: number
    }>('get_settings')
    clipboardClearTime.value = savedSettings.clipboard_clear_time || 30
  } catch (e) {
    console.warn('加载设置失败:', e)
  }

  // 监听全局快捷键事件（保存 unlisten，卸载时统一清理，防止反复进出页面叠加监听）
  const { listen } = await import('@tauri-apps/api/event')
  unlisteners.push(
    await listen('show-quick-search', () => {
      showQuickSearch.value = true
    }),
  )

  // 监听后端锁定事件（由 lock_app 命令或系统托盘触发）
  unlisteners.push(
    await listen('app-locked', () => {
      // 清除本地密码数据
      passwords.value = []
      // 跳转到解锁页面
      router.push('/')
    }),
  )

  // 监听快速添加快捷键事件
  unlisteners.push(
    await listen('show-quick-add', () => {
      showQuickAdd.value = true
    }),
  )

  // 监听密码生成器快捷键事件
  unlisteners.push(
    await listen('show-password-generator', () => {
      showGenerator.value = true
    }),
  )
})

// ==================== 本地状态 ====================

/** 控制密码表单弹窗是否显示 */
const showFormDialog = ref(false)

/** 当前编辑的密码项，null 表示添加模式 */
const editingItem = ref<PasswordItem | null>(null)

/** 控制密码生成器弹窗是否显示 */
const showGenerator = ref(false)

/** 控制快速搜索弹窗是否显示 */
const showQuickSearch = ref(false)

/** 控制快速添加弹窗是否显示 */
const showQuickAdd = ref(false)

/** Toast 提示状态 */
const toast = ref({
  show: false,
  type: 'success' as 'success' | 'error' | 'info',
  message: '',
})

/**
 * 显示 Toast 提示
 * @param type - 提示类型
 * @param message - 提示消息
 */
const showToast = (type: 'success' | 'error' | 'info', message: string) => {
  toast.value = { show: true, type, message }
}

// ==================== 操作方法 ====================

/**
 * 打开添加密码弹窗
 * 将 editingItem 设为 null，弹窗进入添加模式
 */
const openAddDialog = () => {
  editingItem.value = null
  showFormDialog.value = true
}

/**
 * 打开编辑密码弹窗
 * @param item - 要编辑的密码项
 */
const openEditDialog = (item: PasswordItem) => {
  editingItem.value = item
  showFormDialog.value = true
}

/**
 * 通过后端登记待粘贴密码（不进系统剪贴板）
 * 前端全程不接触明文；倒计时内在目标窗口按 Ctrl+V 由后端解密并注入
 * @param id - 密码项 ID
 * @param event - 鼠标事件（用于定位倒计时窗口）
 */
const copyPasswordViaBackend = async (id: string, event?: MouseEvent) => {
  try {
    const x = event?.screenX ?? window.screen.width / 2
    const y = event?.screenY ?? window.screen.height / 2
    const clearTime = await invoke<number>('copy_password_to_clipboard', { id })
    showToast('success', '密码就绪，倒计时内按 Ctrl+V 粘贴')
    // 启动倒计时和光标跟随
    await invoke('show_countdown', { seconds: clearTime, x, y })
    await invoke('start_follow_cursor')
  } catch (e) {
    console.error('复制密码失败:', e)
    showToast('error', String(e))
  }
}

/**
 * 最小化到托盘
 * 隐藏窗口，显示在系统托盘
 */
const handleMinimizeToTray = async () => {
  try {
    await invoke('minimize_to_tray')
  } catch (e) {
    console.error('最小化到托盘失败:', e)
  }
}

/**
 * 锁定应用
 * 只需调用后端命令，后端负责清除密钥并通知所有窗口
 */
const handleLock = async () => {
  try {
    await invoke('lock_app')
  } catch (e) {
    console.error('锁定失败:', e)
  }
}
</script>

<template>
  <div class="h-screen flex flex-col bg-background">
    <!-- 顶部导航栏 -->
    <header class="border-b px-4 py-3 flex items-center justify-between">
      <div class="flex items-center gap-2">
        <IconLock class="h-6 w-6 text-primary" stroke-linecap="round" stroke-linejoin="round" />
        <h1 class="text-lg font-semibold">MoyuPasswd</h1>
      </div>

      <div class="flex items-center gap-2">
        <Button variant="ghost" size="icon" @click="showGenerator = true">
          <IconGrid class="h-5 w-5" />
        </Button>
        <Button variant="ghost" size="icon" @click="router.push('/settings')">
          <IconSettings class="h-5 w-5" />
        </Button>
        <Button variant="ghost" size="icon" @click="toggleTheme">
          <IconSun v-if="isDark" class="h-5 w-5" />
          <IconMoon v-else class="h-5 w-5" />
        </Button>
        <!-- 最小化到托盘按钮 -->
        <Button variant="ghost" size="icon" @click="handleMinimizeToTray" title="最小化到托盘">
          <IconMinus class="h-5 w-5" />
        </Button>
        <Button variant="ghost" size="icon" @click="handleLock">
          <IconLock class="h-5 w-5" />
        </Button>
      </div>
    </header>

    <div class="flex flex-1 overflow-hidden">
      <!-- 左侧分类栏 -->
      <aside class="w-48 border-r p-4 flex flex-col gap-1">
        <div
          v-for="category in categories"
          :key="category.id"
          class="px-3 py-2 rounded-md cursor-pointer text-sm transition-colors"
          :class="{
            'bg-primary text-primary-foreground': activeCategory === category.id,
            'hover:bg-muted': activeCategory !== category.id,
          }"
          @click="setActiveCategory(category.id)"
        >
          {{ category.name }}
        </div>
        <div class="mt-auto pt-4 border-t">
          <div
            class="px-3 py-2 rounded-md cursor-pointer text-sm text-muted-foreground hover:bg-muted transition-colors flex items-center gap-2"
            @click="router.push('/categories')"
          >
            <IconSquarePen class="h-4 w-4" />
            管理分类
          </div>
        </div>
      </aside>

      <!-- 右侧内容区 -->
      <main class="flex-1 flex flex-col overflow-hidden">
        <!-- 搜索和操作栏 -->
        <div class="p-4 border-b flex items-center gap-4">
          <div class="relative flex-1">
            <IconSearch class="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
            <Input
              v-model="searchQuery"
              placeholder="搜索密码..."
              class="pl-9"
            />
          </div>
          <Button @click="openAddDialog">
            <IconPlus class="h-4 w-4 mr-2" />
            添加密码
          </Button>
        </div>

        <!-- 密码列表表格 -->
        <div class="flex-1 overflow-auto p-4">
          <!-- table-fixed 固定列宽分配：内容超长只截断（title 显全文），不再横向撑破表格 -->
          <Table class="table-fixed">
            <colgroup>
              <!-- 名称吃剩余宽度；其余列定宽，总宽适配 900px 窗口下的 ~676px 内容区 -->
              <col />
              <col class="w-[22%]" />
              <col class="w-[104px]" />
              <col class="w-[84px]" />
              <col class="w-[88px]" />
              <!-- 操作列 = 两个 icon 按钮 72px + 间距 4px + 单元格 p-2 内边距 16px，再留 4px 余量 -->
              <col class="w-[96px]" />
            </colgroup>
            <TableHeader>
              <TableRow>
                <TableHead>名称</TableHead>
                <TableHead>用户名</TableHead>
                <TableHead>密码</TableHead>
                <TableHead>分类</TableHead>
                <TableHead>更新时间</TableHead>
                <!-- sticky 列需要不透明背景遮住滚动内容 -->
                <TableHead class="text-center sticky right-0 bg-background">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="item in filteredPasswords"
                :key="item.id"
                class="group"
              >
                <TableCell class="font-medium overflow-hidden">
                  <div class="flex items-center gap-2 min-w-0">
                    <button
                      class="shrink-0 text-muted-foreground hover:text-yellow-500 transition-colors"
                      @click="toggleFavorite(item.id)"
                    >
                      <IconStar class="h-4 w-4" :fill="item.is_favorite ? 'currentColor' : 'none'" />
                    </button>
                    <span class="truncate" :title="item.title">{{ item.title }}</span>
                  </div>
                </TableCell>
                <TableCell class="overflow-hidden">
                  <div class="flex items-center gap-2 min-w-0">
                    <span class="truncate" :title="item.username">{{ item.username }}</span>
                    <button
                      class="shrink-0 text-muted-foreground hover:text-foreground transition-colors"
                      @click="(e) => copyToClipboard(item.username, e)"
                    >
                      <IconCopy class="h-4 w-4" />
                    </button>
                  </div>
                </TableCell>
                <TableCell class="overflow-hidden">
                  <div class="flex items-center gap-2">
                    <span class="shrink-0 text-muted-foreground">••••</span>
                    <PasswordStrength
                      v-if="item.password_strength"
                      :level="item.password_strength"
                      compact
                      class="w-10 shrink-0"
                    />
                    <button
                      class="shrink-0 text-muted-foreground hover:text-foreground transition-colors"
                      @click="(e) => copyPasswordViaBackend(item.id, e)"
                    >
                      <IconCopy class="h-4 w-4" />
                    </button>
                  </div>
                </TableCell>
                <TableCell class="overflow-hidden">
                  <Badge
                    variant="secondary"
                    class="block max-w-full truncate"
                    :title="categories.find(c => c.id === item.category)?.name || item.category"
                  >
                    {{ categories.find(c => c.id === item.category)?.name || item.category }}
                  </Badge>
                </TableCell>
                <TableCell class="text-muted-foreground text-sm overflow-hidden">
                  <span class="truncate block" :title="formatTimestamp(item.updated_at)">
                    {{ formatTimestamp(item.updated_at) }}
                  </span>
                </TableCell>
                <!-- 操作列钉在右侧：任何情况下编辑/删除都可见；
                     group-hover 同步行悬停高亮（sticky 的不透明底色会盖住行 hover） -->
                <TableCell class="sticky right-0 bg-background group-hover:bg-muted/50">
                  <div class="flex items-center justify-center gap-1">
                    <Button variant="ghost" size="icon" @click="openEditDialog(item)">
                      <IconPencil class="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      class="text-destructive hover:text-destructive"
                      @click="deletePassword(item.id)"
                    >
                      <IconTrash class="h-4 w-4" />
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>

          <!-- 空状态 -->
          <div
            v-if="filteredPasswords.length === 0"
            class="flex flex-col items-center justify-center py-12 text-muted-foreground"
          >
            <IconEmpty class="h-12 w-12 mb-4" stroke-width="1.5" />
            <p class="text-lg font-medium">暂无密码</p>
            <p class="text-sm">点击"添加密码"按钮开始使用</p>
          </div>
        </div>
      </main>
    </div>

    <!-- 密码表单弹窗 -->
    <PasswordFormDialog
      v-model:open="showFormDialog"
      :edit-item="editingItem"
      @saved="() => {}"
    />

    <!-- 密码生成器弹窗 -->
    <PasswordGenerator
      v-model:open="showGenerator"
      @generated="() => {}"
    />

    <!-- 快速搜索弹窗 -->
    <QuickSearch v-model:open="showQuickSearch" />

    <!-- 快速添加弹窗 -->
    <QuickAdd v-model:open="showQuickAdd" />

    <!-- Toast 提示 -->
    <Toast
      v-model:show="toast.show"
      :type="toast.type"
      :message="toast.message"
    />
  </div>
</template>
