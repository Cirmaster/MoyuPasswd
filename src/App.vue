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
import { invoke } from '@tauri-apps/api/core'
import { useTheme } from '@/composables/useTheme'
import { useAutoLock } from '@/composables/useAutoLock'

/** 主题管理 */
const { initTheme } = useTheme()

/** 全局自动锁定（不依赖具体页面，切换页面后依然生效） */
const { setLockTimeout } = useAutoLock()

/**
 * 组件挂载时初始化主题，并加载自动锁定时间设置
 */
onMounted(async () => {
  initTheme()
  try {
    const settings = await invoke<{ auto_lock_time: number }>('get_settings')
    setLockTimeout(settings.auto_lock_time || 5)
  } catch (e) {
    console.warn('加载自动锁定设置失败:', e)
  }
})
</script>

<template>
  <RouterView />
</template>
