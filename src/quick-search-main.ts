/**
 * quick-search-main.ts - 快速搜索窗口入口文件
 *
 * 这是独立快速搜索窗口的入口点。
 */

// 导入全局样式
import './assets/main.css'
import { disableContextMenu } from './lib/windowSetup'

// 禁用浏览器默认右键菜单
disableContextMenu()

// Vue 核心
import { createApp } from 'vue'
import { createPinia } from 'pinia'

// 应用组件
import QuickSearchWindow from './windows/QuickSearchWindow.vue'

// 创建 Vue 应用实例
const app = createApp(QuickSearchWindow)

// 注册 Pinia 状态管理插件
app.use(createPinia())

// 挂载应用
app.mount('#app')
