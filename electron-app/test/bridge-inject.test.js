'use strict'

/**
 * bridge-inject 纯函数单元测试（node:test）。
 * 运行：node --test electron-app/test/bridge-inject.test.js
 */

const { test } = require('node:test')
const assert = require('node:assert/strict')
const path = require('node:path')
const { pathToFileURL } = require('node:url')

const {
  bridgeEntryCandidates,
  pickBridgeEntry,
  buildPatchContent,
  bridgePatchPath,
} = require('../src/bridge-inject')

test('candidates include packaged resources and dev repo paths', () => {
  const resourcesPath = path.join('C:', 'app', 'resources')
  const appPath = path.join('C:', 'app', 'resources', 'app.asar')
  const repoRoot = path.join('D:', 'repo')
  const candidates = bridgeEntryCandidates({ resourcesPath, appPath, repoRoot })
  assert.ok(candidates.some((c) => c.endsWith(path.join('bridge', 'lib', 'index.js'))))
  assert.ok(candidates.some((c) => c.endsWith(path.join('desktop-bridge', 'lib', 'index.js'))))
  assert.ok(candidates.some((c) => c.startsWith(repoRoot)))
  assert.ok(candidates.some((c) => c.startsWith(resourcesPath)))
})

test('pickBridgeEntry returns first existing', () => {
  const existing = new Set(['b', 'c'])
  assert.equal(pickBridgeEntry(['a', 'b', 'c'], (p) => existing.has(p)), 'b')
  assert.equal(pickBridgeEntry(['a', 'b'], () => false), null)
})

test('buildPatchContent uses file URL and insert row', () => {
  const entry = path.join('D:', 'repo', 'desktop-bridge', 'lib', 'index.js')
  const content = buildPatchContent(entry)
  assert.match(content, /^- insert:\n/)
  assert.match(content, /id: dsh-desktop-bridge/)
  assert.ok(content.includes(pathToFileURL(entry).href))
  // 反斜杠不应出现在 name 中（file URL 规范化）
  const nameLine = content.split('\n').find((l) => l.includes('name:'))
  assert.ok(!nameLine.includes('\\'))
})

test('bridgePatchPath lives under userData', () => {
  assert.equal(bridgePatchPath('C:/userdata'), path.join('C:/userdata', 'dsh-bridge.patch.yml'))
})