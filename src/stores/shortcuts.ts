/**
 * shortcuts.ts - 快捷键配置 Store
 *
 * 管理全局快捷键配置，支持自定义修改。
 */

import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

/**
 * 后端快捷键配置接口（snake_case）
 */
interface BackendShortcutConfig {
  quick_search: string
  quick_add: string
  password_generator: string
}

/**
 * 前端快捷键配置接口（camelCase）
 */
export interface ShortcutConfig {
  /** 快速搜索快捷键 */
  quickSearch: string
  /** 快速添加快捷键 */
  quickAdd: string
  /** 密码生成器快捷键 */
  passwordGenerator: string
}

/**
 * 默认快捷键配置
 */
const defaultShortcuts: ShortcutConfig = {
  quickSearch: 'CmdOrCtrl+K',
  quickAdd: 'CmdOrCtrl+Shift+N',
  passwordGenerator: 'CmdOrCtrl+Shift+G',
}

/**
 * 将后端配置转换为前端配置
 */
function toFrontendConfig(backend: BackendShortcutConfig): ShortcutConfig {
  return {
    quickSearch: backend.quick_search,
    quickAdd: backend.quick_add,
    passwordGenerator: backend.password_generator,
  }
}

/**
 * 将前端配置转换为后端配置
 */
function toBackendConfig(frontend: ShortcutConfig): BackendShortcutConfig {
  return {
    quick_search: frontend.quickSearch,
    quick_add: frontend.quickAdd,
    password_generator: frontend.passwordGenerator,
  }
}

/**
 * 快捷键配置 Store
 */
export const useShortcutStore = defineStore('shortcuts', () => {
  /**
   * 快捷键配置
   */
  const shortcuts = ref<ShortcutConfig>({ ...defaultShortcuts })

  /**
   * 从后端加载快捷键配置
   */
  async function loadShortcuts() {
    try {
      console.log('加载快捷键配置...')
      const saved = await invoke<BackendShortcutConfig>('get_shortcuts')
      console.log('后端返回的快捷键配置:', saved)
      if (saved) {
        const frontendConfig = toFrontendConfig(saved)
        console.log('转换后的前端配置:', frontendConfig)
        shortcuts.value = frontendConfig
      }
    } catch (e) {
      console.error('加载快捷键配置失败:', e)
    }
  }

  /**
   * 保存快捷键配置到后端
   */
  async function saveShortcuts() {
    try {
      const backendConfig = toBackendConfig(shortcuts.value)
      console.log('保存快捷键配置:', backendConfig)
      await invoke('save_shortcuts', { shortcuts: backendConfig })
      console.log('快捷键配置保存成功')
      
      // 动态更新全局快捷键
      console.log('正在更新全局快捷键...')
      await invoke('update_global_shortcuts')
      console.log('全局快捷键更新成功')
    } catch (e) {
      console.error('保存快捷键配置失败:', e)
      throw e
    }
  }

  /**
   * 更新单个快捷键
   * @param key - 快捷键名称
   * @param value - 快捷键值
   */
  async function updateShortcut(key: keyof ShortcutConfig, value: string) {
    console.log(`更新快捷键: ${key} = ${value}`)
    shortcuts.value = { ...shortcuts.value, [key]: value }
    await saveShortcuts()
  }

  /**
   * 重置为默认快捷键
   */
  async function resetToDefault() {
    shortcuts.value = { ...defaultShortcuts }
    await saveShortcuts()
  }

  /**
   * 格式化快捷键显示
   * @param shortcut - 快捷键字符串
   * @returns 格式化后的显示文本
   */
  function formatShortcut(shortcut: string): string {
    return shortcut
      .replace('CmdOrCtrl', 'Ctrl')
      .replace('Cmd', '⌘')
      .replace('Ctrl', 'Ctrl')
      .replace('Shift', 'Shift')
      .replace('Alt', 'Alt')
      .replace('Super', 'Win')
      .replace(/\+/g, ' + ')
  }

  return {
    shortcuts,
    loadShortcuts,
    saveShortcuts,
    updateShortcut,
    resetToDefault,
    formatShortcut,
  }
})
