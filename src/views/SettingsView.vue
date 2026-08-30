<!--
  SettingsView.vue - 设置页面

  应用的设置界面，包含多个设置分类。
  使用 Tabs 组件实现选项卡切换。

  设置分类：
  1. 通用设置：主题、语言、开机自启、最小化到托盘
  2. 安全设置：自动锁定时间、剪贴板清除时间、密码强度显示
  3. 修改主密码：验证旧密码、设置新密码
  4. 数据管理：导入导出数据、关于信息

  路由路径：/settings
-->

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useTheme } from '@/composables/useTheme'
import { useShortcutStore } from '@/stores/shortcuts'
import Toast from '@/components/Toast.vue'

/** 路由实例 */
const router = useRouter()

/** 主题管理 */
const { theme, setTheme } = useTheme()

/** 快捷键配置 */
const shortcutStore = useShortcutStore()
const { shortcuts, formatShortcut } = shortcutStore

/** 快捷键编辑状态 */
const editingShortcut = ref<string | null>(null)
const shortcutInput = ref('')

/**
 * 页面加载时从后端读取设置
 */
onMounted(async () => {
  try {
    const savedSettings = await invoke<{
      theme: string
      language: string
      auto_lock_time: number
      clipboard_clear_time: number
      auto_start: boolean
      minimize_to_tray: boolean
      show_password_strength: boolean
    }>('get_settings')

    // 应用设置
    settings.value.language = savedSettings.language
    settings.value.autoStart = savedSettings.auto_start
    settings.value.minimizeToTray = savedSettings.minimize_to_tray

    security.value.autoLockTime = savedSettings.auto_lock_time
    security.value.clipboardClearTime = savedSettings.clipboard_clear_time
    security.value.showPasswordStrength = savedSettings.show_password_strength

    // 应用主题
    if (savedSettings.theme && savedSettings.theme !== theme.value) {
      setTheme(savedSettings.theme as 'light' | 'dark' | 'system')
    }

    // 获取开机自启状态
    const autoStartEnabled = await invoke<boolean>('is_auto_start_enabled')
    settings.value.autoStart = autoStartEnabled

    // 加载快捷键配置
    await shortcutStore.loadShortcuts()
  } catch (e) {
    console.error('加载设置失败:', e)
  }
})

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

/**
 * 通用设置
 */
const settings = ref({
  /** 界面语言 */
  language: 'zh-CN',
  /** 是否开机自启 */
  autoStart: false,
  /** 是否最小化到系统托盘 */
  minimizeToTray: true,
  /** 是否关闭时最小化到托盘（而不是退出） */
  closeToTray: true,
})

/**
 * 安全设置
 */
const security = ref({
  /** 自动锁定时间（分钟），无操作后自动锁定应用 */
  autoLockTime: 5,
  /** 剪贴板清除时间（秒），复制密码后自动清除剪贴板 */
  clipboardClearTime: 30,
  /** 是否在密码列表中显示密码强度指示器 */
  showPasswordStrength: true,
  /** 启动时是否需要输入主密码 */
  requireMasterPassword: true,
})

/**
 * 主密码修改表单
 */
const passwordForm = ref({
  /** 当前主密码 */
  currentPassword: '',
  /** 新主密码 */
  newPassword: '',
  /** 确认新密码 */
  confirmPassword: '',
})

/** 修改密码加载状态 */
const changingPassword = ref(false)

/**
 * 开始编辑快捷键
 * @param key - 快捷键名称
 */
const startEditShortcut = (key: string) => {
  editingShortcut.value = key
  shortcutInput.value = ''
}

/**
 * 保存快捷键
 * @param key - 快捷键名称
 */
const saveShortcut = async (key: 'quickSearch' | 'quickAdd' | 'passwordGenerator') => {
  if (!shortcutInput.value) {
    showToast('error', '请按下快捷键')
    return
  }

  try {
    await shortcutStore.updateShortcut(key, shortcutInput.value)
    editingShortcut.value = null
    showToast('success', '快捷键已保存，重启应用后生效')
  } catch (e) {
    showToast('error', '保存失败: ' + String(e))
  }
}

/**
 * 取消编辑快捷键
 */
const cancelEditShortcut = () => {
  editingShortcut.value = null
  shortcutInput.value = ''
}

/**
 * 处理快捷键输入
 * @param e - 键盘事件
 */
const handleShortcutKeydown = (e: KeyboardEvent) => {
  e.preventDefault()

  const parts: string[] = []

  if (e.ctrlKey) parts.push('CmdOrCtrl')
  if (e.shiftKey) parts.push('Shift')
  if (e.altKey) parts.push('Alt')
  if (e.metaKey) parts.push('Super')

  // 忽略单独的修饰键
  if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) {
    return
  }

  // 添加按键
  parts.push(e.key.toUpperCase())

  shortcutInput.value = parts.join('+')
}

/**
 * 重置快捷键为默认
 */
const resetShortcuts = async () => {
  try {
    await shortcutStore.resetToDefault()
    showToast('success', '已重置为默认快捷键，重启应用后生效')
  } catch (e) {
    showToast('error', '重置失败: ' + String(e))
  }
}

/**
 * 保存通用设置
 */
const handleSaveGeneral = async () => {
  try {
    // 保存设置到数据库
    await invoke('save_settings', {
      settings: {
        theme: theme.value,
        language: settings.value.language,
        auto_lock_time: security.value.autoLockTime,
        clipboard_clear_time: security.value.clipboardClearTime,
        auto_start: settings.value.autoStart,
        minimize_to_tray: settings.value.minimizeToTray,
        show_password_strength: security.value.showPasswordStrength,
      },
    })

    // 设置开机自启
    await invoke('set_auto_start', { enable: settings.value.autoStart })

    showToast('success', '通用设置已保存')
  } catch (e) {
    showToast('error', '保存失败: ' + String(e))
  }
}

/**
 * 保存安全设置
 */
const handleSaveSecurity = async () => {
  try {
    await invoke('save_setting', { key: 'autoLockTime', value: String(security.value.autoLockTime) })
    await invoke('save_setting', { key: 'clipboardClearTime', value: String(security.value.clipboardClearTime) })
    showToast('success', '安全设置已保存')
  } catch (e) {
    showToast('error', '保存失败: ' + String(e))
  }
}

/**
 * 修改主密码
 * 1. 验证必填字段
 * 2. 验证两次新密码是否一致
 * 3. 调用后端修改密码
 * 4. 成功后清空表单
 */
const handleChangePassword = async () => {
  // 验证必填字段
  if (!passwordForm.value.currentPassword || !passwordForm.value.newPassword) {
    showToast('error', '请填写完整')
    return
  }

  // 验证新密码长度
  if (passwordForm.value.newPassword.length < 6) {
    showToast('error', '新密码长度至少6位')
    return
  }

  // 验证两次密码是否一致
  if (passwordForm.value.newPassword !== passwordForm.value.confirmPassword) {
    showToast('error', '两次密码不一致')
    return
  }

  // 设置加载状态
  changingPassword.value = true

  try {
    // 调用 Tauri 后端修改密码
    await invoke('change_master_password', {
      oldPassword: passwordForm.value.currentPassword,
      newPassword: passwordForm.value.newPassword,
    })

    // 修改成功
    showToast('success', '主密码修改成功')

    // 清空表单
    passwordForm.value = { currentPassword: '', newPassword: '', confirmPassword: '' }
  } catch (e) {
    // 修改失败
    showToast('error', String(e))
  } finally {
    changingPassword.value = false
  }
}

/**
 * 导出数据
 */
const handleExport = async () => {
  try {
    const data = await invoke<string>('export_data')

    // 创建下载链接
    const blob = new Blob([data], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `moyu-passwd-backup-${new Date().toISOString().slice(0, 10)}.json`
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)

    showToast('success', '数据导出成功')
  } catch (e) {
    showToast('error', '导出失败: ' + String(e))
  }
}

/**
 * 导入数据
 */
const handleImport = async () => {
  try {
    // 创建文件选择器
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = '.json'

    input.onchange = async (e) => {
      const file = (e.target as HTMLInputElement).files?.[0]
      if (!file) return

      const reader = new FileReader()
      reader.onload = async (event) => {
        try {
          const json = event.target?.result as string
          const count = await invoke<number>('import_data', { json })
          showToast('success', `成功导入 ${count} 条密码`)
        } catch (err) {
          showToast('error', '导入失败: ' + String(err))
        }
      }
      reader.readAsText(file)
    }

    input.click()
  } catch (e) {
    showToast('error', '导入失败: ' + String(e))
  }
}
</script>

<template>
  <div class="min-h-screen bg-background">
    <!-- 顶部导航 -->
    <header class="border-b px-4 py-3 flex items-center gap-4">
      <Button variant="ghost" size="icon" @click="router.push('/home')">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          class="h-5 w-5"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <path d="m12 19-7-7 7-7M19 12H5" />
        </svg>
      </Button>
      <h1 class="text-lg font-semibold">设置</h1>
    </header>

    <!-- 内容区 -->
    <div class="max-w-3xl mx-auto p-6">
      <Tabs default-value="general">
        <TabsList class="grid w-full grid-cols-5">
          <TabsTrigger value="general">通用</TabsTrigger>
          <TabsTrigger value="security">安全</TabsTrigger>
          <TabsTrigger value="password">密码</TabsTrigger>
          <TabsTrigger value="shortcuts">快捷键</TabsTrigger>
          <TabsTrigger value="data">数据</TabsTrigger>
        </TabsList>

        <!-- 通用设置 -->
        <TabsContent value="general">
          <Card>
            <CardHeader>
              <CardTitle>通用设置</CardTitle>
              <CardDescription>应用的基本配置</CardDescription>
            </CardHeader>
            <CardContent class="space-y-6">
              <!-- 主题 -->
              <div class="space-y-2">
                <Label>主题</Label>
                <div class="flex gap-2">
                  <Button
                    :variant="theme === 'light' ? 'default' : 'outline'"
                    @click="setTheme('light')"
                  >
                    浅色
                  </Button>
                  <Button
                    :variant="theme === 'dark' ? 'default' : 'outline'"
                    @click="setTheme('dark')"
                  >
                    深色
                  </Button>
                  <Button
                    :variant="theme === 'system' ? 'default' : 'outline'"
                    @click="setTheme('system')"
                  >
                    跟随系统
                  </Button>
                </div>
              </div>

              <!-- 语言 -->
              <div class="space-y-2">
                <Label>语言</Label>
                <select
                  v-model="settings.language"
                  class="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                >
                  <option value="zh-CN">简体中文</option>
                  <option value="en-US">English</option>
                </select>
              </div>

              <!-- 开机自启 -->
              <div class="flex items-center justify-between">
                <div>
                  <Label>开机自启</Label>
                  <p class="text-sm text-muted-foreground">系统启动时自动运行</p>
                </div>
                <input
                  v-model="settings.autoStart"
                  type="checkbox"
                  class="h-4 w-4"
                />
              </div>

              <!-- 最小化到托盘 -->
              <div class="flex items-center justify-between">
                <div>
                  <Label>最小化到托盘</Label>
                  <p class="text-sm text-muted-foreground">关闭窗口时最小化到系统托盘</p>
                </div>
                <input
                  v-model="settings.minimizeToTray"
                  type="checkbox"
                  class="h-4 w-4"
                />
              </div>

              <Button @click="handleSaveGeneral">保存设置</Button>
            </CardContent>
          </Card>
        </TabsContent>

        <!-- 安全设置 -->
        <TabsContent value="security">
          <Card>
            <CardHeader>
              <CardTitle>安全设置</CardTitle>
              <CardDescription>配置安全相关的选项</CardDescription>
            </CardHeader>
            <CardContent class="space-y-6">
              <!-- 自动锁定时间 -->
              <div class="space-y-2">
                <Label>自动锁定时间（分钟）</Label>
                <Input
                  v-model.number="security.autoLockTime"
                  type="number"
                  min="1"
                  max="60"
                />
                <p class="text-sm text-muted-foreground">无操作后自动锁定应用</p>
              </div>

              <!-- 剪贴板清除时间 -->
              <div class="space-y-2">
                <Label>剪贴板清除时间（秒）</Label>
                <Input
                  v-model.number="security.clipboardClearTime"
                  type="number"
                  min="10"
                  max="300"
                />
                <p class="text-sm text-muted-foreground">复制密码后自动清除剪贴板</p>
              </div>

              <!-- 显示密码强度 -->
              <div class="flex items-center justify-between">
                <div>
                  <Label>显示密码强度</Label>
                  <p class="text-sm text-muted-foreground">在密码列表中显示强度指示器</p>
                </div>
                <input
                  v-model="security.showPasswordStrength"
                  type="checkbox"
                  class="h-4 w-4"
                />
              </div>

              <!-- 需要主密码 -->
              <div class="flex items-center justify-between">
                <div>
                  <Label>启动时需要主密码</Label>
                  <p class="text-sm text-muted-foreground">每次启动应用都需要输入主密码</p>
                </div>
                <input
                  v-model="security.requireMasterPassword"
                  type="checkbox"
                  class="h-4 w-4"
                />
              </div>

              <Button @click="handleSaveSecurity">保存设置</Button>
            </CardContent>
          </Card>
        </TabsContent>

        <!-- 修改主密码 -->
        <TabsContent value="password">
          <Card>
            <CardHeader>
              <CardTitle>修改主密码</CardTitle>
              <CardDescription>修改您的主密码</CardDescription>
            </CardHeader>
            <CardContent class="space-y-4">
              <div class="space-y-2">
                <Label for="current-password">当前密码</Label>
                <Input
                  id="current-password"
                  v-model="passwordForm.currentPassword"
                  type="password"
                  placeholder="输入当前主密码"
                />
              </div>

              <div class="space-y-2">
                <Label for="new-password">新密码</Label>
                <Input
                  id="new-password"
                  v-model="passwordForm.newPassword"
                  type="password"
                  placeholder="输入新密码（至少6位）"
                />
              </div>

              <div class="space-y-2">
                <Label for="confirm-password">确认新密码</Label>
                <Input
                  id="confirm-password"
                  v-model="passwordForm.confirmPassword"
                  type="password"
                  placeholder="再次输入新密码"
                />
              </div>

              <Button @click="handleChangePassword" :disabled="changingPassword">
                <svg
                  v-if="changingPassword"
                  class="mr-2 h-4 w-4 animate-spin"
                  xmlns="http://www.w3.org/2000/svg"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                >
                  <path d="M21 12a9 9 0 1 1-6.219-8.56" />
                </svg>
                {{ changingPassword ? '修改中...' : '修改密码' }}
              </Button>
            </CardContent>
          </Card>
        </TabsContent>

        <!-- 快捷键设置 -->
        <TabsContent value="shortcuts">
          <Card>
            <CardHeader>
              <CardTitle>快捷键设置</CardTitle>
              <CardDescription>自定义全局快捷键，点击快捷键可修改</CardDescription>
            </CardHeader>
            <CardContent class="space-y-4">
              <div class="space-y-4">
                <!-- 快速搜索 -->
                <div class="flex items-center justify-between p-4 border rounded-lg">
                  <div>
                    <h3 class="font-medium">快速搜索</h3>
                    <p class="text-sm text-muted-foreground">呼出快速搜索弹窗</p>
                  </div>
                  <div v-if="editingShortcut === 'quickSearch'" class="flex items-center gap-2">
                    <Input
                      v-model="shortcutInput"
                      class="w-48"
                      placeholder="按下快捷键..."
                      autofocus
                      @keydown="handleShortcutKeydown"
                    />
                    <Button size="sm" @click="saveShortcut('quickSearch')">保存</Button>
                    <Button size="sm" variant="outline" @click="cancelEditShortcut">取消</Button>
                  </div>
                  <div
                    v-else
                    class="flex items-center gap-1 px-3 py-1.5 bg-muted rounded cursor-pointer hover:bg-muted/80"
                    @click="startEditShortcut('quickSearch')"
                  >
                    <template v-for="(part, i) in formatShortcut(shortcuts.quickSearch).split(' + ')" :key="i">
                      <kbd class="px-1.5 py-0.5 text-xs font-mono bg-background rounded border">{{ part }}</kbd>
                      <span v-if="i < formatShortcut(shortcuts.quickSearch).split(' + ').length - 1" class="text-muted-foreground">+</span>
                    </template>
                  </div>
                </div>

                <!-- 快速添加 -->
                <div class="flex items-center justify-between p-4 border rounded-lg">
                  <div>
                    <h3 class="font-medium">快速添加</h3>
                    <p class="text-sm text-muted-foreground">呼出快速添加密码弹窗</p>
                  </div>
                  <div v-if="editingShortcut === 'quickAdd'" class="flex items-center gap-2">
                    <Input
                      v-model="shortcutInput"
                      class="w-48"
                      placeholder="按下快捷键..."
                      autofocus
                      @keydown="handleShortcutKeydown"
                    />
                    <Button size="sm" @click="saveShortcut('quickAdd')">保存</Button>
                    <Button size="sm" variant="outline" @click="cancelEditShortcut">取消</Button>
                  </div>
                  <div
                    v-else
                    class="flex items-center gap-1 px-3 py-1.5 bg-muted rounded cursor-pointer hover:bg-muted/80"
                    @click="startEditShortcut('quickAdd')"
                  >
                    <template v-for="(part, i) in formatShortcut(shortcuts.quickAdd).split(' + ')" :key="i">
                      <kbd class="px-1.5 py-0.5 text-xs font-mono bg-background rounded border">{{ part }}</kbd>
                      <span v-if="i < formatShortcut(shortcuts.quickAdd).split(' + ').length - 1" class="text-muted-foreground">+</span>
                    </template>
                  </div>
                </div>

                <!-- 密码生成器 -->
                <div class="flex items-center justify-between p-4 border rounded-lg">
                  <div>
                    <h3 class="font-medium">密码生成器</h3>
                    <p class="text-sm text-muted-foreground">呼出密码生成器弹窗</p>
                  </div>
                  <div v-if="editingShortcut === 'passwordGenerator'" class="flex items-center gap-2">
                    <Input
                      v-model="shortcutInput"
                      class="w-48"
                      placeholder="按下快捷键..."
                      autofocus
                      @keydown="handleShortcutKeydown"
                    />
                    <Button size="sm" @click="saveShortcut('passwordGenerator')">保存</Button>
                    <Button size="sm" variant="outline" @click="cancelEditShortcut">取消</Button>
                  </div>
                  <div
                    v-else
                    class="flex items-center gap-1 px-3 py-1.5 bg-muted rounded cursor-pointer hover:bg-muted/80"
                    @click="startEditShortcut('passwordGenerator')"
                  >
                    <template v-for="(part, i) in formatShortcut(shortcuts.passwordGenerator).split(' + ')" :key="i">
                      <kbd class="px-1.5 py-0.5 text-xs font-mono bg-background rounded border">{{ part }}</kbd>
                      <span v-if="i < formatShortcut(shortcuts.passwordGenerator).split(' + ').length - 1" class="text-muted-foreground">+</span>
                    </template>
                  </div>
                </div>

                <div class="flex items-center justify-between">
                  <p class="text-sm text-muted-foreground">
                    点击快捷键可修改，修改后需重启应用生效。
                  </p>
                  <Button variant="outline" size="sm" @click="resetShortcuts">
                    重置默认
                  </Button>
                </div>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <!-- 数据管理 -->
        <TabsContent value="data">
          <Card>
            <CardHeader>
              <CardTitle>数据管理</CardTitle>
              <CardDescription>导入导出您的密码数据</CardDescription>
            </CardHeader>
            <CardContent class="space-y-6">
              <!-- 导出 -->
              <div class="flex items-center justify-between p-4 border rounded-lg">
                <div>
                  <h3 class="font-medium">导出数据</h3>
                  <p class="text-sm text-muted-foreground">将密码导出为加密文件</p>
                </div>
                <Button variant="outline" @click="handleExport">导出</Button>
              </div>

              <!-- 导入 -->
              <div class="flex items-center justify-between p-4 border rounded-lg">
                <div>
                  <h3 class="font-medium">导入数据</h3>
                  <p class="text-sm text-muted-foreground">从文件导入密码</p>
                </div>
                <Button variant="outline" @click="handleImport">导入</Button>
              </div>

              <!-- 关于 -->
              <div class="p-4 border rounded-lg">
                <h3 class="font-medium mb-2">关于摸鱼密码</h3>
                <p class="text-sm text-muted-foreground">版本: 0.1.0</p>
                <p class="text-sm text-muted-foreground">一个简单安全的密码管理工具</p>
              </div>
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>

    <!-- Toast 提示组件 -->
    <Toast
      v-model:show="toast.show"
      :type="toast.type"
      :message="toast.message"
    />
  </div>
</template>
