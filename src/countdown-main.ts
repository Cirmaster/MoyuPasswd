/**
 * countdown-main.ts - 倒计时窗口入口文件
 *
 * 这是独立倒计时窗口的入口点。
 */

// 导入全局样式
import './assets/main.css'
import { disableContextMenu } from './lib/windowSetup'

// 禁用浏览器默认右键菜单
disableContextMenu()

// Vue 核心
import { createApp } from 'vue'

// 应用组件
import CountdownWindow from './windows/CountdownWindow.vue'

// 创建 Vue 应用实例
const app = createApp(CountdownWindow)

// 挂载应用
app.mount('#app')
