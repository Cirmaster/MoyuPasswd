<!--
  PasswordGenerator.vue - 密码生成器组件

  用于生成安全的随机密码。
  功能：
  1. 可配置密码长度（4-64位）
  2. 可选择包含的字符类型（大写、小写、数字、特殊字符）
  3. 实时显示密码强度
  4. 支持复制到剪贴板
  5. 支持将生成的密码传递给父组件

  使用 Web Crypto API 生成加密安全的随机数。

  @example
  ```vue
  <PasswordGenerator
    v-model:open="showGenerator"
    @generated="onPasswordGenerated"
  />
  ```
-->

<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { calcPasswordStrength } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
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
 * - generated: 密码生成后触发，传递生成的密码
 */
const emit = defineEmits<{
  'update:open': [value: boolean]
  generated: [password: string]
}>()

/**
 * 密码生成配置
 */
const config = ref({
  /** 密码长度，范围 4-64 */
  length: 16,
  /** 是否包含大写字母 A-Z */
  uppercase: true,
  /** 是否包含小写字母 a-z */
  lowercase: true,
  /** 是否包含数字 0-9 */
  numbers: true,
  /** 是否包含特殊字符 !@#$%^&* 等 */
  symbols: true,
})

/** 生成的密码 */
const password = ref('')

/** Toast 提示状态 */
const toast = ref({
  show: false,
  type: 'success' as 'success' | 'error' | 'info',
  message: '',
})

/**
 * 字符集定义
 * 根据配置选择对应的字符集
 */
const charSets = {
  uppercase: 'ABCDEFGHIJKLMNOPQRSTUVWXYZ',
  lowercase: 'abcdefghijklmnopqrstuvwxyz',
  numbers: '0123456789',
  symbols: '!@#$%^&*()_+-=[]{}|;:,.<>?',
}

/**
 * 计算密码强度
 * 基于密码长度和包含的字符类型计算得分
 *
 * 评分规则：
 * - 长度 >= 8: +1 分
 * - 长度 >= 12: +1 分
 * - 长度 >= 16: +1 分
 * - 包含大写字母: +1 分
 * - 包含小写字母: +1 分
 * - 包含数字: +1 分
 * - 包含特殊字符: +1 分
 *
 * 强度等级：
 * - 0-2 分: 弱（红色）
 * - 3-4 分: 中（黄色）
 * - 5 分: 强（绿色）
 * - 6-7 分: 非常强（深绿色）
 */
const strength = computed(() => {
  const level = calcPasswordStrength(password.value)
  switch (level) {
    case 1:
      return { level: 1, text: '弱', color: 'text-destructive' }
    case 2:
      return { level: 2, text: '中', color: 'text-yellow-500' }
    case 3:
      return { level: 3, text: '强', color: 'text-green-500' }
    case 4:
      return { level: 4, text: '非常强', color: 'text-emerald-500' }
    default:
      return { level: 0, text: '', color: '' }
  }
})

/**
 * 生成随机密码
 *
 * 使用 Web Crypto API 的 crypto.getRandomValues() 生成加密安全的随机数。
 * 如果没有选择任何字符类型，会自动启用小写字母。
 */
const generate = () => {
  // 根据配置拼接字符集
  let chars = ''
  if (config.value.uppercase) chars += charSets.uppercase
  if (config.value.lowercase) chars += charSets.lowercase
  if (config.value.numbers) chars += charSets.numbers
  if (config.value.symbols) chars += charSets.symbols

  // 如果没有选择任何字符类型，默认使用小写字母
  if (!chars) {
    chars = charSets.lowercase
    config.value.lowercase = true
  }

  // 使用 Web Crypto API 生成随机数（拒绝采样消除取模偏差）
  const n = chars.length
  // 只接受 [0, limit) 内的随机数，保证 % n 均匀分布
  const limit = Math.floor(0x100000000 / n) * n
  let result = ''
  const array = new Uint32Array(64)
  while (result.length < config.value.length) {
    crypto.getRandomValues(array)
    for (const v of array) {
      if (result.length >= config.value.length) break
      if (v < limit) {
        result += chars[v % n]!
      }
    }
  }
  password.value = result
}

/**
 * 复制密码到剪贴板
 * 使用 Clipboard API
 */
const copyToClipboard = async () => {
  try {
    // 生成密码属于敏感内容，复制后由后端兜底定时清除
    const settings = await invoke<{ clipboard_clear_time: number }>('get_settings')
    await invoke('copy_text_to_clipboard', {
      text: password.value,
      clearAfter: settings.clipboard_clear_time || 30,
    })
    toast.value = { show: true, type: 'success', message: '已复制到剪贴板' }
  } catch {
    toast.value = { show: true, type: 'error', message: '复制失败' }
  }
}

/**
 * 使用生成的密码
 * 将密码传递给父组件并关闭弹窗
 */
const usePassword = () => {
  emit('generated', password.value)
  emit('update:open', false)
}

/**
 * 监听配置变化，自动生成新密码
 * 当用户修改长度或字符类型时，实时更新密码
 */
watch(
  config,
  () => {
    generate()
  },
  { deep: true },
)

/**
 * 监听弹窗打开状态
 * 每次打开时自动生成新密码
 */
watch(
  () => props.open,
  (val) => {
    if (val) generate()
  },
)
</script>

<template>
  <!-- 密码生成器弹窗 -->
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <DialogContent class="sm:max-w-md">
      <DialogHeader>
        <DialogTitle>密码生成器</DialogTitle>
        <DialogDescription>生成安全的随机密码</DialogDescription>
      </DialogHeader>

      <div class="space-y-4">
        <!-- 生成的密码显示区域 -->
        <div class="p-4 bg-muted rounded-lg">
          <div class="flex items-center justify-between mb-2">
            <span class="text-sm text-muted-foreground">生成的密码</span>
            <!-- 密码强度指示器 -->
            <span :class="['text-sm font-medium', strength.color]">
              {{ strength.text }}
            </span>
          </div>
          <!-- 密码内容，使用等宽字体便于阅读 -->
          <div class="font-mono text-lg break-all">{{ password }}</div>
        </div>

        <!-- 密码长度滑块 -->
        <div class="space-y-2">
          <div class="flex items-center justify-between">
            <Label>密码长度</Label>
            <span class="text-sm text-muted-foreground">{{ config.length }} 位</span>
          </div>
          <input
            v-model.number="config.length"
            type="range"
            min="4"
            max="64"
            class="w-full"
          />
        </div>

        <!-- 字符类型选择 -->
        <div class="space-y-3">
          <Label>包含字符</Label>
          <div class="grid grid-cols-2 gap-3">
            <!-- 大写字母 -->
            <label class="flex items-center gap-2">
              <input v-model="config.uppercase" type="checkbox" class="h-4 w-4" />
              <span class="text-sm">大写字母 (A-Z)</span>
            </label>
            <!-- 小写字母 -->
            <label class="flex items-center gap-2">
              <input v-model="config.lowercase" type="checkbox" class="h-4 w-4" />
              <span class="text-sm">小写字母 (a-z)</span>
            </label>
            <!-- 数字 -->
            <label class="flex items-center gap-2">
              <input v-model="config.numbers" type="checkbox" class="h-4 w-4" />
              <span class="text-sm">数字 (0-9)</span>
            </label>
            <!-- 特殊字符 -->
            <label class="flex items-center gap-2">
              <input v-model="config.symbols" type="checkbox" class="h-4 w-4" />
              <span class="text-sm">特殊字符 (!@#...)</span>
            </label>
          </div>
        </div>
      </div>

      <!-- 底部操作按钮 -->
      <DialogFooter class="flex gap-2">
        <!-- 重新生成按钮 -->
        <Button variant="outline" @click="generate">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            class="h-4 w-4 mr-2"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <path d="M21 12a9 9 0 1 1-6.219-8.56" />
          </svg>
          重新生成
        </Button>
        <!-- 复制按钮 -->
        <Button variant="outline" @click="copyToClipboard">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            class="h-4 w-4 mr-2"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
            <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
          </svg>
          复制
        </Button>
        <!-- 使用密码按钮 -->
        <Button @click="usePassword">使用此密码</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>

  <!-- Toast 提示 -->
  <Toast
    v-model:show="toast.show"
    :type="toast.type"
    :message="toast.message"
  />
</template>
