<!--
  App.vue - 应用根组件

  这是 Vue 应用的根组件，所有页面都在此渲染。
  主要职责：
  1. 初始化主题（从 localStorage 读取或跟随系统）
  2. 渲染路由视图（RouterView）

  应用启动流程：
  1. main.ts 创建 Vue 应用
  2. App.vue 挂载，初始化主题
  3. 路由将用户导向解锁页面（/）
-->

<script setup lang="ts">
import { onMounted } from 'vue'
import { RouterView } from 'vue-router'
import { useTheme } from '@/composables/useTheme'
import { useCountdown } from '@/composables/useCountdown'
import CaretCountdown from '@/components/CaretCountdown.vue'

/** 主题管理 */
const { initTheme } = useTheme()

/** 全局倒计时 */
const {
  showCountdown,
  countdownSeconds,
  countdownX,
  countdownY,
  onCountdownEnd,
} = useCountdown()

/**
 * 组件挂载时初始化主题
 */
onMounted(() => {
  initTheme()
})
</script>

<template>
  <RouterView />
  <CaretCountdown
    v-model:show="showCountdown"
    :seconds="countdownSeconds"
    :x="countdownX"
    :y="countdownY"
    @countdown-end="onCountdownEnd"
  />
</template>
