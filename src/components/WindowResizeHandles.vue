<!--
  WindowResizeHandles.vue - 无边框窗口的边缘缩放热区

  主窗口 decorations:false 在 Windows 上仍可从边缘缩放（WS_THICKFRAME 命中测试），
  但 Linux/X11 的窗口管理器只给有边框窗口提供缩放边缘，无边框窗口必须自己
  铺热区、按下时调 startResizeDragging 让窗口管理器进入交互缩放（走 EWMH
  _NET_WM_MOVERESIZE，xfwm4 等主流 WM 均支持）。

  仅主窗口使用（App.vue 挂载）：快捷搜索 / 倒计时窗口 resizable:false，不需要。
  容器 pointer-events-none 不挡内容，只有边缘/角落的窄条吃指针事件。
-->

<script setup lang="ts">
import { getCurrentWindow } from '@tauri-apps/api/window'

/** 缩放方向（@tauri-apps/api 的 ResizeDirection 未导出，本地等价定义） */
type ResizeDirection =
  | 'East'
  | 'West'
  | 'North'
  | 'South'
  | 'NorthEast'
  | 'NorthWest'
  | 'SouthEast'
  | 'SouthWest'

/** 按下热区 → 让窗口管理器接管交互缩放 */
function startResize(direction: ResizeDirection) {
  // .prevent 已拦默认行为（避免拖选文字/抢焦点），这里不 await：
  // WM 接管后本调用会挂到缩放结束，成功与否都不需要结果
  void getCurrentWindow().startResizeDragging(direction)
}
</script>

<template>
  <!-- 固定铺满窗口的透明层；角落条压在边条之上，斜向缩放优先命中 -->
  <div class="pointer-events-none fixed inset-0">
    <!-- 四边（4px 窄条，不侵占内容区点击） -->
    <div
      class="pointer-events-auto absolute top-0 right-0 left-0 h-1 cursor-n-resize"
      @mousedown.prevent="startResize('North')"
    />
    <div
      class="pointer-events-auto absolute right-0 bottom-0 left-0 h-1 cursor-s-resize"
      @mousedown.prevent="startResize('South')"
    />
    <div
      class="pointer-events-auto absolute top-0 bottom-0 left-0 w-1 cursor-w-resize"
      @mousedown.prevent="startResize('West')"
    />
    <div
      class="pointer-events-auto absolute top-0 right-0 bottom-0 w-1 cursor-e-resize"
      @mousedown.prevent="startResize('East')"
    />

    <!-- 四角（8px 方块，斜向缩放的手感区） -->
    <div
      class="pointer-events-auto absolute top-0 left-0 h-2 w-2 cursor-nw-resize"
      @mousedown.prevent="startResize('NorthWest')"
    />
    <div
      class="pointer-events-auto absolute top-0 right-0 h-2 w-2 cursor-ne-resize"
      @mousedown.prevent="startResize('NorthEast')"
    />
    <div
      class="pointer-events-auto absolute bottom-0 left-0 h-2 w-2 cursor-sw-resize"
      @mousedown.prevent="startResize('SouthWest')"
    />
    <div
      class="pointer-events-auto absolute right-0 bottom-0 h-2 w-2 cursor-se-resize"
      @mousedown.prevent="startResize('SouthEast')"
    />
  </div>
</template>
