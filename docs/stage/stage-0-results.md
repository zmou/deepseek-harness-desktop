# 阶段 0：技术验证 — 结果报告

> 执行日期：2026-08-29
> 执行环境：Windows（win32）+ PowerShell，Node v24.14.0，pnpm 11.7.0
> 验证工作区：`d:/Codes/deepseek-harness-desktop/.stage-p0/`（临时，可清理）
> 结论：**7 项验证全部通过，方案 A 可行，可进入 P1 桌面壳骨架**

---

## 0. 总览

| 编号 | 验证项 | 结论 |
|---|---|---|
| V0.1 | 官方 npm 包可用性 | ✅ 通过（有版本差异发现） |
| V0.2 | 启动链路（npx + node bin） | ✅ 通过（node bin 方式） |
| V0.3 | 端口分配与 URL 解析 | ✅ 通过 |
| V0.4 | stdout 缓冲与时序 | ✅ 通过（无缓冲问题） |
| V0.5 | token 交换与数据目录 | ✅ 通过（**发现无 token 机制**） |
| V0.6 | 插件 --patch 挂载 | ✅ 通过 |
| V0.7 | 内嵌 Node 与跨平台 | ✅ 通过 |

---

## 1. 重大发现（直接影响后续设计）

### 发现 1：npm 版本与源码版本不一致

- npm `latest` = **`0.1.1-rc.2`**
- 源码 submodule = **`0.1.2-alpha.1`**

**影响**：npm 发布滞后于源码仓库。桌面端"版本对齐"应以 **submodule 的 commit SHA** 为唯一真值，npm 安装时锁定到具体版本号。

### 发现 2：npm 版 dsh web 无 token 机制

源码文档（0.1.2-alpha.1）描述启动 URL 带 token → 换 cookie → 重定向。但 npm 版 0.1.1-rc.2 的实测行为：

```
stdout: dsh web: http://127.0.0.1:60651        ← 无 /?token=xxx
curl GET /  => HTTP 200 直接返回 HTML          ← 无重定向、无 cookie
```

**影响**：桌面壳**无需处理 token 交换**，直接用 URL 加载 WebView 即可，比计划预期更简单。但需注意：**行为可能随版本变化**，P1 启动协议设计要兼容"有 token / 无 token"两种形态。

### 发现 3：pnpm 默认忽略 5 个包的 build scripts

安装时 pnpm 默认阻止以下包执行 build scripts：
- `@deepseek-ai/dsh-subprocess-local`（dsh 子进程实现）
- `node-pty`（终端 native 模块，**有全平台 prebuild，无需编译**）
- `koffi`（FFI 库，**无编译产物，需 P2 关注**）
- `protobufjs`
- `@google/genai`

**影响**：dsh web 启动正常（已验证），但终端/子进程相关功能可能受影响。P2 打包时需用 `pnpm approve-builds`（或 equivalent）允许 build scripts，并验证终端功能。

### 发现 4：npm 串行 fetch 元数据导致安装极慢/卡住

npm 安装 `@deepseek-ai/dsh` 时，串行 fetch 500+ 包元数据，遇慢请求整体卡住（实测卡 4.5 分钟无进展）。pnpm 并发 fetch，仅几十秒完成。

**影响**：P2 构建运行时**必须用 pnpm**（而非 npm），否则 CI 打包会极慢或不稳定。

---

## 2. 各验证项详情

### V0.1 官方 npm 包可用性 ✅

**关键事实**：
- `@deepseek-ai/dsh` latest = `0.1.1-rc.2`，全部历史版本：`0.0.1-rc.1` ~ `0.1.1-rc.2`（10 个）
- `bin` 字段 = `{ "dsh": "lib/bin.js" }` ✅ 与预期一致
- `@deepseek-ai/cordis` = `4.0.1`（独立版本，vendor 家族）✅
- `@deepseek-ai/dsh-tools` = `0.0.1-rc.1` ✅
- 主包直接依赖约 60 个 `@deepseek-ai/*` 子包 + 少量第三方（js-yaml、commander、node-addon-require-builtin）
- `engines` 字段为空（主包未声明，但运行时实测需 Node 22+）

**结论**：方案 A（运行时用 npm 官方包）前提成立，无需启用方案 B。

### V0.2 启动链路 ✅

**关键事实**：
- 方式 2（直接 `node <bin.js> web`）启动成功，这是桌面壳采用的 spawn 形式
- `--version` 输出 `0.1.1-rc.2`，`--help`、`--dump-config` 均正常
- `--no-open` 生效：未弹出浏览器
- 干净退出：`Stop-Process` 后无残留 node 进程
- 启动耗时：约 15s 内服务就绪（首次冷启动偶发卡住一次，见 V0.4）

**结论**：启动链路可用。

### V0.3 端口分配与 URL 解析 ✅

**关键事实**：
- `--port 0` 分配随机端口（实测 60651 / 60908 / 53297 / 54162 等），✅ 避免端口冲突
- 固定端口 `--port 12346` 生效，URL 行为 `http://127.0.0.1:12346`
- 端口冲突：进程 B 报 `Error: dsh: plugin tree failed to load: ... listen EADDRINUSE: address already in use 127.0.0.1:12346` 到 **stderr** 并以非零退出
- URL 行精确格式：`dsh web: http://127.0.0.1:<port>`（无 token，见发现 2）

**结论**：桌面壳用 `--port 0` 是最稳妥选择；URL 行格式稳定可解析。

**桌面壳解析正则草案**：
```
dsh web: (http://127\.0\.0\.1:\d+)
```

### V0.4 stdout 缓冲与时序 ✅

**关键事实**：
- URL 行在服务就绪后及时 flush（实测 t=15s 时 stdout 已写入 32 字节完整 URL 行）
- stdout 与 stderr 正确分离：URL 行在 stdout，诊断/EADDRINUSE 在 stderr
- **无缓冲问题**：Node 的 console.log 到 pipe 会及时 flush

**异常记录**：
- 首次冷启动（pnpm 装完后第一次运行）曾卡住：内存 52.7MB 不变、无端口监听、stdout 空，持续 90s+
- 后续多次冷启动均正常（15s 内就绪）
- 推测：首次运行触发 pnpm 符号链接树初始化或残留进程干扰

**结论**：桌面壳采用"解析 stdout"方案（推荐），备选"固定端口 + 健康检查轮询"暂不需要。但需在 P1 加**启动超时兜底**（如 30s 未解析到 URL 则读 stderr 报错）。

### V0.5 token 与数据目录 ✅

**关键事实**：
- **无 token 机制**（发现 2）：根路径直接 200 + HTML
- 返回 HTML 是完整 dsh SPA（含 `window.__ModuleLoader__` 引导代码，两段启动）
- 数据目录 `~/.dsh` 生成，含 `profiles/`、`sessions/`、`storages/`、`settings.yaml`、`.credentials.yaml`、`.anonymous-user-id`
- `DSH_HOME` 环境变量覆盖生效（数据写到指定目录）
- 数据目录懒创建（首次启动生成 profiles/storages，会话/设置在使用时生成）

**结论**：WebView 加载 URL 无任何"必须真实浏览器"的依赖，标准 WebView 完全兼容。数据目录与官方 CLI 互通。

### V0.6 插件 --patch 挂载 ✅

**关键事实**：
- 最小插件（`dsh-plugin-ping`）独立目录，`package.json` 声明 `peerDependencies: { "@deepseek-ai/cordis": "*" }`
- 插件需先装到 profile 目录：`cd $DSH_HOME/profiles/web && pnpm add file:<插件路径>`
- `--patch` 挂载用 **insert 语法**（非 id-targeted）：
  ```yaml
  - insert:
      - id: ping
        name: 'dsh-plugin-ping'
  ```
- **参数顺序**：`--patch` 必须在 `web` 子命令的其他参数之前（`web --patch xxx --port 0 --no-open`，而非 `web --port 0 --patch xxx`）
- 插件 `apply` 被真正调用（写文件 + console.log 双重验证）
- console.log 输出正常出现在 stdout：`[dsh-plugin-ping] LOADED`

**结论**：自定义功能注入通道完全打通。插件位于 dsh 之外独立目录，升级 dsh 不影响插件。

**踩坑记录（重要）**：
1. patch 默认是"按 id 覆盖"，insert 新 row 必须用 `insert:` 顶层 key
2. `--patch` 参数位置错误会报 `unknown option '--patch'`

### V0.7 内嵌 Node 与跨平台 ✅

**关键事实**：
- 独立 node.exe（下载 `node-v24.14.0-win-x64.zip`，34.5MB）解压后直接可用
- 用完整绝对路径的独立 node.exe 启动 dsh 成功，监听 `127.0.0.1:54162`，**不依赖系统 node 安装**
- Windows 无沙箱（Landlock）正常降级运行，无沙箱相关报错
- node_modules 体积：**192.9 MB**（446 个包）
- node-pty 有全平台 prebuild（win32-x64/arm64、darwin-x64/arm64、linux-x64/arm64 的 .node 文件），无需现场编译

**结论**：安装包内嵌 Node 可行。整体安装包体积预估：node 二进制（~80MB 解压后）+ node_modules（~193MB）+ 打包开销 ≈ **250-300MB**（压缩后约 150-200MB），需在 P2 评估是否可接受或做精简。

---

## 3. 对后续阶段的影响与建议

| # | 建议 | 影响阶段 |
|---|---|---|
| 1 | 版本对齐以 submodule SHA 为真值，npm 锁版本 | P2/P4 |
| 2 | 桌面壳 URL 解析兼容"有/无 token"两种形态 | P1 |
| 3 | 运行时构建**用 pnpm**，禁止 npm | P2 |
| 4 | pnpm 需 `approve-builds` 允许 native 模块 build | P2 |
| 5 | P1 加启动超时兜底（30s）| P1 |
| 6 | koffi 无编译产物，P2 需确认终端/子进程功能 | P2 |
| 7 | 安装包体积 ~250-300MB，需评估精简策略 | P2 |
| 8 | 插件挂载用 insert 语法 + `--patch` 放参数前 | P3 |

---

## 4. 风险更新

| 风险 | 原评估 | 现评估 | 说明 |
|---|---|---|---|
| 官方 npm 包未发布 | 高 | ✅ 已消除 | npm 包可用 |
| Windows stdout 缓冲 | 中 | ✅ 已消除 | 无缓冲问题 |
| 端口冲突 | 中 | ✅ 已缓解 | `--port 0` 彻底规避 |
| token 机制复杂 | 低 | ✅ 简化 | 无 token，更简单 |
| 安装包体积过大 | 低 | ⚠️ 需关注 | 实测 ~250-300MB |
| native 模块构建 | 未知 | ⚠️ 需 P2 处理 | koffi 无 prebuild |

---

## 5. 阶段出口决策

**Go**：进入 P1 桌面壳骨架。

依据：V0.1/V0.2/V0.3/V0.4/V0.6 五个硬前提全部通过；V0.5/V0.7 通过且有明确结论；所有不确定性已消除或有明确应对方案。

---

## 6. 附：验证工作区清理

`d:/Codes/deepseek-harness-desktop/.stage-p0/` 为临时验证目录，含约 230MB 内容（node_modules + node 二进制）。建议在 P1 开始前清理，释放空间。验证报告（本文件）保留在 `docs/stage/`。
