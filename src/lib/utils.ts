import type { ClassValue } from "clsx"
import { clsx } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/** 将毫秒时间戳格式化为本地日期字符串 */
export function formatTimestamp(timestampMs: number): string {
  return new Date(timestampMs).toLocaleDateString('zh-CN')
}

/**
 * 计算密码强度等级。
 * 评分规则与后端一致：长度 ≥8/≥12/≥16 各 +1，含小写/大写/数字/特殊字符各 +1，
 * 总分 0–7 映射到 1–4 档。
 * @returns 0(空/未知) | 1(弱) | 2(中) | 3(强) | 4(非常强)
 */
export function calcPasswordStrength(password: string): 0 | 1 | 2 | 3 | 4 {
  if (!password) return 0

  let score = 0
  if (password.length >= 8) score++
  if (password.length >= 12) score++
  if (password.length >= 16) score++
  if (/[a-z]/.test(password)) score++
  if (/[A-Z]/.test(password)) score++
  if (/[0-9]/.test(password)) score++
  if (/[^A-Za-z0-9]/.test(password)) score++

  if (score <= 2) return 1
  if (score <= 4) return 2
  if (score <= 5) return 3
  return 4
}
