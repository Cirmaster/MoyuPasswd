/**
 * env.d.ts - Vite 环境变量类型声明
 *
 * 注意：这里不引用 `vite/client` —— 其中的 `declare module '*.svg' { const src: string }`
 * 与 src/svg.d.ts 的组件声明冲突（TypeScript 对同名通配符模块取先加载者），
 * 会让所有图标被推断为 string，导致传给 `icon?: Component` 时报错。
 * 这里只声明本项目实际用到的 import.meta.env。
 */
interface ImportMetaEnv {
  /** 站点基础路径（vite base 配置） */
  readonly BASE_URL: string
  /** 运行模式：development / production 等 */
  readonly MODE: string
  /** 是否开发环境 */
  readonly DEV: boolean
  /** 是否生产环境 */
  readonly PROD: boolean
  /** 是否服务端渲染 */
  readonly SSR: boolean
  /** 自定义环境变量（.env 文件中的 VITE_*） */
  readonly [key: string]: string | boolean | undefined
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}

/** 样式副作用导入（Vite 构建时注入） */
declare module '*.css'
