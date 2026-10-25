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
 *   两个变体各只传【一个】补丁（补丁 = 该变体与 base 的全部差异）：
 *     webview2 -> tauri.conf.no-frontend-build.json（只跳过前端构建）
 *     lite     -> tauri.conf.no-webview2.json（跳过前端构建 + 去掉内嵌 WebView2 运行时）
 *   不叠加多个 --config：tauri-cli 组合多补丁的 merge_patches 是自定义实现，
 *   「null 是否删键」的语义与 RFC 7396 有差异，单补丁可以完全按已验证的语义走。
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
 *   例：MoyuPasswd_0.4.0_x64-setup-webview2.exe / MoyuPasswd_0.4.0_x64-setup-lite.exe
 *   目录 + 文件名双重区分：即使两个安装包被拷到同一目录（或发给别人）也不会互相覆盖/混淆。
 *   （构建中间产物在 src-tauri/target/release/bundle/ 下两套同名、后构建覆盖先构建；
 *     脚本按「文件修改时间晚于本次构建开始时刻」只收集本次新产出的安装包，复制改名收进上面的目录，
 *     所以旧包 / 另一变体的包不会混进来。注意全程不依赖「先删掉旧文件」——
 *     Windows 上删除被杀软占用的文件会报 EPERM，清理动作失败不该把整个构建搞挂。）
 *
 * 【一次跑通的四个关键设计】
 *   1) 前端只构建一次：脚本开头跑一次 `pnpm build`（vue-tsc + vite），之后所有 tauri 调用
 *      都用补丁把 build.beforeBuildCommand 置 null（删键）——tauri 每次调用本来都会重跑一遍
 *      前端构建，一次 release:all 是 2 编译 + 4 打包 + 重试，前端要白构建 6~10 遍。
 *   2) 失败按错误码分流（排障铁律：拒绝访问(5) ≠ 被占用(32)）：
 *      - EBUSY / os error 32 = 有进程正独占文件（杀软实时扫描典型）→ 退避等一会儿再试（5s/10s/20s）；
 *      - EPERM / os error 5 = ACL 或完整性标签挡路（Everyone:(DENY)(DC)、High Mandatory Level 之类）
 *        → 等多久都没用：诊断挡路文件（icacls）、打印修复命令、走抢救通道，不空转重试。
 *   3) 双抢救通道：打包成功但成品没搬进 bundle/ 时（tauri 的 rename 撞锁/撞毒文件），
 *      完整成品往往已经躺在中间目录，直接复制到 release/<变体>/ 最终文件名（新建文件不受
 *      旧文件的锁/ACL 影响）：
 *        MSI  <- wix/x64/output.msi（light 写完、rename 前）
 *        NSIS <- nsis/x64/nsis-output.exe（makensis 写完、rename 前）
 *   4) 开场毒文件扫场：bundle/ 下被锁死的旧安装包会让 tauri「覆盖同名旧文件」必失败，
 *      提前删；删不掉就打印 icacls 诊断与修复命令（管理员执行），并靠抢救通道兜底。
 *
 * 【杀软与权限】
 *   MSI/NSIS 打包都要读 target/release/app.exe，刚链接出来的 exe 会被 360 之类实时扫描短暂独占
 *   （tauri-cli 只报 "failed to run light.exe"，真实报错被吞）。这是概率性问题，等扫完放锁重试即可。
 *   根治：把 src-tauri/target/ 加入杀毒软件的信任/排除目录。
 *   另外打包请固定用同一种权限跑（建议都用普通权限，不要管理员/普通混跑）：
 *   管理员权限产出的文件带 High Mandatory Level 标签，普通权限覆盖不了，混跑会互相卡死。
 */

import { spawn, execSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

// ---- 路径常量：全部相对本脚本定位，不依赖调用时的 CWD ----
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const targetDir = path.join(root, 'src-tauri', 'target', 'release')
const bundleDir = path.join(targetDir, 'bundle')
const outRoot = path.join(root, 'release')
const appExePath = path.join(targetDir, 'app.exe')

/** 要构建的变体清单：id -> 配置补丁参数 + 打包目标（目标拆开逐个跑，原因见文件头） */
const variants = {
  webview2: { configArgs: '--config src-tauri/tauri.conf.no-frontend-build.json', targets: ['nsis', 'msi'] },
  lite: { configArgs: '--config src-tauri/tauri.conf.no-webview2.json', targets: ['nsis', 'msi'] },
}

const buildCmd = (variant, target) => `pnpm tauri build ${variants[variant].configArgs} --bundles ${target}`.trim()
const compileCmd = (variant) => `pnpm tauri build ${variants[variant].configArgs} --no-bundle`.trim()

// ---- 基础工具：sleep / 命令执行 ----
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

// 执行 shell 命令：输出实时转发到控制台，同时留底到 log——
// 失败分流要看真实报错文本（tauri 会把 os error 32 / os error 5 / 拒绝访问 带出来，只是默认被层层吞掉）
const runCapture = (cmd) =>
  new Promise((resolve) => {
    const child = spawn(cmd, { cwd: root, shell: true, stdio: ['ignore', 'pipe', 'pipe'] })
    let log = ''
    child.stdout.on('data', (d) => {
      process.stdout.write(d)
      log += d
    })
    child.stderr.on('data', (d) => {
      process.stderr.write(d)
      log += d
    })
    child.on('error', (e) => resolve({ ok: false, log: `${log}\n${e.message}` }))
    child.on('close', (code) => resolve({ ok: code === 0, log }))
  })

// 同步执行小命令（copy/del/icacls 这类）。360 行为防护会按【进程】拦 node 改"安装包"，
// 而 cmd 的 copy/del 这类命令行工具实测放行，所以复制/删除失败都降级到 shell 再来一遍
const shExec = (cmdline) => execSync(cmdline, { cwd: root, stdio: 'pipe', shell: true })

// 版本/名称统一从配置读（改版本号不用动本脚本）
const readConf = () => JSON.parse(fs.readFileSync(path.join(root, 'src-tauri', 'tauri.conf.json'), 'utf8'))

// ---- 错误性质分流：拒绝访问(5) ≠ 被占用(32)，处理方式完全相反 ----
// EBUSY/os error 32 = 有进程正开着文件（杀软扫描典型）→ 等一会儿重试就能过；
// EPERM/EACCES/os error 5 = ACL 或完整性标签挡路 → 重试无用，必须处理文件本身。
const probeFile = (file) => {
  if (!fs.existsSync(file)) return 'ok' // 不存在=不挡路
  try {
    const fd = fs.openSync(file, 'r+')
    fs.closeSync(fd)
    return 'ok'
  } catch (e) {
    return e.code === 'EBUSY' ? 'busy' : 'denied'
  }
}

// 先看报错文本（快、准），文本看不出再实测嫌疑文件
const classifyError = (log, probes = []) => {
  if (/os error 32|正由另一进程使用|being used by another process|EBUSY/i.test(log)) return 'busy'
  if (/os error 5|拒绝访问|access is denied|EPERM/i.test(log)) return 'denied'
  for (const f of probes) {
    const s = probeFile(f)
    if (s !== 'ok') return s
  }
  return 'other'
}

const clsLabel = (cls) =>
  cls === 'denied'
    ? '拒绝访问(5)：ACL/完整性标签挡路，重试无用'
    : cls === 'busy'
      ? '被占用(32)：多为杀软瞬时扫描'
      : '其他'

// ---- 毒文件诊断：先查文件本身（icacls），再给修复命令 ----
const diagnosed = new Set()
const diagnosePoison = (file) => {
  if (diagnosed.has(file)) return
  diagnosed.add(file)
  console.warn(`\n[build-release] 诊断挡路文件 ${path.relative(root, file)}：`)
  try {
    console.warn(shExec(`icacls "${file}"`).toString().trimEnd())
  } catch {
    /* icacls 跑不起来就跳过，诊断是尽力而为 */
  }
  if (probeFile(file) === 'busy') {
    console.warn('[build-release]   判定：被占用（EBUSY/32）——多为杀软正在扫描，等几秒即可')
    return
  }
  console.warn('[build-release]   判定：拒绝访问（EPERM/5）——ACL 或完整性标签在挡路')
  console.warn('[build-release]   （常见 High Mandatory Level / Everyone:(DENY)(DC)；重试无用，只能修文件本身）')
  console.warn('[build-release]   修复命令（管理员 PowerShell 逐行执行）：')
  console.warn(`      takeown /f "${file}" /a`)
  console.warn(`      icacls "${file}" /reset`)
  console.warn(`      Remove-Item "${file}"`)
}

// ---- 通用工具：带重试执行 / 尽力删除 ----
// Windows 上文件常被杀软瞬时占用（删除报 EPERM、复制报 EBUSY、light.exe 报 LGHT0001），
// 歇 1 秒重试基本都能过；清理类动作失败只告警，绝不中断构建。
const withRetry = async (fn, label, times = 3) => {
  for (let i = 1; ; i++) {
    try {
      return await fn()
    } catch (e) {
      if (i >= times) throw e
      console.log(`[build-release] ${label} 失败（${e.code ?? e.message}），1 秒后重试 ${i}/${times - 1}…`)
      await sleep(1000)
    }
  }
}

const robustCopy = (src, dest) => {
  try {
    fs.copyFileSync(src, dest)
  } catch {
    shExec(`copy /y "${src}" "${dest}"`)
  }
}

const robustDelete = (file) => {
  try {
    fs.unlinkSync(file)
    return true
  } catch {
    try {
      shExec(`del /f /q "${file}"`)
      return true
    } catch {
      return false
    }
  }
}

// 尽力删除：删不掉（被杀软占用等）只告警不中断——清理旧包不该把构建搞挂
const tryDelete = (file) => {
  if (!robustDelete(file)) {
    console.warn(`[build-release] 警告：旧安装包 ${path.basename(file)} 删不掉（常见于杀软占用），已跳过，不影响本次产物`)
  }
}

const listInstallers = (dir) => {
  if (!fs.existsSync(dir)) return []
  return fs
    .readdirSync(dir)
    .filter((e) => /\.(exe|msi)$/i.test(e))
    .map((e) => path.join(dir, e))
}

/**
 * 清一个目录里的旧安装包。目的：tauri 把产物 rename 进 bundle/ 时是【覆盖同名旧文件】，
 * 旧文件若被锁/被 ACL 卡住就报拒绝访问(os error 5)；提前清掉，覆盖变纯新建，撞锁概率大降。
 * 删不掉的先诊断（拒绝访问 = 毒文件，打印 icacls 与修复命令）再删；
 * 返回 true = 该目录已无挡路文件。清理绝不中断构建。
 */
const sweepInstallers = (dir) => {
  if (!fs.existsSync(dir)) return true
  let clear = true
  for (const entry of fs.readdirSync(dir)) {
    if (!/\.(exe|msi)$/i.test(entry)) continue
    const f = path.join(dir, entry)
    if (probeFile(f) === 'denied') {
      diagnosePoison(f)
      if (!robustDelete(f)) {
        clear = false
        continue
      }
      console.log(`[build-release] 挡路文件已清除：${entry}`)
      continue
    }
    tryDelete(f)
  }
  return clear
}

const preCleanTargets = (target) => sweepInstallers(path.join(bundleDir, target))

// ---- 抢救通道：打包成功、成品没搬进 bundle/ 时的后悔药 ----
// tauri 把中间成品 rename 进 bundle/ 的一步撞锁/撞毒文件时（light 的 MoveFile、NSIS 的 fs::rename
// 都【不做任何重试】，dotnet/runtime#3618 同款问题），完整成品往往已经躺在中间目录。
// 【为什么直接写到 release/<变体>/ 而不是 bundle/】bundle/ 里的同名旧文件可能被
// 「禁止删除」ACL 或 High 完整性标签锁死：删不掉、也覆盖不了。而【新建】文件不受那条规则影响，
// 所以直接把成品写到 release/ 的最终文件名，绕开毒文件，一步到位。
// 文件名与 collectInstallers 的改名规则一致（原名插 -{变体} 后缀）。

// MSI：light 先写完 output.msi、再写 output.wixpdb，最后才搬走；
//   两个都在且 pdb 不比 msi 旧 = 压包已完成。再验 OLE 魔数（D0 CF 11 E0）+ 本次新写 兜底。
const salvageWixOutput = async (variant, buildStart) => {
  const wixDir = path.join(targetDir, 'wix', 'x64')
  const temp = path.join(wixDir, 'output.msi')
  const pdb = path.join(wixDir, 'output.wixpdb')
  if (!fs.existsSync(temp)) return null
  if (fs.statSync(temp).mtimeMs < buildStart - 5000) return null // 旧产物，不是本次的
  if (!fs.existsSync(pdb) || fs.statSync(pdb).mtimeMs < fs.statSync(temp).mtimeMs) return null // 还在压包中
  try {
    const fd = fs.openSync(temp, 'r')
    const head = Buffer.alloc(4)
    fs.readSync(fd, head, 0, 4, 0)
    fs.closeSync(fd)
    if (!head.equals(Buffer.from([0xd0, 0xcf, 0x11, 0xe0]))) return null // 不是完整 MSI
    const conf = readConf()
    const lang = conf.bundle?.windows?.wix?.language ?? 'en-US'
    const outDir = path.join(outRoot, variant)
    fs.mkdirSync(outDir, { recursive: true })
    const dest = path.join(outDir, `${conf.productName}_${conf.version}_x64_${lang}-${variant}.msi`)
    await withRetry(() => robustCopy(temp, dest), '抢救 output.msi', 8)
    console.log(`[build-release] 抢救成功：MSI 已直接就位 ${path.relative(root, dest)}（绕开被锁的 bundle/msi）`)
    return dest
  } catch (e) {
    console.warn(`[build-release] 抢救 output.msi 失败：${e.code ?? e.message}`)
    return null
  }
}

// NSIS：makensis 写完 nsis/x64/nsis-output.exe 后 tauri 用 fs::rename 搬进 bundle/nsis/，同款场景。
// 【完整性判据】makensis 自己失败（tauri bail "Failed to bundle app with makensis"）时产物可能
//   不完整，不抢救；rename 阶段失败 = makensis 已 0 退出 = 产物完整。再加三道实测：
//   本次构建新写、>1MB、头 1MB 有 NullsoftInst 魔数（NSIS 包头在文件开头附近，实测偏移 ~54KB；
//   注意它在头部，截断写入也可能带着它，所以不能只靠魔数判完整性）。
const salvageNsisOutput = async (variant, buildStart, log) => {
  if (/failed to bundle app with makensis/i.test(log)) return null
  const staging = path.join(targetDir, 'nsis', 'x64', 'nsis-output.exe')
  if (!fs.existsSync(staging)) return null
  const st = fs.statSync(staging)
  if (st.mtimeMs < buildStart - 5000) return null // 旧产物
  if (st.size < 1024 * 1024) return null // 太小，多半没写完
  try {
    const fd = fs.openSync(staging, 'r')
    const head = Buffer.alloc(Math.min(1024 * 1024, st.size))
    fs.readSync(fd, head, 0, head.length, 0)
    fs.closeSync(fd)
    if (!head.includes('NullsoftInst')) return null // 不是有效的 NSIS 包
    const conf = readConf()
    const outDir = path.join(outRoot, variant)
    fs.mkdirSync(outDir, { recursive: true })
    const dest = path.join(outDir, `${conf.productName}_${conf.version}_x64-setup-${variant}.exe`)
    await withRetry(() => robustCopy(staging, dest), '抢救 nsis-output.exe', 8)
    console.log(`[build-release] 抢救成功：NSIS 安装包已直接就位 ${path.relative(root, dest)}（绕开被锁的 bundle/nsis）`)
    return dest
  } catch (e) {
    console.warn(`[build-release] 抢救 nsis-output.exe 失败：${e.code ?? e.message}`)
    return null
  }
}

const salvage = (variant, target, buildStart, log) =>
  target === 'msi' ? salvageWixOutput(variant, buildStart) : salvageNsisOutput(variant, buildStart, log)

// ---- 收集产物：把本次构建新产出的安装包（exe/msi）复制到 release/<变体>/ 并加变体后缀 ----
// 只收「修改时间晚于本次构建开始」的文件（buildStart 放宽 5 秒容错）+ 抢救出来的成品（salvaged）：
// bundle 里上一变体/上一次的旧包不动它、也不收它，从根上避免「删旧文件被杀软锁住报 EPERM」搞挂构建。
async function collectInstallers(variant, buildStart, salvaged = []) {
  const outDir = path.join(outRoot, variant)
  fs.mkdirSync(outDir, { recursive: true })

  const collected = []
  // 递归找安装包：NSIS 在 bundle/nsis/*.exe，WiX 在 bundle/msi/*.msi
  const walk = async (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name)
      if (entry.isDirectory()) {
        await walk(full)
        continue
      }
      if (!/\.(exe|msi)$/i.test(entry.name)) continue
      // 时间戳早于本次构建开始（-5 秒容错）的都是旧产物，跳过
      if (fs.statSync(full).mtimeMs < buildStart - 5000) continue
      // 文件名插入变体后缀（-webview2 / -lite），保证两套包同目录也不重名
      const destName = entry.name.replace(/\.(exe|msi)$/i, `-${variant}.$1`)
      const dest = path.join(outDir, destName)
      await withRetry(() => robustCopy(full, dest), `复制 ${entry.name}`)
      collected.push(dest)
    }
  }
  await walk(bundleDir)
  collected.push(...salvaged) // 抢救直出 release/ 的成品也算本次产物

  if (collected.length === 0) {
    console.error(`[build-release] ${variant}: 在 ${bundleDir} 没找到本次构建产出的 exe/msi 安装包，构建可能失败了`)
    process.exit(1)
  }

  // 清掉 release/<变体>/ 里本次之外的旧安装包，让目录里只留本次产物；
  // 只删 exe/msi 文件本身，且删不掉（被杀软占用）只告警跳过、绝不中断构建
  const freshNames = new Set(collected.map((f) => path.basename(f)))
  for (const entry of fs.readdirSync(outDir)) {
    if (/\.(exe|msi)$/i.test(entry) && !freshNames.has(entry)) tryDelete(path.join(outDir, entry))
  }

  for (const f of collected) {
    const mb = (fs.statSync(f).size / 1024 / 1024).toFixed(1)
    console.log(`[build-release] ${variant}: ${path.relative(root, f)} (${mb} MB)`)
  }
}

// ---- 构建命令执行（编译用）：失败按错误码分流重试，退避 5s/10s/20s ----
const runWithRetry = async (label, cmd, probes = [appExePath]) => {
  const backoffs = [5000, 10000, 20000]
  for (let attempt = 1; ; attempt++) {
    const res = await runCapture(cmd)
    if (res.ok) return true
    if (attempt > backoffs.length) break
    console.log(
      `\n[build-release] ${label}: 第 ${attempt} 次失败（${clsLabel(classifyError(res.log, probes))}），${backoffs[attempt - 1] / 1000} 秒后重试…`
    )
    await sleep(backoffs[attempt - 1])
  }
  console.error(`[build-release] ${label}: 多次尝试均失败`)
  return false
}

// ---- 打包单个目标（nsis 或 msi）：各自清场、失败先抢救、再按错误码决定重试还是收工 ----
const bundleTarget = async (variant, target, buildStart) => {
  const backoffs = [5000, 10000]
  const salvaged = []
  for (let attempt = 1; ; attempt++) {
    preCleanTargets(target) // 删同名旧包让 rename 变纯新建；删不掉会在这里诊断并打印修复命令
    const res = await runCapture(buildCmd(variant, target))
    if (res.ok) return { ok: true, salvaged }

    // 先抢救：成品可能已经完整，只是没搬进 bundle/
    const saved = await salvage(variant, target, buildStart, res.log)
    if (saved) {
      salvaged.push(saved)
      return { ok: true, salvaged }
    }

    const cls = classifyError(res.log, [appExePath, ...listInstallers(path.join(bundleDir, target))])
    console.log(`\n[build-release] ${variant}-${target}: 第 ${attempt} 次失败（${clsLabel(cls)}）`)
    if (attempt >= 3) break

    if (cls === 'denied') {
      // 拒绝访问靠等待解决不了：处理挡路文件，删干净才继续；删不掉直接收工并给修复命令
      if (!preCleanTargets(target)) {
        console.error(
          `[build-release] ${variant}-${target}: 挡路文件删不掉，不再空转重试；按上面打印的修复命令处理后重跑即可补上。`
        )
        return { ok: false, salvaged }
      }
      continue
    }
    await sleep(backoffs[Math.min(attempt - 1, backoffs.length - 1)])
  }
  return { ok: false, salvaged }
}

// ---- 主流程 ----
const arg = (process.argv[2] ?? 'all').toLowerCase()
const todo = arg === 'all' ? ['webview2', 'lite'] : [arg]
if (!todo.every((v) => v in variants)) {
  console.error(`[build-release] 未知参数 "${arg}"，可选：all | webview2 | lite`)
  process.exit(1)
}

// 0) 前端只构建一次：后面的 tauri 调用全靠补丁跳过 beforeBuildCommand，不再重复跑 vue-tsc + vite
console.log('[build-release] ===== 前端构建（vue-tsc + vite，只跑这一次） =====')
if (!(await runWithRetry('前端构建', 'pnpm build'))) {
  console.error('[build-release] 前端构建失败，终止。')
  process.exit(1)
}
if (!fs.existsSync(path.join(root, 'dist', 'index.html'))) {
  console.error('[build-release] dist/index.html 不存在，前端构建产物异常，终止。')
  process.exit(1)
}

// 1) 开场扫场：bundle/ 下的毒文件（锁死的旧安装包）会让 rename 覆盖同名旧包必失败，提前清
for (const target of ['nsis', 'msi']) preCleanTargets(target)

const missing = [] // 记录没打成的「变体-目标」，最后统一报错退出
for (const variant of todo) {
  console.log(`\n[build-release] ===== 构建 ${variant} 版 =====`)
  const buildStart = Date.now() // 记下构建起点：收集/抢救只认这个时刻之后写出来的安装包

  // 2) 编译一次（--no-bundle 不打包），编译期失败照样按错误码重试
  if (!(await runWithRetry(`${variant} 编译`, compileCmd(variant)))) {
    missing.push(`${variant}-(编译)`)
    continue
  }
  // 3) 歇 3 秒再打包：让杀软把刚编译出的 app.exe 扫完放锁（dotnet/runtime#3618 同款解法）
  await sleep(3000)

  // 4) nsis / msi 分目标打包：各自清场、失败先抢救、再按错误码分流重试
  const salvaged = []
  for (const target of variants[variant].targets) {
    const r = await bundleTarget(variant, target, buildStart)
    if (!r.ok) missing.push(`${variant}-${target}`)
    salvaged.push(...r.salvaged)
  }

  await collectInstallers(variant, buildStart, salvaged)
}

if (missing.length > 0) {
  console.error(`\n[build-release] 以下包没打成：${missing.join('、')}`)
  console.error('[build-release] 已打成的包都在 release/ 下。若是「拒绝访问(5)」，按上面打印的修复命令处理挡路文件后重跑；')
  console.error('[build-release] 若是「被占用(32)」，把杀毒软件「文件实时防护」的信任目录配上 src-tauri/target/ 后重跑即可补上。')
  process.exit(1)
}

console.log('\n[build-release] 全部完成，产物目录：')
console.log(`  ${path.relative(root, outRoot)}/webview2  自带 WebView2 运行时（体积大，任何机器可装）`)
console.log(`  ${path.relative(root, outRoot)}/lite      不自带（体积小，装机时联网补装 WebView2）`)
