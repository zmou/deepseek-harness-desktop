'use strict'

/**
 * dsh web 后端协议客户端（HTTP unary RPC，无 WebSocket）。
 *
 * 与 `tauri-app/src-tauri/src/protocol.rs` 语义一致，事件来源见
 * `docs/versions/v0.1.7-rc.2/completion-notification-event-discovery.md`：
 * - `session/list`（wire 键 `_request`）低频轮询会话与 running 状态；
 * - `session/projections`（`{"request":{"sessionId":"..."}}`）取 asOfSeq（= 日志尾 seq）；
 * - `session/page`（`{"request":{...}}`，throughSeq = asOfSeq）取尾部记录，判定
 *   最新 `turn/end` 的 `reason.kind === 'completed'`。
 *
 * 解析/判定为纯函数（可独立测试）；HTTP 用 Electron `net.request` +
 * `useSessionCookies: true` 自动携带会话 cookie（与 `downloadToFile` 一致）。
 */

let _rpcId = 0
function nextRpcId() {
  _rpcId += 1
  return `dsh-desktop-${_rpcId}`
}

/** 构建一次 unary RPC 的请求体。 */
function buildRpcBody(endpoint, args) {
  return {
    type: 'client-request',
    rpcId: nextRpcId(),
    method: endpoint,
    payload: { args },
  }
}

/** 解析 `session/list` 响应 value（{items:[SessionSummary]}）。 */
function parseListSessions(value) {
  const out = []
  const items = value && Array.isArray(value.items) ? value.items : []
  for (const item of items) {
    if (!item || typeof item.sessionId !== 'string') continue
    out.push({
      sessionId: item.sessionId,
      running: item.running === true,
      blank: item.blank === true,
      origin: typeof item.origin === 'string' ? item.origin : undefined,
      parentSessionId:
        typeof item.parentSessionId === 'string' ? item.parentSessionId : undefined,
    })
  }
  return out
}

/** 解析 `session/projections` 的 asOfSeq；空日志（-1）返回 null。 */
function parseProjectionCursor(value) {
  if (!value || typeof value.asOfSeq !== 'number') return null
  return value.asOfSeq >= 0 ? value.asOfSeq : null
}

/** 从 `session/page` 的 records 找最新一条 `turn/end`。 */
function findLatestTurnEnd(records) {
  let best = null
  if (!Array.isArray(records)) return null
  for (const record of records) {
    const event = record && record.event
    if (!event || event.type !== 'turn/end') continue
    const seq = typeof event.seq === 'number' ? event.seq : null
    if (seq === null) continue
    const turn = event.data && typeof event.data.turn === 'number' ? event.data.turn : 0
    const kind = event.data && event.data.reason && typeof event.data.reason.kind === 'string'
      ? event.data.reason.kind
      : ''
    if (!best || seq > best.seq) best = { seq, turn, kind }
  }
  return best
}

/** 是否正常完成（规格 §1.2：仅 completed）。 */
function isNormalCompletion(turnEnd) {
  return Boolean(turnEnd) && turnEnd.kind === 'completed'
}

/**
 * 执行一次 unary RPC（Electron 主进程）。
 * @param {object} net - Electron `net` 模块
 * @param {string} baseUrl - 形如 `http://127.0.0.1:<port>`
 * @param {string} endpoint - 形如 `session/list`
 * @param {object} args - wire 键已就位的 args 对象
 * @returns {Promise<{ok:true,value:any}|{ok:false,code:string,message:string}>}
 */
function rpcUnary(net, baseUrl, endpoint, args) {
  return new Promise((resolve, reject) => {
    const url = `${baseUrl.replace(/\/$/, '')}/api/${endpoint}`
    const body = JSON.stringify(buildRpcBody(endpoint, args))
    const request = net.request({
      url,
      method: 'POST',
      useSessionCookies: true,
    })
    request.setHeader('Content-Type', 'application/json')
    request.on('response', (response) => {
      let text = ''
      response.on('data', (chunk) => {
        text += chunk
      })
      response.on('end', () => {
        try {
          const parsed = JSON.parse(text)
          const result = parsed && parsed.result
          if (!result) {
            resolve({ ok: false, code: 'malformed', message: 'missing result envelope' })
            return
          }
          if (result.ok === true) {
            resolve({ ok: true, value: result.value })
          } else {
            const error = result.error || {}
            resolve({
              ok: false,
              code: typeof error.code === 'string' ? error.code : '',
              message: typeof error.message === 'string' ? error.message : '',
            })
          }
        } catch (error) {
          reject(new Error(`rpc ${endpoint} response decode failed: ${messageOf(error)}`))
        }
      })
    })
    request.on('error', reject)
    request.write(body)
    request.end()
  })
}

/**
 * 会话列表（unary session/list）。
 * @returns {Promise<Array<{sessionId,running,blank,origin?,parentSessionId?}>>}
 */
async function listSessions(net, baseUrl) {
  const res = await rpcUnary(net, baseUrl, 'session/list', { _request: {} })
  if (!res.ok) throw new Error(`session/list: ${res.code} ${res.message}`)
  return parseListSessions(res.value)
}

/** 某顶层会话的日志尾 seq；会话不存在/空日志返回 null。 */
async function sessionCursor(net, baseUrl, sessionId) {
  const res = await rpcUnary(net, baseUrl, 'session/projections', {
    request: { sessionId },
  })
  if (!res.ok) {
    if (res.code === 'session/not-found') return null
    throw new Error(`session/projections: ${res.code} ${res.message}`)
  }
  return parseProjectionCursor(res.value)
}

/** 取某顶层会话尾部 records（含 turn/end）。 */
async function sessionTail(net, baseUrl, sessionId, throughSeq) {
  const res = await rpcUnary(net, baseUrl, 'session/page', {
    request: {
      address: { kind: 'session', sessionId },
      throughSeq,
      maxMessages: 50,
    },
  })
  if (!res.ok) throw new Error(`session/page: ${res.code} ${res.message}`)
  return res.value && Array.isArray(res.value.records) ? res.value.records : []
}

/** 一站式：判断某顶层会话是否刚「正常完成」，返回 {seq,turn} 或 null。 */
async function latestCompletion(net, baseUrl, sessionId) {
  const cursor = await sessionCursor(net, baseUrl, sessionId)
  if (cursor === null) return null
  const records = await sessionTail(net, baseUrl, sessionId, cursor)
  const turnEnd = findLatestTurnEnd(records)
  if (!isNormalCompletion(turnEnd)) return null
  return { seq: turnEnd.seq, turn: turnEnd.turn }
}

function messageOf(error) {
  return error && error.message ? error.message : String(error)
}

module.exports = {
  buildRpcBody,
  parseListSessions,
  parseProjectionCursor,
  findLatestTurnEnd,
  isNormalCompletion,
  rpcUnary,
  listSessions,
  sessionCursor,
  sessionTail,
  latestCompletion,
}