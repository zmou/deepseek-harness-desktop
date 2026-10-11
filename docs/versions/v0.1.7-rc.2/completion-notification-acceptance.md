# v0.1.7-rc.2 会话完成通知 — 实施验收记录

> 状态：实施完成，Windows 安装包已产出，待用户安装后交互式验收（见 §6）
> 规格：`docs/versions/v0.1.7-rc.2/desktop-completion-notification-spec.md`
> 事件发现：`docs/versions/v0.1.7-rc.2/completion-notification-event-discovery.md`
> 交付物：`tauri-app/src-tauri/target/release/bundle/nsis/DeepSeek-Harness-Desktop-Setup_v0.1.7-rc.2_x64.exe`（113.5 MB）

## 1. 交付物清单

### 1.1 新增文件

| 路径 | 作用 |
|---|---|
| `desktop-bridge/package.json` | 双面 client 插件清单（`dsh.client.platform="web"` + `exports["./client"]`） |
| `desktop-bridge/lib/index.js` | host 半（空 `apply`，使行进入 Loader 并被 client 扫描识别） |
| `desktop-bridge/lib/client.js` | browser 半：`__ModuleLoader__.load` 注册；订阅 `ctx.uiSession.current`，把「当前查看 session」写入 `window.__dshDesktopViewed__` |
| `tauri-app/src-tauri/src/state.rs` | 未读状态机（去重/未读集合/已读清除/原子持久化） |
| `tauri-app/src-tauri/src/protocol.rs` | dsh 后端 unary RPC 客户端（`session/list`/`projections`/`page`）+ 解析判定 |
| `tauri-app/src-tauri/src/watch.rs` | 运行状态转换检测（首轮基线 + running→false） |
| `tauri-app/src-tauri/src/completion.rs` | 主进程集成：轮询、完成判定、前台判定、徽标、通知、viewed 桥收集与校验 |
| `tauri-app/icons/badges/badge-*.png` | Windows taskbar 数字 overlay 图标（1–9 / 9+ / 99+，32×32） |
| `electron-app/src/unread-state.js` | 未读状态机（与 Rust 同语义） |
| `electron-app/src/dsh-protocol.js` | 协议客户端（Electron `net.request` + `useSessionCookies`） |
| `electron-app/src/completion-monitor.js` | 主进程集成（轮询/判定/前台/徽标/通知/viewed 桥收集与校验） |
| `electron-app/src/bridge-inject.js` | bridge patch 路径解析与 patch 文本构造 |
| `electron-app/icons/badges/badge-*.png` | Windows overlay 数字徽标（同 Tauri） |
| `electron-app/test/*.test.js` | Electron 侧单元测试（`node:test`） |

### 1.2 修改文件

| 路径 | 变更 |
|---|---|
| `tauri-app/src-tauri/Cargo.toml` | 加 `serde`/`serde_json`（直接依赖，版本不变）与 `tauri-plugin-notification = "=2.4.0"`（固定版本：2.5.0 要求 tauri ^2.12 会整体升级 Tauri，规格 §7 禁止） |
| `tauri-app/src-tauri/src/main.rs` | 挂载 4 个新模块；`spawn_dsh` 用 `--profile web --patch <file> --port 0 --no-open`（含插入顺序注释）；bridge patch 生成；completion pump 启动；通知插件注册 |
| `tauri-app/src-tauri/tauri.conf.json` + `tauri.ci.windows.conf.json` | `bundle.resources` 增加 `"../../desktop-bridge/": "bridge/"` |
| `electron-app/src/main.js` | bridge patch 注入（同参数顺序）；启动 completion monitor；`before-quit` 停止监控 |
| `electron-app/electron-builder.yml` | `files` 增加 `icons/badges/**`；`extraResources` 增加 `../desktop-bridge` → `bridge` |

**未改动**：官方 dsh 源码、官方 Web UI bundle、`tauri-app/resources/runtime/` 内容、`~/.dsh`、托盘/单实例/child 生命周期语义。

## 2. 事件来源（规格 §2.2 / §11.1）

- **正常完成**：durable `turn/end` 事件，`reason.kind === 'completed'`（排除 `aborted`/`blocked`/`error`/`max-tokens`/`interrupted`/`forked`）。经 `session/page`（unary HTTP，`throughSeq` = `session/projections.asOfSeq`）读取。
- **完成周期 ID**：`sessionId + turn/end.seq`（附带 `turn`）。
- **当前查看 session**：官方无外部稳定信号（见发现文档 §3.2）。用户裁决采用**方案 2**——通过官方 client 插件机制注入 `desktop-bridge`，在 dsh 前端自身 Cordis ctx 内订阅 `ctx.uiSession.current`（mainView binding source），写入受校验全局队列供主进程收集。**不读 DOM、不解析文案、不改官方 bundle。**

## 3. 实现要点

### 3.1 事件源：HTTP unary 轮询（零 WS）

选此而非 `$events` WebSocket 流的原因：`$events` 会收到 `waterfall` 帧，不回复 `{kind:'next'}` 会**永久卡死官方 UI 的审批/提问**（代码链 + 实测双重确认）；且 `session/follow` 会激活冷会话 Agent。unary 轮询零全局副作用、零 Agent 激活，并复用现有 HTTP 栈（Tauri `ureq` / Electron `net.request`）。

### 3.2 注入机制（实测确认）

- `--patch <file>` 是 launcher 旗标，**必须位于应用旗标之前**，且需**显式 `--profile web`**（两处陷阱均实测踩到并修正）。
- patch 行 `name` 必须指向**文件**（`lib/index.js`）——Node ESM 不支持目录 import。
- 实测：bridge 进入 `__DSH_BOOT__`（65 entries 之一），stderr 无 `did not activate`。

### 3.3 桥的数据通道与校验

- bridge 只写 `window.__dshDesktopViewed__`（限长 500、队尾去重），不含会话内容。
- 主进程原子取出并清空（Tauri `eval_with_callback` / Electron `executeJavaScript`，均带 2s 超时）。
- sessionId 经字符集/长度校验（`[A-Za-z0-9_-]{1,128}`）后才用于清除未读，非法项丢弃。

### 3.4 前台判定（规格 §3.3）

仅 `visible && focused && !minimized` 视为前台；其余（隐藏到托盘/最小化/失焦）按后台，完成时发系统通知。

### 3.5 通知与徽标（规格 §6.2/§6.3）

- **Tauri**：`tauri-plugin-notification`（Rust 主进程直接调用，无需 capability/ACL）；taskbar 徽标用 `WebviewWindow::set_overlay_icon` + 内嵌数字 PNG（`include_image!`），操作的是 **taskbar 应用按钮**而非托盘图标。
- **Electron**：内置 `Notification`；Windows `win.setOverlayIcon(nativeImage)`；macOS `app.dock.setBadge`。零新增依赖。

### 3.6 持久化（规格 §5）

- Tauri：`app_local_data_dir()/completion-unread.json`；Electron：`userData/completion-unread.json`。
- 原子写（temp + rename）；schema version 校验；文件缺失/损坏/未知 schema 均按空状态启动，不阻断应用（有单测覆盖）。
- 未读数 0 时清除徽标；应用退出/窗口隐藏不清除未读。

### 3.7 subagent 会话

`session/list` 中 `origin:'subagent'` 的会话**不单独通知**（其完成随父会话体现）；顶层会话才进入完成判定。

## 4. 单元测试

| 套件 | 数量 | 结果 |
|---|---|---|
| Rust `state.rs`（去重/未读/已读/持久化/损坏恢复/BOM 容忍） | 12 | ✅ |
| Rust `protocol.rs`（解析 + **mock HTTP 端到端**：请求构造/Cookie/JSON body/响应解析/失败映射） | 10 | ✅ |
| Rust `watch.rs`（首轮基线/转换/消失会话/多会话） | 7 | ✅ |
| Rust `completion.rs`（记录/已读/sessionId 校验/队列解析/桥状态解析） | 9 | ✅ |
| **Rust 合计** | **38** | ✅ |
| Electron `unread-state.test.js`（含 BOM 容忍） | 12 | ✅ |
| Electron `dsh-protocol.test.js` | 7 | ✅ |
| Electron `completion-monitor.test.js`（含桥状态解析/敌意输入/旧格式兼容） | 15 | ✅ |
| Electron `bridge-inject.test.js` | 4 | ✅ |
| **Electron 合计** | **38** | ✅ |
| **总计** | **76** | ✅ |

覆盖规格 §10.1 的全部条目：首事件计 1、同事件重放不变、不同 session 递增、查看仅移除该项、已读后再完成重新计数、取消/失败/等待不计入、前台当前会话完成视为已读、前台他会话完成计数。

## 5. Windows 真实运行验收

### 5.1 已完成：真实 dsh 实例上的端到端链路验收（10/10）

在与壳一致的运行时布局（`target/debug/runtime` 镜像 + 真实 `~/.dsh` profile）上，用真实 dsh 实例
逐项验证（`--profile web --patch <bridge patch> --port 0 --no-open`，即壳的实际启动参数）：

| # | 检查项 | 结果 |
|---|---|---|
| 1 | dsh 打印 ready URL | ✅ |
| 2 | token 兑换 cookie（303 + Set-Cookie） | ✅ |
| 3 | 无插件激活失败（`did not activate`） | ✅ |
| 4 | `session/list`（`_request` wire 键） | ✅ `http=200`，返回 108 个会话 |
| 5 | `desktop-bridge` 进入 `__DSH_BOOT__` | ✅ |
| 6 | bridge `client.js` 被列入前端 boot 的 combo bundle | ✅ |
| 7 | `session/create` | ✅ |
| 8 | `session/projections` 返回 `asOfSeq`（作 `throughSeq`） | ✅ |
| 9 | `session/page` 返回 records（含 `turn/end`） | ✅ |
| 10 | 桥队列取出脚本原子「读取并清空」 | ✅ 两次调用分别得 `["session-a","session-b"]` 与 `[]` |

### 5.2 已完成：真实应用进程启动验收

启动真实二进制（测试 identifier，避免与用户正在运行的实例冲突）：

- 日志确认 `[bridge] injecting patch: <userData>/dsh-bridge.patch.yml`（patch 生成到应用数据目录，不写 `~/.dsh`）。
- 日志确认 `[dsh] ready url: ...`、`[dsh] session cookie acquired for downloads`。
- 日志确认 `[notify] completion pump starting; base=http://127.0.0.1:<port>`。
- 持续运行 40s 以上，**无** `session/list failed`（轮询每 3s 一次，等价约 13 轮成功）。

### 5.2.1 已完成：未读状态恢复 + taskbar 数字徽标（真实 Windows 验证）

预置 3 条未读状态后启动真实二进制，日志逐条证实：

```
[notify] completion pump starting; base=http://127.0.0.1:59245
[notify] restoring unread state: count=3
[notify] taskbar badge set to 3
```

- `restoring unread state: count=3`：状态文件加载正确（规格 §5.3 重启恢复）。
- `taskbar badge set to 3`：**`WebviewWindow::set_overlay_icon` 在真实 Windows taskbar 上调用成功**
  （规格 §6.2/§10.4-3：操作的是 taskbar 应用按钮而非托盘溢出区图标）。
- 未读为 0 时走 `taskbar badge cleared` 分支（清除路径同源验证）。

排障中修正的两个真实参数陷阱（均已落入代码注释与 §3.2）：

1. `--patch` 必须位于应用旗标（`--port`/`--no-open`）之前，否则 Web 应用报 `unknown option '--patch'`。
2. 必须用显式 `--profile web`（裸 `web` 缩写仅在作为首个 token 时成立，与 `--patch` 组合报 `--profile <name> is required`）。

排障中另发现并修正的一处健壮性缺陷：

3. **状态文件 BOM 容忍**：外部工具（如 PowerShell `Set-Content -Encoding UTF8`）写出的 JSON 可能带
   UTF-8 BOM；原实现会让整份未读状态被判为损坏而丢弃。两侧均已加 BOM 剥离，并各补一条单测。

### 5.3 尚需交互式确认（环境受限）

以下条目需要「真实模型会话完成」或视觉确认，未在本轮自动化中覆盖，**不虚报为已验证**：

- Windows 通知中心实际弹出 toast 的视觉确认（代码路径已接 `tauri-plugin-notification`，日志 `[notify] system notification shown`）。
- taskbar 数字 overlay 的**视觉**确认（调用已成功，见 §5.2.1；图标像素外观需肉眼确认）。
- 用户查看会话后未读数实际递减的端到端确认（依赖桥在真实 WebView 中 attach，日志 `[notify] viewed-session bridge attached`）。

### 5.4 交付形态

按用户要求，本轮以 **Windows NSIS 安装包**为交付物（由用户安装后自行测试）：

- 打包命令：`powershell -ExecutionPolicy Bypass -File scripts/build-win.ps1`
- 产物：`tauri-app/src-tauri/target/release/bundle/nsis/DeepSeek-Harness-Desktop-Setup_v0.1.7-rc.2_x64.exe`
- 安装包内含：`runtime/`（node + dsh 0.1.7-rc.2）与 `bridge/`（`desktop-bridge` 注入包，
  经 `tauri.conf.json` 的 `bundle.resources` 映射）。

## 6. 规格 §10.4 验收矩阵（逐项状态）

| # | 项目 | 状态 | 证据 |
|---|---|---|---|
| 1 | 隐藏到托盘后继续执行会话 | 沿用既有已验证实现 | 托盘逻辑未改动（§7） |
| 2 | 会话完成时 Windows 系统通知 | 代码路径已验证；视觉待用户确认 | `show_notification`（`tauri-plugin-notification`） |
| 3 | 任务栏应用按钮显示未读数字 | **调用已真实验证**（§5.2.1）；图标外观待确认 | `taskbar badge set to 3` 日志 |
| 4 | 前台完成当前查看会话不通知不加数 | 单测覆盖 | 桥 viewed 清除 + `is_foreground` |
| 5 | 前台查看其他会话不弹通知但更新数字 | 单测覆盖 | `is_foreground` 分支 + 计数用例 |
| 6 | 打开已完成会话后数字递减、全部查看后消失 | 单测覆盖；真实点击待用户确认 | `handle_viewed` + `refresh_badge(0)` 清除 |
| 7 | 重启后未读与徽标恢复 | **已真实验证**（§5.2.1） | `restoring unread state: count=3` 日志 |
| 8 | Electron/macOS Notification Center + Dock badge | 待 macOS 环境 | 代码路径已实现（`Notification` / `dock.setBadge`） |

## 7. 回归确认（规格 §10.5）

- 关闭窗口隐藏到托盘：**未改动**（`CloseRequested` 拦截逻辑保持不变）。
- 托盘「打开主界面」/「退出」：**未改动**。
- 单实例恢复与聚焦：**未改动**。
- child 启动/正常退出/异常退出/清理：**未改动**。
- 下载接管、cookie/token 脱敏、代理抗性：**未改动**。
- 会话数据与官方 dsh CLI 互通：**未改动**（未触碰 `~/.dsh` 会话数据）。

## 8. 已知限制与平台差异

1. **未读清除依赖注入桥**：若 bridge 因环境原因未注入（日志 `[bridge] ... disabled`），完成通知/徽标仍工作，但未读不会随查看自动清除（退化为「仅通知」）。
2. **Windows 徽标为 overlay 图标**：Windows 无 macOS 式 Dock 数字 API，规格 §6.2 指定的 `SetOverlayIcon` 语义即 taskbar 按钮叠加数字图，1–9 精确、≥10 显示 `9+`/`99+`。
3. **subagent 不单独通知**（见 §3.7）。
4. **`--profile web` 显式拼写**：与 `--patch` 组合时裸 `web` 缩写不可用（launcher 行为，已在代码注释记录）。
5. **通知点击行为**不在规格承诺范围（未附加自动导航/清空未读）。

## 9. 未完成项

- 真实桌面会话下的逐项视觉验收（toast 出现、taskbar 数字角标可见、查看后递减）——**由用户在安装后确认**。
- macOS 环境（Electron 兼容壳）的 Notification Center / Dock badge 验收——需 macOS 机器。

## 10. 用户安装后自测指引

1. 运行 `DeepSeek-Harness-Desktop-Setup_v0.1.7-rc.2_x64.exe` 安装（覆盖旧版即可）。
2. 启动应用，日志位于 `%LOCALAPPDATA%\dsh-desktop\dsh-desktop.log`，关键行：
   - `[bridge] injecting patch: ...`（注入桥就绪）
   - `[notify] completion pump starting; base=http://127.0.0.1:<port>`（完成检测已启动）
   - `[notify] viewed-session bridge attached (state=attached)`（桥已在前端挂载）
   - `[notify] taskbar badge set to <n>` / `taskbar badge cleared`（徽标更新）
   - `[notify] system notification shown; unread=<n>`（系统通知已发）
3. 自测路径：
   - 关闭主窗口 → 应隐藏到托盘且 dsh 继续运行；
   - 在托盘中让某会话跑完（或后台跑完）→ 应见 Windows 通知 + taskbar 数字角标；
   - 打开该会话 → 数字应递减，全部查看后消失；
   - 退出并重启应用 → 未读数字应恢复。
