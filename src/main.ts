/**
 * main.ts - 应用入口文件
 *
 * 这是 Vue 应用的入口点，负责：
 * 1. 导入全局样式（Tailwind CSS）
 * 2. 创建 Vue 应用实例
 * 3. 注册插件（Pinia 状态管理、Vue Router 路由）
 * 4. 挂载应用到 DOM
 *
 * 启动顺序：
 * 1. 导入 CSS 样式
 * 2. 创建 App 实例
 * 3. 注册 Pinia（状态管理）
 * 4. 注册 Router（路由）
 * 5. 挂载到 #app 元素
 */

// 导入全局样式（包含 Tailwind CSS 和主题变量）
import './assets/main.css'

// Vue 核心
import { createApp } from 'vue'
import { createPinia } from 'pinia'

// 应用组件和路由
import App from './App.vue'
import router from './router'

// 创建 Vue 应用实例
const app = createApp(App)

// 注册 Pinia 状态管理插件
// Pinia 用于管理全局状态（密码数据、用户设置等）
app.use(createPinia())

// 注册 Vue Router 路由插件
// Router 用于页面导航（解锁页、主页、设置页等）
app.use(router)

// 挂载应用到 DOM 中的 #app 元素
app.mount('#app')
