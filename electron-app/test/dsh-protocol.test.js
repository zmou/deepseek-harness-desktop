'use strict'

/**
 * dsh-protocol 解析/判定纯函数单元测试（node:test）。
 * 运行：node --test electron-app/test/dsh-protocol.test.js
 */

const { test } = require('node:test')
const assert = require('node:assert/strict')

const {
  buildRpcBody,
  parseListSessions,
  parseProjectionCursor,
  findLatestTurnEnd,
  isNormalCompletion,
} = require('../src/dsh-protocol')

test('buildRpcBody has correct envelope', () => {
  const body = buildRpcBody('session/list', { _request: {} })
  assert.equal(body.type, 'client-request')
  assert.equal(body.method, 'session/list')
  assert.deepEqual(body.payload, { args: { _request: {} } })
  assert.equal(typeof body.rpcId, 'string')
  assert.ok(body.rpcId.startsWith('dsh-desktop-'))
})

test('parseListSessions extracts fields', () => {
  const value = {
    items: [
      { sessionId: 's1', running: true, blank: false, updatedAt: 1 },
      {
        sessionId: 's2',
        running: false,
        blank: true,
        origin: 'subagent',
        parentSessionId: 's1',
        updatedAt: 2,
      },
    ],
  }
  const sessions = parseListSessions(value)
  assert.equal(sessions.length, 2)
  assert.equal(sessions[0].sessionId, 's1')
  assert.equal(sessions[0].running, true)
  assert.equal(sessions[0].origin, undefined)
  assert.equal(sessions[1].origin, 'subagent')
  assert.equal(sessions[1].parentSessionId, 's1')
})

test('parseListSessions skips malformed', () => {
  assert.equal(parseListSessions({ items: [{ running: true }, { sessionId: 42 }] }).length, 0)
  assert.equal(parseListSessions(null).length, 0)
})

test('parseProjectionCursor reads asOfSeq', () => {
  assert.equal(parseProjectionCursor({ asOfSeq: 1899, values: {} }), 1899)
  assert.equal(parseProjectionCursor({ asOfSeq: -1, values: {} }), null)
  assert.equal(parseProjectionCursor({ values: {} }), null)
  assert.equal(parseProjectionCursor(null), null)
})

test('findLatestTurnEnd picks max seq', () => {
  const records = [
    { type: 'event', event: { type: 'user/message', seq: 100, time: 1, data: {} } },
    {
      type: 'event',
      event: {
        type: 'turn/end',
        seq: 200,
        time: 2,
        data: { turn: 1, reason: { kind: 'completed' } },
      },
    },
    { type: 'event', event: { type: 'step/end', seq: 300, time: 3, data: {} } },
    {
      type: 'event',
      event: {
        type: 'turn/end',
        seq: 400,
        time: 4,
        data: { turn: 2, reason: { kind: 'interrupted' } },
      },
    },
  ]
  const t = findLatestTurnEnd(records)
  assert.equal(t.seq, 400)
  assert.equal(t.turn, 2)
  assert.equal(t.kind, 'interrupted')
})

test('findLatestTurnEnd returns null without turn/end', () => {
  const records = [
    { type: 'event', event: { type: 'user/message', seq: 100, time: 1, data: {} } },
  ]
  assert.equal(findLatestTurnEnd(records), null)
  assert.equal(findLatestTurnEnd(null), null)
})

test('isNormalCompletion only completed', () => {
  assert.equal(isNormalCompletion({ seq: 1, turn: 1, kind: 'completed' }), true)
  for (const bad of ['aborted', 'blocked', 'error', 'max-tokens', 'interrupted', 'forked']) {
    assert.equal(isNormalCompletion({ seq: 1, turn: 1, kind: bad }), false)
  }
  assert.equal(isNormalCompletion(null), false)
})