// build-runtime.mjs — 组装 dsh 桌面端运行时到 tauri-app/resources/runtime/
// 产物结构：
//   resources/runtime/node/          node 二进制（node.exe / node）
//   resources/runtime/dsh-runtime/   pnpm 安装的 @deepseek-ai/dsh + node_modules
//
// 用法：node scripts/build-runtime.mjs
// 环境变量：NODE_VERSION 覆盖默认 node 版本；NODE_MIRROR 覆盖下载源

import { execSync } from 'node:child_process'
import {
  existsSync,
  mkdirSync,
  rmSync,
  writeFileSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  renameSync,
  statSync,
  cpSync,
} from 'node:fs'
import { join, dirname, extname } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = join(__dirname, '..')

// runtime 默认放在仓库内 tauri-app/resources/runtime/（三端一致，gitignore）。
//
// 历史说明：早期用 pnpm 安装时，`.pnpm` 嵌套 + 项目深路径叠加曾突破 makensis
// 的 260 字符限制，runtime 一度被放到 Windows 短路径 `D:\rt` 规避。
// 现改用 npm 扁平布局后，实测最长路径约 235 字符（< 260），已可安全放回仓库内。
//
// 逃生舱：若未来依赖树再次变深，可设 DSH_RUNTIME_DIR 指到短路径（如 D:\rt），
// 或参考 docs/stage/stage-2-packaging.md 的历史踩坑记录。
const RUNTIME_DIR =
  process.env.DSH_RUNTIME_DIR
  || join(ROOT, 'tauri-app', 'resources', 'runtime')
const NODE_DIR = join(RUNTIME_DIR, 'node')
const DSH_DIR = join(RUNTIME_DIR, 'dsh-runtime')

const NODE_VERSION = process.env.NODE_VERSION || 'v24.14.0'
const NODE_BASE_URL = process.env.NODE_MIRROR || 'https://nodejs.org/dist'
// dsh 运行时版本：固定完整版本号，与 GitHub Release tag 对齐（不带 v 前缀）。
// npm 的 latest/next 与 alpha 可能不同步，因此必须显式写死版本，
// 不能依赖 dist-tag 解析。升级时同步修改：
//   1. 本文件 DSH_VERSION
//   2. tauri-app/src-tauri/tauri.conf.json 的 version
//   3. tauri-app/src-tauri/Cargo.toml 的 version
// 或用 scripts/build-win.ps1 -DshVersion <version> 一键同步。
const DSH_VERSION = process.env.DSH_VERSION || '0.1.5-rc.2'

// glob 兜底排除的依赖 / 构建产物目录（见 patchGlobTool 说明）
const BULK_DIRS = [
  'node_modules',
  'target',
  'dist',
  'build',
  'out',
  '.next',
  'vendor',
  'coverage',
  '.pnpm-store',
  '.turbo',
  '.cache',
  'bin',
  'obj',
  '.venv',
  'venv',
  '__pycache__',
]

// 平台 -> node 发行版命名映射
// binPath: node 二进制在发行版压缩包内的相对路径
// stripComponents: tar --strip-components 的层数（提取单文件时去掉前 N 层）
const DIST_MAP = {
  'win32-x64': { name: `node-${NODE_VERSION}-win-x64`, ext: 'zip', bin: 'node.exe', binPath: 'node.exe', strip: 1 },
  'win32-arm64': { name: `node-${NODE_VERSION}-win-arm64`, ext: 'zip', bin: 'node.exe', binPath: 'node.exe', strip: 1 },
  'darwin-x64': { name: `node-${NODE_VERSION}-darwin-x64`, ext: 'tar.gz', bin: 'node', binPath: 'bin/node', strip: 2 },
  'darwin-arm64': { name: `node-${NODE_VERSION}-darwin-arm64`, ext: 'tar.gz', bin: 'node', binPath: 'bin/node', strip: 2 },
  'linux-x64': { name: `node-${NODE_VERSION}-linux-x64`, ext: 'tar.xz', bin: 'node', binPath: 'bin/node', strip: 2 },
  'linux-arm64': { name: `node-${NODE_VERSION}-linux-arm64`, ext: 'tar.xz', bin: 'node', binPath: 'bin/node', strip: 2 },
}

function distInfo() {
  const key = `${process.platform}-${process.arch}`
  const info = DIST_MAP[key]
  if (!info) throw new Error(`[build-runtime] unsupported platform: ${key}`)
  return info
}

function dirSize(dir) {
  let total = 0
  const stack = [dir]
  while (stack.length > 0) {
    const current = stack.pop()
    if (!existsSync(current)) continue
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const full = join(current, entry.name)
      if (entry.isDirectory()) {
        stack.push(full)
        continue
      }
      // Dirent 没有 size 字段，必须 stat；pnpm 目录里大量符号链接，
      // 个别不可访问的路径直接跳过而不是让总数变成 NaN。
      try {
        total += statSync(full).size
      } catch {
        /* unreadable entry, ignore */
      }
    }
  }
  return total
}

async function download(url, dest) {
  console.log(`[build-runtime] downloading ${url}`)
  const res = await fetch(url, { redirect: 'follow' })
  if (!res.ok) throw new Error(`[build-runtime] download failed: HTTP ${res.status} ${url}`)
  const buf = Buffer.from(await res.arrayBuffer())
  writeFileSync(dest, buf)
  console.log(`[build-runtime] downloaded ${(buf.length / 1024 / 1024).toFixed(1)} MB`)
}

async function downloadNode() {
  const info = distInfo()
  const nodeBin = join(NODE_DIR, info.bin)

  // 增量复用：node 二进制已存在则跳过
  if (existsSync(nodeBin)) {
    console.log(`[build-runtime] node binary exists, skip: ${nodeBin}`)
    return
  }

  mkdirSync(RUNTIME_DIR, { recursive: true })
  mkdirSync(NODE_DIR, { recursive: true })
  const archivePath = join(RUNTIME_DIR, `${info.name}.${info.ext}`)

  await download(`${NODE_BASE_URL}/${NODE_VERSION}/${info.name}.${info.ext}`, archivePath)

  // 只提取 node 二进制单文件（--strip-components 去掉前 N 层），
  // 避免解压整个发行版（含 npm 等无用文件），也避免后续批量删除
  const srcInArchive = `${info.name}/${info.binPath}`
  const cmd = `tar -xf "${archivePath}" -C "${NODE_DIR}" --strip-components ${info.strip} "${srcInArchive}"`
  console.log(`[build-runtime] extracting node binary: ${cmd}`)
  execSync(cmd, { stdio: 'inherit' })

  // 删除压缩包（单文件，非批量）
  rmSync(archivePath, { force: true })
  console.log(`[build-runtime] node ready: ${nodeBin}`)
}

/** 读取指定 runtime 目录里已安装的 dsh 版本；未安装返回 null。 */
function installedDshVersion(dir = DSH_DIR) {
  const pkg = join(dir, 'node_modules', '@deepseek-ai', 'dsh', 'package.json')
  if (!existsSync(pkg)) return null
  try {
    const version = JSON.parse(readFileSync(pkg, 'utf8')).version
    return typeof version === 'string' ? version : null
  } catch {
    return null
  }
}

/**
 * 统计 `@deepseek-ai/dsh*` 家族包的版本分布。
 *
 * 家族包之间按精确版本互相依赖，混入旧版本会在插件加载阶段报
 * “does not provide an export named ...”，必须当成构建失败。
 * 只统计 dsh 前缀：native 包（如 `@deepseek-ai/node-addon-landlock-run`）
 * 走自己的版本线，不能拿来比对。
 */
function dshFamilyVersions(dir = DSH_DIR) {
  // npm 扁平布局：全部家族包物化在顶层 node_modules 下，直接扫即可。
  const scopes = [join(dir, 'node_modules', '@deepseek-ai')]

  const histogram = {}
  const seen = new Set()
  for (const scope of scopes) {
    if (!existsSync(scope)) continue
    for (const entry of readdirSync(scope)) {
      const pkg = join(scope, entry, 'package.json')
      if (!existsSync(pkg)) continue
      try {
        const manifest = JSON.parse(readFileSync(pkg, 'utf8'))
        if (
          typeof manifest.name === 'string'
          && manifest.name.startsWith('@deepseek-ai/dsh')
          && typeof manifest.version === 'string'
        ) {
          const key = `${manifest.name}@${manifest.version}`
          if (seen.has(key)) continue
          seen.add(key)
          histogram[manifest.version] = (histogram[manifest.version] ?? 0) + 1
        }
      } catch {
        /* 忽略无法解析的包 */
      }
    }
  }
  return histogram
}

/**
 * 把 node_modules 里发现的全部 dsh 家族包名钉死到目标版本（npm overrides）。
 *
 * 家族包的依赖声明是 `^0.1.2-alpha.2` 这类范围，npm 每次都解析到满足范围的最新版
 * （实测混入了 alpha.3），必须用 overrides 强制对齐，否则版本混合会让插件加载失败。
 */
function pinFamilyOverrides(dir) {
  const scope = join(dir, 'node_modules', '@deepseek-ai')
  if (!existsSync(scope)) return
  const overrides = {}
  for (const entry of readdirSync(scope)) {
    const pkg = join(scope, entry, 'package.json')
    if (!existsSync(pkg)) continue
    try {
      const manifest = JSON.parse(readFileSync(pkg, 'utf8'))
      // 主包由 package.json 的直接依赖精确控制，放进 overrides 会报
      // EOVERRIDE（npm 不允许 override 与直接依赖冲突），只钉其余家族成员。
      if (
        typeof manifest.name === 'string'
        && manifest.name.startsWith('@deepseek-ai/dsh')
        && manifest.name !== '@deepseek-ai/dsh'
      ) {
        overrides[manifest.name] = DSH_VERSION
      }
    } catch {
      /* 忽略无法解析的包 */
    }
  }
  const count = Object.keys(overrides).length
  if (count === 0) return
  const pkgPath = join(dir, 'package.json')
  const pkg = JSON.parse(readFileSync(pkgPath, 'utf8'))
  pkg.overrides = overrides
  writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + '\n')
  console.log(`[build-runtime] pinned ${count} family packages to ${DSH_VERSION}`)
}

/** 写入一个 runtime 目录需要的基础配置。 */
function writeRuntimeConfig(dir) {
  mkdirSync(dir, { recursive: true })
  const pkgJson = { name: 'dsh-runtime', version: '0.0.0', private: true }
  writeFileSync(join(dir, 'package.json'), JSON.stringify(pkgJson, null, 2) + '\n')
}

function installDsh() {
  const binJs = join(DSH_DIR, 'node_modules', '@deepseek-ai', 'dsh', 'lib', 'bin.js')
  const current = installedDshVersion()
  // 家族树里的残留旧版本同样要重装：只升级主包会让插件加载阶段炸掉
  const stale = Object.keys(dshFamilyVersions()).filter((version) => version !== DSH_VERSION)

  // 增量复用：主包版本一致、产物存在、且家族树无残留时才跳过
  if (current === DSH_VERSION && existsSync(binJs) && stale.length === 0) {
    console.log(`[build-runtime] dsh ${DSH_VERSION} already installed, skip`)
    return
  }

  // 升级或修混合版本：在空目录里重新解析整棵依赖树。
  // 实测在既有 node_modules 上 pnpm add（连删掉 lockfile 重装也一样）会保留上百个
  // 旧版本家族包，启动即报 “@deepseek-ai/dsh-llm does not provide an export named ...”。
  //
  // 顺序很关键：必须先把旧目录让位，再在「最终路径」上安装。Windows 上 pnpm 用
  // 绝对路径的 junction 链接包，若装到 staging 再改名，所有链接都会指向旧路径而断裂
  // （实测 `@deepseek-ai/dsh` 变成空目录）。
  let backup
  if (existsSync(DSH_DIR)) {
    // 保留为回滚点，不自动删除：runtime 目录里文件数以万计，批量删除会被拦。
    // 备份必须放在 runtime 目录之外，否则会被整个打进安装包。
    const backupRoot = join(dirname(RUNTIME_DIR), '.dsh-runtime-backup')
    mkdirSync(backupRoot, { recursive: true })
    backup = join(backupRoot, `dsh-runtime.${new Date().toISOString().replace(/[:.]/g, '-')}`)
    renameSync(DSH_DIR, backup)
    console.log(`[build-runtime] previous runtime kept at ${backup}`)
  }

  try {
    writeRuntimeConfig(DSH_DIR)
    // 用 npm 而不是 pnpm：pnpm 的链接布局（.pnpm + junction）与桌面分发三方冲突——
    // 1) Tauri 打包不跟随 junction，顶层 scope 目录会被打空（装出来 bin.js 不存在）；
    // 2) runtime 目录不能搬移（junction 是绝对路径）；
    // 3) 手工物化又会破坏 Node 的模块解析（pnpm 靠路径规范化到 .pnpm 找兄弟依赖）。
    // npm 的扁平布局把所有包物化成真实目录，打包、搬移、运行都不再有链接问题。
    // node-pty/koffi 等原生模块的安装脚本由 npm 直接执行（pnpm 的 allowBuilds 白名单不再需要）。
    const npmInstall = () =>
      execSync(
        `npm install @deepseek-ai/dsh@${DSH_VERSION} --omit=dev --no-audit --no-fund --loglevel=warn`,
        {
          cwd: DSH_DIR,
          stdio: 'inherit',
          env: { ...process.env, NODE_OPTIONS: '' },
        },
      )
    console.log(`[build-runtime] installing @deepseek-ai/dsh@${DSH_VERSION} via npm (flat layout)`)
    npmInstall()

    // 家族依赖是 ^x.y.z-alpha.n 范围声明，npm 会解析到满足范围的最新版（实测混入
    // alpha.3），所以先装一遍收集完整家族包名，再用 overrides 全部钉死重装。
    let mixed = Object.keys(dshFamilyVersions()).filter((version) => version !== DSH_VERSION)
    if (mixed.length > 0) {
      console.log(`[build-runtime] npm resolved newer family versions: ${mixed.join(', ')}`)
      pinFamilyOverrides(DSH_DIR)
      npmInstall()
    }

    // 主包版本对了不代表整棵家族树对了，混合版本会炸在插件加载阶段
    const installed = installedDshVersion()
    if (installed !== DSH_VERSION) {
      throw new Error(
        `[build-runtime] expected dsh ${DSH_VERSION}, got ${installed ?? 'not installed'}`,
      )
    }
    mixed = Object.keys(dshFamilyVersions()).filter((version) => version !== DSH_VERSION)
    if (mixed.length > 0) {
      throw new Error(`[build-runtime] mixed dsh versions: ${mixed.join(', ')}`)
    }
  } catch (error) {
    // 安装失败就回滚，避免留下一个装了一半的 runtime
    if (backup !== undefined) {
      renameSync(DSH_DIR, `${DSH_DIR}.failed-${Date.now()}`)
      renameSync(backup, DSH_DIR)
      console.log(`[build-runtime] rolled back to the previous runtime`)
    }
    throw error
  }

  console.log(`[build-runtime] dsh ${DSH_VERSION} installed`)
}

function verify() {
  const info = distInfo()
  const nodeBin = join(NODE_DIR, info.bin)
  const binJs = join(DSH_DIR, 'node_modules', '@deepseek-ai', 'dsh', 'lib', 'bin.js')

  if (!existsSync(nodeBin)) throw new Error(`[build-runtime] node binary missing: ${nodeBin}`)
  if (!existsSync(binJs)) throw new Error(`[build-runtime] dsh bin.js missing: ${binJs}`)

  // 主包版本对了不代表整棵家族树对了：混入旧版本会在启动时炸在插件加载阶段
  const histogram = dshFamilyVersions()
  console.log(`[build-runtime] dsh family versions: ${JSON.stringify(histogram)}`)
  const stale = Object.keys(histogram).filter((version) => version !== DSH_VERSION)
  if (stale.length > 0) {
    throw new Error(
      `[build-runtime] mixed dsh versions: expected all ${DSH_VERSION}, also found ${stale.join(', ')}`,
    )
  }

  const nodeVersion = execSync(`"${nodeBin}" --version`, { encoding: 'utf8' }).trim()
  const dshVersion = execSync(`"${nodeBin}" "${binJs}" --version`, { encoding: 'utf8' }).trim()
  const sizeMB = (dirSize(RUNTIME_DIR) / 1024 / 1024).toFixed(1)

  console.log(`[build-runtime] node: ${nodeVersion}`)
  console.log(`[build-runtime] dsh: ${dshVersion}`)
  console.log(`[build-runtime] runtime size: ${sizeMB} MB`)
}

/**
 * 把 node_modules 顶层（含 scope 内一层）的 junction 物化成真实目录。
 *
 * Tauri 遍历 resources 时不跟随符号链接：顶层包若保持 junction，
 * 打出来的安装包里只有空的 scope 目录，运行时 `bin.js not found`（实测）。
 * pnpm 无法配置不用链接（node-linker=hoisted 在 pnpm 11 已移除，
 * package-import-method 只控制 store 导入方式），所以在安装后统一物化。
 *
 * 只处理顶层两层：node_modules/<pkg>、node_modules/<scope>/<pkg>。
 * .pnpm 内部的兄弟链接保持原样——运行时的模块解析会落到已物化的顶层。
 * 删除用 rmSync（对 junction 等价于 rmdir，只删链接本身不动目标内容）。
 */
function materializeTopLevelLinks() {
  if (process.platform !== 'win32') return
  const nm = join(DSH_DIR, 'node_modules')
  let count = 0
  for (const entry of readdirSync(nm, { withFileTypes: true })) {
    const level1 = join(nm, entry.name)
    if (entry.isSymbolicLink()) {
      materializeOne(level1)
      count++
      continue
    }
    if (!entry.isDirectory()) continue
    for (const sub of readdirSync(level1, { withFileTypes: true })) {
      const level2 = join(level1, sub.name)
      if (sub.isSymbolicLink()) {
        materializeOne(level2)
        count++
      }
    }
  }
  if (count > 0) console.log(`[build-runtime] materialized ${count} top-level package links`)
}

function materializeOne(linkPath) {
  let target = readlinkSync(linkPath)
  // node 对 junction 返回 \\?\ 前缀的绝对路径，去掉以便后续操作
  if (target.startsWith('\\\\?\\')) target = target.slice(4)
  rmSync(linkPath, { force: true })
  cpSync(target, linkPath, { recursive: true })
}

/**
 * 修正 dsh 的 glob 工具，使其与 Claude Code / Codex 的文件发现行为一致。
 *
 * dsh 原实现在 `buildGlobCommand()` 里硬编码了 `--no-ignore --hidden`：
 * `--no-ignore` 让 ripgrep 无视 .gitignore，强制爬进 node_modules。实测在一个
 * 含 73 万 node_modules 文件的工作区里，`rg --files` 需要 118 秒；再叠加
 * `--sort=modified`（必须先遍历完整棵树才输出第一个字节），30 秒的
 * `SEARCH_TIMEOUT_MS` 内 stdout 恒为 0B，最终报
 * "tool call timed out after 30000ms" —— 表面现象就是"工具调用一直转圈后卡死"。
 *
 * Claude Code / Codex 的 Glob 尊重 .gitignore，因此天然跳过 node_modules：
 * 同样命令 453ms 返回。这里对齐该行为（去掉 --no-ignore），并额外补上依赖 /
 * 构建目录的兜底排除 —— 这些目录正是 Claude Code 在有 .gitignore 时也会跳过的，
 * 因此对使用体验没有差别，但保证项目缺少 .gitignore 时也不会退化。
 *
 * 实测（同一工作区）：
 *   原实现（--no-ignore --hidden --sort）  -> 45s+ 未完成，stdout 0B
 *   仅去掉 --no-ignore                     -> 16.6s
 *   去掉 --no-ignore + 兜底排除（本补丁）  -> 1.8s
 *
 * 幂等：已打补丁（含 GLOB_BULK_EXCLUDES 且无 --no-ignore）则跳过。
 */
function patchGlobTool(dir = DSH_DIR) {
  const file = join(dir, 'node_modules', '@deepseek-ai', 'dsh-tool-fs-search', 'lib', 'index.js')
  if (!existsSync(file)) {
    console.log(`[build-runtime] dsh-tool-fs-search not found, skip glob patch: ${file}`)
    return
  }
  let src = readFileSync(file, 'utf8')

  if (src.includes('GLOB_BULK_EXCLUDES') && !src.includes('"--no-ignore"')) {
    console.log('[build-runtime] glob tool already patched, skip')
    return
  }

  // 1) 去掉 --no-ignore：尊重 .gitignore，与 Claude Code / Codex 一致
  src = src.replace('\t\t"--no-ignore",\n\t\t"--hidden",', '\t\t"--hidden",')

  // 2) 插入 GLOB_BULK_EXCLUDES 常量（紧跟 GLOB_VCS_EXCLUDES 定义之后）
  if (!src.includes('GLOB_BULK_EXCLUDES')) {
    const anchor = 'const GLOB_VCS_EXCLUDES = ['
    const anchorAt = src.indexOf(anchor)
    if (anchorAt === -1) throw new Error('[build-runtime] GLOB_VCS_EXCLUDES not found in glob tool')
    const endAt = src.indexOf('];', anchorAt)
    if (endAt === -1) throw new Error('[build-runtime] GLOB_VCS_EXCLUDES end not found')
    const insertAt = endAt + 2
    const lines = BULK_DIRS.map((name) => `\t"${name}"`).join(',\n')
    const block =
      '\n\n/**\n* 依赖与构建产物目录：即使项目没有 .gitignore 也跳过。\n'
      + '* Claude Code / Codex 的 Glob 尊重 .gitignore 时同样不会搜这些目录。\n*/\n'
      + `const GLOB_BULK_EXCLUDES = [\n${lines}\n];`
    src = src.slice(0, insertAt) + block + src.slice(insertAt)
  }

  // 3) buildGlobCommand 里附加兜底排除
  const vcsSpread = '...GLOB_VCS_EXCLUDES.flatMap((name) => [`--glob=!**/${name}`, `--glob=!**/${name}/**`])'
  if (src.includes(vcsSpread) && !src.includes('...GLOB_BULK_EXCLUDES.flatMap')) {
    const bulkSpread =
      '...GLOB_BULK_EXCLUDES.flatMap((name) => [`--glob=!**/${name}`, `--glob=!**/${name}/**`])'
    src = src.replace(vcsSpread, `${vcsSpread},\n\t\t${bulkSpread}`)
  }

  // 校验：必须同时满足"已去掉 --no-ignore"且"已加入兜底排除"，否则不落盘
  if (src.includes('"--no-ignore"') || !src.includes('GLOB_BULK_EXCLUDES')) {
    throw new Error('[build-runtime] glob tool patch incomplete, refusing to write')
  }
  writeFileSync(file, src)
  console.log('[build-runtime] patched glob tool: dropped --no-ignore, added bulk excludes')
}

// ---------------------------------------------------------------------------
// 发布裁剪（prune）：删除分发时用不到的开发/调试/文档文件。**默认关闭，需显式开启。**
//
// 历史：最初为解决 macOS 安装包过大而引入——当时 dmg 用 hdiutil 默认的 UDZO(zlib)
// 弱压缩，几乎压不动这些文件，dmg 反而比源目录更大（实测 504MB）。但真正的解法是
// ULMO(LZMA) 强压缩（build-mac.sh 已接入），裁剪并非必需。
//
// 实测对比（dsh 0.1.5-rc.1 + ULMO，macOS x64，2026-09-18，见 CHANGELOG）：
//   不裁剪：runtime 331.5MB → dmg 73.2MB（安装后 415MB）
//   裁剪后：runtime 213.8MB → dmg 56.3MB（安装后 254MB）
// 即裁剪只省 16.9MB 下载 / 161MB 磁盘，而删掉的 ~118MB 里有约一半是第三方包内的
// .ts/.map——属于“运行期理论上用不到、但真被加载就极难排查”的一类，收益与风险不成
// 比例，因此默认改为**完整保留 runtime**。
//
// 若将来确需瘦身，开启后的收益优先级：.pdb（跨平台发布包里夹带的 Windows 调试符号，
// 在 mac/linux 上纯废）→ .map → docs/test 目录 → 最后才动 .ts。
//
// 开启方式：DSH_RUNTIME_PRUNE=1 node scripts/build-runtime.mjs
//
// 安全性（开启时）：
//   - dsh 的 npm 发布产物是编译后的 JS（lib/*.js），运行时不加载 .ts/.map；
//   - .map/.pdb 只服务调试器与崩溃堆栈还原，不影响功能；
//   - 文档/示例/测试目录不会被 require/import。
// ---------------------------------------------------------------------------
const PRUNE_DELETE_EXTS = [
  '.map',      // source map（调试用）
  '.md',       // 文档
  '.ts',       // TypeScript 源码 / 类型声明（运行时只跑编译后的 .js）
  '.mts',      // TS ESM 源码
  '.cts',      // TS CJS 源码
  '.tsbuildinfo', // TS 增量编译缓存
  '.pdb',      // Windows 调试符号
  '.cc',       // C++ 源码
  '.h',        // C/C++ 头文件
  '.hh',       // C++ 头文件
]
const PRUNE_DELETE_DIRS = [
  'docs',
  'examples',
  'example',
  'test',
  'tests',
  '__tests__',
  'spec',
  'benchmark',
  'benchmarks',
  'fixtures',
  '.github',
]
// .ts 保留名单：若未来某个包出现"运行期才被加载的 .ts"（如 tsx 直跑源码的包），
// 把包名加到这里即可精确豁免，而不是全局放弃裁剪。
const PRUNE_KEEP_TS = []

function pruneDevArtifacts() {
  // 默认不裁剪：完整保留 runtime（理由与实测数据见上方注释块）
  if (process.env.DSH_RUNTIME_PRUNE !== '1') {
    console.log(
      '[build-runtime] prune skipped (default: keep runtime complete); set DSH_RUNTIME_PRUNE=1 to slim it',
    )
    return
  }
  const roots = [DSH_DIR, NODE_DIR].filter((d) => existsSync(d))
  let removed = 0
  let savedBytes = 0
  let keptTs = 0

  const walk = (dir, scopeRoot) => {
    let entries
    try {
      entries = readdirSync(dir, { withFileTypes: true })
    } catch {
      return
    }
    for (const entry of entries) {
      const full = join(dir, entry.name)
      if (entry.isSymbolicLink()) continue // 不穿越链接（junction 目标是另存的原文件）
      if (entry.isDirectory()) {
        if (PRUNE_DELETE_DIRS.includes(entry.name)) {
          const s = dirSize(full)
          rmSync(full, { recursive: true, force: true })
          removed++
          savedBytes += s
          continue
        }
        walk(full, scopeRoot)
        continue
      }
      if (!entry.isFile()) continue
      const ext = extname(entry.name).toLowerCase()
      if (PRUNE_DELETE_EXTS.includes(ext)) {
        if (ext === '.ts') {
          const rel = full.slice(scopeRoot.length + 1).replace(/\\/g, '/')
          const kept = PRUNE_KEEP_TS.some((p) => rel.startsWith(p))
          if (kept) {
            keptTs++
            continue
          }
        }
        try {
          const s = statSync(full).size
          rmSync(full, { force: true })
          removed++
          savedBytes += s
        } catch {
          /* 个别不可访问文件跳过 */
        }
      }
    }
  }

  for (const root of roots) walk(root, root)
  console.log(
    `[build-runtime] pruned ${removed} dev/doc files (-${(savedBytes / 1024 / 1024).toFixed(1)} MB)` +
      (keptTs > 0 ? `, kept ${keptTs} .ts by allowlist` : ''),
  )
}

async function main() {
  // 清除 IDE/宿主注入的 NODE_OPTIONS（其 --require shim 会干扰 dsh 启动，实测导致 boot 卡死）
  delete process.env.NODE_OPTIONS

  console.log(`[build-runtime] platform: ${process.platform}-${process.arch}, node ${NODE_VERSION}`)
  await downloadNode()
  installDsh()
  materializeTopLevelLinks()
  // 修正 glob 工具（去掉 --no-ignore 等）：必须在 npm install 之后，
  // 否则重装依赖会把补丁覆盖回官方实现。
  patchGlobTool()
  // 可选裁剪（默认关闭，DSH_RUNTIME_PRUNE=1 开启）：必须在 patch 之后
  // （patch 要读 lib/index.js，被删了会炸），且在 verify 之前（verify 直接
  // 反映最终产物体积）。
  pruneDevArtifacts()
  verify()
  console.log('[build-runtime] done')
}

main().catch((err) => {
  console.error(err.message || err)
  process.exit(1)
})
