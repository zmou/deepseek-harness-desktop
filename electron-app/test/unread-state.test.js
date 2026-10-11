'use strict'

/**
 * unread-state 单元测试（node:test，与 Rust 侧 state.rs 的测试一一对应）。
 * 运行：node --test electron-app/test/unread-state.test.js
 */

const { test } = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')

const {
  createUnreadStore,
  loadUnreadStore,
  saveUnreadStore,
  onCompleted,
  onViewed,
  unreadCount,
  unreadSessions,
  isUnread,
} = require('../src/unread-state')

function tmpPath(name) {
  const dir = path.join(os.tmpdir(), 'dsh-desktop-state-test-js')
  fs.mkdirSync(dir, { recursive: true })
  return path.join(dir, name)
}

test('first completion adds unread', () => {
  const s = createUnreadStore()
  assert.equal(onCompleted(s, 's1', 10, 1, 1000), 'new-unread')
  assert.equal(unreadCount(s), 1)
  assert.equal(isUnread(s, 's1'), true)
})

test('same seq replay keeps count', () => {
  const s = createUnreadStore()
  onCompleted(s, 's1', 10, 1, 1000)
  assert.equal(onCompleted(s, 's1', 10, 1, 2000), 'duplicate')
  assert.equal(unreadCount(s), 1)
})

test('distinct sessions count independently', () => {
  const s = createUnreadStore()
  onCompleted(s, 's1', 10, 1, 1000)
  onCompleted(s, 's2', 20, 1, 1000)
  onCompleted(s, 's3', 30, 1, 1000)
  assert.equal(unreadCount(s), 3)
  assert.deepEqual(unreadSessions(s), ['s1', 's2', 's3'])
})

test('viewed removes only that session', () => {
  const s = createUnreadStore()
  onCompleted(s, 's1', 10, 1, 1000)
  onCompleted(s, 's2', 20, 1, 1000)
  assert.equal(onViewed(s, 's1'), 'cleared')
  assert.equal(unreadCount(s), 1)
  assert.equal(isUnread(s, 's1'), false)
  assert.equal(isUnread(s, 's2'), true)
  assert.equal(onViewed(s, 's1'), 'noop')
})

test('new completion cycle after read reunreads', () => {
  const s = createUnreadStore()
  onCompleted(s, 's1', 10, 1, 1000)
  onViewed(s, 's1')
  assert.equal(unreadCount(s), 0)
  assert.equal(onCompleted(s, 's1', 40, 2, 2000), 'new-unread')
  assert.equal(unreadCount(s), 1)
})

test('completion replaces older cycle', () => {
  const s = createUnreadStore()
  onCompleted(s, 's1', 10, 1, 1000)
  assert.equal(onCompleted(s, 's1', 40, 2, 2000), 'new-unread')
  assert.equal(unreadCount(s), 1)
  const entry = s.unread.get('s1')
  assert.equal(entry.seq, 40)
  assert.equal(entry.turn, 2)
  assert.equal(entry.first_completed_at, 2000)
})

test('persistence roundtrip', () => {
  const file = tmpPath('roundtrip.json')
  fs.rmSync(file, { force: true })
  {
    const s = createUnreadStore()
    onCompleted(s, 's1', 10, 1, 1000)
    onCompleted(s, 's2', 20, 1, 1000)
    onViewed(s, 's1')
    saveUnreadStore(s, file)
  }
  {
    const s = loadUnreadStore(file)
    assert.equal(unreadCount(s), 1)
    assert.equal(isUnread(s, 's1'), false)
    assert.equal(isUnread(s, 's2'), true)
    const entry = s.unread.get('s2')
    assert.equal(entry.seq, 20)
    assert.equal(entry.first_completed_at, 1000)
  }
  fs.rmSync(file, { force: true })
})

test('missing file loads empty', () => {
  const s = loadUnreadStore(tmpPath('nope.json'))
  assert.equal(unreadCount(s), 0)
})

test('corrupt file loads empty', () => {
  const file = tmpPath('corrupt.json')
  fs.writeFileSync(file, 'not json {')
  const s = loadUnreadStore(file)
  assert.equal(unreadCount(s), 0)
  fs.rmSync(file, { force: true })
})

test('unknown schema loads empty', () => {
  const file = tmpPath('schema.json')
  fs.writeFileSync(
    file,
    JSON.stringify({
      schema_version: 999,
      unread: { s1: { seq: 10, turn: 1, first_completed_at: 1000, read_at: null } },
    }),
  )
  const s = loadUnreadStore(file)
  assert.equal(unreadCount(s), 0)
  fs.rmSync(file, { force: true })
})

test('save is atomic temp rename', () => {
  const file = tmpPath('atomic.json')
  fs.rmSync(file, { force: true })
  const s = createUnreadStore()
  onCompleted(s, 's1', 10, 1, 1000)
  saveUnreadStore(s, file)
  assert.equal(fs.existsSync(`${file}.tmp`), false)
  fs.rmSync(file, { force: true })
})

test('bom prefixed file still loads', () => {
  const file = tmpPath('bom.json')
  const body = JSON.stringify({
    schema_version: 1,
    unread: { s1: { seq: 10, turn: 1, first_completed_at: 1000, read_at: null } },
  })
  fs.writeFileSync(file, `\uFEFF${body}`)
  const s = loadUnreadStore(file)
  assert.equal(unreadCount(s), 1)
  assert.equal(isUnread(s, 's1'), true)
  fs.rmSync(file, { force: true })
})
