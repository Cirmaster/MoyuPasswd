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
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
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

/** 快捷键配置（计算属性，保持响应性） */
const shortcuts = computed(() => shortcutStore.shortcuts)

/** 格式化快捷键显示 */
const formatShortcut = shortcutStore.formatShortcut

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
      close_to_tray: boolean
      show_password_strength: boolean
      show_on_startup: boolean
    }>('get_settings')

    // 应用通用设置
    settings.value.language = savedSettings.language
    settings.value.autoStart = savedSettings.auto_start
    settings.value.closeToTray = savedSettings.close_to_tray ?? true
    settings.value.showOnStartup = savedSettings.show_on_startup ?? true

    // 应用安全设置
    security.value.autoLockTime = savedSettings.auto_lock_time
    security.value.clipboardClearTime = savedSettings.clipboard_clear_time
    security.value.showPasswordStrength = savedSettings.show_password_strength

    // 应用主题
    if (savedSettings.theme && savedSettings.theme !== theme.value) {
      setTheme(savedSettings.theme as 'light' | 'dark' | 'system')
    }

    // 获取开机自启状态（允许失败）
    try {
      const autoStartEnabled = await invoke<boolean>('is_auto_start_enabled')
      settings.value.autoStart = autoStartEnabled
    } catch (autoStartError) {
      console.warn('获取开机自启状态失败:', autoStartError)
      settings.value.autoStart = false
    }

    // 获取系统认证状态
    try {
      const [available, enabled] = await Promise.all([
        invoke<boolean>('is_system_auth_available'),
        invoke<boolean>('is_system_auth_enabled'),
      ])
      systemAuth.value.available = available
      systemAuth.value.enabled = enabled
    } catch (e) {
      console.warn('获取系统认证状态失败:', e)
    }

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
  /** 是否关闭时最小化到托盘（而不是退出） */
  closeToTray: true,
  /** 启动时是否显示主窗口 */
  showOnStartup: true,
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
})

/**
 * 系统认证设置
 */
const systemAuth = ref({
  /** 系统认证是否可用 */
  available: false,
  /** 系统认证是否已启用 */
  enabled: false,
  /** 操作加载状态 */
  loading: false,
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
const startEditShortcut = async (key: string) => {
  editingShortcut.value = key
  // 初始化显示当前快捷键
  const currentShortcut = shortcuts.value[key as keyof typeof shortcuts.value]
  shortcutInput.value = currentShortcut || ''
  
  // 注销全局快捷键，防止编辑时触发
  try {
    await invoke('unregister_all_shortcuts')
  } catch (e) {
    console.warn('注销快捷键失败:', e)
  }
  
  // 强制聚焦输入框
  nextTick(() => {
    forceFocusInput()
  })
}

/**
 * 强制聚焦输入框（编辑模式下锁定焦点）
 */
const forceFocusInput = () => {
  const input = document.querySelector('[data-shortcut-input]') as HTMLInputElement
  if (input) {
    input.focus()
  }
}

/**
 * 输入框失去焦点时，如果还在编辑模式，强制重新聚焦
 */
const handleInputBlur = () => {
  if (editingShortcut.value) {
    // 延迟一下，避免和其他点击事件冲突
    setTimeout(() => {
      if (editingShortcut.value) {
        forceFocusInput()
      }
    }, 10)
  }
}

/**
 * 标准化快捷键格式
 * 将 Ctrl 转换为 CmdOrCtrl，确保格式一致
 */
const normalizeShortcut = (shortcut: string): string => {
  return shortcut
    .replace(/^Ctrl\+/, 'CmdOrCtrl+')
    .replace(/\+Ctrl\+/, '+CmdOrCtrl+')
    .trim()
}

/**
 * 保存快捷键
 * @param key - 快捷键名称
 */
const saveShortcut = async (key: 'quickSearch' | 'quickAdd' | 'passwordGenerator') => {
  if (!shortcutInput.value || shortcutInput.value.endsWith('...')) {
    showToast('error', '请按下完整的快捷键组合')
    return
  }

  // 标准化快捷键格式
  const normalizedShortcut = normalizeShortcut(shortcutInput.value)

  // 验证快捷键格式（至少包含一个修饰键+一个普通键）
  const parts = normalizedShortcut.split('+')
  const modifiers = ['CmdOrCtrl', 'Shift', 'Alt', 'Super', 'Ctrl', 'Meta']
  const hasModifier = parts.some(p => modifiers.includes(p))
  const hasKey = parts.some(p => !modifiers.includes(p) && p.length > 0)
  
  if (!hasModifier || !hasKey) {
    showToast('error', '快捷键必须包含修饰键（Ctrl/Shift/Alt）和一个普通键')
    return
  }

  try {
    console.log(`保存快捷键: ${key} = ${normalizedShortcut}`)
    // 直接调用 store 的 updateShortcut 方法（会自动重新注册全局快捷键）
    await shortcutStore.updateShortcut(key, normalizedShortcut)
    // 退出编辑模式
    editingShortcut.value = null
    shortcutInput.value = ''
    showToast('success', '快捷键已保存并立即生效')
  } catch (e) {
    console.error('保存快捷键失败:', e)
    showToast('error', '保存失败: ' + String(e))
    // 保存失败也要重新注册全局快捷键
    try {
      await invoke('update_global_shortcuts')
    } catch (e2) {
      console.warn('重新注册快捷键失败:', e2)
    }
  }
}

/**
 * 退出快捷键编辑模式并恢复全局快捷键
 *
 * 所有退出编辑态的路径（保存、取消、离开页面、组件卸载）统一走这里，
 * 避免「进入编辑后直接离开页面导致全局快捷键未被恢复」的问题。
 */
const exitEditMode = async () => {
  if (!editingShortcut.value) return

  editingShortcut.value = null
  shortcutInput.value = ''

  // 重新注册全局快捷键（update_global_shortcuts 内部先注销再注册，幂等）
  try {
    await invoke('update_global_shortcuts')
  } catch (e) {
    console.warn('重新注册快捷键失败:', e)
  }
}

/**
 * 取消编辑快捷键
 */
const cancelEditShortcut = () => {
  exitEditMode()
}

/**
 * 全局键盘事件拦截器（在编辑模式下阻止系统快捷键）
 */
const globalKeydownInterceptor = (e: KeyboardEvent) => {
  if (editingShortcut.value) {
    // 检查事件目标是否是快捷键输入框
    const target = e.target as HTMLElement
    const isShortcutInput = target.hasAttribute('data-shortcut-input')
    
    // 如果是快捷键输入框，不拦截事件，让它正常处理
    if (isShortcutInput) {
      return
    }
    
    // 否则拦截事件，防止触发系统/应用快捷键
    e.preventDefault()
    e.stopPropagation()
    if (e.stopImmediatePropagation) {
      e.stopImmediatePropagation()
    }
  }
}

/**
 * 监听编辑状态变化，启用/禁用全局拦截
 */
watch(editingShortcut, (newValue) => {
  if (newValue) {
    // 编辑模式：启用全局拦截
    document.addEventListener('keydown', globalKeydownInterceptor, { capture: true })
  } else {
    // 非编辑模式：禁用全局拦截
    document.removeEventListener('keydown', globalKeydownInterceptor, { capture: true })
  }
})

/**
 * 组件卸载时确保移除拦截器
 */
onUnmounted(() => {
  document.removeEventListener('keydown', globalKeydownInterceptor, { capture: true })
  // 离开页面时若仍处于编辑态，恢复全局快捷键（兜底）
  exitEditMode()
})

/**
 * 预设快捷键列表（避免与系统快捷键冲突）
 */
const presetShortcuts = [
  { label: 'Ctrl + K', value: 'CmdOrCtrl+K' },
  { label: 'Ctrl + G', value: 'CmdOrCtrl+G' },
  { label: 'Ctrl + M', value: 'CmdOrCtrl+M' },
  { label: 'Ctrl + Shift + K', value: 'CmdOrCtrl+Shift+K' },
  { label: 'Ctrl + Shift + A', value: 'CmdOrCtrl+Shift+A' },
  { label: 'Ctrl + Shift + D', value: 'CmdOrCtrl+Shift+D' },
  { label: 'Ctrl + Shift + M', value: 'CmdOrCtrl+Shift+M' },
  { label: 'Ctrl + Alt + K', value: 'CmdOrCtrl+Alt+K' },
  { label: 'Ctrl + Alt + G', value: 'CmdOrCtrl+Alt+G' },
  { label: 'Ctrl + Alt + A', value: 'CmdOrCtrl+Alt+A' },
]

/**
 * 选择预设快捷键
 */
const selectPresetShortcut = (value: string) => {
  shortcutInput.value = value
}

/**
 * 不可使用的系统快捷键列表
 */
const reservedShortcuts = [
  'Ctrl+C', 'Ctrl+V', 'Ctrl+X', 'Ctrl+Z', 'Ctrl+A',  // 剪贴板和撤销
  'Ctrl+S', 'Ctrl+P', 'Ctrl+F', 'Ctrl+N', 'Ctrl+O',  // 常用系统快捷键
  'Alt+Tab', 'Alt+F4', 'Ctrl+Alt+Del',                  // 系统切换
  'CmdOrCtrl+C', 'CmdOrCtrl+V', 'CmdOrCtrl+X', 'CmdOrCtrl+Z', 'CmdOrCtrl+A',
  'CmdOrCtrl+S', 'CmdOrCtrl+P', 'CmdOrCtrl+F', 'CmdOrCtrl+N', 'CmdOrCtrl+O',
]

/**
 * 检查是否为保留快捷键
 */
const isReservedShortcut = (shortcut: string): boolean => {
  const normalized = normalizeShortcut(shortcut)
  return reservedShortcuts.some(reserved => 
    normalizeShortcut(reserved) === normalized
  )
}

/**
 * 处理快捷键输入（在 capture 阶段拦截）
 * @param e - 键盘事件
 */
const handleShortcutKeydown = (e: KeyboardEvent) => {
  console.log('键盘事件:', e.key, e.ctrlKey, e.shiftKey, e.altKey, e.metaKey)
  
  // 阻止事件冒泡和默认行为，防止触发系统/应用快捷键
  e.preventDefault()
  e.stopPropagation()
  if (e.stopImmediatePropagation) {
    e.stopImmediatePropagation()
  }

  const parts: string[] = []

  // 收集修饰键（统一使用 CmdOrCtrl 表示 Ctrl/Cmd）
  if (e.ctrlKey || e.metaKey) parts.push('CmdOrCtrl')
  if (e.shiftKey) parts.push('Shift')
  if (e.altKey) parts.push('Alt')

  // 单独的修饰键时，显示当前已按下的修饰键
  if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) {
    if (parts.length > 0) {
      shortcutInput.value = parts.join('+') + '+...'
    }
    return
  }

  // 忽略只有修饰键没有普通键的情况
  if (parts.length === 0) {
    return
  }

  // 添加普通按键
  // 特殊按键映射
  const keyMap: Record<string, string> = {
    ' ': 'Space',
    'ArrowUp': 'Up',
    'ArrowDown': 'Down',
    'ArrowLeft': 'Left',
    'ArrowRight': 'Right',
    'Escape': 'Esc',
    'Delete': 'Del',
    'Backspace': 'Backspace',
    'Enter': 'Enter',
    'Tab': 'Tab',
    'Home': 'Home',
    'End': 'End',
    'PageUp': 'PageUp',
    'PageDown': 'PageDown',
    'Insert': 'Ins',
  }
  
  const keyName = keyMap[e.key] || (e.key.length === 1 ? e.key.toUpperCase() : e.key)
  parts.push(keyName)

  const newShortcut = parts.join('+')
  console.log('生成的快捷键:', newShortcut)
  shortcutInput.value = newShortcut
  
  // 检查是否为保留快捷键
  if (isReservedShortcut(newShortcut)) {
    showToast('error', '此快捷键是系统保留的，请选择其他组合')
  }
}

/**
 * 重置快捷键为默认
 */
const resetShortcuts = async () => {
  try {
    await shortcutStore.resetToDefault()
    showToast('success', '已重置为默认快捷键')
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
        close_to_tray: settings.value.closeToTray,
        show_password_strength: security.value.showPasswordStrength,
        show_on_startup: settings.value.showOnStartup,
      },
    })

    // 设置开机自启（允许失败，不影响其他设置保存）
    try {
      await invoke('set_auto_start', { enable: settings.value.autoStart })
    } catch (autoStartError) {
      console.warn('设置开机自启失败（可能不支持）:', autoStartError)
      // 不显示错误给用户，因为某些系统可能不支持
    }

    showToast('success', '通用设置已保存')
  } catch (e) {
    showToast('error', '保存失败: ' + String(e))
  }
}

/**
 * 切换系统认证状态
 */
const toggleSystemAuth = async () => {
  systemAuth.value.loading = true
  try {
    if (systemAuth.value.enabled) {
      await invoke('disable_system_auth')
      systemAuth.value.enabled = false
      showToast('success', '系统快速解锁已禁用')
    } else {
      await invoke('enable_system_auth')
      systemAuth.value.enabled = true
      showToast('success', '系统快速解锁已启用')
    }
  } catch (e) {
    showToast('error', String(e))
  } finally {
    systemAuth.value.loading = false
  }
}

/**
 * 保存安全设置
 */
const handleSaveSecurity = async () => {
  try {
    // 使用 save_settings 批量保存安全设置
    await invoke('save_settings', {
      settings: {
        theme: theme.value,
        language: settings.value.language,
        auto_lock_time: security.value.autoLockTime,
        clipboard_clear_time: security.value.clipboardClearTime,
        auto_start: settings.value.autoStart,
        close_to_tray: settings.value.closeToTray,
        show_password_strength: security.value.showPasswordStrength,
        show_on_startup: settings.value.showOnStartup,
      },
    })
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

/** 备份密码（用于加密导出/解密导入） */
const backupPassword = ref('')

/**
 * 导出数据
 */
const handleExport = async () => {
  if (!backupPassword.value) {
    showToast('error', '请先设置备份密码')
    return
  }
  try {
    const data = await invoke<string>('export_data', { exportPassword: backupPassword.value })

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
          const count = await invoke<number>('import_data', { json, importPassword: backupPassword.value })
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

              <!-- 关闭时最小化到托盘 -->
              <div class="flex items-center justify-between">
                <div>
                  <Label>关闭时最小化到托盘</Label>
                  <p class="text-sm text-muted-foreground">点击关闭按钮时最小化到托盘而不是退出</p>
                </div>
                <input
                  v-model="settings.closeToTray"
                  type="checkbox"
                  class="h-4 w-4"
                />
              </div>

              <!-- 启动时显示主窗口 -->
              <div class="flex items-center justify-between">
                <div>
                  <Label>启动时显示主窗口</Label>
                  <p class="text-sm text-muted-foreground">应用启动时直接显示主窗口，关闭后仅驻留系统托盘</p>
                </div>
                <input
                  v-model="settings.showOnStartup"
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

              <!-- 系统快速解锁 -->
              <div v-if="systemAuth.available" class="flex items-center justify-between">
                <div>
                  <Label>系统快速解锁</Label>
                  <p class="text-sm text-muted-foreground">
                    使用系统认证（PIN、指纹、面部识别等）解锁密码库，无需输入主密码
                  </p>
                </div>
                <Button
                  size="sm"
                  :variant="systemAuth.enabled ? 'default' : 'outline'"
                  :disabled="systemAuth.loading"
                  @click="toggleSystemAuth"
                >
                  {{ systemAuth.loading ? '处理中...' : (systemAuth.enabled ? '已启用' : '启用') }}
                </Button>
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
                <div class="p-4 border rounded-lg">
                  <div class="flex items-center justify-between mb-3">
                    <div>
                      <h3 class="font-medium">快速搜索</h3>
                      <p class="text-sm text-muted-foreground">呼出快速搜索弹窗</p>
                    </div>
                    <div
                      v-if="editingShortcut !== 'quickSearch'"
                      class="flex items-center gap-1 px-3 py-1.5 bg-muted rounded cursor-pointer hover:bg-muted/80"
                      @click="startEditShortcut('quickSearch')"
                    >
                      <template v-for="(part, i) in formatShortcut(shortcuts.quickSearch).split(' + ')" :key="i">
                        <kbd class="px-1.5 py-0.5 text-xs font-mono bg-background rounded border">{{ part }}</kbd>
                        <span v-if="i < formatShortcut(shortcuts.quickSearch).split(' + ').length - 1" class="text-muted-foreground">+</span>
                      </template>
                      <svg xmlns="http://www.w3.org/2000/svg" class="h-3 w-3 ml-1 text-muted-foreground" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
                      </svg>
                    </div>
                  </div>
                  
                  <!-- 编辑模式 -->
                  <div v-if="editingShortcut === 'quickSearch'" class="space-y-3">
                    <div class="flex items-center gap-2">
                      <div class="relative flex-1">
                        <Input
                          v-model="shortcutInput"
                          class="font-mono"
                          :placeholder="'请按下组合键...'"
                          readonly
                          autofocus
                          data-shortcut-input
                          @keydown="handleShortcutKeydown"
                          @blur="handleInputBlur"
                        />
                      </div>
                      <Button size="sm" @click="saveShortcut('quickSearch')">保存</Button>
                      <Button size="sm" variant="outline" @click="cancelEditShortcut">取消</Button>
                    </div>
                    
                    <!-- 预设快捷键 -->
                    <div>
                      <p class="text-xs text-muted-foreground mb-2">或选择预设快捷键：</p>
                      <div class="flex flex-wrap gap-2">
                        <Button
                          v-for="preset in presetShortcuts"
                          :key="preset.value"
                          size="sm"
                          variant="outline"
                          class="h-7 text-xs"
                          :class="{ 'border-primary': shortcutInput === preset.value }"
                          @click="selectPresetShortcut(preset.value)"
                        >
                          {{ preset.label }}
                        </Button>
                      </div>
                    </div>
                    
                    <p class="text-xs text-muted-foreground">
                      💡 提示：避免使用 Ctrl+C、Ctrl+V 等系统快捷键
                    </p>
                  </div>
                </div>

                <!-- 快速添加 -->
                <div class="p-4 border rounded-lg">
                  <div class="flex items-center justify-between mb-3">
                    <div>
                      <h3 class="font-medium">快速添加</h3>
                      <p class="text-sm text-muted-foreground">呼出快速添加密码弹窗</p>
                    </div>
                    <div
                      v-if="editingShortcut !== 'quickAdd'"
                      class="flex items-center gap-1 px-3 py-1.5 bg-muted rounded cursor-pointer hover:bg-muted/80"
                      @click="startEditShortcut('quickAdd')"
                    >
                      <template v-for="(part, i) in formatShortcut(shortcuts.quickAdd).split(' + ')" :key="i">
                        <kbd class="px-1.5 py-0.5 text-xs font-mono bg-background rounded border">{{ part }}</kbd>
                        <span v-if="i < formatShortcut(shortcuts.quickAdd).split(' + ').length - 1" class="text-muted-foreground">+</span>
                      </template>
                      <svg xmlns="http://www.w3.org/2000/svg" class="h-3 w-3 ml-1 text-muted-foreground" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
                      </svg>
                    </div>
                  </div>
                  
                  <!-- 编辑模式 -->
                  <div v-if="editingShortcut === 'quickAdd'" class="space-y-3">
                    <div class="flex items-center gap-2">
                      <div class="relative flex-1">
                        <Input
                          v-model="shortcutInput"
                          class="font-mono"
                          :placeholder="'请按下组合键...'"
                          readonly
                          autofocus
                          data-shortcut-input
                          @keydown="handleShortcutKeydown"
                          @blur="handleInputBlur"
                        />
                      </div>
                      <Button size="sm" @click="saveShortcut('quickAdd')">保存</Button>
                      <Button size="sm" variant="outline" @click="cancelEditShortcut">取消</Button>
                    </div>
                    
                    <!-- 预设快捷键 -->
                    <div>
                      <p class="text-xs text-muted-foreground mb-2">或选择预设快捷键：</p>
                      <div class="flex flex-wrap gap-2">
                        <Button
                          v-for="preset in presetShortcuts"
                          :key="preset.value"
                          size="sm"
                          variant="outline"
                          class="h-7 text-xs"
                          :class="{ 'border-primary': shortcutInput === preset.value }"
                          @click="selectPresetShortcut(preset.value)"
                        >
                          {{ preset.label }}
                        </Button>
                      </div>
                    </div>
                    
                    <p class="text-xs text-muted-foreground">
                      💡 提示：避免使用 Ctrl+C、Ctrl+V 等系统快捷键
                    </p>
                  </div>
                </div>

                <!-- 密码生成器 -->
                <div class="p-4 border rounded-lg">
                  <div class="flex items-center justify-between mb-3">
                    <div>
                      <h3 class="font-medium">密码生成器</h3>
                      <p class="text-sm text-muted-foreground">呼出密码生成器弹窗</p>
                    </div>
                    <div
                      v-if="editingShortcut !== 'passwordGenerator'"
                      class="flex items-center gap-1 px-3 py-1.5 bg-muted rounded cursor-pointer hover:bg-muted/80"
                      @click="startEditShortcut('passwordGenerator')"
                    >
                      <template v-for="(part, i) in formatShortcut(shortcuts.passwordGenerator).split(' + ')" :key="i">
                        <kbd class="px-1.5 py-0.5 text-xs font-mono bg-background rounded border">{{ part }}</kbd>
                        <span v-if="i < formatShortcut(shortcuts.passwordGenerator).split(' + ').length - 1" class="text-muted-foreground">+</span>
                      </template>
                      <svg xmlns="http://www.w3.org/2000/svg" class="h-3 w-3 ml-1 text-muted-foreground" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
                      </svg>
                    </div>
                  </div>
                  
                  <!-- 编辑模式 -->
                  <div v-if="editingShortcut === 'passwordGenerator'" class="space-y-3">
                    <div class="flex items-center gap-2">
                      <div class="relative flex-1">
                        <Input
                          v-model="shortcutInput"
                          class="font-mono"
                          :placeholder="'请按下组合键...'"
                          readonly
                          autofocus
                          data-shortcut-input
                          @keydown="handleShortcutKeydown"
                          @blur="handleInputBlur"
                        />
                      </div>
                      <Button size="sm" @click="saveShortcut('passwordGenerator')">保存</Button>
                      <Button size="sm" variant="outline" @click="cancelEditShortcut">取消</Button>
                    </div>
                    
                    <!-- 预设快捷键 -->
                    <div>
                      <p class="text-xs text-muted-foreground mb-2">或选择预设快捷键：</p>
                      <div class="flex flex-wrap gap-2">
                        <Button
                          v-for="preset in presetShortcuts"
                          :key="preset.value"
                          size="sm"
                          variant="outline"
                          class="h-7 text-xs"
                          :class="{ 'border-primary': shortcutInput === preset.value }"
                          @click="selectPresetShortcut(preset.value)"
                        >
                          {{ preset.label }}
                        </Button>
                      </div>
                    </div>
                    
                    <p class="text-xs text-muted-foreground">
                      💡 提示：避免使用 Ctrl+C、Ctrl+V 等系统快捷键
                    </p>
                  </div>
                </div>

                <div class="flex items-center justify-between pt-2">
                  <p class="text-sm text-muted-foreground">
                    修改后立即生效
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
              <!-- 备份密码 -->
              <div class="space-y-2">
                <Label for="backup-password">备份密码</Label>
                <Input
                  id="backup-password"
                  v-model="backupPassword"
                  type="password"
                  placeholder="导出/导入时使用的密码"
                />
                <p class="text-sm text-muted-foreground">导出文件将用此密码加密，导入加密文件时需输入相同密码。</p>
              </div>

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
