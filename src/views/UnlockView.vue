<!--
  UnlockView.vue - 解锁页面

  这是应用的入口页面，用户需要输入主密码才能进入密码库。
  主要功能：
  1. 检查是否已设置主密码
  2. 如果未设置，进入"设置主密码"模式
  3. 如果已设置且系统认证可用，优先使用系统认证
  4. 系统认证失败时，显示主密码输入框作为备用
  5. 亮色/暗色主题切换

  路由路径：/
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { useTheme } from '@/composables/useTheme'

/** 路由实例，用于页面跳转 */
const router = useRouter()

/** 主题管理：isDark 是否暗色模式，toggleTheme 切换主题 */
const { isDark, toggleTheme } = useTheme()

/** 用户输入的主密码 */
const password = ref('')

/** 确认密码（设置模式时使用） */
const confirmPassword = ref('')

/** 错误提示信息，为空时不显示 */
const error = ref('')

/** 加载状态，为 true 时显示加载动画并禁用按钮 */
const loading = ref(false)

/** 是否已设置主密码 */
const hasMasterPassword = ref(false)

/** 是否为设置模式（首次使用） */
const isSetupMode = ref(false)

/** 系统认证是否可用 */
const systemAuthAvailable = ref(false)

/** 系统认证是否已启用 */
const systemAuthEnabled = ref(false)

/** 是否正在尝试系统认证 */
const attemptingSystemAuth = ref(false)

/** 是否显示主密码输入（系统认证失败后的备用） */
const showPasswordInput = ref(false)

/** 锁定倒计时秒数（暴力破解防护） */
const lockoutRemaining = ref(0)

/** 锁定倒计时定时器 */
let lockoutTimer: ReturnType<typeof setInterval> | null = null

/**
 * 启动锁定倒计时
 * @param seconds - 倒计时秒数
 */
const startLockoutCountdown = (seconds: number) => {
  lockoutRemaining.value = seconds
  if (lockoutTimer) clearInterval(lockoutTimer)
  lockoutTimer = setInterval(() => {
    lockoutRemaining.value--
    if (lockoutRemaining.value <= 0) {
      if (lockoutTimer) clearInterval(lockoutTimer)
      lockoutTimer = null
      error.value = ''
    }
  }, 1000)
}

/**
 * 解析错误信息中的等待秒数
 * @param msg - 错误信息
 * @returns 秒数，如果解析失败返回 0
 */
const parseLockoutSeconds = (msg: string): number => {
  const match = msg.match(/(\d+)\s*秒/)
  return match && match[1] ? parseInt(match[1], 10) : 0
}

/**
 * 尝试系统认证解锁
 * 如果成功则直接跳转到主页，失败则显示主密码输入框
 */
const trySystemAuth = async () => {
  attemptingSystemAuth.value = true
  error.value = ''

  try {
    const success = await invoke<boolean>('unlock_with_system_auth')
    if (success) {
      // 系统认证成功，直接跳转
      router.push('/home')
      return
    }
    // 用户取消了认证，显示主密码输入
    showPasswordInput.value = true
  } catch (e) {
    console.warn('系统认证失败:', e)
    // 系统认证失败，显示主密码输入
    showPasswordInput.value = true
    error.value = String(e)
  } finally {
    attemptingSystemAuth.value = false
  }
}

/**
 * 页面加载时检查状态
 */
onMounted(async () => {
  try {
    // 检查是否已设置主密码
    hasMasterPassword.value = await invoke<boolean>('has_master_password')

    if (!hasMasterPassword.value) {
      // 未设置主密码，进入设置模式
      isSetupMode.value = true
      showPasswordInput.value = true
      return
    }

    // 已设置主密码，检查系统认证
    const [available, enabled] = await Promise.all([
      invoke<boolean>('is_system_auth_available'),
      invoke<boolean>('is_system_auth_enabled'),
    ])

    systemAuthAvailable.value = available
    systemAuthEnabled.value = enabled

    if (available && enabled) {
      // 系统认证已启用：不显示密码输入框，只显示系统认证按钮
      showPasswordInput.value = false
    } else {
      // 系统认证未启用：显示密码输入框
      showPasswordInput.value = true
    }
  } catch (e) {
    console.error('初始化失败:', e)
    showPasswordInput.value = true
  }
})

/**
 * 组件卸载时清理倒计时定时器
 */
onUnmounted(() => {
  if (lockoutTimer) {
    clearInterval(lockoutTimer)
    lockoutTimer = null
  }
})

/**
 * 处理解锁操作（主密码）
 */
const handleUnlock = async () => {
  // 锁定中不允许操作
  if (lockoutRemaining.value > 0) return

  // 密码为空时显示提示
  if (!password.value) {
    error.value = '请输入主密码'
    return
  }

  // 设置加载状态，清空之前的错误
  loading.value = true
  error.value = ''

  try {
    // 调用 Tauri 后端验证主密码
    const isValid = await invoke<boolean>('verify_master_password', {
      password: password.value,
    })

    // 清空密码输入（无论成功失败）
    password.value = ''

    if (isValid) {
      // 验证成功，跳转到密码列表页
      router.push('/home')
    } else {
      // 密码错误，显示错误提示
      error.value = '密码错误，请重试'
    }
  } catch (e) {
    // 清空密码输入
    password.value = ''

    // 捕获异常，检查是否是锁定错误
    const msg = String(e)
    const seconds = parseLockoutSeconds(msg)
    if (seconds > 0) {
      startLockoutCountdown(seconds)
      error.value = msg
    } else {
      error.value = msg
    }
  } finally {
    // 无论成功失败，都取消加载状态
    loading.value = false
  }
}

/**
 * 处理设置主密码操作
 * 首次使用时设置主密码
 */
const handleSetup = async () => {
  // 验证密码
  if (!password.value) {
    error.value = '请输入主密码'
    return
  }

  if (password.value.length < 6) {
    error.value = '密码长度至少6位'
    return
  }

  if (password.value !== confirmPassword.value) {
    error.value = '两次密码不一致'
    return
  }

  // 设置加载状态，清空之前的错误
  loading.value = true
  error.value = ''

  try {
    // 调用 Tauri 后端设置主密码
    await invoke('set_master_password', {
      password: password.value,
    })

    // 设置成功，跳转到密码列表页
    router.push('/home')
  } catch (e) {
    // 捕获异常，显示错误提示
    error.value = String(e)
  } finally {
    // 无论成功失败，都取消加载状态
    loading.value = false
  }
}
</script>

<template>
  <!-- 全屏居中布局，relative 用于定位主题切换按钮 -->
  <div class="min-h-screen flex items-center justify-center bg-background p-4 relative">

    <!-- 主题切换按钮，固定在右上角 -->
    <Button
      variant="ghost"
      size="icon"
      class="absolute top-4 right-4"
      @click="toggleTheme"
    >
      <!-- 太阳图标：当处于暗色模式时显示，点击切换到亮色 -->
      <svg
        v-if="isDark"
        xmlns="http://www.w3.org/2000/svg"
        class="h-5 w-5"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <circle cx="12" cy="12" r="4" />
        <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
      </svg>
      <!-- 月亮图标：当处于亮色模式时显示，点击切换到暗色 -->
      <svg
        v-else
        xmlns="http://www.w3.org/2000/svg"
        class="h-5 w-5"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
      </svg>
    </Button>

    <!-- 主卡片容器 -->
    <Card class="w-full max-w-md">
      <!-- 卡片头部：Logo + 标题 + 描述 -->
      <CardHeader class="text-center">
        <!-- 锁图标容器 -->
        <div class="mx-auto mb-4 w-16 h-16 rounded-full bg-primary/10 flex items-center justify-center">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            class="h-8 w-8 text-primary"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
            <path d="M7 11V7a5 5 0 0 1 10 0v4" />
          </svg>
        </div>
        <CardTitle class="text-2xl">默语密匣</CardTitle>
        <CardDescription>
          <template v-if="isSetupMode">首次使用，请设置主密码</template>
          <template v-else-if="attemptingSystemAuth">请在弹出的窗口中完成认证</template>
          <template v-else-if="showPasswordInput">输入主密码解锁您的密码库</template>
          <template v-else>点击下方按钮解锁密码库</template>
        </CardDescription>
      </CardHeader>

      <!-- 系统认证进行中 -->
      <CardContent v-if="attemptingSystemAuth" class="text-center py-8">
        <div class="flex flex-col items-center gap-4">
          <!-- 加载动画 -->
          <svg
            class="h-12 w-12 animate-spin text-primary"
            xmlns="http://www.w3.org/2000/svg"
            fill="none"
            viewBox="0 0 24 24"
          >
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" />
            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
          </svg>
          <p class="text-sm text-muted-foreground">请在弹出的窗口中选择解锁方式</p>
          <Button variant="outline" size="sm" @click="showPasswordInput = true; attemptingSystemAuth = false">
            使用主密码解锁
          </Button>
        </div>
      </CardContent>

      <!-- 系统认证已启用：只显示认证按钮 -->
      <CardContent v-else-if="!showPasswordInput && !isSetupMode" class="text-center py-6">
        <Button class="w-full h-12 text-base" @click="trySystemAuth">
          解锁
        </Button>
        <div class="mt-4">
          <Button variant="link" size="sm" @click="showPasswordInput = true">
            使用主密码解锁
          </Button>
        </div>
      </CardContent>

      <!-- 密码输入表单（系统认证未启用，或用户选择使用主密码） -->
      <CardContent v-else>
        <form @submit.prevent="isSetupMode ? handleSetup() : handleUnlock()" class="space-y-4">
          <div class="space-y-2">
            <Label for="password">主密码</Label>
            <Input
              id="password"
              v-model="password"
              type="password"
              :placeholder="isSetupMode ? '设置主密码（至少6位）' : '请输入主密码'"
              :disabled="loading"
              autofocus
            />
          </div>

          <!-- 设置模式时显示确认密码 -->
          <div v-if="isSetupMode" class="space-y-2">
            <Label for="confirm-password">确认密码</Label>
            <Input
              id="confirm-password"
              v-model="confirmPassword"
              type="password"
              placeholder="再次输入主密码"
              :disabled="loading"
            />
          </div>

          <!-- 系统认证可用但用户选择了主密码，显示切换按钮 -->
          <div v-if="!isSetupMode && systemAuthAvailable && systemAuthEnabled" class="flex justify-center">
            <Button variant="link" size="sm" @click="showPasswordInput = false">
              使用系统认证解锁
            </Button>
          </div>

          <!-- 错误提示：仅在 error 有值时显示 -->
          <p v-if="error" class="text-sm text-destructive">{{ error }}</p>
        </form>
      </CardContent>

      <!-- 卡片底部：解锁/设置按钮（仅在显示密码输入时） -->
      <CardFooter v-if="showPasswordInput">
        <Button
          class="w-full"
          :disabled="loading || lockoutRemaining > 0"
          @click="isSetupMode ? handleSetup() : handleUnlock()"
        >
          <!-- 加载动画：仅在 loading 时显示 -->
          <svg
            v-if="loading"
            class="mr-2 h-4 w-4 animate-spin"
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <path d="M21 12a9 9 0 1 1-6.219-8.56" />
          </svg>
          <!-- 按钮文字：根据模式、加载状态和锁定状态切换 -->
          <template v-if="lockoutRemaining > 0">
            请等待 {{ lockoutRemaining }} 秒
          </template>
          <template v-else>
            {{ loading ? '处理中...' : (isSetupMode ? '设置密码' : '解锁') }}
          </template>
        </Button>
      </CardFooter>
    </Card>
  </div>
</template>
