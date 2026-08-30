<!--
  PasswordStrength.vue - 密码强度指示器组件

  显示密码的安全强度等级。
  根据密码长度、字符类型计算强度。

  @example
  ```vue
  <PasswordStrength password="MyP@ssw0rd!" />
  ```
-->

<script setup lang="ts">
import { computed } from 'vue'

/**
 * 组件 Props
 */
interface Props {
  /** 要评估的密码 */
  password: string
}

const props = defineProps<Props>()

/**
 * 密码强度计算
 *
 * 评分规则：
 * - 长度 >= 8: +1 分
 * - 长度 >= 12: +1 分
 * - 长度 >= 16: +1 分
 * - 包含小写字母: +1 分
 * - 包含大写字母: +1 分
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
  const pwd = props.password
  if (!pwd) return { level: 0, text: '', color: '', percent: 0 }

  let score = 0
  if (pwd.length >= 8) score++
  if (pwd.length >= 12) score++
  if (pwd.length >= 16) score++
  if (/[a-z]/.test(pwd)) score++
  if (/[A-Z]/.test(pwd)) score++
  if (/[0-9]/.test(pwd)) score++
  if (/[^A-Za-z0-9]/.test(pwd)) score++

  if (score <= 2) return { level: 1, text: '弱', color: 'bg-destructive', percent: 25 }
  if (score <= 4) return { level: 2, text: '中', color: 'bg-yellow-500', percent: 50 }
  if (score <= 5) return { level: 3, text: '强', color: 'bg-green-500', percent: 75 }
  return { level: 4, text: '非常强', color: 'bg-emerald-500', percent: 100 }
})
</script>

<template>
  <div v-if="password" class="flex items-center gap-2">
    <!-- 强度条 -->
    <div class="flex-1 h-1.5 bg-muted rounded-full overflow-hidden">
      <div
        class="h-full transition-all duration-300 rounded-full"
        :class="strength.color"
        :style="{ width: `${strength.percent}%` }"
      />
    </div>
    <!-- 强度文字 -->
    <span
      class="text-xs font-medium shrink-0"
      :class="{
        'text-destructive': strength.level === 1,
        'text-yellow-500': strength.level === 2,
        'text-green-500': strength.level === 3,
        'text-emerald-500': strength.level === 4,
      }"
    >
      {{ strength.text }}
    </span>
  </div>
</template>
