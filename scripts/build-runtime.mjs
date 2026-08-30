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
  renameSync,
  readdirSync,
  symlinkSync,
} from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = join(__dirname, '..')
const RUNTIME_DIR = join(ROOT, 'tauri-app', 'resources', 'runtime')
const NODE_DIR = join(RUNTIME_DIR, 'node')
const DSH_DIR = join(RUNTIME_DIR, 'dsh-runtime')

const NODE_VERSION = process.env.NODE_VERSION || 'v24.14.0'
const NODE_BASE_URL = process.env.NODE_MIRROR || 'https://nodejs.org/dist'

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
      if (entry.isDirectory()) stack.push(full)
      else total += entry.size
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

function installDsh() {
  const binJs = join(DSH_DIR, 'node_modules', '@deepseek-ai', 'dsh', 'lib', 'bin.js')

  // 增量复用：dsh 已存在则跳过
  if (existsSync(binJs)) {
    console.log(`[build-runtime] dsh exists, skip: ${binJs}`)
    return
  }

  mkdirSync(DSH_DIR, { recursive: true })

  // pnpm 11 用 pnpm-workspace.yaml 的 allowBuilds 白名单
  // （onlyBuiltDependencies 已废弃，package.json 的 pnpm 字段不再被读取）
  const workspaceYaml = [
    'packages:',
    "  - '.'",
    'allowBuilds:',
    "  '@deepseek-ai/dsh-subprocess-local': true",
    "  '@google/genai': true",
    '  koffi: true',
    '  node-pty: true',
    '  protobufjs: true',
    '',
  ].join('\n')
  writeFileSync(join(DSH_DIR, 'pnpm-workspace.yaml'), workspaceYaml)

  // 基础 package.json（pnpm 字段已不被 pnpm 11 读取，配置放 workspace.yaml）
  const pkgJson = { name: 'dsh-runtime', version: '0.0.0', private: true }
  writeFileSync(join(DSH_DIR, 'package.json'), JSON.stringify(pkgJson, null, 2) + '\n')

  console.log('[build-runtime] installing @deepseek-ai/dsh via pnpm (allowBuilds set)')
  execSync('pnpm add @deepseek-ai/dsh --reporter=append-only', {
    cwd: DSH_DIR,
    stdio: 'inherit',
  })
}

function verify() {
  const info = distInfo()
  const nodeBin = join(NODE_DIR, info.bin)
  const binJs = join(DSH_DIR, 'node_modules', '@deepseek-ai', 'dsh', 'lib', 'bin.js')

  if (!existsSync(nodeBin)) throw new Error(`[build-runtime] node binary missing: ${nodeBin}`)
  if (!existsSync(binJs)) throw new Error(`[build-runtime] dsh bin.js missing: ${binJs}`)

  const nodeVersion = execSync(`"${nodeBin}" --version`, { encoding: 'utf8' }).trim()
  const dshVersion = execSync(`"${nodeBin}" "${binJs}" --version`, { encoding: 'utf8' }).trim()
  const sizeMB = (dirSize(RUNTIME_DIR) / 1024 / 1024).toFixed(1)

  console.log(`[build-runtime] node: ${nodeVersion}`)
  console.log(`[build-runtime] dsh: ${dshVersion}`)
  console.log(`[build-runtime] runtime size: ${sizeMB} MB`)
}

// Windows 上创建 junction D:\drt -> dsh-runtime，规避 makensis 的 260 字符路径限制
function createWindowsJunction() {
  if (process.platform !== 'win32') return
  const junction = 'D:\\drt'
  if (existsSync(junction)) {
    console.log(`[build-runtime] junction exists, skip: ${junction}`)
    return
  }
  try {
    symlinkSync(DSH_DIR, junction, 'junction')
    console.log(`[build-runtime] created junction ${junction} -> dsh-runtime`)
  } catch (e) {
    console.log(`[build-runtime] junction create failed (may already exist): ${e.message}`)
  }
}

async function main() {
  // 清除 IDE/宿主注入的 NODE_OPTIONS（其 --require shim 会干扰 dsh 启动，实测导致 boot 卡死）
  delete process.env.NODE_OPTIONS

  console.log(`[build-runtime] platform: ${process.platform}-${process.arch}, node ${NODE_VERSION}`)
  await downloadNode()
  installDsh()
  verify()
  createWindowsJunction()
  console.log('[build-runtime] done')
}

main().catch((err) => {
  console.error(err.message || err)
  process.exit(1)
})
