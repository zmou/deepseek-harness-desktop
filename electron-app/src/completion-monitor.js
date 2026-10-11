'use strict'

/**
 * 会话完成通知 — Electron 主进程集成（轮询、完成判定、前台判定、徽标、通知）。
 *
 * 语义与 `tauri-app/src-tauri/src/completion.rs` 一致，对齐
 * `docs/versions/v0.1.7-rc.2/desktop-completion-notification-spec.md`：
 * - 完成判定：HTTP unary 轮询 `session/list` → running true→false 转换 →
 *   `latestCompletion()`（`turn/end reason.kind==='completed'`）确认；
 *   首轮仅建基线（规格 §10.2）。
 * - 前台判定（§3.3）：`isVisible() && isFocused() && !isMinimized()` 才算前台；
 *   其余状态（隐藏到托盘 / 最小化 / 失焦）均按后台，完成可发通知。
 * - 徽标：Windows `win.setOverlayIcon`（taskbar 应用按钮，规格 §6.3 要求
 *   BrowserWindow/taskbar overlay 而非托盘图标）；macOS `app.dock.setBadge`。
 * - 通知：Electron 内置 `Notification`（Windows toast / macOS 通知中心），
 *   由主进程创建（规格 §7）；失败/权限关闭不影响未读状态（§4.3/§10.2）。
 * - 持久化：`app.getPath('userData')`，原子写（temp+rename）（§5.1/§5.3）。
 *
 * 本模块导出纯函数（可测）与 Electron 接线（start/unref 时传 app/BrowserWindow）。
 * 零新增依赖。
 */

const path = require('node:path')

const {
  listSessions,
  latestCompletion,
} = require('./dsh-protocol')
const {
  createUnreadStore,
  loadUnreadStore,
  saveUnreadStore,
  onCompleted,
  onViewed,
  unreadCount,
} = require('./unread-state')

/** 轮询间隔（毫秒）。 */
const POLL_INTERVAL_MS = 3000

/** 前台判定（规格 §3.3）：仅 visible && focused && !minimized 属于前台。 */
function isForeground(win) {
  if (!win || win.isDestroyed()) return false
  try {
    return win.isVisible() && win.isFocused() && !win.isMinimized()
  } catch {
    return false
  }
}

/** 会话运行状态观察器（与 Rust watch.rs 同语义）。 */
function createWatcher() {
  return { known: new Map(), initialized: false }
}

/**
 * 观察一轮，返回 (watcher, stoppedSessionIds)。
 * - 首轮：记录 running 基线，不产出转换。
 * - 后续：running true→false 的会话记为停止。
 * - 消失的会话：上次 running=true 视为停止。
 */
function observeSessions(watcher, sessions) {
  const stopped = []
  const next = new Map()
  for (const s of sessions) {
    const was = watcher.known.get(s.sessionId)
    next.set(s.sessionId, s.running === true)
    if (!watcher.initialized) continue
    if (was === true && s.running !== true) stopped.push(s.sessionId)
  }
  if (watcher.initialized) {
    for (const [id, running] of watcher.known) {
      if (running === true && !next.has(id)) stopped.push(id)
    }
  }
  watcher.known = next
  watcher.initialized = true
  return stopped
}

/**
 * 处理一轮轮询结果：对停止且非 subagent 的顶层会话做完成确认。
 * @param {object} deps - { net, baseUrl, log }
 * @returns {Promise<Array<{sessionId,seq,turn}>>} 新确认的正常完成
 */
async function detectCompletions({ net, baseUrl, log }, watcher, sessions) {
  const stopped = observeSessions(watcher, sessions)
  const completions = []
  for (const sessionId of stopped) {
    const summary = sessions.find((s) => s.sessionId === sessionId)
    if (!summary) continue
    if (summary.origin === 'subagent') {
      log(`[notify] skip subagent completion: ${sessionId}`)
      continue
    }
    try {
      const result = await latestCompletion(net, baseUrl, sessionId)
      if (result) completions.push({ sessionId, seq: result.seq, turn: result.turn })
      else log(`[notify] session stopped but not normal completion: ${sessionId}`)
    } catch (error) {
      log(`[notify] completion check failed ${sessionId}: ${messageOf(error)}`)
    }
  }
  return completions
}

/** 把完成事件并入未读状态机，返回新未读列表（供通知/徽标）。 */
function recordCompletions(store, completions, nowMs) {
  const fresh = []
  for (const c of completions) {
    if (onCompleted(store, c.sessionId, c.seq, c.turn, nowMs) === 'new-unread') {
      fresh.push(c)
    }
  }
  return fresh
}

/**
 * 处理「用户查看 session」信号（来自注入 client 插件桥）。
 * @returns {boolean} 是否清除了未读
 */
function handleViewed(store, filePath, sessionId) {
  const cleared = onViewed(store, sessionId) === 'cleared'
  if (cleared) {
    try {
      saveUnreadStore(store, filePath)
    } catch (error) {
      logErr(`[notify] failed to persist after viewed: ${messageOf(error)}`)
    }
  }
  return cleared
}

function logErr(msg) {
  // eslint-disable-next-line no-console
  console.error(msg)
}

function messageOf(error) {
  return error && error.message ? error.message : String(error)
}

/**
 * 更新 taskbar/Dock 徽标。未读数为 0 时清除。
 * @param {import('electron').BrowserWindow|null|undefined} win
 * @param {import('electron').App} app
 */
function refreshBadge(win, app, count, log) {
  if (count <= 0) {
    // 清除
    if (win && !win.isDestroyed()) {
      try {
        win.setOverlayIcon(null, '')
      } catch (error) {
        log(`[notify] clear overlay icon failed: ${messageOf(error)}`)
      }
    }
    if (process.platform === 'darwin') {
      app.dock.setBadge('')
    }
    return
  }
  // 有未读：Windows 数字 overlay icon（taskbar button，规格 §6.3）/
  // macOS Dock badge
  if (process.platform === 'win32') {
    const badgePath = badgePathFor(count)
    if (badgePath && win && !win.isDestroyed()) {
      try {
        const { nativeImage } = require('electron')
        const image = nativeImage.createFromPath(badgePath)
        win.setOverlayIcon(image, `${count} unread`)
      } catch (error) {
        log(`[notify] set overlay icon failed: ${messageOf(error)}`)
      }
    }
  } else if (process.platform === 'darwin') {
    app.dock.setBadge(String(count))
  }
}

/** 未读数 → 数字徽标 PNG 路径（1-9 精确 / 10-99 `9+` / ≥100 `99+`）。 */
function badgePathFor(count) {
  const fs = require('node:fs')
  const label = count <= 9 ? String(count) : count <= 99 ? '9+' : '99+'
  const candidates = [
    // 打包后：asar 内 icons/badges
    path.join(appPathDir(), 'icons', 'badges', `badge-${label}.png`),
    // 开发：electron-app/icons/badges
    path.join(__dirname, '..', 'icons', 'badges', `badge-${label}.png`),
  ]
  for (const c of candidates) {
    if (fs.existsSync(c)) return c
  }
  return null
}

function appPathDir() {
  // 在 packaged app 中 `app.getAppPath()` 指向 asar 内；__dirname 也指向 asar。
  // 返回可解析 icons/badges 的相对基：优先 asar 内的 `icons`。
  const { app } = require('electron')
  return path.join(app.getAppPath())
}

/** 显示系统通知（规格 §3.1/§3.4）。 */
function showNotification(unreadCountValue, log) {
  try {
    const { Notification } = require('electron')
    if (!Notification.isSupported()) {
      log('[notify] system notifications not supported')
      return
    }
    const body =
      unreadCountValue <= 1
        ? '有会话已完成，等待查看。'
        : `有 ${unreadCountValue} 个会话已完成，等待查看。`
    const n = new Notification({ title: 'DeepSeek Harness Desktop', body })
    n.show()
    log(`[notify] system notification shown; unread=${unreadCountValue}`)
  } catch (error) {
    log(`[notify] system notification failed: ${messageOf(error)}`)
  }
}

/**
 * 校验注入桥上报的 sessionId（规格 §7：长度与字符集校验，不信任 payload）。
 */
function isValidSessionId(sessionId) {
  return (
    typeof sessionId === 'string' &&
    sessionId.length > 0 &&
    sessionId.length <= 128 &&
    /^[A-Za-z0-9_-]+$/.test(sessionId)
  )
}

/** 解析注入桥队列 JSON（`["sid",...]`），过滤非法项。 */
function parseViewedQueue(json) {
  try {
    const parsed = JSON.parse(json)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(isValidSessionId)
  } catch {
    return []
  }
}

/**
 * 解析队列收集结果 JSON（`{viewed,ready,state}`）——与 Tauri 侧 `parse_drain_result`
 * 同语义；兼容裸数组形式（旧脚本/测试）。
 */
function parseDrainResult(json) {
  let parsed
  try {
    parsed = JSON.parse(json)
  } catch {
    return { viewed: [], bridgeReady: false, bridgeState: 'malformed' }
  }
  if (Array.isArray(parsed)) {
    return { viewed: parsed.filter(isValidSessionId), bridgeReady: false, bridgeState: 'array' }
  }
  return {
    viewed:
      parsed && Array.isArray(parsed.viewed) ? parsed.viewed.filter(isValidSessionId) : [],
    bridgeReady: Boolean(parsed) && parsed.ready === true,
    bridgeState:
      parsed && typeof parsed.state === 'string' ? parsed.state : 'absent',
  }
}

/**
 * 页面内脚本：原子取出并清空注入桥队列，返回 `{viewed,ready,state}` JSON。
 * 与 Tauri 侧 `DRAIN_VIEWED_SCRIPT` 保持一致。
 */
const DRAIN_VIEWED_SCRIPT = `(function () {
  try {
    var q = window.__dshDesktopViewed__;
    var taken = [];
    if (Array.isArray(q) && q.length > 0) { taken = q.slice(0); q.length = 0; }
    return JSON.stringify({
      viewed: taken,
      ready: window.__dshDesktopBridgeReady__ === true,
      state: typeof window.__dshDesktopBridgeState__ === 'string' ? window.__dshDesktopBridgeState__ : 'absent'
    });
  } catch (e) {
    return '{"viewed":[],"ready":false,"state":"error"}';
  }
})()`

/**
 * 通过 `webContents.executeJavaScript` 取回并清空注入桥队列（带超时）。
 * @returns {Promise<{viewed:string[],bridgeReady:boolean,bridgeState:string}>}
 */
async function drainViewedFromWebContents(win, timeoutMs = 2000) {
  if (!win || win.isDestroyed()) {
    return { viewed: [], bridgeReady: false, bridgeState: 'no-window' }
  }
  const raw = await new Promise((resolve) => {
    const timer = setTimeout(() => resolve('[]'), timeoutMs)
    win.webContents
      .executeJavaScript(DRAIN_VIEWED_SCRIPT, true)
      .then((value) => {
        clearTimeout(timer)
        resolve(typeof value === 'string' ? value : '[]')
      })
      .catch(() => {
        clearTimeout(timer)
        resolve('[]')
      })
  })
  return parseDrainResult(raw)
}

/**
 * 启动完成通知监控（Electron 接线）。
 * @param {object} opts - { app, getWindow, net, baseUrl, stateFile, log }
 * @returns {{ stop: () => void, getStore: () => object }} 控制句柄
 */
function startCompletionMonitor({ app, getWindow, net, baseUrl, stateFile, log }) {
  let store
  try {
    store = loadUnreadStore(stateFile)
  } catch (error) {
    log(`[notify] failed to load unread state: ${messageOf(error)}`)
    store = createUnreadStore()
  }

  const watcher = createWatcher()

  // 管道：`window-all-closed`/`before-quit` 时停止轮询
  let stopped = false
  let bridgeStateLogged = false

  const interval = setInterval(async () => {
    if (stopped) return
    try {
      // 1) 收集注入桥上报的「用户已查看 session」，清除对应未读（规格 §4.4）
      const winNow = getWindow()
      if (winNow) {
        const drain = await drainViewedFromWebContents(winNow)
        if (!bridgeStateLogged && drain.bridgeReady) {
          log(`[notify] viewed-session bridge attached (state=${drain.bridgeState})`)
          bridgeStateLogged = true
        }
        let cleared = false
        for (const sid of drain.viewed) {
          if (handleViewed(store, stateFile, sid)) {
            log(`[notify] session viewed; unread cleared: ${sid}`)
            cleared = true
          }
        }
        if (cleared) refreshBadge(winNow, app, unreadCount(store), log)
      }

      // 2) 轮询会话列表，检测「运行 -> 停止」并确认正常完成
      const sessions = await listSessions(net, baseUrl)
      const completions = await detectCompletions({ net, baseUrl, log }, watcher, sessions)
      if (completions.length === 0) return
      const fresh = recordCompletions(store, completions, Date.now())
      if (fresh.length === 0) return
      try {
        saveUnreadStore(store, stateFile)
      } catch (error) {
        log(`[notify] failed to persist unread state: ${messageOf(error)}`)
      }
      const count = unreadCount(store)
      const win = getWindow()
      refreshBadge(win, app, count, log)
      if (!isForeground(win)) {
        showNotification(count, log)
      }
      log(`[notify] completions recorded: ${fresh.map((c) => c.sessionId).join(',')}; unread=${count}`)
    } catch (error) {
      log(`[notify] poll round failed: ${messageOf(error)}`)
    }
  }, POLL_INTERVAL_MS)
  // unref：不阻止 Electron 退出
  if (typeof interval.unref === 'function') interval.unref()

  // 启动即恢复徽标（规格 §5.3）
  const win = getWindow()
  log(`[notify] restoring unread state: count=${unreadCount(store)}`)
  refreshBadge(win, app, unreadCount(store), log)

  return {
    stop() {
      stopped = true
      clearInterval(interval)
    },
    getStore() {
      return store
    },
  }
}

/** 状态文件路径（userData 下）。 */
function stateFilePath(app) {
  return path.join(app.getPath('userData'), 'completion-unread.json')
}

module.exports = {
  POLL_INTERVAL_MS,
  isForeground,
  createWatcher,
  observeSessions,
  detectCompletions,
  recordCompletions,
  handleViewed,
  refreshBadge,
  showNotification,
  startCompletionMonitor,
  stateFilePath,
  isValidSessionId,
  parseViewedQueue,
  parseDrainResult,
  DRAIN_VIEWED_SCRIPT,
  drainViewedFromWebContents,
}