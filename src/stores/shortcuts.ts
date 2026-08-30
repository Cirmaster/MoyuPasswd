/**
 * shortcuts.ts - 快捷键配置 Store
 *
 * 管理全局快捷键配置，支持自定义修改。
 */

import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

/**
 * 快捷键配置接口
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
      const saved = await invoke<ShortcutConfig | null>('get_shortcuts')
      if (saved) {
        shortcuts.value = saved
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
      await invoke('save_shortcuts', { shortcuts: shortcuts.value })
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
    shortcuts.value[key] = value
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
