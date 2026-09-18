# Electron 兼容版实施方案规格（macOS 12 及以下）

> 状态：**待评审**（评审通过前不得开始实施）
> 日期：2026-09-17
> 关联文档：`docs/dsh-desktop-analysis.md`、`docs/implementation-plan.md`、`tauri-app/README.md`

---

## 0. 摘要

在 macOS 12（Monterey）及更老的机器上，Tauri 版依赖的系统 WKWebView 引擎停留在 Safari 15.4 水平（WebKit 17613），无法解析 dsh 官方前端 bundle 中的现代语法（正则 lookbehind、动态 import 第二参数），导致模块加载即抛 `SyntaxError`，React 永不挂载 → 白屏。

本方案为这些老机器**独立增加一条 Electron 打包链路**：Electron 自带 Chromium 引擎，不依赖系统 WebKit，已验证可完整解析官方前端。两套打包方式**完全独立、互不影响**，macOS 发布时同时产出「Tauri 默认版」与「Electron 兼容版」两个 dmg，用户按系统版本自选。

---

## 1. 背景与问题根因

### 1.1 现象

macOS 12 用户打开软件后白屏，无任何 UI。

### 1.2 根因（已实证）

| 环节 | 结论 | 证据 |
|---|---|---|
| dsh 子进程启动 | ✅ 正常 | 日志显示 node 起来、ready URL 解析到、cookie 换到 |
| WebView ↔ 本地服务网络 | ✅ 正常 | `lsof` 显示 WKWebView 与 server 有 5 条 ESTABLISHED 连接 |
| **渲染引擎解析 JS** | ❌ **失败** | 系统 JSC（= WKWebView 引擎）解析两个 bundle 均 `SyntaxError` |

系统 WebKit 版本 `17613`（Safari 15.4 等价，2022-03）。macOS 12 的系统 WebKit 停在此处；用户即便安装新版 Safari App，**WKWebView 仍只用系统自带 WebKit 框架**，不受 Safari App 版本影响。

官方前端 bundle（`dsh-web-frontend/dist/assets/`）两处语法硬伤：

1. `vendor-*.js`：lookbehind 正则 `(?<=^|\s|\p{P}|\p{S})` —— JavaScriptCore 到 Safari 16.4 才支持
2. `index-*.js`：动态 import 带第二参数（import attributes）—— 需 Safari 17.2+ / Chromium 123+

二者都是 `type="module"` 脚本，**解析阶段**即崩溃，导致整棵模块图加载失败、`<div id="root">` 永远为空。

### 1.3 验证结论

| 引擎 | index bundle | vendor bundle |
|---|---|---|
| 系统 JSC（WebKit 17613，WKWebView 同款） | `SyntaxError: import call expects exactly one argument` | `SyntaxError: Invalid regular expression: invalid group specifier name` |
| V8（Chromium 线，Electron/Chrome 同款） | **PARSE OK** | **PARSE OK** |

→ 换 Chromium 引擎即可根治。

---

## 2. 目标与非目标

### 2.1 目标

1. 新增一条**独立**的 Electron 打包链路，产出面向 macOS 12 及以下的兼容版 dmg
2. Electron 版**完整复刻** Tauri 版全部桌面层能力（见 §8 对齐清单）
3. 两套链路互不影响：Tauri 版保持现状零改动，Electron 版自洽
4. macOS 发布同时产出两个 dmg，用户按系统版本自选下载

### 2.2 非目标（明确不做）

- 不修改 dsh 任何源码 / 官方前端产物（维持「零侵入」红线）
- 不改动 Tauri 版任何现有代码
- 不支持 macOS 11 及以下（本轮确认，见 §4 决策记录）
- 不做 Electron 版自动更新（与 Tauri 版一致，二期再议）
- Windows / Linux 不引入 Electron（WebView2 / WebKitGTK 无此问题）

---

## 3. 总体架构

```
┌──────────────────────────────────────────────────────────────┐
│                        macOS 发布                            │
│                                                              │
│  ┌───────────────────┐          ┌──────────────────────┐     │
│  │ Tauri 版（默认）   │          │ Electron 版（兼容）    │     │
│  │ WKWebView 引擎     │          │ Chromium 引擎         │     │
│  │ 系统 WebKit 17613  │          │ 自带（Chromium 150）   │     │
│  │ macOS 13+ 用户     │          │ macOS 12 及以下用户     │     │
│  └─────────┬─────────┘          └───────────┬──────────┘     │
│            │                                │                │
│            │    共享同一份 dsh 运行时        │                │
│            └──────────┬─────────────────────┘                │
│                       ▼                                      │
│      tauri-app/resources/runtime/                            │
│      ├── node/           内嵌 Node v24.14.0（独立二进制）     │
│      └── dsh-runtime/    @deepseek-ai/dsh@<锁定版本>          │
│                       │                                      │
│                       ▼                                      │
│      spawn `node <dsh bin.js> web --port 0 --no-open`       │
│      （两套壳共用同一启动协议，与官方 1:1）                     │
└──────────────────────────────────────────────────────────────┘
```

### 3.1 分层与独立性

沿用 `implementation-plan.md` 的三层解耦原则：

- **L1 上游（dsh）**：只读，两套壳都只通过「进程 + stdout + HTTP」交互，零源码侵入
- **L2 桌面壳**：`tauri-app/`（Rust）与新增 `electron-app/`（Node）**完全平行**，互不 import、互不引用对方产物
- **运行时（runtime）**：`build-runtime.mjs` 产出的 `resources/runtime/` 由两套壳**共享复用**（只读输入）

**红线**：
1. Electron 壳与 Tauri 壳各自独立，不共享任何运行时代码（仅共享 runtime 产物与文档规则）
2. 不改 dsh 源码 / 官方前端产物
3. Electron 版不 import Tauri 版任何模块

---

## 4. 决策记录（已确认）

| # | 决策点 | 结论 |
|---|---|---|
| D1 | 支持范围 | **仅 macOS 12**（不含 macOS 11 / 10.15） |
| D2 | 能力范围 | **完整复刻** Tauri 版全部桌面层能力 |
| D3 | 版本号 | 与 dsh 同步 + `-electron` 后缀 |
| D4 | 体积 | 接受 ~200MB dmg |
| D5 | 运行时复用 | 复用 `build-runtime.mjs` 产物（内嵌独立 node），不用 Electron 内置 Node 跑 dsh |
| D6 | 打包工具 | electron-builder |
| D7 | Electron 版本 | 锁定 **43.x**（详见 §5.1） |

---

## 5. 技术选型

### 5.1 Electron 版本锁定（关键决策）

版本约束来自两个方向的夹逼：

| 约束 | 要求 | 对应版本 |
|---|---|---|
| 语法下限 | lookbehind（Chromium ≥ 62）、import attributes（Chromium ≥ 123） | Electron ≥ 30 |
| macOS 12 支持下限 | Electron 38 起要求 macOS 12 | Electron ≥ 38 |
| **macOS 12 支持上限** | Chromium 151 起放弃 macOS 12（Chrome 官方公告，2026-07-28） | **Electron ≤ 43** |

**结论：锁定 Electron 43.x** —— 它是支持 macOS 12 的最后一档，且组件版本完全满足需求：

- Chromium `150.0.7871.46`（≥123，语法完全覆盖）
- Node `24.17.0`（满足 dsh `engines: ^22.19 || >=24`）
- V8 `15.0`

**风险与应对**：Electron 官方仅维护最新 3 个稳定版，43 会随 44/45/46 发布逐步 EOL、失去安全更新。此代价已被 D4 接受，应对措施见 §13 R4。

**CI 强制约束**：构建脚本必须断言打包产物中 `Electron Framework` 的 `LSMinimumSystemVersion ≤ 12.0`，防止依赖锁升级（如 `^43` 误解析到 44）悄悄抬高门槛。做法见 §11.3。

### 5.2 打包工具

选 **electron-builder**（`@electron/packager` 亦可，但 builder 的 dmg/签名/公证一体化更省事）：

- 产出：`dmg` + `.app`
- 支持 `extraResources` 把 runtime 打进 app
- 支持 dmg 自定义（背景、Applications 快捷方式）与 Tauri 版观感对齐

### 5.3 运行时方案（D5）

**复用** `scripts/build-runtime.mjs` 产物 `tauri-app/resources/runtime/`（内嵌 `node` v24.14.0 + `dsh-runtime`），**不**使用 Electron 内置 Node 跑 dsh。理由：

1. 与 Tauri 版 100% 一致，启动协议、路径、日志行为完全相同
2. node v24.14 明确满足 dsh `engines`，不绑定 Electron 版本
3. 规避 `ELECTRON_RUN_AS_NODE` 的隐患：dsh 内部若用 `process.execPath` fork 子进程（worker_threads / subprocess），在 Electron 二进制下会误启 GUI

> 备选（本轮**不做**）：用 `ELECTRON_RUN_AS_NODE` 复用 Electron 内置 Node，可省 ~121MB（dmg 可再小约 50MB），但引入 fork 行为不确定性，POC 阶段若有富余可另行评估。

### 5.4 渲染进程架构

与 Tauri 版一致：**无前端源码**，主进程 spawn dsh 后直接 `BrowserWindow.loadURL(readyUrl)`，1:1 还原官方 Web UI。`electron-app/` 下不维护任何 SPA 代码。

---

## 6. 目录结构

```
deepseek-harness-desktop/
├── tauri-app/                          # L2 桌面壳（Rust，现状，零改动）
├── electron-app/                       # L2 桌面壳（Node，新增，完全独立）
│   ├── package.json                    # electron + electron-builder 依赖与脚本
│   ├── electron-builder.yml            # 打包配置（dmg / extraResources / 签名）
│   ├── src/
│   │   ├── main.js                     # 主进程：spawn/管道/URL/窗口/下载/单实例/生命周期
│   │   ├── runtime.js                  # 定位内嵌 node + dsh bin.js（复用 runtime 产物）
│   │   ├── redact.js                   # 日志脱敏（移植 Rust 侧 redact_url 规则）
│   │   └── hide-session-dialog.js      # 隐藏官方 Session 导出弹窗（复刻 Tauri 常量）
│   ├── icons/                          # 应用图标（复用 tauri-app/src-tauri/icons/ 源文件）
│   └── resources/                      # 构建期由 build-runtime.mjs 产物填充（gitignore）
│       └── runtime/{node, dsh-runtime}
├── scripts/
│   ├── build-runtime.mjs               # 现状，共享复用（零改动）
│   └── build-mac-electron.sh           # 新增：Electron 版一键打包
└── .github/workflows/build.yml         # 新增 macOS Electron 构建 job（见 §11）
```

> `electron-app/resources/runtime/` 与 `tauri-app/resources/runtime/` 指向同一套 `build-runtime.mjs` 输出，构建脚本内部用拷贝/软链组装，不复制进 git。

---

## 7. 主进程详细设计（`electron-app/src/main.js`）

主进程是唯一入口，职责与 Tauri 版 `main.rs` 一一对应。以下为设计规格，代码以实施阶段为准。

### 7.1 启动流程

```
1. 单实例锁：app.requestSingleInstanceLock()
   - 失败 → app.quit()；'second-instance' 事件 → 恢复/聚焦主窗口
2. app.whenReady() 后：
   a. runtime.resolveNode() / resolveDshBin()   —— 复用 resources/runtime，回退 DSH_BIN 环境变量
   b. spawn dsh：child_process.spawn(node, [bin, 'web', '--port', '0', '--no-open'], { env: 剔除 NODE_OPTIONS })
   c. 管道泵送：stdout/stderr 逐行 on('data')，经 redact.js 脱敏后写日志
   d. 解析 ready URL：正则 `dsh web: (http://[^\s]+)`，30s 超时且子进程已退则报错退出
   e. 创建 BrowserWindow(1400×900)，loadURL(readyUrl)
   f. 注入「隐藏 Session 弹窗」CSS/脚本（§7.7）
3. 子进程退出监听：child.on('exit')，异常退出 → 关闭所有窗口
4. 退出回收：before-quit / window-all-closed → kill 子进程
```

### 7.2 路径解析（`runtime.js`）

| 优先级 | node | dsh bin.js |
|---|---|---|
| 1 | — | `DSH_BIN` 环境变量（存在才用） |
| 2 | `<app>/Contents/Resources/runtime/node/node` | `<app>/Contents/Resources/runtime/dsh-runtime/node_modules/@deepseek-ai/dsh/lib/bin.js` |
| 3 | 系统 PATH 的 node | dev 回退 `.stage-p0/...` |

> Electron 打包后 `process.resourcesPath` 即 `Contents/Resources`，比 Tauri 的 `resource_dir()` 更直白，无需 `normalize_path` 的 `\\?\` 处理（那是 Windows 专用）。

### 7.3 管道泵送

- `child.stdout.on('data')` + `child.stderr.on('data')`，逐行 `redact.logLine()` 后写日志文件
- **关键**：必须持续消费，防止管道缓冲塞满导致 dsh 事件循环假死（沿用 Tauri 版踩坑结论）
- ready URL 从 stdout 行中正则提取

### 7.4 日志脱敏（`redact.js`）

移植 `main.rs` 的 `redact_url` 规则，行为一致：

1. 用正则 `https?://[^\s]+` 找出文本中的 URL
2. 解析 query，若含 `token` 参数则替换其值为 `***`，其余参数保留
3. 日志文件路径与 Tauri 版一致：`~/.dsh/dsh-desktop.log`（macOS/Linux）

### 7.5 会话 cookie

**无需任何额外处理**。Chromium 网络栈自带 cookie jar，`loadURL(readyUrl)` 首次访问时自动完成 `GET /?token=… → 303 + Set-Cookie` 的兑换，后续下载请求自动携带 cookie。

> 对比：Tauri 版因 WKWebView/下载走两条栈，必须 Rust 侧用 ureq 手动换 cookie；Electron 版下载也走同一 session，此复杂度**直接消除**。

### 7.6 下载接管（「另存为」）

```
session.defaultSession.on('will-download', (event, item, webContents) => {
  1. 从 item.getURL() 解析 sessionId → 生成安全文件名（规则与 Tauri session_zip_filename 一致）
  2. 阻止默认行为：等待用户选择
  3. dialog.showSaveDialog(mainWindow, {
       defaultPath: path.join(记住的上次目录 || 系统下载目录, filename),
       filters: [{ name: 'ZIP 压缩文件', extensions: ['zip'] }]
     })
  4. 用户确认 → 记住本次目录 → item.setSavePath(filePath)
  5. 用户取消 → item.cancel()
  6. item.on('done') 记录成功/失败（脱敏 URL）
})
```

「记住上次下载目录」在本方案中**跨平台可用**（不再像 Tauri 版那样只认 `LOCALAPPDATA`），目录状态文件路径统一走 `app.getPath('userData')` 或 `~/.dsh`。

### 7.7 隐藏 Session 导出弹窗（`hide-session-dialog.js`）

复刻 Tauri 版 `HIDE_SESSION_DIALOG_SCRIPT` 常量，方式二选一（实施阶段定）：

- 方案 A：`webContents.insertCSS(css)`（`did-finish-load` 时注入，ARIA 选择器 `[role="dialog"][aria-modal="true"][aria-label*="Session" i]` 隐藏）
- 方案 B：`webContents.executeJavaScript` 注入同款 MutationObserver 脚本

> 与 Tauri 版保持同一套 ARIA 选择器与兜底逻辑，避免两版行为漂移。

### 7.8 代理抗性

- `session.defaultSession.setProxy({ mode: 'direct' })`，并 `app.commandLine.appendSwitch('no-proxy-server')`
- 原因同 Tauri 版：避免本机代理（Clash 等）把 localhost 流量挟持导致 WebSocket 卡死
- 补充：LLM 等外网请求由 dsh node 进程发起（不走 Chromium 栈），故 Chromium 层设直连不影响模型调用

### 7.9 单实例 / 生命周期

- 单实例：`app.requestSingleInstanceLock()` + `second-instance` → `win.show()/focus()`
- 生命周期：
  - `window-all-closed` → kill 子进程 → `app.quit()`（macOS 下与 Tauri 行为一致，关闭窗口即回收）
  - `before-quit` → 兜底 kill 子进程
  - 子进程异常退出 → 关闭所有窗口

---

## 8. 与 Tauri 版能力对齐清单

| # | 能力 | Tauri 版实现 | Electron 版实现 | 对齐 |
|---|---|---|---|---|
| 1 | 1:1 还原官方 Web UI | WKWebView loadURL | BrowserWindow loadURL | ✅ |
| 2 | 内嵌 Node 运行时 | resources/runtime | 复用同一产物 | ✅ |
| 3 | 数据互通 `~/.dsh` | 不覆盖 DSH_HOME | 不覆盖 DSH_HOME | ✅ |
| 4 | 单实例恢复/聚焦 | single-instance 插件 | requestSingleInstanceLock | ✅ |
| 5 | 下载「另存为」+ 记住目录 | 取消下载 + 自 fetch | will-download + setSavePath | ✅（更简） |
| 6 | 日志 token 脱敏 | redact_url | redact.js 移植 | ✅ |
| 7 | 隐藏 Session 弹窗 | init script + CSS | insertCSS / 脚本 | ✅ |
| 8 | 代理抗性 | --no-proxy-server（仅 Win 有效） | setProxy(direct)（全平台） | ✅（mac 补强） |
| 9 | 进程守护（异常退关窗） | 轮询 try_wait | child.on('exit') | ✅ |
| 10 | 管道防塞泵送 | 双线程 pump | on('data') 泵送 | ✅ |
| 11 | NODE_OPTIONS 清除 | env_remove | 剔除 env | ✅ |
| 12 | 会话 cookie 兑换 | ureq 手动换 | Chromium 自动 | ✅（更简） |

---

## 9. 打包与构建

### 9.1 electron-builder 配置要点（`electron-builder.yml`）

```yaml
appId: com.deepseek.dsh-desktop-electron
productName: DeepSeek Harness Desktop
# 版本号在脚本中由 dsh 版本拼接 -electron（见 §10）
directories:
  output: release
  buildResources: icons
files:
  - src/**
  - package.json
extraResources:
  # 复用 build-runtime.mjs 产物，打进 Contents/Resources/runtime/
  - from: ../tauri-app/resources/runtime
    to: runtime
mac:
  target: [dmg]
  category: public.app-category.developer-tools
  icon: icons/icon.icns
  # 签名：默认 ad-hoc；正式分发填证书（与 Tauri 版一致，先占位）
  # identity: null
dmg:
  title: DeepSeek Harness Desktop
  contents:
    - { x: 130, y: 220 }
    - { x: 410, y: 220, type: link, path: /Applications }
```

> `minimumSystemVersion` **不显式设置**，继承 Electron 43 自身的最低要求（macOS 12）；CI 侧断言其 ≤ 12.0（§11.3）。

### 9.2 一键打包脚本（`scripts/build-mac-electron.sh`）

```
1. 检查依赖：node / npm / cargo 无需 / hdiutil
2. node scripts/build-runtime.mjs          # 复用现有产物（若已存在则跳过）
3. cd electron-app && npm install          # 安装 electron + electron-builder（锁版本）
4. npx electron-builder --mac dmg          # 出 .app + .dmg
5. 校验产物 Info.plist 的 LSMinimumSystemVersion ≤ 12.0
6. 重命名 dmg 为 DeepSeek-Harness-Desktop_<ver>_<arch>-electron.dmg
```

### 9.3 版本命名（§10 细则）

- dmg 文件名：`DeepSeek-Harness-Desktop_<dsh版本>_<arch>-electron.dmg`
- app 内部 `CFBundleShortVersionString`：`<dsh版本>-electron`
- `productName` / 应用显示名：`DeepSeek Harness Desktop`（与 Tauri 版一致，仅靠 dmg 文件名区分）

---

## 10. 版本策略

- 主版本 = 内置 `@deepseek-ai/dsh` 版本（沿用现有约定）
- Electron 版额外追加 `-electron` 后缀，用于：
  1. dmg 文件名区分两版
  2. app 内部版本号（便于用户与日志、`关于` 面板识别兼容版）
- 升级 dsh 版本时，**两套壳同步升级**：改 `build-runtime.mjs` 的 `DSH_VERSION` + `tauri.conf.json`/`Cargo.toml` + `electron-builder.yml` 版本位（脚本自动同步，见 §11.4）

---

## 11. CI 设计（`.github/workflows/build.yml`）

### 11.1 新增 macOS Electron job

在现有 macOS job 基础上拆分/新增（与 Tauri 版并行，共享 `build-runtime.mjs` 步骤）：

```yaml
- job: build-macos-electron
  runs-on: macos-latest
  steps:
    - checkout
    - setup node 22
    - node scripts/build-runtime.mjs        # 与 Tauri 版共享产物
    - cd electron-app && npm ci && npx electron-builder --mac dmg
    - assert LSMinimumSystemVersion <= 12.0   # 门槛守护
    - upload-artifact dsh-desktop-macos-electron
```

### 11.2 产物命名

- Tauri 版：`**/*.dmg`（现有）
- Electron 版：新增独立 artifact 名 `dsh-desktop-macos-electron`，dmg 重命名带 `-electron` 后缀

### 11.3 门槛守护（关键）

构建脚本 / CI 断言：

```bash
VER=$(plutil -p "<app>/Contents/Frameworks/Electron Framework.framework/Resources/Info.plist" \
      | grep LSMinimumSystemVersion | awk -F'"' '{print $2}')
# 必须 <= 12.0；否则说明依赖锁解析到了 44+，直接 fail
```

### 11.4 版本三处同步

升级 dsh 时一次性同步（沿用现有 `build-win.ps1 -DshVersion` 思路，扩展脚本覆盖）：

1. `scripts/build-runtime.mjs` 的 `DSH_VERSION`
2. Tauri：`tauri.conf.json` / `Cargo.toml`
3. Electron：`electron-app/electron-builder.yml` 或构建脚本版本位

---

## 12. 测试与验收标准

### 12.1 功能验收（Definition of Done）

1. macOS 12.7.6 实机：双击 Electron 版 dmg 安装后打开，**无白屏**，官方 UI 完整渲染
2. 能正常创建会话、发送消息、WebSocket 稳定（与 Tauri 版在新机器上的表现一致）
3. 会话导出走「另存为」对话框，且能记住上次目录、成功写入 zip
4. 日志中 token 全部为 `***`，可安全粘贴
5. 官方 Session 导出乐观弹窗被隐藏，遮罩不残留
6. 重复启动 → 恢复已有窗口，无第二实例
7. 关闭窗口 → 无残留 node 进程；手动 kill node → 窗口自动关闭
8. 数据与官方 CLI / Tauri 版互通（同一 `~/.dsh`）

### 12.2 自动化检查

- `node --check electron-app/src/*.js`（语法）
- CI 门槛断言 `LSMinimumSystemVersion ≤ 12.0`
- 产物冒烟：CI 内用 `open -a` + 截图比对（可选，首版可手动）

### 12.3 老机器实测清单（发布前必做）

- macOS 12.7.6（目标机）
- 可选：macOS 13+ 一台，确认 Electron 版在新技术系统也正常（不应只跑老机器）

---

## 13. 风险与应对

| # | 风险 | 等级 | 应对 |
|---|---|---|---|
| R1 | Electron 43 逐步 EOL，Chromium 无安全更新 | 高 | 已接受；每半年评估一次（见 R4）；文档显著提示 |
| R2 | 依赖锁 `^43` 误升 44 → 老机器装不上 | 高 | CI 断言 `LSMinimumSystemVersion ≤ 12.0`（§11.3） |
| R3 | 两套壳能力漂移（后续改一处忘另一处） | 中 | §8 对齐清单作为 checklist；改能力必须同时更新两版 + 测试 |
| R4 | Chromium 150 对 dsh 前端仍有未知兼容差异 | 中 | POC 阶段用 Chromium 150 实机渲染验证（§14 阶段 0） |
| R5 | 体积 ~200MB 对老机器磁盘/带宽压力 | 中 | 已接受；可选后续用 ELECTRON_RUN_AS_NODE 优化（D5 备选） |
| R6 | dsh 官方升级后前端用更新语法 | 低 | 语法门槛随 Chromium 150 已远超现状；升级时回归测试 |
| R7 | 公证/签名流程两套都要配置 | 低 | 与 Tauri 版共用同一套证书/公证脚本 |

---

## 14. 实施计划（分阶段）

> 阶段 0 完成后**必须回来评审**再继续，POC 未通过不得进入阶段 1。

### 阶段 0：POC 渲染验证（0.5 天）

- 用 Electron 43 空壳 `loadURL(readyUrl)`，在 macOS 12 实机确认：
  - 官方 UI 完整渲染、无白屏
  - WebSocket 连通、能发消息
  - `will-download` 下载能成功（cookie 自动携带）
- 产出：结论 + 是否需要调整技术选型

### 阶段 1：主进程完整实现（1~2 天）

- 按 §7 实现 `main.js` / `runtime.js` / `redact.js` / `hide-session-dialog.js`
- 覆盖 §8 全部能力

### 阶段 2：打包链路（1 天）

- `electron-builder.yml` + `build-mac-electron.sh` + 复用 `build-runtime.mjs`
- 产出 dmg，命名/版本号落地

### 阶段 3：CI 与文档（0.5~1 天）

- `.github/workflows/build.yml` 新增 macOS Electron job + 门槛断言
- README 下载表加「Electron 兼容版」列 + 选版指引
- CHANGELOG 记录

### 阶段 4：验收（0.5~1 天）

- §12 全部验收项过一遍（重点 macOS 12 实机）

---

## 15. 附：关键伪代码骨架

> 仅示意设计意图，实施阶段以可运行代码为准。

```js
// electron-app/src/main.js（骨架）
const { app, BrowserWindow, session, dialog } = require('electron')
const { spawn } = require('child_process')
const { resolveNode, resolveDshBin } = require('./runtime')
const redact = require('./redact')

const gotLock = app.requestSingleInstanceLock()
if (!gotLock) { app.quit() }

let child = null

function spawnDsh() {
  child = spawn(resolveNode(), [resolveDshBin(), 'web', '--port', '0', '--no-open'], {
    env: { ...process.env, NODE_OPTIONS: '' },
  })
  let url
  child.stdout.on('data', (buf) => {
    for (const line of buf.toString().split('\n')) {
      redact.logLine(line)
      const m = line.match(/dsh web: (http:\/\/[^\s]+)/)
      if (m) url = m[1]
    }
  })
  child.stderr.on('data', (buf) => redact.logLines(buf.toString()))
  // 30s 超时 / 提前退出的兜底逻辑
  return url
}

app.whenReady().then(() => {
  const url = waitReadyUrl()
  const win = new BrowserWindow({ width: 1400, height: 900, webPreferences: {} })
  session.defaultSession.setProxy({ mode: 'direct' })
  session.defaultSession.on('will-download', (e, item) => {
    // §7.6 另存为逻辑
  })
  win.loadURL(url)
  win.webContents.on('did-finish-load', () => win.webContents.insertCSS(HIDE_SESSION_DIALOG_CSS))
})

app.on('second-instance', () => { /* focus win */ })
app.on('before-quit', () => { child?.kill() })
```

---

## 16. 下一步

1. 评审本规格（尤其 §4 决策、§5.1 版本锁定、§8 对齐清单）
2. 通过后启动 **阶段 0（POC）**
3. 阶段 0 结论回评审，再决定是否进入完整实现
