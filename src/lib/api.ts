/**
 * api.ts - 后端统一响应入口
 *
 * 后端所有 Tauri 命令的返回 JSON 统一为包裹结构（见 src-tauri/src/response.rs）：
 *
 * ```json
 * { "code": 0, "message": "ok", "data": <业务数据> }
 * { "code": 1, "message": "错误信息", "data": null }
 * ```
 *
 * 本模块把包裹解开后交给调用方：
 * - 成功（code=0）：返回 `data`，与直接用 `invoke` 拿业务数据的旧写法完全一致
 * - 失败（code≠0 或后端 Err）：抛出 `ApiError`，其 `toString()` 即后端错误信息，
 *   因此现有 `catch` 里的 `String(e)`、`msg.includes('...')` 等写法无需改动
 *
 * 用法：把 `import { invoke } from '@tauri-apps/api/core'` 换成本文件即可，签名一致。
 */

import { invoke as tauriInvoke } from '@tauri-apps/api/core'

/** 后端统一响应包裹结构（与 Rust 侧 ApiResponse 字段一一对应） */
export interface ApiResponse<T> {
  /** 响应码：0 成功，非 0 失败 */
  code: number
  /** 人类可读信息（成功为 "ok"，失败为具体原因） */
  message: string
  /** 业务数据；失败时为 null */
  data: T | null
}

/** 后端业务错误（携带响应码；toString 直接给出错误信息） */
export class ApiError extends Error {
  /** 后端响应码（0 成功；非 0 失败） */
  code: number

  constructor(message: string, code = 1) {
    super(message)
    this.name = 'ApiError'
    this.code = code
  }

  /** 让 `String(e)` 直接得到后端错误信息，兼容旧 catch 写法 */
  toString(): string {
    return this.message
  }
}

/** 把任意 rejection 归一成 ApiError（后端 Err 侧也是同一包裹结构） */
function toApiError(e: unknown): ApiError {
  if (e instanceof ApiError) return e
  if (e && typeof e === 'object' && 'message' in e) {
    const obj = e as { message: unknown; code?: unknown }
    return new ApiError(String(obj.message), typeof obj.code === 'number' ? obj.code : 1)
  }
  return new ApiError(String(e))
}

/**
 * 调用后端命令并解开统一响应包裹
 *
 * @param cmd - 命令名（与 #[tauri::command] 函数名一致）
 * @param args - 命令参数
 * @returns 成功时返回 `data`；失败抛出 `ApiError`
 */
export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  let res: ApiResponse<T>
  try {
    res = await tauriInvoke<ApiResponse<T>>(cmd, args)
  } catch (e) {
    // 后端 Err 侧的包裹结构以 rejection 形式抛出，这里归一成 ApiError
    throw toApiError(e)
  }

  if (res && typeof res === 'object' && 'code' in res) {
    if (res.code === 0) return (res.data ?? undefined) as T
    throw new ApiError(res.message, res.code)
  }
  // 兜底：后端理论上一定返回包裹结构；万一不是，原样透传避免功能中断
  return res as unknown as T
}
