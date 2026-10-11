'use strict'

/**
 * completion-monitor 纯逻辑单元测试（node:test）。
 * 运行：node --test electron-app/test/completion-monitor.test.js
 */

const { test } = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')

const {
  createWatcher,
  observeSessions,
  recordCompletions,
  handleViewed,
  isValidSessionId,
  parseViewedQueue,
  parseDrainResult,
} = require('../src/completion-monitor')
const {
  createUnreadStore,
  onCompleted,
  unreadCount,
  isUnread,
} = require('../src/unread-state')

function summary(id, running, origin) {
  return {
    sessionId: id,
    running,
    blank: false,
    origin: origin || undefined,
    parentSessionId: undefined,
  }
}

test('first round establishes baseline (no stop)', () => {
  const w = createWatcher()
  const stopped = observeSessions(w, [summary('s1', false)])
  assert.deepEqual(stopped, [])
})

test('running to stopped detected', () => {
  const w = createWatcher()
  observeSessions(w, [summary('s1', true)])
  const stopped = observeSessions(w, [summary('s1', false)])
  assert.deepEqual(stopped, ['s1'])
})

test('stays stopped no repeat', () => {
  const w = createWatcher()
  observeSessions(w, [summary('s1', true)])
  observeSessions(w, [summary('s1', false)])
  const stopped = observeSessions(w, [summary('s1', false)])
  assert.deepEqual(stopped, [])
})

test('removed running session stops', () => {
  const w = createWatcher()
  observeSessions(w, [summary('s1', true)])
  const stopped = observeSessions(w, [])
  assert.deepEqual(stopped, ['s1'])
})

test('removed idle session ignored', () => {
  const w = createWatcher()
  observeSessions(w, [summary('s1', false)])
  const stopped = observeSessions(w, [])
  assert.deepEqual(stopped, [])
})

test('recordCompletions only new unread', () => {
  const store = createUnreadStore()
  const c1 = { sessionId: 's1', seq: 10, turn: 1 }
  const c2 = { sessionId: 's1', seq: 10, turn: 1 } // replay
  const c3 = { sessionId: 's2', seq: 20, turn: 1 }
  const fresh = recordCompletions(store, [c1, c2, c3], 1000)
  assert.equal(fresh.length, 2)
  assert.deepEqual(fresh.map((c) => c.sessionId).sort(), ['s1', 's2'])
  assert.equal(unreadCount(store), 2)
})

test('handleViewed clears and persists', () => {
  const dir = path.join(os.tmpdir(), 'dsh-notify-test-js')
  fs.mkdirSync(dir, { recursive: true })
  const file = path.join(dir, 'viewed.json')
  fs.rmSync(file, { force: true })
  const store = createUnreadStore()
  onCompleted(store, 's1', 10, 1, 1000)
  assert.equal(handleViewed(store, file, 's1'), true)
  assert.equal(unreadCount(store), 0)
  assert.equal(handleViewed(store, file, 's1'), false)
  fs.rmSync(file, { force: true })
})

test('handleViewed noop for unknown', () => {
  const store = createUnreadStore()
  assert.equal(handleViewed(store, path.join(os.tmpdir(), 'nope.json'), 'nope'), false)
})

test('isValidSessionId rejects hostile input', () => {
  assert.equal(isValidSessionId('session-898194d2-59a4-4321-aa00-56ee29e2ca85'), true)
  assert.equal(isValidSessionId('abc_123'), true)
  assert.equal(isValidSessionId(''), false)
  assert.equal(isValidSessionId('has space'), false)
  assert.equal(isValidSessionId('semi;colon'), false)
  assert.equal(isValidSessionId('../../etc/passwd'), false)
  assert.equal(isValidSessionId('x'.repeat(129)), false)
  assert.equal(isValidSessionId(null), false)
  assert.equal(isValidSessionId(42), false)
})

test('parseViewedQueue filters invalid entries', () => {
  const parsed = parseViewedQueue('["session-a","session-b","bad id","",42,null,"../../x"]')
  assert.deepEqual(parsed, ['session-a', 'session-b'])
})

test('parseViewedQueue handles malformed json', () => {
  assert.deepEqual(parseViewedQueue('not json'), [])
  assert.deepEqual(parseViewedQueue('{"a":1}'), [])
  assert.deepEqual(parseViewedQueue('[]'), [])
})

test('parseDrainResult reads viewed and bridge state', () => {
  const d = parseDrainResult(
    '{"viewed":["session-a","bad id","session-b"],"ready":true,"state":"attached"}',
  )
  assert.deepEqual(d.viewed, ['session-a', 'session-b'])
  assert.equal(d.bridgeReady, true)
  assert.equal(d.bridgeState, 'attached')
})

test('parseDrainResult handles absent bridge and malformed', () => {
  const absent = parseDrainResult('{"viewed":[],"ready":false,"state":"absent"}')
  assert.deepEqual(absent.viewed, [])
  assert.equal(absent.bridgeReady, false)
  assert.equal(absent.bridgeState, 'absent')

  const malformed = parseDrainResult('not json')
  assert.deepEqual(malformed.viewed, [])
  assert.equal(malformed.bridgeReady, false)
  assert.equal(malformed.bridgeState, 'malformed')
})

test('parseDrainResult ignores hostile session ids', () => {
  const d = parseDrainResult(
    '{"viewed":["../../etc/passwd","has space","ok_1",42,null],"ready":true,"state":"attached"}',
  )
  assert.deepEqual(d.viewed, ['ok_1'])
})

test('parseDrainResult accepts legacy bare array', () => {
  const d = parseDrainResult('["session-a","bad id"]')
  assert.deepEqual(d.viewed, ['session-a'])
  assert.equal(d.bridgeReady, false)
  assert.equal(d.bridgeState, 'array')
})