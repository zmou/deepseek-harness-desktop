# 会话完成通知 — 事件发现记录

> 状态：进行中（事件发现；协议细节已确认，待 viewed-session 信号结论与一次实测样本）
> 版本：v0.1.7-rc.2
> 对应规格：`desktop-completion-notification-spec.md` §2.2 / §12 第 1 步
> 本记录回答规格 §2.2 的五个前置问题，并给出事件来源结论。
> 协议细节部分由两个独立探查代理完成（含对本地运行实例的只读探测）。

## 1. 官方 dsh 运行时事件模型（已确认）

### 1.1 会话完成事件

dsh web 的会话是**持久化 durable 事件日志**（append-only，JSON），由 `@deepseek-ai/dsh-session` 定义（`lib/types/types.d.ts` `SessionEventMap`）。

正常完成信号是 **`turn/end` 事件**：

```ts
'turn/end': {
  turn: number;
  reason: TurnEndReason; // {kind:'completed'|'aborted'|'blocked'|'error'|'max-tokens'|'interrupted'|'forked', ...}
};
```

- `reason.kind === 'completed'` 表示一次会话执行**正常完成**（规格 §1.2 要求的语义）。
- `aborted`（取消）、`error`（失败）、`blocked`（等待审批被拒/归档）、`interrupted`（崩溃遗留）等**不**计入完成。
- 每个事件携带 `seq`（单调序列号）与 `turn`（turn 序号）——**足以构成完成周期 ID**（规格 §4.2：去重键 = session ID + 完成周期/turn ID；`seq` 是更严格的单调键）。

### 1.2 会话状态事件（转发事件）

Host 通过 Cordis 事件总线发射，经 API Gateway 的 `$events` 逻辑流转发到浏览器（`@deepseek-ai/dsh-api-remotes/lib/types/remote-events.d.ts`）：

- `api-session/status(sessionId, running: boolean)` — Agent running 状态变化（`@deepseek-ai/dsh-api-session-controller/lib/index.js` 从 `agent/status` 转发：`status === "running"`）。
- `api-session/error(sessionId, message)` — Agent 失败（从 `agent/error` 转发）。
- `api-session/added(sessionId, summary)` / `removed(sessionId)` / `activity(sessionId, updatedAt)`。

注意：`api-session/status` **只有 running boolean**，无法区分正常完成/取消/失败/等待；`turn/end reason.kind` 才能精确区分。

### 1.3 「当前查看 session」信号

- 前端 UI 用 `retainedBy.mainView > 0` 表达「主界面当前查看的 session」（`@deepseek-ai/dsh-client-ui-session/lib/client.js` `isMain()` / `publishMain()`），由 UI 导航 retain/release 维护。
- 前端**不使用 URL hash / pathname / pushState 表达当前 session**（bundle 静态确认：`location.hash`、`history.pushState`、`location.pathname` 出现次数为 0）。
- `UiSession.sessionStatus` observable 含 `completionUnread: boolean`（`observeRunning`：running→false 且非 mainView 时标记）。**该语义不区分完成原因**（任何停止都算），与规格 §1.2 冲突，不能直接复用。
- （待确认）前端是否有官方桌面桥接钩子能把「当前查看 session」/session 状态暴露给壳。

### 1.4 传输层

- 浏览器与 Host 的 RPC：**HTTP POST**，`POST /api/<namespace>/<method>`，body `{type:'client-request', rpcId, method, payload}`，响应 `{type:'server-response', rpcId, result:{ok, value|error}}`（`@deepseek-ai/dsh-client-connection/lib/client.js` `createWebConnectionRpc`）。
- WebSocket 流：`/api/remote.mux`，逻辑流协议 `RemoteStreamClientMessage`（open/item/end/cancel）+ `RemoteStreamServerMessage`（item/error/end）（`@deepseek-ai/dsh-api-gateway/lib/types/stream-protocol.d.ts`）。
- `$events` 流 = 转发事件流；`session/follow` = 单会话 durable 事件流；`session/control` = 全局 projections 流。
- 认证：进程级 launch token 换 HttpOnly cookie（现有 `exchange_session_cookie` 已实现）。trust fence 允许**非浏览器客户端走 loopback**（`@deepseek-ai/dsh-client-connection/lib/types/api-request-trust.d.ts`）。
- `session/page` 是 unary RPC，返回 `SessionPage`（`records: SessionHistoryRecord[]` 含原始 durable 事件，**含 `turn/end`**），输入 `address` + `throughSeq`/`beforeSeq`/`turnWindow`（host 侧 `page()` handler 确认）。

## 2. 传输与认证（已确认，含对运行实例的实测）

### 2.1 认证

- 入口 token 只在页面 URL（`GET /?token=...` → `303` + `Set-Cookie`），WS/RPC **不接受 token**。
- 浏览器会话 cookie 名 `dsh-auth-<hash>`，值 `v1.<payload>.<hmac>`；HttpOnly + SameSite=Strict。
- **WS 只认 `Cookie` 头**（无 token 参数）：`ws://127.0.0.1:<port>/api/remote.mux`（`dsh-api-gateway/lib/types/client/stream-client.js:441-449`）。
- trust fence（`dsh-client-connection/lib/index.js:205-219`）：Host 为 loopback（127.0.0.0/8、localhost、[::1]）即放行；`origin` 缺失直接放行 → **非浏览器客户端可直连**。实测：无 cookie → 401；Host=evil → 403；伪造 cookie → 401。
- token 可重复兑换（常量时间比较，非一次性）→ 主进程可随时重新 `GET /?token=` 换 cookie（Tauri 现有 `exchange_session_cookie` 正是此做法）。Electron 用 `net.request({useSessionCookies:true})` 直接复用 jar。
- Node/undici 与 ws 库均可显式带 `Cookie` 头（实测 ✔）。
- **心跳**：服务端每 2s ping，连续 2 次无 pong 即 terminate（`dsh-api-gateway/lib/index.js:172,236-253,551`）。ws/undici 自动回 pong；自研客户端必须回。

### 2.2 流协议（`/api/remote.mux`）

- 只承载 stream，**拒收 unary**（`index.js:746-749` `gateway/signature-invalid`）。
- 打开消息（`RemoteStreamClientMessage`）：`{"type":"open","streamId":"<任意非空>","endpoint":"<名>","payload":{...}}`，**必须恰好 4 键**；重复 streamId 直接关 socket（`index.js:123-128,316`）。
- 下行外层统一 `{"type":"item","streamId":"...","value":...}` / `{"type":"end",...}` / `{"type":"error",...}`。
- `$events` 流：
  - 打开 payload 必须恰为 `{"args":{}}`（`REMOTE_EVENT_STREAM_PAYLOAD`；`index.js:802` 严格校验 args 为空对象）。
  - 第一帧必为 `ready`：`{"type":"ready","clientId":"<uuid>","host":{"home":"..."}}`。
  - `emit` 帧：`{"type":"emit","event":"api-session/status","args":["<sid>",true]}`（args 为事件原参数）。
  - **`waterfall` 帧：`{"type":"waterfall","event":"approval/request|user-questions/request","eventId":"<uuid>","agentId":"...","request":{...}}` — 必须回复！** 连接建立时所有 pending waterfall 会立即投给新客户端；不回 `{kind:'next'}`，官方 UI 审批/提问永久挂起（`api-remotes/lib/index.js:140-232`、`dsh-api-gateway/lib/index.js:814,899-909`）。回复走 unary：`POST /api/$events/result`，body `{"type":"client-request","rpcId":"r","method":"$events/result","payload":{"args":{"clientId":"<ready 的 clientId>","eventId":"<waterfall 的 eventId>","outcome":{"kind":"next"}}}}`。
- `session/follow`：
  - 打开 payload：`{"args":{"request":{"address":{"kind":"session","sessionId":"<id>"},"maxMessages":50,"turnWindow":{"minMessages":1,"minTurns":1}}}}`。
  - 下行：先 `snapshot`（`{type:'snapshot', header, cursor, records, hasMore, projections, assistantStream?}`，records = 尾部页含 `turn/end`；`cursor` = 日志尾 seq），随后严格连续 `event` 帧（`{type:'event', event: SessionWireEvent}`）。
  - seq 跳号抛 `gateway/internal`。
- `session/control`：打开 payload `{"args":{}}`；首帧 `baseline`（全局 projections），后续 `projection` 帧。

### 2.3 unary RPC（HTTP POST）

- `POST /api/<namespace>/<method>`，`Content-Type: application/json`（`dsh-client-connection/lib/index.js:673-702`）。
- 请求体：`{"type":"client-request","rpcId":"<任意非空>","method":"session/list","payload":{"args":{...}}}`；`method` 必须等于 URL 末段。
- 响应体：`{"type":"server-response","rpcId":"<回显>","result":{"ok":true,"value":...}}`；失败 `{"result":{"ok":false,"error":{...}}}`，HTTP 仍 200；**rpcId 必须自比对**。
- **payload args 键名来自生成描述符（wire 名），不能随意命名**：
  - `session/list` → `{"_request":{}}`（下划线！）
  - `session/page` / `session/follow` → `{"request":{...}}`
  - `session/projections` → `{"request":{"sessionId":"..."}}`
  - 多/少一个键都报 `gateway/arguments-invalid`。
- `session/page`：`{"address":..., "throughSeq": <必需>}`；**`throughSeq:-1` 返回空页**（paginate 证明）；正确用法是拿 follow snapshot 的 `cursor` 当 `throughSeq` 往回翻页。records 含全部事件类型（`pageRecords` 原样包 `turn/end`）。

## 3. 结论

### 3.1 已确认可用的稳定、非侵入信号

- ✅ **正常完成**：durable `turn/end` 事件 `reason.kind==='completed'`，通过 `session/follow`（WS 流，snapshot 自带尾部页）或 `session/page`（unary，需 cursor）可读。这是稳定、官方、无官方源码改动的信号。
- ✅ **完成周期 ID**：`sessionId + turn/end.seq`（或 `turn`）——稳定、单调、可去重。
- ✅ **主进程可直连**：受信任 loopback 客户端，Cookie 头认证，非浏览器无 Origin 限制（§2）。
- ✅ **运行状态**：`session/list`（unary）返回每 session 的 `running`；`session/control` 流/`$events` 可观察 running 变化（`api-session/status`，仅布尔 running）。

### 3.2 未找到稳定、非侵入的信号：当前查看 session

规格 §2.2 问题 3「哪个稳定事件能表示用户当前实际打开/正在查看的 session ID」：

- ❌ 无 URL 路由：bundle 全程无 `location.hash`/`pathname`/`pushState`（唯一命中是 PDF 阅读器页码，与 session 导航无关）。
- ❌ 不在转发事件流：`API_REMOTE_FORWARDED_EVENTS` 白名单只有 `api-session/{added,removed,status,error,activity}`，无 selection/mainView 事件；`session/control` projections 亦不含。
- ❌ `mainView` 仅存在于 Cordis ctx 内存（`retainedBy.mainView`，由 `dsh-client-ui-workspace` 的 UI 导航 retain/release），无任何包写到全局。
- ❌ 无官方桌面桥可订阅 UI 内部事件：唯一「壳→前端」桥范本是 `__DSH_LOCALE__`（单向读 + `onChange` 回调），**无订阅内部会话事件/状态的官方扩展点**。`dshDesktopBoot`（`.ready()`/`.failed()`）是另一桌面产品线契约，本 Tauri 仓库未使用，且它只注入 boot 状态，不含事件订阅。
- 另确认：`completionUnread`（前端 `UiSession`）语义与规格冲突——它把**任何** running→false 都标 unread（不区分完成/取消/失败），直接复用会产生误报。

### 3.3 触发规格暂停条款

规格 §4.4（line 162-166）：「如果官方 UI 没有稳定的『当前查看 session』信号，不得用 DOM 文本推断；按第 2.2 节暂停并报告。」

规格 §2.2（line 73-77）：「如果找不到稳定且非侵入的事件来源，暂停后续实现，向项目负责人提交发现结果、不可行原因和替代方案；不得以 DOM 轮询或脆弱启发式替代。」

→ **本功能缺少「用户查看/清除未读」这一半的稳定信号；完成/通知一半可稳定实现。** 是否继续、用什么替代方案，需向项目负责人（用户）呈报决策。

## 4. 架构方向（事件源已定稿）

- **主进程事件源：HTTP unary 轮询（无 WS）**。理由：
  - `session/list`（unary，wire 键 `_request`）低频轮询发现会话与 running 变化。
  - running→false 时，`session/projections`（unary，`{"request":{"sessionId":"..."}}`）返回 `asOfSeq`（= `cursorBefore(session.seq)` = **日志尾 seq**，`dsh-session-projection/lib/index.js:153`），**可直接作为 `session/page` 的 `throughSeq`**。
  - `session/page`（unary）返回尾部 records（含 `turn/end`），判定 `reason.kind==='completed'`。
  - **零 WS、零 waterfall 副作用、零 Agent 激活**（page/projections 均冷安全，不 promote）。
  - 复用现有 `ureq`（HTTP），无需新增 WS 依赖。
- **subagent 会话**：`session/list` 含 `origin:'subagent'` 项；`session/page` 对其需要 `{kind:'subagent', parentSessionId, childSessionId, mode}`。通知场景只对**顶层会话**发通知（subagent 完成随父会话体现），记录于验收。
- **已读判定**：依赖 client 插件注入桥（用户已裁决方案 2）——插件在前端 Cordis ctx 内订阅 `ctx.uiSession` 拿真实 `mainView`（当前查看 sessionId），经受校验全局桥回传主进程。
- **前台判定**：桌面壳原生窗口状态（`is_focused()`/`is_visible()`/`is_minimized()`），不依赖 WebView。
- **未读持久化**：Tauri `%LOCALAPPDATA%`（`app_local_data_dir`）/ Electron `userData`，不碰 `~/.dsh`；原子写（temp+rename）。
- **通知/徽标**：Tauri 需新增最小依赖（Windows toast + taskbar overlay）；Electron 内置 `Notification` + `setOverlayIcon`/`dock.setBadge` 零依赖。

## 5. 方案 2 落地（注入机制已实测验证）

用户裁决采用方案 2（官方 client 插件注入）。注入链已用真实 dsh 实例逐项验证：

1. **bridge 包结构**（`desktop-bridge/`，双面膜）：`package.json` 的 `dsh.client.platform="web"` +
   `exports["./client"]`，host 半 `lib/index.js` 空 `apply`，browser 半 `lib/client.js`
   用 `window.__ModuleLoader__.load({id, factory})` 注册。
2. **注入方式**：`dsh --patch <file>`（launcher 旗标）。patch 文件内容
   `- insert: - {id: dsh-desktop-bridge, name: <file URL>}`。
   - ⚠️ **`--patch` 必须位于 `web` 之前**：launcher 只解析首个未识别 token 之前的自身旗标，
     `web` 之后原样交给 Web 应用（把 `--patch` 放后面实测报 `unknown option '--patch'`）。
   - ⚠️ `name` 必须指向**文件**（`lib/index.js`）而非包目录：Node ESM 不支持目录 import
     （实测目录形式报 `ERR_UNSUPPORTED_DIR_IMPORT` → dsh 报 `failed to import`，
     bridge 不进 boot graph）。
3. **实测证据**（隔离 `DSH_HOME`，未触碰用户 `~/.dsh` 与正在运行的实例）：
   - patch dump：`--patch` 行被规范化为 `file:///D:/.../desktop-bridge` 并出现在 composition 中。
   - **运行时**：`__DSH_BOOT__` 的 65 个 entries 中包含 `dsh-desktop-bridge`；
     其 `client.js` 被并入 combo bundle 列表；stderr 无 `did not activate` 诊断。
   - **协议契约**（真实实例）：`session/list`（`_request`）→ 200 + rpcId 回显；
     `session/projections` → `asOfSeq=2`；`session/page`（`request` + `throughSeq`）→ records 可取。
4. **规格 §8 合规**：patch 文件生成到**应用数据目录**，bridge 包随壳分发
   （Tauri `bundle.resources` → `bridge/`；Electron `extraResources` → `bridge/`），
   **不写 `~/.dsh`、不改 `tauri-app/resources/runtime/`、不改官方 bundle**。

### 5.1 桥的数据通道（受校验）

- bridge 的 browser 半在 `apply(ctx)` 内订阅 `ctx.uiSession.current`（mainView binding
  source，`createBindingSource` 提供 `value`/`getSnapshot`/`subscribe`），把「当前查看
  sessionId 变化」推入 `window.__dshDesktopViewed__`（数组，限长 500，队尾去重）。
- 主进程周期**原子取出并清空**该队列（Tauri `eval_with_callback` / Electron
  `executeJavaScript`），对每个 id 做字符集/长度校验后才用于清除未读（规格 §7）。
- 不读 DOM、不解析文案、不推断 UI——数据来自前端自身 Cordis 状态。

## 6. 待办

- [x] 协议/传输细节全确认（含实测）
- [x] 前端 viewed-session / 桌面桥 / completionUnread 语义（无官方信号；completionUnread 不区分完成原因）
- [x] 用户裁决：方案 2（官方 client 插件注入）
- [x] client 插件注入机制可行性验证（bridge 进入 `__DSH_BOOT__`，含两处参数陷阱）
- [x] 独立状态逻辑（Rust `state.rs` + Electron `unread-state.js`）
- [x] 协议客户端 + 转换检测（Rust `protocol.rs`/`watch.rs` + Electron `dsh-protocol.js`）
- [x] Tauri 主进程集成（`completion.rs`：轮询/判定/前台/徽标/通知/viewed 桥）+ `--patch` 注入
- [x] Electron 主进程集成（`completion-monitor.js`/`bridge-inject.js`）+ `--patch` 注入
- [x] 打包配置包含 bridge（tauri.conf resources / electron-builder extraResources）
- [ ] Windows 真实验收（toast / taskbar 数字角标 / viewed 清除）+ 回归 + 验收记录
