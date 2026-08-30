/**
 * useTheme.ts - 主题管理 Composable
 *
 * 提供全局的主题管理功能，支持：
 * 1. 亮色模式 (light)
 * 2. 暗色模式 (dark)
 * 3. 跟随系统 (system)
 *
 * 主题设置会自动保存到 localStorage，下次打开应用时恢复。
 * 同时会监听系统主题变化，当选择"跟随系统"时自动切换。
 *
 * @example
 * ```ts
 * const { theme, isDark, toggleTheme, setTheme } = useTheme()
 *
 * // 切换亮暗
 * toggleTheme()
 *
 * // 设置特定主题
 * setTheme('dark')
 * ```
 */

import { ref, watch } from 'vue'

/** 主题类型：亮色、暗色、跟随系统 */
type Theme = 'light' | 'dark' | 'system'

/** 当前主题设置，默认跟随系统 */
const theme = ref<Theme>('system')

/** 是否处于暗色模式（响应式） */
const isDark = ref(false)

/**
 * 获取系统主题偏好
 * 通过 matchMedia API 检测操作系统是否使用暗色模式
 * @returns 'dark' | 'light'
 */
function getSystemTheme(): 'light' | 'dark' {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

/**
 * 将主题应用到 DOM
 * 通过在 <html> 元素上添加/移除 'dark' 类名来切换主题
 * Tailwind CSS 的暗色模式通过这个类名生效
 *
 * @param value - 要应用的主题类型
 */
function applyTheme(value: Theme) {
  // 如果是跟随系统，先获取系统实际主题
  const resolved = value === 'system' ? getSystemTheme() : value

  // 更新响应式状态
  isDark.value = resolved === 'dark'

  // 切换 <html> 元素的 dark 类名
  document.documentElement.classList.toggle('dark', isDark.value)
}

// 监听系统主题变化
// 当用户在系统设置中切换主题时，自动更新应用主题
if (typeof window !== 'undefined') {
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
    // 只有在"跟随系统"模式下才响应系统变化
    if (theme.value === 'system') {
      applyTheme('system')
    }
  })
}

/**
 * 主题管理 Hook
 *
 * @returns 包含以下属性和方法的对象：
 * - theme: 当前主题设置（响应式）
 * - isDark: 是否暗色模式（响应式）
 * - initTheme: 初始化主题（应用启动时调用）
 * - setTheme: 设置主题
 * - toggleTheme: 切换亮暗模式
 */
export function useTheme() {
  /**
   * 初始化主题
   * 从 localStorage 读取保存的主题设置，如果没有则默认使用"跟随系统"
   * 应用启动时调用一次即可
   */
  function initTheme() {
    const saved = localStorage.getItem('theme') as Theme | null
    theme.value = saved || 'system'
    applyTheme(theme.value)
  }

  /**
   * 设置主题并保存到 localStorage
   * @param value - 要设置的主题类型
   */
  function setTheme(value: Theme) {
    theme.value = value
    localStorage.setItem('theme', value)
    applyTheme(value)
  }

  /**
   * 切换亮暗模式
   * 如果当前是暗色则切换到亮色，反之亦然
   */
  function toggleTheme() {
    const next = isDark.value ? 'light' : 'dark'
    setTheme(next)
  }

  // 监听 theme 变化，自动应用
  watch(theme, (val) => applyTheme(val))

  return {
    theme,
    isDark,
    initTheme,
    setTheme,
    toggleTheme,
  }
}
