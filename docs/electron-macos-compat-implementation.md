# Electron 兼容版完整实施计划（不分阶段，一次性落地）

> 状态：**待评审**（评审通过前不得开始执行）
> 日期：2026-09-17
> 前置规格：`docs/electron-macos-compat-spec.md`（设计决策与选型已评审通过）
> 本计划覆盖规格文档 §14 全部内容，将原本的阶段 0~4 合并为**一份连贯、无 gate 的一次性实施清单**。

---

## 0. 目标与范围

一次性完成 Electron 兼容版的**全部**交付：

1. 独立 `electron-app/` 工程，完整复刻 Tauri 版全部桌面层能力（规格 §8 对齐清单）
2. 复用 `build-runtime.mjs` 产物，Electron 43.x 锁定
3. `electron-builder` 产出 `DeepSeek-Harness-Desktop_v<ver>_<arch>-electron.dmg`
4. CI 集成 + 门槛守护 + 文档更新
5. 通过规格 §12 全部验收（含 macOS 12 实机）

**边界（不得越界）**：不改 dsh 源码 / 官方前端产物；不改动 `tauri-app/` 任何现有代码；Electron 与 Tauri 两套壳零耦合。

---

## 1. 任务总览

| 编号 | 任务 | 产出 | 依赖 |
|---|---|---|---|
| T1 | 工程骨架初始化 | `electron-app/` 可安装 | — |
| T2 | runtime 复用与路径解析 | `src/runtime.js` | T1 |
| T3 | 主进程启动链路 | `src/main.js`（spawn/管道/URL/窗口） | T2 |
| T4 | 日志脱敏 | `src/redact.js` | T3 |
| T5 | 下载接管「另存为」 | `src/main.js` 下载段 | T3 |
| T6 | 隐藏 Session 弹窗 | `src/hide-session-dialog.js` | T3 |
| T7 | 代理抗性 | `src/main.js` session 段 | T3 |
| T8 | 生命周期与进程守护 | `src/main.js` 生命周期段 | T3 |
| T9 | 打包配置与脚本 | `electron-builder.yml` + `scripts/build-mac-electron.sh` | T2~T8 |
| T10 | macOS 12 实机渲染冒烟 | 验证记录 | T3、T9 |
| T11 | CI 集成与门槛守护 | `.github/workflows/build.yml` | T9 |
| T12 | 文档更新 | README / README.en / CHANGELOG | T9 |
| T13 | 最终验收 | 验收记录 | 全部 |

> T3~T8 都落在 `src/main.js`（及其子模块），实质是同一个文件的各部分，拆开列是为让验收点清晰；实施时可合并提交。

---

## 2. 详细任务清单

### T1 工程骨架初始化

**目标**：`electron-app/` 目录可 `npm install`、能启动最小空壳窗口。

**步骤**：

1. 创建目录结构：
   ```
   electron-app/
   ├── package.json
   ├── electron-builder.yml
   ├── src/
   │   ├── main.js
   │   ├── runtime.js
   │   ├── redact.js
   │   └── hide-session-dialog.js
   ├── icons/            # 从 tauri-app/src-tauri/icons/ 拷贝 icon.icns / icon.png
   └── resources/        # 构建期填充（加入 .gitignore）
   ```
2. `package.json` 关键字段：
   ```json
   {
     "name": "dsh-desktop-electron",
     "version": "0.1.5-rc.1-electron",
     "main": "src/main.js",
     "private": true,
     "scripts": {
       "start": "electron .",
       "dist": "electron-builder --mac dmg"
     },
     "devDependencies": {
       "electron": "43.7.1",
       "electron-builder": "^26.0.12"
     }
   }
   ```
   > `electron` 精确锁 `43.x`（**不得用 `^`**，防止解析到 44 抬高 macOS 门槛；具体 patch 号以 `npm view electron@43 version` 最新为准）。
3. 图标：`cp tauri-app/src-tauri/icons/icon.icns electron-app/icons/`、`cp tauri-app/src-tauri/icons/icon.png electron-app/icons/`。
4. `.gitignore` 追加：`electron-app/node_modules/`、`electron-app/release/`、`electron-app/resources/runtime/`。
5. 临时验证：`npm install` 成功、`npx electron --version` 输出 `v43.x`。

**产出**：可安装依赖的 `electron-app/`。

**验收**：`npm install` 无错；`npx electron --version` 为 43.x。

---

### T2 runtime 复用与路径解析（`src/runtime.js`）

**目标**：定位内嵌 node 与 dsh bin.js，复用 `build-runtime.mjs` 产物。

**规格依据**：§7.2。

**步骤**：

1. 实现 `resolveNode()`：
   - 优先 `process.resourcesPath/runtime/node/node`
   - 回退系统 PATH `node`（dev）
2. 实现 `resolveDshBin()`：
   - 优先级：`DSH_BIN` 环境变量（存在才用）→ `process.resourcesPath/runtime/dsh-runtime/node_modules/@deepseek-ai/dsh/lib/bin.js` → dev 回退 `<repo>/../.stage-p0/node_modules/@deepseek-ai/dsh/lib/bin.js`
3. 若都找不到，抛出含提示的错误（`set DSH_BIN or run build-runtime`）。
4. 导出 `log(msg)`：dev 打 console，全模式追加写 `~/.dsh/dsh-desktop.log`。

**产出**：`src/runtime.js`。

**验收**：`node -e "require('./src/runtime')"` 能解析出路径（dev 环境走回退分支）；路径打印正确。

---

### T3 主进程启动链路（`src/main.js` 核心段）

**目标**：spawn dsh → 泵送管道 → 解析 ready URL → 开窗加载，单实例锁就位。

**规格依据**：§7.1、§7.3、§7.9。

**步骤**：

1. 顶部 `requestSingleInstanceLock()`，拿不到锁即 `app.quit()`；监听 `second-instance` 恢复/聚焦主窗口。
2. `app.whenReady()`：
   - `spawn(resolveNode(), [resolveDshBin(), 'web', '--port', '0', '--no-open'], { env: {...process.env, NODE_OPTIONS: ''} })`
   - `child.stdout.on('data')` 逐行：`redact.logLine()` + 正则 `/dsh web: (http:\/\/[^\s]+)/` 提取 URL
   - `child.stderr.on('data')` 逐行：`redact.logLine()`（**持续消费，防 64KB 管道塞满**）
   - 30s 超时或子进程提前退出 → 写日志 → `app.quit()` 退出
3. 拿到 URL 后 `new BrowserWindow({ width: 1400, height: 900 })`，`win.loadURL(url)`。
4. 失败兜底：`win.webContents.on('did-fail-load')` 记录日志。

**产出**：可跑通「起 dsh → 开窗」的最小闭环。

**验收**：dev 模式（`DSH_BIN` 指向 `.stage-p0`）能拉起 dsh、窗口加载官方 UI 不白屏；日志出现 `ready url`。

---

### T4 日志脱敏（`src/redact.js`）

**目标**：移植 Rust 侧 `redact_url` 规则，行为一致。

**规格依据**：§7.4。

**步骤**：

1. 实现 `redactUrl(text)`：
   - 正则 `https?://[^\s]+` 找出 URL
   - 用 `new URL()` 解析，若 query 含 `token` 则替换值为 `***`
   - 仅当存在 `token` 参数时才改写，否则原样返回
2. 实现 `logLine(line)` / `logLines(chunk)`：追加写 `~/.dsh/dsh-desktop.log`，dev 同时 console。

**产出**：`src/redact.js`。

**验收**：单测几条样例（含/不含 token 的 URL、`dsh web: http://.../?token=xxx` 整行）输出符合预期。

---

### T5 下载接管「另存为」（`src/main.js` 下载段）

**目标**：会话导出走系统另存为对话框，记住上次目录，成功写盘。

**规格依据**：§7.6。

**步骤**：

1. `session.defaultSession.on('will-download', (event, item) => { ... })`：
   - `const url = item.getURL()`，从 `sessionId` query 生成安全文件名（规则与 Tauri `session_zip_filename` 一致：保留 `[A-Za-z0-9_-]`，其余 `_`，`dsh-session-<id>.zip`）
   - `dialog.showSaveDialog(mainWindow, { defaultPath: join(记住目录 || app.getPath('downloads'), filename), filters: [{ name: 'ZIP 压缩文件', extensions: ['zip'] }] })`
   - 确认 → 记住 `dirname(filePath)`（写到 `app.getPath('userData')/last-download-dir.txt`）→ `item.setSavePath(filePath)`
   - 取消 → `item.cancel()`
   - `item.once('done')` 记录成功/失败（脱敏 URL）
2. 读取/写入记住目录：`last-download-dir.txt`（纯文本一行路径，目录失效则回退系统下载目录）。

**产出**：下载接管完整可用。

**验收**：导出会话 → 弹另存为 → 选目录写 zip 成功；再次导出默认定位到上次目录；取消不落盘。

---

### T6 隐藏 Session 弹窗（`src/hide-session-dialog.js`）

**目标**：隐藏官方 `SessionLogDownloadDialog` 乐观弹窗及遮罩，与 Tauri 版行为一致。

**规格依据**：§7.7。

**步骤**：

1. 复刻 ARIA 选择器规则：
   ```css
   [role="dialog"][aria-modal="true"][aria-label*="Session" i]{display:none!important}
   :has(>[role="dialog"][aria-modal="true"][aria-label*="Session" i]){display:none!important}
   ```
2. `win.webContents.on('did-finish-load')` 时 `insertCSS(css)`；兜底用 `executeJavaScript` 注入同款 MutationObserver（复刻 Tauri 常量逻辑）。
3. 导出 CSS 常量与注入函数。

**产出**：`src/hide-session-dialog.js`。

**验收**：导出会话时不再出现「Session 导出已开始下载」弹窗，且无残留半透明遮罩。

---

### T7 代理抗性（`src/main.js` session 段）

**目标**：Chromium 层不走系统代理，避免 localhost 流量被挟持。

**规格依据**：§7.8。

**步骤**：

1. `session.defaultSession.setProxy({ mode: 'direct' })`
2. `app.commandLine.appendSwitch('no-proxy-server')`

**产出**：无代理干扰。

**验收**：开启系统代理（如 Clash）环境下，UI 加载与 WebSocket 正常。

---

### T8 生命周期与进程守护（`src/main.js` 生命周期段）

**目标**：关窗回收 node 进程；node 异常退出关窗；无残留。

**规格依据**：§7.9。

**步骤**：

1. `child.on('exit', (code, signal) => { 非 0 退出 → 关闭所有窗口 })`
2. `app.on('window-all-closed', () => { killChild(); app.quit() })`
3. `app.on('before-quit', () => { killChild() })`（兜底）
4. `killChild()` 幂等：`child && child.kill()`。

**产出**：生命周期闭环。

**验收**：关闭窗口后 `ps` 无残留 node；手动 `kill` node 后窗口自动关闭。

---

### T9 打包配置与脚本

**目标**：electron-builder 产出合规 dmg，版本号 `-electron` 后缀，门槛守护。

**规格依据**：§9、§10、§11.3。

**步骤**：

1. `electron-builder.yml` 完整配置（详见规格 §9.1，要点：`appId: com.deepseek.dsh-desktop-electron`、`extraResources` 指向 `../tauri-app/resources/runtime` → `runtime`、mac `target: [dmg]`、dmg 拖拽布局、图标）。
2. `scripts/build-mac-electron.sh`：
   ```
   1) 依赖检查：node / npm / hdiutil
   2) node scripts/build-runtime.mjs            # 复用（已存在则跳过）
   3) cd electron-app && npm install             # 锁 electron@43.x
   4) 同步版本号：从 build-runtime.mjs 读 DSH_VERSION，写入 electron-builder.yml / package.json（拼接 -electron）
   5) npx electron-builder --mac dmg
   6) 断言产物 Info.plist 的 LSMinimumSystemVersion ≤ 12.0（不满足即 fail）
   7) 重命名 dmg → DeepSeek-Harness-Desktop_v<ver>_<arch>-electron.dmg
   ```
3. 版本三处同步逻辑抽成脚本内部函数，升级 dsh 时与 `build-win.ps1 -DshVersion` 联动（后续单独补 `-DshVersion` 参数或复用同一版本源）。

**产出**：可一键出包的脚本 + dmg 产物。

**验收**：`bash scripts/build-mac-electron.sh` 出 `DeepSeek-Harness-Desktop_v0.1.5-rc.1_aarch64-electron.dmg`；门槛断言通过。

---

### T10 macOS 12 实机渲染冒烟

**目标**：在目标机器（macOS 12.7.6）验证 Electron 版无白屏、功能闭环。

**步骤**：

1. 在 macOS 12 实机安装 dmg 打开。
2. 验证：官方 UI 完整渲染、能建会话发消息、WebSocket 稳定、下载「另存为」成功（cookie 自动携带）。
3. 若发现 Chromium 150 对 dsh 前端仍有差异，记录并评估是否需调整选型（规格 §13 R4）。

**产出**：实机验证记录。

**验收**：无白屏，核心链路全部可用。

---

### T11 CI 集成与门槛守护

**目标**：`.github/workflows/build.yml` 新增 macOS Electron job，自动化出包 + 门槛断言。

**规格依据**：§11。

**步骤**：

1. 新增 job `build-macos-electron`（`macos-latest`）：
   - checkout → setup-node 22 → `node scripts/build-runtime.mjs`
   - `cd electron-app && npm ci && npx electron-builder --mac dmg`
   - 门槛断言（§11.3）
   - `upload-artifact` 命名 `dsh-desktop-macos-electron`
2. 与现有 Tauri macOS job 并行（共享 runtime 产物步骤）。

**产出**：CI 自动出 Electron 版 dmg。

**验收**：推 tag 触发 CI，两个 macOS 产物（Tauri + Electron）均成功上传。

---

### T12 文档更新

**目标**：用户可见信息同步。

**步骤**：

1. `README.md`：下载表加「Electron 兼容版」列 + 选版指引（macOS 12 及以下选 Electron 版）+ 构建章节加 `build-mac-electron.sh`。
2. `README.en.md`：同步。
3. `CHANGELOG.md`：`[Unreleased]` 记录 Electron 兼容版。
4. `docs/` 文档索引（README 文档表）加两份新文档链接。

**产出**：文档同步。

**验收**：README 下载表含两版 dmg 及清晰选版说明。

---

### T13 最终验收

**目标**：规格 §12 全项通过。

**步骤**：

1. 逐条过 §12.1 功能验收（8 项）。
2. 过 §12.2 自动化检查（`node --check`、门槛断言）。
3. 过 §12.3 实机清单（macOS 12.7.6 + 可选 macOS 13+ 各一台）。
4. 记录结果，关闭本计划。

**产出**：验收记录。

---

## 3. 执行顺序与依赖

```
T1 ─► T2 ─► T3 ─┬─► T4
                 ├─► T5
                 ├─► T6
                 ├─► T7
                 └─► T8
                      │
        T3 + T9 ──► T10 ──► T11
                      │
        T9 ──────────► T12
                      │
              全部 ──► T13
```

- T2~T8 都依赖 T1；T4~T8 依赖 T3（共享 `main.js` 与主进程状态）
- T10 依赖 T3（能起窗口）与 T9（有可安装产物）二者任一满足即可先做渲染验证；最终以 T9 产物为准
- T11、T12 依赖 T9；T13 依赖全部

## 4. 关键实现提醒（踩坑点，务必遵守）

1. **electron 依赖精确锁 43.x**，禁用 `^`，否则解析到 44 → macOS 12 装不上（规格 §5.1）
2. **管道必须持续泵送**，否则 64KB 缓冲塞满导致 dsh 假死（沿用 Tauri 版历史 bug，规格 §7.3）
3. **`NODE_OPTIONS` 必须清除**，否则 IDE 注入的 `--require` shim 卡死 dsh boot（规格 §7.1）
4. **下载走 Chromium cookie jar**，无需 Rust 侧手动换 cookie（规格 §7.5，这是 Electron 版的主要简化点）
5. **门槛断言** `LSMinimumSystemVersion ≤ 12.0` 必须在打包脚本和 CI 都做（规格 §11.3）
6. **不显式设 `minimumSystemVersion`**，继承 Electron 43 自身要求（规格 §9.1）
7. 图标直接复用 `tauri-app/src-tauri/icons/` 源文件，不新制（规格 §6）
8. `electron-app/resources/runtime/` 加入 `.gitignore`，不提交产物（规格 §6）

## 5. 完成定义（Definition of Done）

全部满足才视为完成：

1. `electron-app/` 独立可装、可一键出包，与 `tauri-app/` 零耦合
2. 规格 §8 对齐清单 12 项全部对齐
3. macOS 12.7.6 实机无白屏、核心链路可用
4. CI 自动出两版 macOS dmg（Tauri + Electron）
5. 文档与 CHANGELOG 同步
6. `git diff` 确认 `tauri-app/` 与 dsh 源码零改动

## 6. 参考

- 设计决策与选型：`docs/electron-macos-compat-spec.md`
- 能力对齐清单：`docs/electron-macos-compat-spec.md` §8
- 风险与应对：`docs/electron-macos-compat-spec.md` §13
- Tauri 版参考实现：`tauri-app/src-tauri/src/main.rs`
