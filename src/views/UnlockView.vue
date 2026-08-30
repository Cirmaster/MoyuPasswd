<!--
  UnlockView.vue - 解锁页面

  这是应用的入口页面，用户需要输入主密码才能进入密码库。
  主要功能：
  1. 检查是否已设置主密码
  2. 如果未设置，进入"设置主密码"模式
  3. 如果已设置，进入"验证主密码"模式
  4. 验证成功跳转到主页
  5. 亮色/暗色主题切换

  路由路径：/
-->

<script setup lang="ts">
import { ref, onMounted } from 'vue'
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

/**
 * 页面加载时检查是否已设置主密码
 */
onMounted(async () => {
  try {
    hasMasterPassword.value = await invoke<boolean>('has_master_password')
    // 如果未设置主密码，进入设置模式
    if (!hasMasterPassword.value) {
      isSetupMode.value = true
    }
  } catch (e) {
    console.error('检查主密码失败:', e)
  }
})

/**
 * 处理解锁操作
 * 验证主密码，成功后跳转到主页
 */
const handleUnlock = async () => {
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

    if (isValid) {
      // 验证成功，跳转到密码列表页
      router.push('/home')
    } else {
      // 密码错误，显示错误提示
      error.value = '密码错误，请重试'
    }
  } catch (e) {
    // 捕获异常，显示错误提示
    error.value = String(e)
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
        <CardTitle class="text-2xl">摸鱼密码</CardTitle>
        <CardDescription>
          {{ isSetupMode ? '首次使用，请设置主密码' : '输入主密码解锁您的密码库' }}
        </CardDescription>
      </CardHeader>

      <!-- 卡片内容：密码输入表单 -->
      <CardContent>
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

          <!-- 错误提示：仅在 error 有值时显示 -->
          <p v-if="error" class="text-sm text-destructive">{{ error }}</p>
        </form>
      </CardContent>

      <!-- 卡片底部：解锁/设置按钮 -->
      <CardFooter>
        <Button
          class="w-full"
          :disabled="loading"
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
          <!-- 按钮文字：根据模式和加载状态切换 -->
          {{ loading ? '处理中...' : (isSetupMode ? '设置密码' : '解锁') }}
        </Button>
      </CardFooter>
    </Card>
  </div>
</template>
