/**
 * svg.d.ts - .svg 文件作为 Vue 组件导入的类型声明
 *
 * 由 vite-svg-loader 处理（见 vite.config.ts），
 * 图标源文件在 src/assets/svg/。
 */
declare module '*.svg' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<Record<string, unknown>, Record<string, unknown>, unknown>
  export default component
}
