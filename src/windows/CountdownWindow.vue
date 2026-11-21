<!--
  CountdownWindow.vue - 全局倒计时悬浮窗口

  在鼠标位置显示的倒计时，用于剪贴板清除。
  点击数字取消倒计时并关闭窗口。

  两种情况下气泡会从圆点扩成提示条：
  - 后端命中终端注入黑名单（terminal-inject-confirm）：告知用户「再按 Ctrl+V 确认」；
  - 后端注入失败（inject-result）：这是注入链路的唯一可见反馈——注入失败时按键被吞、
    屏幕上什么都不会发生。提示条会说明失败，并在可能时提供「点此放入剪贴板」入口。
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { disableContextMenu } from '@/lib/windowSetup'

/** 注入结果事件载荷（与 Rust 的 inject::InjectResult 一一对应） */
interface InjectResult {
  /** 是否已把密码送进目标窗口 */
  ok: boolean
  /** 本次会话代数（「放入剪贴板」时回传，后端据此拒绝过期授权） */
  seq: number
  /** 是否还能改为剪贴板交付（只是提供入口，写入仍需用户点击） */
  can_fallback: boolean
}

/** 提示条状态 */
interface HintState {
  /** 显示文案 */
  text: string
  /** 有值 = 可点击，点击后把该会话的待粘贴密码放入剪贴板 */
  consentSeq?: number
}

/** 剩余秒数 */
const remaining = ref(10)

/** 当前提示条（null = 显示倒计时圆点） */
const hint = ref<HintState | null>(null)

/** 计时器 */
let timer: ReturnType<typeof setInterval> | null = null

/** 提示条自动收起计时器 */
let hintTimer: ReturnType<typeof setTimeout> | null = null

// 圆点/提示条设计尺寸常量统一在 src-tauri/src/window_sizes.rs 管理
//（COUNTDOWN_DOT_* / COUNTDOWN_HINT_*），经 scale.rs 换算成物理尺寸应用；
// 前端不传裸数值

/**
 * 切换气泡形态并调整窗口尺寸（圆点 ⇄ 提示条）
 *
 * 设计尺寸由后端 window_sizes.rs 常量统一管理，按渲染缩放换算成
 * 物理尺寸应用（见 scale.rs 缩放模型）；前端不传裸数值。
 */
const setCountdownExpanded = async (expanded: boolean) => {
  try {
    const { invoke } = await import('@/lib/api')
    await invoke('set_countdown_expanded', { expanded })
  } catch (e) {
    console.warn('调整窗口尺寸失败:', e)
  }
}

/**
 * 显示提示条，5 秒后自动收起
 *
 * 5 秒与后端终端二次确认窗口 TTL 一致；交付结果提示同样走这个节奏，
 * 避免气泡长期占着屏幕（结果本身还有审计日志可查）。
 */
const showHint = async (state: HintState) => {
  hint.value = state
  await setCountdownExpanded(true)

  if (hintTimer) clearTimeout(hintTimer)
  hintTimer = setTimeout(() => {
    void hideHint()
  }, 5000)
}

/**
 * 收起提示条并恢复圆点尺寸
 */
const hideHint = async () => {
  if (hintTimer) {
    clearTimeout(hintTimer)
    hintTimer = null
  }
  if (hint.value) {
    hint.value = null
    await setCountdownExpanded(false)
  }
}

/**
 * 开始倒计时
 *
 * 尺寸复位的 invoke 是异步的：必须先等它落地再展开提示条，
 * 否则「复位成圆点」和「扩成提示条」两条命令会互相覆盖，提示条被裁成 16px。
 */
const startCountdown = async (seconds: number = 10) => {

  // 先停止之前的计时器
  if (timer) {
    clearInterval(timer)
    timer = null
  }

  // 每次显示强制复位为圆点尺寸：治愈任何卡住的窗口尺寸
  // （提示条扩窗后复位失败等场景；圆点内容钉左上角，尺寸对了就贴着光标）
  await hideHint()
  await setCountdownExpanded(false)

  remaining.value = seconds

  timer = setInterval(() => {
    remaining.value--

    if (remaining.value <= 0) {
      stopCountdown()
      // 剪贴板清除由后端在复制时调度（copy_text_to_clipboard → schedule_clear），
      // 不依赖本窗口存活，前端只需关闭窗口
      void closeWindow()
    }
  }, 1000)
}

/**
 * 停止倒计时
 */
const stopCountdown = () => {
  if (timer) {
    clearInterval(timer)
    timer = null
  }
}

/**
 * 清除剪贴板并关闭
 */
const clearClipboard = async () => {
  try {
    // 调用后端原生清空剪贴板（不向历史追加空记录）
    const { invoke } = await import('@/lib/api')
    await invoke('clear_clipboard')
  } catch (e) {
    console.error('清除剪贴板失败:', e)
  }
  closeWindow()
}

/**
 * 关闭窗口
 *
 * 归 0 / 取消后的收窗动作。主路径 win.close() 真正销毁窗口（不留隐藏壳，
 * 下次复制创建全新窗口，杜绝残留状态）；失败时退回后端 hide_countdown 兜底——
 * 它是应用自带命令（Rust 侧收窗 + 停光标跟随），不受窗口插件权限约束，
 * 也不依赖前端动态 import 的窗口 API，保证圆点一定能消失。
 */
const closeWindow = async () => {

  // 先停止光标跟随（不等待结果）
  try {
    const { invoke } = await import('@/lib/api')
    invoke('stop_follow_cursor').catch(err => console.warn('stop_follow_cursor failed:', err))
  } catch (e) {
    console.warn('Failed to invoke stop_follow_cursor:', e)
  }

  // 关闭窗口（必须执行）
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    const win = getCurrentWindow()
    await win.close()
  } catch (e) {
    console.error('Failed to close window:', e)
    // 兜底：走后端命令收窗（不依赖窗口 API 权限与动态 import）
    try {
      const { invoke } = await import('@/lib/api')
      await invoke('hide_countdown')
    } catch (e2) {
      console.error('Failed to hide countdown window:', e2)
    }
  }
}

/**
 * 点击数字取消倒计时并关闭
 */
const handleClick = async () => {
  stopCountdown()
  await clearClipboard()
}

/** 授权请求是否在途（防连点重复写剪贴板） */
let consentInFlight = false

/**
 * 用户点击提示条的「放入剪贴板」：把本次会话的待粘贴密码写进系统剪贴板
 *
 * 一次点击一次授权——后端不自动做这件事（剪贴板任何进程都能读，属于安全降级）。
 * 传 seq 是为了让后端拒绝过期授权：提示条在屏幕上挂了 5 秒，期间用户可能已经复制了别的。
 */
const grantClipboardConsent = async (seq: number) => {
  if (consentInFlight) return
  consentInFlight = true
  try {
    const { invoke } = await import('@/lib/api')
    const { ttl } = await invoke<{ ttl: number; mode: string }>(
      'copy_pending_password_to_clipboard',
      { seq },
    )
    // 倒计时的含义从「待粘贴 TTL」换成「剪贴板还剩多久清空」
    if (ttl > 0) await startCountdown(ttl)
    await showHint({ text: '密码已放入剪贴板，请按 Ctrl+V' })
  } catch (e) {
    await showHint({ text: '未能放入剪贴板，请重新复制' })
    console.warn('改投剪贴板失败:', e)
  } finally {
    consentInFlight = false
  }
}

/**
 * 点击提示条
 *
 * - 有 consentSeq：放入剪贴板（用户授权）
 * - 没有（终端二次确认提示）：只收起来，不要顺带清剪贴板
 */
const handleHintClick = async () => {
  const current = hint.value
  if (!current) return
  if (current.consentSeq === undefined) {
    await hideHint()
    return
  }
  await grantClipboardConsent(current.consentSeq)
}

/**
 * 处理注入结果（唯一能把「按键被吞」变成可见反馈的地方）
 *
 * - 成功：不提示。字符已经上屏，是比任何提示都可靠的反馈，弹条只是噪音；
 * - 失败但还能改投剪贴板：提示条给出入口，等用户点击（不自动放宽）；
 * - 彻底失败：只提示，别让用户以为密码已经过去了。
 */
const handleInjectResult = async (result: InjectResult | null | undefined) => {
  if (!result || result.ok) return

  if (result.can_fallback) {
    await showHint({ text: '粘贴失败，点此放入剪贴板', consentSeq: result.seq })
    return
  }

  await showHint({ text: '粘贴失败，请重新复制密码' })
}

/**
 * 初始化
 */
onMounted(async () => {
  // 禁用浏览器默认右键菜单（入口已拦一道；组件内再拦，保证热替换后也生效）
  disableContextMenu()

  try {
    // 动态导入 Tauri API
    const { invoke } = await import('@/lib/api')
    const { listen } = await import('@tauri-apps/api/event')

    // 从后端获取倒计时秒数并启动
    const seconds = await invoke<number>('get_countdown_seconds')
    void startCountdown(seconds)

    // 监听重置倒计时事件
    await listen<{ seconds: number }>('reset-countdown', (event) => {
      void startCountdown(event.payload.seconds || 10)
    })

    // 监听终端注入二次确认提示（命中黑名单时后端触发）
    await listen<{ process: string }>('terminal-inject-confirm', () => {
      void showHint({ text: '目标是终端，再按 Ctrl+V 确认' })
    })

    // 监听注入结果（失败时后端触发；成功不发，避免每次粘贴都弹一下）
    await listen<InjectResult>('inject-result', (event) => {
      void handleInjectResult(event.payload)
    })

    // 监听取消倒计时事件
    await listen('cancel-countdown', () => {
      stopCountdown()
      void closeWindow()
    })

    // 后端已完成清除（兜底），前端只需关闭窗口
    await listen('clipboard-cleared', () => {
      stopCountdown()
      void closeWindow()
    })

  } catch (e) {
    console.error('Failed to initialize countdown window:', e)
  }
})

onUnmounted(() => {
  stopCountdown()
  if (hintTimer) {
    clearTimeout(hintTimer)
    hintTimer = null
  }
})
</script>

<template>
  <div class="countdown-container" @click="handleClick">
    <!-- 提示条：终端二次确认 / 交付结果。
         可点击的（有 consentSeq）点了把密码放入剪贴板；
         不可点击的只收起——不能顺带清剪贴板，降级后剪贴板里可能就是密码 -->
    <div v-if="hint" class="hint-bar" @click.stop="handleHintClick()">
      {{ hint.text }}
    </div>
    <!-- 默认倒计时圆点 -->
    <div v-else class="countdown-circle">
      <span class="countdown-number">{{ remaining }}</span>
    </div>
  </div>
</template>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body {
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: transparent;
  -webkit-app-region: no-drag;
}
</style>

<style scoped>
.countdown-container {
  width: 100%;
  height: 100%;
  display: flex;
  /* 钉在窗口左上角：光标跟随锚定的是窗口左上角（cursor+15px），
     内容居中会在窗口大于 16×16 时（如提示条复位失败）让圆点飘离光标 */
  align-items: flex-start;
  justify-content: flex-start;
  background: transparent;
  cursor: pointer;
  user-select: none;
}

.countdown-circle {
  /* 尺寸与 src-tauri/src/window_sizes.rs 的 COUNTDOWN_DOT_* 对应，两处需一致 */
  width: 16px;
  height: 16px;
  border-radius: 50%;
  /* 颜色走主题键帽令牌，跟随色系主题与明暗模式（原来是硬编码蓝） */
  background: var(--keycap);
  display: flex;
  align-items: center;
  justify-content: center;
  transition: transform 0.15s ease, filter 0.15s ease;
}

.countdown-circle:hover {
  transform: scale(1.2);
  filter: brightness(1.15);
}

.countdown-number {
  font-size: 10px;
  font-weight: 700;
  color: var(--keycap-foreground);
  font-family: 'SF Mono', 'Fira Code', monospace;
}

.hint-bar {
  /* 高度与 src-tauri/src/window_sizes.rs 的 COUNTDOWN_HINT_HEIGHT 对应，两处需一致；
     宽度按 COUNTDOWN_HINT_WIDTH（200px）排版，文案须控制在 16 个汉字以内 */
  height: 24px;
  padding: 0 10px;
  border-radius: 12px;
  background: rgba(217, 119, 6, 0.95);
  color: #ffffff;
  font-size: 11px;
  font-weight: 600;
  line-height: 24px;
  white-space: nowrap;
  overflow: hidden;
}
</style>
