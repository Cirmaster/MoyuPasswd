<!--
  PasswordStrength.vue - 密码强度指示器组件

  显示密码的安全强度等级。
  强度等级由后端（或前端 calcPasswordStrength）计算后传入，
  本组件只负责渲染，不重复实现评分逻辑。

  @example
  ```vue
  <PasswordStrength :level="3" />
  ```
-->

<script setup lang="ts">
import { computed } from 'vue'

/**
 * 组件 Props
 */
interface Props {
  /** 密码强度等级：0(空/未知) | 1(弱) | 2(中) | 3(强) | 4(非常强) */
  level: number
  /** 紧凑模式：只显示强度条（文字进 title），用于表格等窄列场景 */
  compact?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  compact: false,
})

const strength = computed(() => {
  switch (props.level) {
    case 1:
      return { level: 1, text: '弱', color: 'bg-destructive', percent: 25 }
    case 2:
      return { level: 2, text: '中', color: 'bg-yellow-500', percent: 50 }
    case 3:
      return { level: 3, text: '强', color: 'bg-green-500', percent: 75 }
    case 4:
      return { level: 4, text: '非常强', color: 'bg-emerald-500', percent: 100 }
    default:
      return { level: 0, text: '', color: '', percent: 0 }
  }
})
</script>

<template>
  <div v-if="level > 0" class="flex items-center gap-2" :title="compact ? `密码强度：${strength.text}` : undefined">
    <!-- 强度条 -->
    <div class="flex-1 h-1.5 bg-muted rounded-full overflow-hidden">
      <div
        class="h-full transition-all duration-300 rounded-full"
        :class="strength.color"
        :style="{ width: `${strength.percent}%` }"
      />
    </div>
    <!-- 强度文字（紧凑模式隐藏，文字进 title） -->
    <span
      v-if="!compact"
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
