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
import { useAutoLock } from '@/composables/useAutoLock'

/** 主题管理 */
const { initTheme } = useTheme()

/** 全局用户活动上报（自动锁定由后端空闲检测统一负责，以系统空闲为准） */
useAutoLock()

/**
 * 组件挂载时初始化主题
 */
onMounted(() => {
  initTheme()
})
</script>

<template>
  <RouterView />
</template>
