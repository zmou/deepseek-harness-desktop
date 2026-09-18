'use strict'

/**
 * 运行时路径解析（规格 §7.2）。
 *
 * 复用 scripts/build-runtime.mjs 的产物 `tauri-app/resources/runtime/`
 * （内嵌 node + @deepseek-ai/dsh），与 Tauri 版共享同一份运行时、同一套启动协议。
 * 打包后该目录位于 `<app>/Contents/Resources/runtime/`（extraResources）。
 */

const fs = require('node:fs')
const path = require('node:path')

const { log } = require('./redact')

const NODE_BIN_NAME = process.platform === 'win32' ? 'node.exe' : 'node'
const DSH_BIN_RELATIVE = path.join(
  'runtime',
  'dsh-runtime',
  'node_modules',
  '@deepseek-ai',
  'dsh',
  'lib',
  'bin.js',
)

/** 打包后即 `<app>/Contents/Resources`；dev 下指向 Electron 自带的 resources（不含 runtime）。 */
function resourcesDir() {
  return typeof process.resourcesPath === 'string' ? process.resourcesPath : null
}

/** dev 回退用的 .stage-p0 dsh（Tauri 版同款开发产物的 bin.js 相对路径）。 */
const STAGE_DSH_RELATIVE = path.join(
  '.stage-p0',
  'node_modules',
  '@deepseek-ai',
  'dsh',
  'lib',
  'bin.js',
)

/**
 * node 二进制：内嵌产物优先，dev 回退系统 PATH 的 node。
 * @returns {string} node 可执行文件路径或命令名
 */
function resolveNode() {
  const dir = resourcesDir()
  if (dir) {
    const embedded = path.join(dir, 'runtime', 'node', NODE_BIN_NAME)
    if (fs.existsSync(embedded)) return embedded
  }
  return NODE_BIN_NAME
}

/**
 * dsh 的 bin.js：`DSH_BIN` > 内嵌产物 > dev 回退。
 * @returns {string} bin.js 的绝对路径
 */
function resolveDshBin() {
  // 1. DSH_BIN 环境变量（最高优先级，排障/调试用）
  const fromEnv = process.env.DSH_BIN
  if (fromEnv) {
    if (fs.existsSync(fromEnv)) return fromEnv
    throw new Error(`DSH_BIN points to a missing file: ${fromEnv}`)
  }

  const candidates = []

  // 2. 内嵌产物
  const dir = resourcesDir()
  if (dir) candidates.push(path.join(dir, DSH_BIN_RELATIVE))

  // 3. dev 回退：仓库根（electron-app/src → ../..）
  const repoRoot = path.join(__dirname, '..', '..')
  candidates.push(path.join(repoRoot, 'tauri-app', 'resources', DSH_BIN_RELATIVE))
  candidates.push(path.join(repoRoot, STAGE_DSH_RELATIVE))
  candidates.push(path.join(process.cwd(), STAGE_DSH_RELATIVE))
  candidates.push(path.join(process.cwd(), '..', STAGE_DSH_RELATIVE))

  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) return candidate
  }
  throw new Error('dsh bin.js not found; set DSH_BIN or run scripts/build-runtime.mjs')
}

module.exports = { resolveNode, resolveDshBin, log }
