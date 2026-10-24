<!--
  WindowControls.vue - 自绘窗口控制条

  配合 tauri.conf.json 主窗口的 decorations:false 使用（系统原生标题栏已关闭），
  提供最小化 / 最大化·还原 / 关闭三个按钮，样式走主题令牌，融入应用自身风格。

  - 拖拽移动窗口由所在容器的 data-tauri-drag-region 负责（本组件自身不带拖拽属性）
  - 关闭调 window.close()，触发 Rust 侧 on_window_event 的 CloseRequested：
    按 close_to_tray 设置决定隐藏到托盘还是真正退出，与原生关闭按钮行为一致
  - 最大化状态通过 onResized 监听刷新，图标在 最大化/还原 间切换
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'

/** 是否已最大化（决定显示「最大化」还是「还原」图标） */
const maximized = ref(false)

/** 窗口尺寸变化监听的解绑函数 */
let unlisten: (() => void) | undefined

onMounted(async () => {
  const win = getCurrentWindow()
  try {
    maximized.value = await win.isMaximized()
    unlisten = await win.onResized(async () => {
      maximized.value = await win.isMaximized()
    })
  } catch (e) {
    console.warn('初始化窗口控制条失败:', e)
  }
})

onUnmounted(() => {
  unlisten?.()
})

/** 最小化到任务栏 */
const minimize = () => {
  void getCurrentWindow().minimize()
}

/** 最大化 / 还原 */
const toggleMaximize = () => {
  void getCurrentWindow().toggleMaximize()
}

/** 关闭（是否退到托盘由后端 close_to_tray 设置决定） */
const close = () => {
  void getCurrentWindow().close()
}
</script>

<template>
  <!-- 通栏标题按钮（IDEA/Windows 风格）：矩形无圆角占满标题栏高度，
       悬浮时整个区域点亮，关闭悬浮整块泛红；左侧细分隔线把窗口三键与应用操作区分开 -->
  <div class="flex items-stretch select-none">
    <!-- 细分隔线：短居中、低对比，只暗示分组不抢视线 -->
    <div class="mx-2 h-4 w-px self-center rounded-full bg-foreground/15"></div>
    <!-- 最小化 -->
    <button
      type="button"
      class="flex h-full w-10 items-center justify-center rounded-none text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
      title="最小化"
      @click="minimize"
    >
      <svg class="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2">
        <path d="M3.5 8h9" stroke-linecap="round" />
      </svg>
    </button>

    <!-- 最大化 / 还原 -->
    <button
      type="button"
      class="flex h-full w-10 items-center justify-center rounded-none text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
      :title="maximized ? '还原' : '最大化'"
      @click="toggleMaximize"
    >
      <!-- 最大化：单框 -->
      <svg v-if="!maximized" class="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2">
        <rect x="3.75" y="3.75" width="8.5" height="8.5" rx="1.2" />
      </svg>
      <!-- 还原：叠框 -->
      <svg v-else class="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2">
        <rect x="3.25" y="5.25" width="7.5" height="7.5" rx="1.2" />
        <path d="M5.75 5.25V4.25a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-1" />
      </svg>
    </button>

    <!-- 关闭：悬浮整块泛红，与 destructive 语义一致 -->
    <button
      type="button"
      class="flex h-full w-10 items-center justify-center rounded-none text-muted-foreground transition-colors hover:bg-destructive hover:text-white"
      title="关闭"
      @click="close"
    >
      <svg class="h-3.5 w-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2">
        <path d="M4 4l8 8M12 4l-8 8" stroke-linecap="round" />
      </svg>
    </button>
  </div>
</template>
