#!/usr/bin/env node
/**
 * 发布构建脚本：一次产出两套 Windows 安装包
 *
 * 【为什么有两套】
 *   自带 WebView2 版（webview2/）：tauri.conf.json 里 webviewInstallMode 为 fixedRuntime，
 *     内嵌 src-tauri/webview2-runtime/（Fixed Version 156.0.4314.42），
 *     目标机器什么都不用预装、不用联网，安装包约 200~250MB。
 *   不自带版（lite/）：通过 `--config src-tauri/tauri.conf.no-webview2.json` 打配置补丁生成。
 *     补丁里 `"path": null` 的作用是**删除**内嵌运行时目录键——Tauri 的配置合并是
 *     JSON Merge Patch（RFC 7396，tauri-cli 与 tauri-build 两侧都是 json_patch::merge），
 *     null 表示删键而不是"设为空值"；同时把安装模式退回 embedBootstrapper：
 *     安装包只有几 MB，装的时候联网补装 WebView2（目标机已带 WebView2 则直接跳过）。
 *     若想连"装的时候补装"都不要（完全不碰 WebView2），把补丁里的 type 改成 "skip" 即可。
 *
 * 【为什么用 --config 补丁而不是复制一份 tauri.conf.json】
 *   两份完整配置会漂移（改了一处忘另一处）；补丁只声明差异项，其余全部继承主配置。
 *
 * 【用法】（在项目根目录 moyu-passwd-code/ 下执行）
 *   pnpm release:all            两套都打（默认）
 *   pnpm release:webview2       只打自带 WebView2 版
 *   pnpm release:lite           只打不自带版
 *   或直接： node scripts/build-release.mjs [all|webview2|lite]
 *
 * 【产物位置与命名】
 *   release/webview2/  自带 WebView2 运行时版，文件名带 -webview2 后缀
 *   release/lite/      不自带版，文件名带 -lite 后缀
 *   例：MoyuPasswd_0.3.0_x64-setup-webview2.exe / MoyuPasswd_0.3.0_x64-setup-lite.exe
 *   目录 + 文件名双重区分：即使两个安装包被拷到同一目录（或发给别人）也不会互相覆盖/混淆。
 *   （构建中间产物在 src-tauri/target/release/bundle/ 下两套同名、后构建覆盖先构建，
 *     但脚本每次构建完立刻复制改名收进上面的目录，不受影响。）
 */

import { execSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

// ---- 路径常量：全部相对本脚本定位，不依赖调用时的 CWD ----
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const bundleDir = path.join(root, 'src-tauri', 'target', 'release', 'bundle')
const outRoot = path.join(root, 'release')
// 不自带版的配置补丁；路径相对项目根（pnpm 脚本运行时 CWD 就是项目根）
const liteConfig = 'src-tauri/tauri.conf.no-webview2.json'

/** 要构建的变体清单：id -> 打包命令 */
const variants = {
  webview2: 'pnpm tauri build',
  lite: `pnpm tauri build --config ${liteConfig}`,
}

// ---- 收集产物：把 bundle 目录下的安装包（exe/msi）复制到 release/<变体>/ 并加变体后缀 ----
function collectInstallers(variant) {
  const outDir = path.join(outRoot, variant)
  fs.rmSync(outDir, { recursive: true, force: true })
  fs.mkdirSync(outDir, { recursive: true })

  const collected = []
  // 递归找安装包：NSIS 在 bundle/nsis/*.exe，WiX 在 bundle/msi/*.msi
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (/\.(exe|msi)$/i.test(entry.name)) {
        // 文件名插入变体后缀（-webview2 / -lite），保证两套包同目录也不重名
        const destName = entry.name.replace(/\.(exe|msi)$/i, `-${variant}.$1`)
        const dest = path.join(outDir, destName)
        fs.copyFileSync(full, dest)
        collected.push(dest)
      }
    }
  }
  walk(bundleDir)

  if (collected.length === 0) {
    console.error(`[build-release] ${variant}: 在 ${bundleDir} 没找到 exe/msi 安装包，构建可能失败了`)
    process.exit(1)
  }
  for (const f of collected) {
    const mb = (fs.statSync(f).size / 1024 / 1024).toFixed(1)
    console.log(`[build-release] ${variant}: ${path.relative(root, f)} (${mb} MB)`)
  }
}

// ---- 主流程：按参数逐个变体「构建 -> 收集」，后构建的不会覆盖前一个 ----
const arg = (process.argv[2] ?? 'all').toLowerCase()
const todo = arg === 'all' ? ['webview2', 'lite'] : [arg]
if (!todo.every((v) => v in variants)) {
  console.error(`[build-release] 未知参数 "${arg}"，可选：all | webview2 | lite`)
  process.exit(1)
}

for (const variant of todo) {
  console.log(`\n[build-release] ===== 构建 ${variant} 版 =====`)
  execSync(variants[variant], { cwd: root, stdio: 'inherit', shell: true })
  collectInstallers(variant)
}

console.log('\n[build-release] 全部完成，产物目录：')
console.log(`  ${path.relative(root, outRoot)}/webview2  自带 WebView2 运行时（体积大，任何机器可装）`)
console.log(`  ${path.relative(root, outRoot)}/lite      不自带（体积小，装机时联网补装 WebView2）`)
