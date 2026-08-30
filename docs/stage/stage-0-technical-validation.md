# 阶段 0：技术验证 — 执行计划

> 状态：待执行
> 日期：2026-08-29
> 上级计划：`docs/implementation-plan.md`（阶段 0，P0）
> 目标：用最小代价消除方案中的所有"不确定"，为 P1 桌面壳骨架提供已验证的事实基础

---

## 0. 概述

### 0.1 目标

在**不写任何桌面壳代码**的前提下，通过一系列命令级验证，确认以下核心假设全部成立：

1. 官方 npm 包 `@deepseek-ai/dsh` 已发布且可安装、可启动（方案 A 的前提）
2. `dsh web --port 0 --no-open` 启动协议稳定，stdout 可被可靠解析出实际 URL
3. Windows 下 Node stdout 管道缓冲不会导致 URL 延迟/丢失（本方案最大技术风险）
4. token 交换 + cookie 机制与系统 WebView 完全兼容
5. `--patch` 能挂载独立 cordis 插件（自定义功能的注入通道）
6. 内嵌 Node 二进制的方式在目标平台可行

### 0.2 完成标准（Definition of Done）

以下 7 个验证项（V0.1 ~ V0.7）**全部通过**，且每项产出可复现的记录：

- 所有"预期结果"与实际一致
- 所有"通过标准"达成
- 验证结论写入 `docs/stage/stage-0-results.md`（验证报告）
- 若某项失败，记录失败原因 + 已选定的备选方案

### 0.3 时间预算

| 项 | 预算 |
|---|---|
| 纯验证执行 | 0.5 天 |
| 失败排查与备选方案确认 | 0.5 天 |
| **合计** | **1 天** |

### 0.4 环境现状（已确认）

| 项 | 值 | 是否满足 |
|---|---|---|
| 操作系统 | Windows（win32）+ PowerShell | — |
| Node | v24.14.0 | ✅ 满足 `^22.19 \|\| >=24` |
| npm | 11.9.0 | ✅ |
| pnpm | 11.7.0 | ✅ |
| corepack | 0.34.6 | ✅ |
| 网络 | 需能访问 npm registry | 待验证 |

---

## 1. 通用前置准备

### 1.1 创建验证工作区

所有验证在独立目录进行，**不污染桌面端仓库**：

```powershell
# 在工作区外建一个独立验证目录
$stage = "D:/Codes/dsh-p0-validation"
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Set-Location $stage
```

### 1.2 验证网络与 registry 可达性

```powershell
npm ping
```

- 预期：返回 `PONG`
- 失败排查：检查代理/镜像配置（`npm config get registry`），确保指向官方 `https://registry.npmjs.org/`

### 1.3 记录基线信息（写入验证报告开头）

```powershell
node --version
npm --version
npm view @deepseek-ai/dsh version          # 关键：官方 npm 最新版本
npm view @deepseek-ai/dsh versions --json  # 全部历史版本
```

---

## 2. 验证任务

> 说明：每个任务独立可执行；建议按顺序执行，因为 V0.1 的产出是 V0.2~V0.5 的前置。

---

### V0.1 官方 npm 包可用性验证

**目的**：确认方案 A（运行时用 npm 官方包）的前提成立——`@deepseek-ai/dsh` 及其关键依赖已发布到 npm。

**前置**：1.2 网络可达。

**步骤**：

```powershell
# 1. 确认主包发布状态
npm view @deepseek-ai/dsh version
npm view @deepseek-ai/dsh dist-tags --json
npm view @deepseek-ai/dsh bin --json
npm view @deepseek-ai/dsh engines --json

# 2. 确认插件开发所需的关键依赖包也已发布
npm view @deepseek-ai/cordis version
npm view @deepseek-ai/dsh-tools version   # 若报 404，记录哪些包未发布

# 3. 查看主包依赖清单（确认运行时体积量级）
npm view @deepseek-ai/dsh dependencies --json
```

**预期结果**：
- `@deepseek-ai/dsh` 返回版本号（当前源码 `0.1.2-alpha.1`，npm 上可能已是更新版本或尚未发布 alpha）
- `bin` 字段指向 `lib/bin.js`（与 families.ts 的 `installedEntry` 一致）
- `engines` 明确声明 `^22.19 || >=24`
- `@deepseek-ai/cordis` 有独立版本号

**通过标准**：
- [ ] `@deepseek-ai/dsh` 在 npm 上存在且 `bin` 指向 `lib/bin.js`
- [ ] 记录 npm 最新版本号，与 submodule 的 `package.json` 版本做比对

**失败排查 / 备选**：
- 若 `@deepseek-ai/dsh` 尚未发布（404）→ **方案 A 不可行**，改用**方案 B**（submodule 源码自构建，见上级计划 4.3）
- 若 `@deepseek-ai/cordis` 未发布 → 自定义插件无法独立开发，需改为"插件源码放桌面端仓库、随 submodule 一起构建"（记录到风险）

**产出物**：npm 包版本、bin 入口、engines、关键依赖清单，写入验证报告。

---

### V0.2 启动链路验证（npx 与直接 node bin 两种方式）

**目的**：确认 `dsh web` 能通过两种方式启动，且直接 `node <bin.js>` 的方式可行（这是桌面壳 spawn 的形式）。

**前置**：V0.1 通过。

**步骤**：

```powershell
# 方式 1：官方推荐 npx
npx -y @deepseek-ai/dsh web --port 0 --no-open
# 观察 stdout 输出，Ctrl+C 终止

# 方式 2：直接 node bin（桌面壳将采用的方式）
# 先安装到本地，拿到 bin.js 路径
npm install @deepseek-ai/dsh
node ./node_modules/@deepseek-ai/dsh/lib/bin.js web --port 0 --no-open
# 观察 stdout 输出，Ctrl+C 终止
```

**预期结果**：
- 两种方式都能启动，stdout 打印类似：`dsh web: http://127.0.0.1:<随机端口>/?token=<token>`
- `--no-open` 生效：**不会**弹出默认浏览器窗口
- `--port 0` 生效：端口是 OS 分配的随机值，而非固定 3080

**通过标准**：
- [ ] 两种方式均能启动并打印 URL 行
- [ ] `--no-open` 下无浏览器弹出
- [ ] `--port 0` 下端口为随机值
- [ ] `Ctrl+C` 能干净终止进程，无残留 node 进程（用 `Get-Process node` 确认）

**失败排查**：
- 若直接 `node bin.js` 报模块找不到 → 检查是否需 `--experimental-*` 或 ESM loader flag，记录到风险
- 若启动即退出 → 查看 stderr，可能是 frontend dist 缺失（源码 checkout 才需 build，npm 包应已内置）

**产出物**：两种启动方式的完整命令、stdout 原文样例、进程终止验证结论。

---

### V0.3 端口分配与 URL 解析验证

**目的**：验证桌面壳"spawn → 解析 stdout → 拿到 URL"这一核心链路所需的关键行为。

**前置**：V0.2 通过。

**步骤**：

```powershell
# 1. 固定端口验证（确认 3080 默认值 + 指定端口）
node ./node_modules/@deepseek-ai/dsh/lib/bin.js web --no-open
# 预期：打印 http://127.0.0.1:3080/...
# Ctrl+C 终止

node ./node_modules/@deepseek-ai/dsh/lib/bin.js web --port 12345 --no-open
# 预期：打印 http://127.0.0.1:12345/...
# Ctrl+C 终止

# 2. 端口冲突行为验证
# 开两个窗口，第二个用相同端口，观察 EADDRINUSE 报错格式（桌面壳需据此提示用户）
# 窗口 A: node .../bin.js web --port 12346 --no-open
# 窗口 B: node .../bin.js web --port 12346 --no-open
# 记录 B 的 stderr 输出

# 3. 启动 URL 的精确格式（供桌面壳正则匹配用）
# 记录 URL 行的精确前缀、token 参数格式
```

**预期结果**：
- 默认端口 3080，`--port N` 覆盖
- 端口冲突时进程报 `EADDRINUSE` 并退出（或打印诊断）
- URL 行格式稳定：`dsh web: http://127.0.0.1:<port>/?token=<token>`

**通过标准**：
- [ ] 记录默认端口、自定义端口、随机端口的三种 URL 样例
- [ ] 记录端口冲突的 stderr 报错格式（用于 P1 错误处理设计）
- [ ] 总结出桌面壳解析 URL 的正则表达式原型

**产出物**：三种端口场景的 stdout 原文、冲突报错原文、正则表达式草案。

---

### V0.4 stdout 缓冲与时序验证（本阶段最大风险）

**目的**：验证 Windows 下 Node stdout 管道缓冲是否会导致 URL 行延迟或丢失，这决定桌面壳的 URL 获取方案（解析 stdout vs 固定端口轮询）。

**前置**：V0.3 通过。

**步骤**：

```powershell
# 1. 用管道实时读取 stdout，验证 URL 行能否及时到达
node ./node_modules/@deepseek-ai/dsh/lib/bin.js web --port 0 --no-open 2>err.log | ForEach-Object {
    $_ | Tee-Object -FilePath stdout.log   # 同时写文件
    # 手动观察：URL 行是否在启动后 1-2 秒内出现
}
# Ctrl+C 终止后检查 stdout.log 内容

# 2. 验证输出是否被缓冲（关键对比实验）
# 实验 A：stdout 直接连终端（TTY）
node ./node_modules/@deepseek-ai/dsh/lib/bin.js web --port 0 --no-open
# 观察 URL 行出现时机

# 实验 B：stdout 重定向到文件（pipe）
node ./node_modules/@deepseek-ai/dsh/lib/bin.js web --port 0 --no-open > out.txt 2>err.txt
# 不立即终止，等 5 秒后 Ctrl+C，检查 out.txt 是否已写入 URL 行

# 3. 检查 stderr 是否有干扰解析的内容
Get-Content err.txt
```

**预期结果**：
- URL 行在服务就绪后立即打印，pipe 模式下也能及时 flush（Node 的 `console.log` 通常不做块缓冲）
- 若 URL 行延迟或缺失 → 记录，进入失败排查

**通过标准**：
- [ ] 确认 URL 行在 pipe 模式下能及时到达（启动后 2 秒内）
- [ ] 确认 stdout 与 stderr 分离（URL 行在 stdout，诊断在 stderr）
- [ ] 得出结论：桌面壳采用"解析 stdout"方案（推荐）还是"固定端口 + 健康检查轮询"方案（备选）

**失败排查 / 备选**：
- 若 stdout 缓冲导致 URL 延迟 → 备选方案：桌面壳改用**固定端口 + HTTP 轮询 `http://127.0.0.1:<port>/` 直到 200**，或启动时用 `--port N`（先探测空闲端口）再轮询
- 若 URL 行缺失 → 检查是否有 `printUrl` 配置开关（web-app 有 `printUrl` 字段，默认 true）

**产出物**：缓冲行为结论、最终选定的 URL 获取方案。

---

### V0.5 token 交换与数据目录机制验证

**目的**：确认 token→cookie 交换机制与系统 WebView 兼容，且数据目录行为符合预期（与官方 CLI 互通）。

**前置**：V0.3 通过。

**步骤**：

```powershell
# 1. 启动并记录完整 URL
node ./node_modules/@deepseek-ai/dsh/lib/bin.js web --port 12347 --no-open
# 复制打印的完整 URL（含 token）

# 2. 用 curl 模拟浏览器 token 交换流程
# 2a. 直接访问带 token 的 URL，观察重定向
curl.exe -i "http://127.0.0.1:12347/?token=<token>"
# 预期：302 重定向，Set-Cookie 带签名会话 cookie

# 2b. 访问根路径，确认页面可加载
curl.exe -i "http://127.0.0.1:12347/"
# 预期：200，返回 index.html

# 2c. 验证无 token 时的行为（安全边界）
curl.exe -i "http://127.0.0.1:12347/api/..."   # 应被拒或要求认证

# 3. 数据目录验证
# 启动后检查用户目录下是否生成 ~/.dsh
Get-ChildItem "$env:USERPROFILE\.dsh" -Force
# 预期：存在 profiles/、sessions 等目录结构

# 4. DSH_HOME 覆盖验证（可选，为未来"应用专属目录"预留）
$env:DSH_HOME = "D:/Codes/dsh-p0-validation/test-home"
node ./node_modules/@deepseek-ai/dsh/lib/bin.js web --port 12348 --no-open
# 确认数据写到 test-home 而非 ~/.dsh
```

**预期结果**：
- token URL 302 重定向 + Set-Cookie，根路径 200 返回 HTML
- 无 token 的 API 请求被认证机制拦截
- `~/.dsh` 生成，DSH_HOME 覆盖生效

**通过标准**：
- [ ] token 交换流程为纯标准 HTTP（302 + Cookie），**无任何"必须真实浏览器"的依赖**（这是 WebView 兼容的关键结论）
- [ ] 确认 WebView（WebView2/WKWebView）能执行该标准流程
- [ ] 数据目录默认 `~/.dsh`，DSH_HOME 可覆盖

**失败排查**：
- 若 token 交换依赖浏览器指纹/特定 User-Agent → 记录，P1 需在 WebView 里注入对应 header（低概率）
- 若 curl 无法拿到 200 → 检查是否因 curl 未带 cookie 被拒，改用 `-c`/`-b` 保存 cookie 重放

**产出物**：token 交换 HTTP 交互记录、数据目录结构、WebView 兼容性结论。

---

### V0.6 自定义插件 --patch 挂载验证

**目的**：验证"独立 cordis 插件 + `--patch` 注入"这条自定义功能通道可行，这是整个方案"功能不丢"的基石。

**前置**：V0.4 通过。

**步骤**：

```powershell
# 1. 创建最小可运行插件包（独立目录，不碰 dsh 源码）
$plugin = "D:/Codes/dsh-p0-validation/dsh-plugin-ping"
New-Item -ItemType Directory -Force -Path $plugin | Out-Null
Set-Location $plugin

# 2. 初始化 package.json（依赖 npm 上的 cordis）
# 手写 package.json：
# {
#   "name": "dsh-plugin-ping",
#   "version": "0.0.1",
#   "type": "module",
#   "main": "src/index.js",
#   "peerDependencies": { "@deepseek-ai/cordis": "*" }
# }

# 3. 写最小插件 src/index.js（注册一个日志或命令，证明被加载）
# export const name = 'ping'
# export function apply(ctx) {
#   console.log('[dsh-plugin-ping] loaded')   // 启动时可见的输出，证明被加载
# }

# 4. 写 cordis.patch.yml 挂载该插件
# - id: ping
#   name: 'dsh-plugin-ping'

# 5. 安装依赖
npm install

# 6. 用 --patch 挂载启动
node "D:/Codes/dsh-p0-validation/node_modules/@deepseek-ai/dsh/lib/bin.js" web --port 0 --no-open --patch "$plugin/cordis.patch.yml"
# 观察 stdout 是否出现 [dsh-plugin-ping] loaded
```

**预期结果**：
- 启动日志中出现插件加载标记 `[dsh-plugin-ping] loaded`
- 说明 `--patch` 成功把独立 npm 包插件注入运行时

**通过标准**：
- [ ] 插件被加载（日志可见）
- [ ] 插件源码位于 dsh 源码**之外**的独立目录
- [ ] 记录 `--patch` 的路径参数格式（绝对路径 vs 相对路径）

**失败排查**：
- 若插件未加载 → 检查 patch 的 yaml 格式（参考 web-app 的 cordis.patch.yml 结构，`id`/`name` 字段），以及插件包能否被 Node 解析（需在能被 require/import 的位置）
- 若 `@deepseek-ai/cordis` 装不上 → 回到 V0.1 的结论，走备选方案

**产出物**：最小插件源码、patch 文件、加载成功证明、`--patch` 用法记录。

---

### V0.7 内嵌 Node 与跨平台可行性验证

**目的**：验证"安装包内嵌 Node"的方式可行，并识别三端差异（含 Linux Landlock 沙箱在 Win/mac 缺失的影响）。

**前置**：V0.2 通过。

**步骤**：

```powershell
# 1. 下载并解压一个独立的官方 Node 二进制（模拟内嵌场景）
# 下载 Windows x64 zip：
# https://nodejs.org/dist/v24.14.0/node-v24.14.0-win-x64.zip
# 解压到 D:/Codes/dsh-p0-validation/embedded-node/

# 2. 用解压出的 node.exe（非系统 PATH 中的 node）启动 dsh
"D:/Codes/dsh-p0-validation/embedded-node/node.exe" `
  "./node_modules/@deepseek-ai/dsh/lib/bin.js" web --port 0 --no-open
# 观察能否正常启动

# 3. 验证不依赖任何系统级 node 安装
# 临时从 PATH 移除 node 后重复第 2 步（或用完整绝对路径，本质已证明独立性）

# 4. 三端差异调研（Windows 为主，macOS/Linux 记录为待验证项）
# - Windows：沙箱（Landlock）不存在，确认 dsh 在无沙箱下正常降级运行
# - 检查启动日志有无 sandbox 相关报错
```

**预期结果**：
- 内嵌的独立 node.exe 能完整启动 dsh，不依赖系统 Node 安装
- Windows 无沙箱时 dsh 正常降级运行（不因缺 Landlock 崩溃）

**通过标准**：
- [ ] 独立 node.exe 启动成功
- [ ] 记录 Windows 下无沙箱的运行表现（正常 / 有告警 / 需禁用某些功能）
- [ ] 产出内嵌 Node 的目录结构草案（供 P2 打包脚本参考）

**失败排查**：
- 若独立 node 启动失败 → 检查是否缺运行时 DLL（Windows 下 node.exe 是自包含的，一般无此问题）
- 若沙箱相关报错 → 记录报错原文，评估是否需要 patch 掉 sandbox 相关 row（记录到风险，P2 处理）

**产出物**：内嵌 Node 启动结论、沙箱降级表现、内嵌目录结构草案。

---

## 3. 验证矩阵汇总

| 编号 | 验证项 | 关键假设 | 风险等级 | 状态 |
|---|---|---|---|---|
| V0.1 | 官方 npm 包可用性 | 方案 A 前提 | 高 | 待执行 |
| V0.2 | 启动链路（npx + node bin） | 桌面壳 spawn 形式 | 高 | 待执行 |
| V0.3 | 端口分配与 URL 解析 | 解析 stdout 的核心依据 | 高 | 待执行 |
| V0.4 | stdout 缓冲与时序 | 本方案最大风险 | 高 | 待执行 |
| V0.5 | token 交换与数据目录 | WebView 兼容性 | 中 | 待执行 |
| V0.6 | 插件 --patch 挂载 | 自定义功能通道 | 高 | 待执行 |
| V0.7 | 内嵌 Node 与跨平台 | 安装包内嵌前提 | 中 | 待执行 |

---

## 4. 执行期间的记录规范

每个验证任务完成后，**立即**将以下内容追加到 `docs/stage/stage-0-results.md`：

```markdown
## V0.x <任务名>

**结论**：✅ 通过 / ❌ 失败 / ⚠️ 有保留

**关键事实**：
- （逐条记录与预期不符或需强调的事实）

**stdout/stderr 原文**（如有价值）：
```

**铁律**：所有命令的实际输出都必须截图或存文本，不能凭记忆写结论。

---

## 5. 阶段出口标准（Go / No-Go）

完成 V0.1 ~ V0.7 后，按以下标准决策是否进入 P1：

**Go（进入 P1 桌面壳骨架）**：
- V0.1、V0.2、V0.3、V0.4、V0.6 **全部通过**（这 5 项是方案 A 的硬前提）
- V0.5、V0.7 通过或"有保留但已选定备选方案"

**No-Go（回到方案调整）**：
- V0.1 失败（官方未发布 npm 包）→ 切方案 B，重新评估工期
- V0.4 失败且备选方案（端口轮询）也被证明不可行 → 重新设计启动协议
- V0.6 失败（`--patch` 无法挂载独立插件）→ 自定义功能的注入通道需重新设计

---

## 6. 交付物清单

| 文件 | 说明 |
|---|---|
| `docs/stage/stage-0-results.md` | 验证报告（7 项结论 + 原文记录） |
| `D:/Codes/dsh-p0-validation/` | 验证工作区（可清理，但报告保留） |
| 正则表达式草案 | 供 P1 解析 URL 用（写入报告） |
| 内嵌 Node 目录结构草案 | 供 P2 打包用（写入报告） |

---

## 7. 风险预登记（验证过程中重点观察）

| 风险 | 观察点 | 若发生则 |
|---|---|---|
| 官方 npm 包未发布 alpha | V0.1 404 | 切方案 B |
| Windows stdout 缓冲 | V0.4 URL 延迟 | 切端口轮询方案 |
| cordis 未发布导致插件无法独立开发 | V0.1/V0.6 | 插件随仓库构建 |
| token 交换依赖浏览器特征 | V0.5 curl 失败 | WebView 注入 header |
| Windows 无沙箱崩溃 | V0.7 报错 | patch 掉 sandbox row |
