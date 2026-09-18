'use strict'

/**
 * 日志脱敏与日志落盘。
 *
 * 移植 tauri-app/src-tauri/src/main.rs 的 `redact_url` / `log`，
 * 保证两套壳（Tauri / Electron）的日志语义与位置完全一致（规格 §7.4）。
 */

const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')

/** 找出文本中出现的 http(s) URL。 */
const URL_RE = /https?:\/\/[^\s]+/g

/**
 * 单个 URL：仅当 query 里存在 `token` 时把它的值替换为 `***`，
 * 其余参数与原顺序保持不变；解析失败或没有 token 时原样返回。
 */
function redactSingleUrl(url) {
  let parsed
  try {
    parsed = new URL(url)
  } catch {
    return url
  }
  const entries = [...parsed.searchParams.entries()]
  if (!entries.some(([key]) => key === 'token')) return url
  const params = new URLSearchParams()
  for (const [key, value] of entries) {
    params.append(key, key === 'token' ? '***' : value)
  }
  parsed.search = params.toString()
  return parsed.toString()
}

/** 把一段文本里出现的所有 URL 逐个脱敏（日志行通常不是纯 URL）。 */
function redactUrl(text) {
  if (typeof text !== 'string') return text
  return text.replace(URL_RE, (match) => redactSingleUrl(match))
}

/**
 * 日志文件路径：Windows 用 `%LOCALAPPDATA%\dsh-desktop\`，其他平台用 `~/.dsh/`，
 * 文件名 `dsh-desktop.log`（与 Tauri 版一致）。
 */
function logPath() {
  let dir
  if (process.platform === 'win32') {
    if (!process.env.LOCALAPPDATA) return null
    dir = path.join(process.env.LOCALAPPDATA, 'dsh-desktop')
  } else {
    const home = process.env.HOME || os.homedir()
    if (!home) return null
    dir = path.join(home, '.dsh')
  }
  try {
    fs.mkdirSync(dir, { recursive: true })
  } catch {
    return null
  }
  return path.join(dir, 'dsh-desktop.log')
}

let cachedDev = null

/** dev（未打包）时同时打 console；打包后只写日志文件。 */
function isDev() {
  if (cachedDev === null) {
    try {
      const { app } = require('electron')
      cachedDev = !app || !app.isPackaged
    } catch {
      // 纯 node 环境（如单测/语法检查）视作 dev
      cachedDev = true
    }
  }
  return cachedDev
}

/** 原样写一行（不做脱敏）。 */
function log(msg) {
  const line = String(msg)
  if (isDev()) console.log(line)
  const file = logPath()
  if (!file) return
  try {
    fs.appendFileSync(file, `${line}\n`)
  } catch {
    /* 日志写失败不影响主流程 */
  }
}

/** 脱敏后写一行。 */
function logLine(line) {
  log(redactUrl(line))
}

/**
 * 按行切分后逐行脱敏写入。
 *
 * 注意：本函数不跨 chunk 缓冲半行，stdout/stderr 的完整行切分由
 * main.js 的 `createLineReader` 负责（Rust 侧等价于 BufReader::lines）。
 */
function logLines(chunk) {
  for (const line of String(chunk).split(/\r?\n/)) {
    if (line.length > 0) logLine(line)
  }
}

module.exports = { redactUrl, redactSingleUrl, log, logLine, logLines, logPath }
