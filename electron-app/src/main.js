'use strict'

/**
 * DeepSeek Harness Desktop —— Electron 兼容版主进程（规格 §7）。
 *
 * 与 Tauri 版 `tauri-app/src-tauri/src/main.rs` 职责一一对应：
 * spawn dsh → 泵送管道 → 解析 ready URL → 开窗加载官方 Web UI → 生命周期守护。
 * 两套壳零耦合，仅共享 `tauri-app/resources/runtime/` 这一份只读运行时产物。
 */

const { app, BrowserWindow, dialog, net, session } = require('electron')
const { spawn } = require('node:child_process')
const fs = require('node:fs')
const path = require('node:path')

const redact = require('./redact')
const { resolveDshBin, resolveNode } = require('./runtime')
const hideSessionDialog = require('./hide-session-dialog')

const log = redact.log

/**
 * ready URL 正则。
 *
 * 必须取整条 URL（`http://[^\s]+`）而不是只取到端口：dsh 0.1.2 起 URL 带
 * `?token=` 一次性凭据，只取到端口会让窗口以无凭据地址打开 → 401 白屏。
 */
const READY_URL_RE = /dsh web: (http:\/\/[^\s]+)/
const READY_URL_TIMEOUT_MS = 30 * 1000
/** 「记住上次下载目录」的状态文件名（落在 `app.getPath('userData')`）。 */
const LAST_DOWNLOAD_DIR_FILE = 'last-download-dir.txt'

let child = null
let mainWindow = null
let quitting = false

// 代理抗性（1/2）：必须在 app ready 之前追加开关。
// 本机代理（Clash/mihomo 等）会把 localhost 的 HTTP/WebSocket 也代理出去，
// 导致前端与本地 server 的 WebSocket 通信异常/卡死。
app.commandLine.appendSwitch('no-proxy-server')

if (!app.requestSingleInstanceLock()) {
  // 第二个实例：直接退出，由第一个实例的 'second-instance' 回调恢复窗口
  app.quit()
} else {
  start()
}

function start() {
  app.on('second-instance', () => {
    if (!mainWindow) return
    if (mainWindow.isMinimized()) mainWindow.restore()
    mainWindow.show()
    mainWindow.focus()
  })

  app.on('window-all-closed', () => {
    // macOS 下与 Tauri 版行为一致：关闭窗口即回收子进程并退出
    quitting = true
    killChild()
    app.quit()
  })

  // 兜底：任何退出路径都回收子进程
  app.on('before-quit', () => {
    quitting = true
    killChild()
  })

  // 外部信号（如活动监视器「退出」、`kill <pid>`）也要回收，避免 node 子进程成为孤儿
  for (const signal of ['SIGINT', 'SIGTERM']) {
    process.on(signal, () => {
      log(`[app] received ${signal}, shutting down`)
      quitting = true
      killChild()
      app.quit()
    })
  }

  app.whenReady()
    .then(onReady)
    .catch((error) => {
      log(`[dsh] startup failed: ${messageOf(error)}`)
      quitting = true
      killChild()
      app.quit()
    })
}

async function onReady() {
  // 代理抗性（2/2）：Chromium 网络栈直连。
  // LLM 等外网请求由 dsh 的 node 进程发起（不走 Chromium），故此处直连不影响模型调用。
  try {
    await session.defaultSession.setProxy({ mode: 'direct' })
  } catch (error) {
    log(`[web] setProxy failed: ${messageOf(error)}`)
  }

  session.defaultSession.on('will-download', handleWillDownload)

  const readyUrl = await startDsh()
  createWindow(readyUrl)
}

/**
 * spawn `node <dsh bin.js> web --port 0 --no-open`，并返回 ready URL 的 Promise。
 *
 * 关键：spawn 后必须**立即**持续泵送 stdout/stderr。管道缓冲约 64KB，读端不消费
 * 会让 dsh 阻塞在 write 上、事件循环假死（表现为「会话载入不出来 + 工具调用卡死」）。
 */
function startDsh() {
  const node = resolveNode()
  const bin = resolveDshBin()
  log(`[dsh] using node: ${node}`)
  log(`[dsh] using bin: ${bin}`)

  const env = { ...process.env }
  // 清除宿主/IDE 注入的 NODE_OPTIONS（其 --require shim 会卡死 dsh 启动）
  delete env.NODE_OPTIONS

  const proc = spawn(node, [bin, 'web', '--port', '0', '--no-open'], {
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  })
  child = proc

  proc.on('exit', (code, signal) => onChildExit(code, signal))

  return new Promise((resolve, reject) => {
    let settled = false
    let timer = null
    const settle = (error, url) => {
      if (settled) return
      settled = true
      if (timer) clearTimeout(timer)
      if (error) reject(error)
      else resolve(url)
    }

    timer = setTimeout(
      () => settle(new Error('timed out (30s) waiting for dsh URL')),
      READY_URL_TIMEOUT_MS,
    )

    proc.on('error', (error) => settle(error))

    createLineReader(proc.stdout, (line) => {
      log(`[dsh] ${redact.redactUrl(line)}`)
      const match = READY_URL_RE.exec(line)
      if (!match) return
      log(`[dsh] ready url: ${redact.redactUrl(match[1])}`)
      settle(null, match[1])
    })
    createLineReader(proc.stderr, (line) => {
      log(`[dsh-err] ${redact.redactUrl(line)}`)
    })

    proc.once('exit', (code, signal) => {
      settle(new Error(`dsh exited early (code=${code}, signal=${signal})`))
    })
  })
}

/** 用解析到的 ready URL 创建主窗口。 */
function createWindow(readyUrl) {
  const win = new BrowserWindow({
    width: 1400,
    height: 900,
    title: 'DeepSeek Harness Desktop',
    // 页面内容全部来自 dsh 官方前端，主进程不暴露任何 Node 能力
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  })
  mainWindow = win
  win.on('closed', () => {
    if (mainWindow === win) mainWindow = null
  })

  // 隐藏官方 Session 导出乐观弹窗（与真实「另存为」流程去重）
  hideSessionDialog.attach(win.webContents)

  win.webContents.on('did-fail-load', (event, errorCode, errorDescription, validatedURL) => {
    log(`[web] did-fail-load ${errorCode} ${errorDescription} ${redact.redactUrl(validatedURL)}`)
  })

  win.loadURL(readyUrl)
}

/**
 * 下载接管（规格 §7.6）。
 *
 * Chromium 网络栈自带 cookie jar：首次 `GET /?token=…` 完成 303 + Set-Cookie 兑换后，
 * 会话 cookie 就在默认 session 的 jar 里，下载请求直接复用同一 session 取用即可，
 * 无需像 Tauri 版那样在壳侧手动发一次 token 兑换请求拿 cookie（规格 §7.5，
 * 这是 Electron 版的主要简化点）。
 *
 * 实现说明：Electron 默认会把下载**静默**写进「下载」目录（不会弹任何对话框），
 * 因此这里在 `will-download` 里直接 `preventDefault()` 取消 Chromium 自身的下载，
 * 改由本进程弹原生「另存为」，用户确认后再用 `net.request`（同一 session，共享
 * cookie jar）拉取并写盘 —— 与 Tauri 版「取消 WebView 下载 + 壳侧自 fetch」
 * 的结构一致（规格 §8 对齐清单第 5 项）。
 *
 * 不用 `item.setSavePath()`：该 API 要求必须在 `will-download` 回调内**同步**调用，
 * 无法等异步对话框返回后再落盘。
 */
function handleWillDownload(event, item) {
  const url = item.getURL()
  // 取消 Chromium 自身的下载（它不会弹「另存为」）
  event.preventDefault()

  const filename = sessionZipFilename(url)
  const defaultPath = path.join(rememberedDownloadDir() || app.getPath('downloads'), filename)
  const urlDisplay = redact.redactUrl(url)
  const options = {
    title: '保存会话导出',
    defaultPath,
    filters: [{ name: 'ZIP 压缩文件', extensions: ['zip'] }],
  }

  log(`[download] requested: ${urlDisplay} -> ${defaultPath}`)

  const choice = mainWindow
    ? dialog.showSaveDialog(mainWindow, options)
    : dialog.showSaveDialog(options)

  choice
    .then(({ canceled, filePath }) => {
      if (canceled || !filePath) {
        log(`[download] cancelled by user: ${urlDisplay}`)
        return null
      }
      rememberDownloadDir(path.dirname(filePath))
      log(`[download] destination chosen: ${filePath}`)
      return downloadToFile(url, filePath).then(
        (bytes) => log(`[download] saved ${bytes} bytes to ${filePath}`),
        (error) => {
          removePartialFile(filePath)
          log(`[download] failed: ${messageOf(error)}`)
        },
      )
    })
    .catch((error) => log(`[download] failed: ${messageOf(error)}`))
}

/**
 * 用当前 session 请求下载 URL 并流式写盘，返回写入字节数。
 *
 * `useSessionCookies: true` 是关键：它让请求带上 session cookie jar 里的
 * `dsh-auth-*` 会话 cookie（0.1.2 起 web 服务只认 cookie，缺了会 401）。
 * 该选项默认 `false`，不开启的话 `net.request` 不会自动携带 cookie。
 */
function downloadToFile(url, filePath) {
  return new Promise((resolve, reject) => {
    const request = net.request({ url, session: session.defaultSession, useSessionCookies: true })
    request.on('error', reject)
    request.on('response', (response) => {
      const status = response.statusCode
      if (status < 200 || status >= 300) {
        response.on('data', () => {})
        response.on('end', () => reject(new Error(`HTTP ${status}`)))
        return
      }
      const out = fs.createWriteStream(filePath)
      let received = 0
      response.on('data', (chunk) => {
        received += chunk.length
      })
      response.on('error', reject)
      out.on('error', reject)
      out.on('finish', () => resolve(received))
      response.pipe(out)
    })
    request.end()
  })
}

/** 失败时清掉半截文件，不留残骸。 */
function removePartialFile(filePath) {
  try {
    fs.rmSync(filePath, { force: true })
  } catch {
    /* 忽略 */
  }
}

/**
 * 从下载 URL 的 `sessionId` query 重建安全文件名，规则与 dsh 前端
 * `sessionLogZipFilename` 一致（保留 `[A-Za-z0-9_-]`，其余替换为 `_`）。
 */
function sessionZipFilename(rawUrl) {
  let sessionId = 'download'
  try {
    sessionId = new URL(rawUrl).searchParams.get('sessionId') || 'download'
  } catch {
    /* 非法 URL 用兜底名 */
  }
  return `dsh-session-${String(sessionId).replace(/[^A-Za-z0-9_-]/g, '_')}.zip`
}

/** 「记住上次下载目录」状态文件路径。 */
function downloadDirStatePath() {
  return path.join(app.getPath('userData'), LAST_DOWNLOAD_DIR_FILE)
}

/** 读取记住的上次下载目录；不存在或已失效时返回 null。 */
function rememberedDownloadDir() {
  try {
    const dir = fs.readFileSync(downloadDirStatePath(), 'utf8').trim()
    if (dir && fs.statSync(dir).isDirectory()) return dir
  } catch {
    /* 未记录或目录已删除 */
  }
  return null
}

/** 记录本次选择的下载目录，供下次对话框默认定位。 */
function rememberDownloadDir(dir) {
  try {
    fs.mkdirSync(app.getPath('userData'), { recursive: true })
    fs.writeFileSync(downloadDirStatePath(), dir)
  } catch (error) {
    log(`[download] failed to remember directory: ${messageOf(error)}`)
  }
}

/**
 * dsh 子进程退出：关闭所有窗口（进程守护）。
 *
 * 只要后端进程没了，UI 就一定不可用，因此**无论退出码**都关窗——实测 dsh 会捕获
 * SIGTERM 并以 `code=0` 优雅退出，若只按「非 0 才关窗」判断，`kill <dsh pid>` 之后
 * 会留下一个后端已死的空窗口（规格 §12.1.7 要求此时窗口自动关闭）。
 */
function onChildExit(code, signal) {
  child = null
  if (quitting) return
  log(`[dsh] child exited: code=${code} signal=${signal}`)
  for (const win of BrowserWindow.getAllWindows()) win.close()
}

/** 幂等回收子进程。 */
function killChild() {
  if (!child) return
  if (child.exitCode === null && child.signalCode === null) {
    try {
      child.kill()
    } catch {
      /* 已退出，忽略 */
    }
  }
}

/**
 * 逐行回调（跨 chunk 的半行缓冲到下一次），等价于 Rust 侧 `BufReader::lines`。
 * @param {NodeJS.ReadableStream|null} stream
 * @param {(line: string) => void} onLine
 */
function createLineReader(stream, onLine) {
  if (!stream) return
  stream.setEncoding('utf8')
  let buffer = ''
  stream.on('data', (chunk) => {
    buffer += chunk
    let index = buffer.indexOf('\n')
    while (index !== -1) {
      onLine(buffer.slice(0, index).replace(/\r$/, ''))
      buffer = buffer.slice(index + 1)
      index = buffer.indexOf('\n')
    }
  })
  stream.on('end', () => {
    if (buffer.length === 0) return
    onLine(buffer.replace(/\r$/, ''))
    buffer = ''
  })
}

function messageOf(error) {
  return error && error.message ? error.message : String(error)
}
