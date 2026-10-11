'use strict'

/**
 * dsh-desktop-bridge 注入（Electron 壳）。
 *
 * 与 `tauri-app/src-tauri/src/main.rs` 的 `write_bridge_patch` 语义一致：
 * dsh launcher 的 `--patch <file>` 是「额外 patch overlay」，patch 文件在
 * profile 层之后应用，支持 insert 新插件行，且行 `name` 允许**绝对路径 / file URL**
 * （见 `@deepseek-ai/dsh-app-boot` 的模块解析）。
 *
 * 因此壳可以：
 * - 把 patch 文件生成到应用数据目录（`app.getPath('userData')`）；
 * - 让 patch 的 name 指向**随壳分发**的 bridge 包 host 半入口（`lib/index.js` 文件 URL）；
 * 从而完全**不写 `~/.dsh`、不改 `tauri-app/resources/runtime/`**（规格 §8）。
 *
 * 本模块只做路径解析与 patch 文本构造（纯函数，可测），落盘由调用方完成。
 */

const path = require('node:path')
const { pathToFileURL } = require('node:url')

/** bridge 包在安装产物/开发仓库中的相对路径（host 半入口）。 */
const BRIDGE_ENTRY_RELATIVE = path.join('bridge', 'lib', 'index.js')
const BRIDGE_ENTRY_RELATIVE_DEV = path.join('desktop-bridge', 'lib', 'index.js')

/**
 * 解析 bridge host 半入口的候选路径（按优先级）。
 * @param {object} paths - { resourcesPath, appPath, repoRoot }
 * @returns {string[]} 绝对路径候选
 */
function bridgeEntryCandidates({ resourcesPath, appPath, repoRoot }) {
  const candidates = []
  // 打包：extraResources 落在 resourcesPath/bridge（macOS: Contents/Resources）
  if (resourcesPath) {
    candidates.push(path.join(resourcesPath, BRIDGE_ENTRY_RELATIVE))
    candidates.push(path.join(resourcesPath, BRIDGE_ENTRY_RELATIVE_DEV))
  }
  // 打包（asar 内）：appPath/../desktop-bridge
  if (appPath) {
    candidates.push(path.join(appPath, '..', BRIDGE_ENTRY_RELATIVE_DEV))
    candidates.push(path.join(appPath, BRIDGE_ENTRY_RELATIVE_DEV))
  }
  // 开发：仓库根 desktop-bridge
  if (repoRoot) {
    candidates.push(path.join(repoRoot, BRIDGE_ENTRY_RELATIVE_DEV))
  }
  return candidates
}

/**
 * 选中第一个存在的候选路径。
 * @param {string[]} candidates
 * @param {(p: string) => boolean} exists
 * @returns {string|null}
 */
function pickBridgeEntry(candidates, exists) {
  for (const candidate of candidates) {
    if (exists(candidate)) return candidate
  }
  return null
}

/**
 * 构造 bridge 注入 patch 的文本。
 *
 * `name` 必须是 **file URL**（dsh 会把裸绝对路径规范化为 file URL；显式给
 * file URL 更稳，且避免 Windows 反斜杠在 YAML 中的转义歧义）。
 * @param {string} entryPath - bridge host 半入口绝对路径
 * @returns {string} YAML patch 文本
 */
function buildPatchContent(entryPath) {
  const fileUrl = pathToFileURL(entryPath).href
  return `- insert:\n  - id: dsh-desktop-bridge\n    name: ${fileUrl}\n`
}

/**
 * 生成 patch 文件路径（应用数据目录下，固定名，可重复覆盖）。
 * @param {string} userDataPath
 * @returns {string}
 */
function bridgePatchPath(userDataPath) {
  return path.join(userDataPath, 'dsh-bridge.patch.yml')
}

module.exports = {
  BRIDGE_ENTRY_RELATIVE,
  BRIDGE_ENTRY_RELATIVE_DEV,
  bridgeEntryCandidates,
  pickBridgeEntry,
  buildPatchContent,
  bridgePatchPath,
}