'use strict'

/**
 * 会话完成通知 — 未读状态机（纯逻辑，无 Electron 依赖，可独立测试）。
 *
 * 语义与 `tauri-app/src-tauri/src/state.rs` 完全一致，对齐
 * `docs/versions/v0.1.7-rc.2/desktop-completion-notification-spec.md` §4/§5：
 * - 未读数 = 不同 session 数（不是完成事件数、不是通知数、不是历史总数）。
 * - 同一 session 同一完成周期（seq）的重复/重放事件只计一次。
 * - 用户查看某 session 后清除该 session 的未读；仅恢复窗口/聚焦不清除。
 * - 状态跨应用重启保留；文件缺失/损坏按空状态处理，不阻断启动。
 *
 * 两套壳各自存储（规格 §5.1），此处落盘到 Electron `app.getPath('userData')`
 * 下的状态文件；本模块不 import Electron，由调用方传入路径。
 */

const fs = require('node:fs')
const path = require('node:path')

const SCHEMA_VERSION = 1

/** 从空集合创建。 */
function createUnreadStore() {
  return { unread: new Map() }
}

/**
 * 从持久化文件加载；文件缺失、损坏或 schema 未知时返回空状态（不抛错）。
 * @param {string} filePath
 */
function loadUnreadStore(filePath) {
  let text
  try {
    text = fs.readFileSync(filePath, 'utf8')
  } catch {
    return createUnreadStore()
  }
  try {
    // 容忍 UTF-8 BOM：外部工具写出的状态文件可能带 BOM，
    // 不处理会让整份未读状态被当成损坏而丢弃。
    const parsed = JSON.parse(text.replace(/^\uFEFF/, ''))
    if (parsed && parsed.schema_version === SCHEMA_VERSION && parsed.unread && typeof parsed.unread === 'object') {
      const store = createUnreadStore()
      for (const [sessionId, entry] of Object.entries(parsed.unread)) {
        if (entry && typeof entry.seq === 'number' && typeof entry.turn === 'number') {
          store.unread.set(sessionId, {
            seq: entry.seq,
            turn: entry.turn,
            first_completed_at: entry.first_completed_at,
            read_at: entry.read_at ?? null,
          })
        }
      }
      return store
    }
  } catch {
    /* 损坏：按空状态 */
  }
  return createUnreadStore()
}

/**
 * 原子持久化：写临时文件后 rename 替换。
 * @param {object} store
 * @param {string} filePath
 */
function saveUnreadStore(store, filePath) {
  const payload = {
    schema_version: SCHEMA_VERSION,
    unread: Object.fromEntries(
      [...store.unread.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
    ),
  }
  fs.mkdirSync(path.dirname(filePath), { recursive: true })
  const tmp = `${filePath}.tmp`
  fs.writeFileSync(tmp, JSON.stringify(payload))
  fs.renameSync(tmp, filePath)
}

/**
 * 记录一次正常完成事件。
 * @param {object} store
 * @param {string} sessionId
 * @param {number} seq 完成事件（turn/end）的 session 日志序列号
 * @param {number} turn turn 序号
 * @param {number} nowMs Unix epoch 毫秒
 * @returns {'new-unread'|'duplicate'}
 */
function onCompleted(store, sessionId, seq, turn, nowMs) {
  const existing = store.unread.get(sessionId)
  if (existing && existing.seq === seq) return 'duplicate'
  store.unread.set(sessionId, {
    seq,
    turn,
    first_completed_at: nowMs,
    read_at: null,
  })
  return 'new-unread'
}

/**
 * 用户查看了某 session：从未读集合移除。
 * @returns {'cleared'|'noop'}
 */
function onViewed(store, sessionId) {
  return store.unread.delete(sessionId) ? 'cleared' : 'noop'
}

/** 当前未读数（不同 session 数）。 */
function unreadCount(store) {
  return store.unread.size
}

/** 未读 session 集合（稳定排序）。 */
function unreadSessions(store) {
  return [...store.unread.keys()].sort()
}

/** 某 session 是否未读。 */
function isUnread(store, sessionId) {
  return store.unread.has(sessionId)
}

module.exports = {
  createUnreadStore,
  loadUnreadStore,
  saveUnreadStore,
  onCompleted,
  onViewed,
  unreadCount,
  unreadSessions,
  isUnread,
}
