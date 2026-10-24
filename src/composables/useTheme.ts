/**
 * useTheme.ts - 主题管理 Composable
 *
 * 主题分两个正交维度：
 * 1. 明暗模式 (theme)：light / dark / system，切换 <html> 上的 .dark 类
 * 2. 色系主题 (colorScheme)：classic / aurora / dusk，切换 <html> 上的 data-theme 属性
 *
 * 两者的值分别持久化到 localStorage（'theme' / 'colorScheme'），
 * 各窗口（主窗口 / 快捷搜索 / 倒计时）启动时读取恢复；
 * 并监听 storage 事件，一个窗口改主题，其他已打开的窗口实时跟随。
 *
 * @example
 * ```ts
 * const { theme, isDark, colorScheme, toggleTheme, setColorScheme } = useTheme()
 *
 * toggleTheme()              // 切换明暗
 * setColorScheme('dusk')     // 切换色系
 * ```
 */

import { ref, watch } from 'vue'

/** 明暗模式：亮色、暗色、跟随系统 */
type Theme = 'light' | 'dark' | 'system'

/** 色系主题 id，与 src/assets/themes/ 下的文件一一对应 */
export type ColorScheme = 'classic' | 'aurora' | 'dusk' | 'islands'

/** 当前明暗模式，默认暗色（初次运行即 Islands 暗色） */
const theme = ref<Theme>('dark')

/** 当前色系主题，默认 Islands */
const colorScheme = ref<ColorScheme>('islands')

/** 是否处于暗色模式（响应式） */
const isDark = ref(false)

/** localStorage 存储键：明暗模式 */
const STORAGE_KEY_THEME = 'theme'

/** localStorage 存储键：色系主题 */
const STORAGE_KEY_SCHEME = 'colorScheme'

/**
 * 获取系统主题偏好
 * 通过 matchMedia API 检测操作系统是否使用暗色模式
 * @returns 'dark' | 'light'
 */
function getSystemTheme(): 'light' | 'dark' {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

/**
 * 将明暗模式应用到 DOM
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

/**
 * 将色系主题应用到 DOM
 * 在 <html> 元素上写 data-theme 属性，主题 CSS 按属性选择器换整套令牌值
 *
 * @param value - 色系主题 id
 */
function applyColorScheme(value: ColorScheme) {
  document.documentElement.dataset.theme = value
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

  // 跨窗口同步：设置页改主题后，快捷搜索 / 倒计时等已打开的窗口实时跟随
  window.addEventListener('storage', (e) => {
    if (e.key === STORAGE_KEY_THEME && e.newValue) {
      theme.value = e.newValue as Theme
      applyTheme(theme.value)
    } else if (e.key === STORAGE_KEY_SCHEME && e.newValue) {
      colorScheme.value = e.newValue as ColorScheme
      applyColorScheme(colorScheme.value)
    }
  })
}

/**
 * 主题管理 Hook
 *
 * @returns 包含以下属性和方法的对象：
 * - theme: 当前明暗模式（响应式）
 * - isDark: 是否暗色模式（响应式）
 * - colorScheme: 当前色系主题（响应式）
 * - initTheme: 初始化主题（应用启动时调用）
 * - setTheme: 设置明暗模式
 * - toggleTheme: 切换明暗模式
 * - setColorScheme: 设置色系主题
 */
export function useTheme() {
  /**
   * 初始化主题
   * 从 localStorage 读取保存的明暗模式与色系主题，应用启动时调用一次即可。
   * HTML 入口里的防闪脚本已提前写过一遍属性，这里再调用保证 Vue 状态同步。
   */
  function initTheme() {
    const savedTheme = localStorage.getItem(STORAGE_KEY_THEME) as Theme | null
    theme.value = savedTheme || 'dark'
    applyTheme(theme.value)

    const savedScheme = localStorage.getItem(STORAGE_KEY_SCHEME) as ColorScheme | null
    if (
      savedScheme === 'classic' ||
      savedScheme === 'aurora' ||
      savedScheme === 'dusk' ||
      savedScheme === 'islands'
    ) {
      colorScheme.value = savedScheme
    }
    applyColorScheme(colorScheme.value)
  }

  /**
   * 设置明暗模式并保存到 localStorage
   * @param value - 要设置的主题类型
   */
  function setTheme(value: Theme) {
    theme.value = value
    localStorage.setItem(STORAGE_KEY_THEME, value)
    applyTheme(value)
  }

  /**
   * 切换明暗模式
   * 如果当前是暗色则切换到亮色，反之亦然
   */
  function toggleTheme() {
    const next = isDark.value ? 'light' : 'dark'
    setTheme(next)
  }

  /**
   * 设置色系主题并保存到 localStorage
   * @param value - 色系主题 id
   */
  function setColorScheme(value: ColorScheme) {
    colorScheme.value = value
    localStorage.setItem(STORAGE_KEY_SCHEME, value)
    applyColorScheme(value)
  }

  // 监听 theme 变化，自动应用
  watch(theme, (val) => applyTheme(val))

  return {
    theme,
    isDark,
    colorScheme,
    initTheme,
    setTheme,
    toggleTheme,
    setColorScheme,
  }
}
