/**
 * registry.ts - 色系主题注册表
 *
 * 设置页主题卡按这里的顺序展示。新增主题需要两步：
 * 1. src/assets/themes/<id>.css 里定义 [data-theme="<id>"] 明暗令牌值
 * 2. 在下面 themes 数组加一项
 *
 * id 必须与 CSS 里的 data-theme 值一致（也存入 localStorage 的 colorScheme 键）。
 */

import type { ColorScheme } from '@/composables/useTheme'

/** 主题元数据 */
export interface ThemeMeta {
  /** 主题 id，对应 CSS 选择器 [data-theme="id"] */
  id: ColorScheme
  /** 卡片上的主题名 */
  name: string
  /** 一句话风格描述 */
  desc: string
}

/** 内置主题清单（顺序即设置页展示顺序） */
export const themes: ThemeMeta[] = [
  {
    id: 'classic',
    name: '经典中性',
    desc: '灰紫中性色，最初的默认外观',
  },
  {
    id: 'aurora',
    name: '电光青',
    desc: '电光青与深空蓝，键帽式高对比',
  },
  {
    id: 'dusk',
    name: '暮光黄昏',
    desc: '暮紫配云霞紫红，暖金点睛',
  },
  {
    id: 'islands',
    name: 'Islands',
    desc: 'IDEA 2025.3+ 默认主题，分层岛屿与品牌蓝',
  },
]
