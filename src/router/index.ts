/**
 * router/index.ts - 路由配置
 *
 * 定义应用的所有页面路由。
 * 使用懒加载方式导入组件，优化首屏加载速度。
 *
 * 路由列表：
 * - / (unlock): 解锁页面，应用入口
 * - /home: 密码列表主页
 * - /settings: 设置页面
 * - /categories: 分类管理页面
 */

import { createRouter, createWebHistory } from 'vue-router'

/**
 * 创建路由实例
 *
 * history: 使用 HTML5 History 模式（无 # 号）
 * routes: 路由配置数组
 */
const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    /**
     * 解锁页面（应用入口）
     * 路径: /
     * 用户需要输入主密码才能进入应用
     */
    {
      path: '/',
      name: 'unlock',
      // 懒加载：只在访问该路由时才加载组件
      component: () => import('../views/UnlockView.vue'),
    },
    /**
     * 密码列表主页
     * 路径: /home
     * 显示所有密码，支持搜索、分类筛选、CRUD 操作
     */
    {
      path: '/home',
      name: 'home',
      component: () => import('../views/HomeView.vue'),
    },
    /**
     * 设置页面
     * 路径: /settings
     * 包含通用设置、安全设置、修改密码、数据管理
     */
    {
      path: '/settings',
      name: 'settings',
      component: () => import('../views/SettingsView.vue'),
    },
    /**
     * 分类管理页面
     * 路径: /categories
     * 查看、添加、删除密码分类
     */
    {
      path: '/categories',
      name: 'categories',
      component: () => import('../views/CategoryView.vue'),
    },
  ],
})

export default router
