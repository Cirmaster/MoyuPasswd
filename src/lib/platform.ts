/**
 * platform.ts - 平台判断
 *
 * 前端平台门控（显示/隐藏平台专属设置项等）统一从这里取值，
 * 避免各组件散写 navigator.userAgent（见 docs/linux-平台开发.md 跨平台开发约定）。
 * 以后需要更细的判断（架构、系统版本）可在此扩展，或改接 Rust 侧 `std::env::consts::OS` 命令。
 */

const ua = navigator.userAgent

/** 是否 Windows 平台 */
export const isWindows = ua.includes('Windows')

/** 是否 macOS 平台 */
export const isMac = ua.includes('Macintosh') || ua.includes('Mac OS')

/** 是否 Linux 平台（排除 Android） */
export const isLinux = ua.includes('Linux') && !ua.includes('Android')
