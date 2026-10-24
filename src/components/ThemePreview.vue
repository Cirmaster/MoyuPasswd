<!--
  ThemePreview.vue - 主题假界面预览

  设置页主题卡里的迷你界面预览：容器挂 data-theme（加 .dark 表示暗色半区），
  令牌在容器上局部解析，不用切换全局主题就能预览任意主题的明/暗两套配色。

  内容刻意复刻快捷弹窗的关键元素（chip / 主按钮 / 徽标 / 列表选中行 / 复制密码键帽），
  让预览接近真实界面效果。只使用主题文件里定义过的核心令牌。
-->

<script setup lang="ts">
import type { ColorScheme } from '@/composables/useTheme'

const props = defineProps<{
  /** 要预览的色系主题 id */
  theme: ColorScheme
  /** 是否预览暗色配色（加 .dark 类） */
  dark?: boolean
}>()
</script>

<template>
  <div
    :data-theme="props.theme"
    :class="props.dark ? 'dark' : undefined"
    class="space-y-1.5 rounded-md border border-border bg-background p-2"
  >
    <!-- 分类 chip：选中走填充档 -->
    <div class="flex gap-1">
      <div class="rounded bg-select px-1.5 py-0.5 text-[11px] text-select-foreground">全部</div>
      <div class="rounded bg-muted px-1.5 py-0.5 text-[11px] text-foreground">收藏</div>
      <div class="rounded bg-muted px-1.5 py-0.5 text-[11px] text-foreground">最近</div>
    </div>

    <!-- 主按钮（data-slot="button" 供暮光主题挂渐变）+ 徽标 + 链接 -->
    <div class="flex items-center gap-1.5">
      <div
        data-slot="button"
        class="rounded-md bg-primary px-2 py-1 text-[11px] text-primary-foreground"
      >
        新增密码
      </div>
      <div class="rounded-full bg-primary-ink/10 px-1.5 py-0.5 text-[11px] text-primary-ink">工作</div>
      <div class="text-[11px] text-primary-ink underline">管理分类</div>
    </div>

    <!-- 快捷弹窗列表：普通行 / 选中行（含复制密码键帽）/ hover 行 -->
    <div class="overflow-hidden rounded-md border border-border bg-card">
      <div class="px-2 py-1">
        <div class="text-[11px] text-card-foreground">服务器登录</div>
      </div>
      <div class="flex items-center justify-between bg-action px-2 py-1">
        <div class="text-[11px] text-action-foreground">生产数据库</div>
        <div class="rounded bg-keycap px-1.5 py-0.5 text-[11px] text-keycap-foreground">复制密码</div>
      </div>
      <div class="bg-accent px-2 py-1">
        <div class="text-[11px] text-accent-foreground">邮箱账号</div>
      </div>
    </div>
  </div>
</template>
