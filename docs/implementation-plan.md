# DeepSeek Harness 桌面端封装 — 实施计划

> 状态：待评审
> 日期：2026-08-29
> 关联文档：`docs/dsh-desktop-analysis.md`（项目分析与方案选型）

---

## 0. 决策记录（已确认）

| 决策点 | 结论 |
|---|---|
| 桌面壳框架 | **Tauri**（Rust 主进程 + 系统 WebView） |
| Node 运行时 | **安装包内嵌**（sidecar / resources 打包官方 Node 二进制） |
| 自定义功能 | **仅用独立 cordis plugin 包**，绝不修改 dsh 源码 |
| 版本同步 | **git submodule** + CI 自动开 PR |
| 首期范围 | 1:1 复刻 web + 1 个示范插件 + Windows/macOS/Linux 三端安装包 |

---

## 1. 目标与范围

### 1.1 目标

把 DeepSeek Harness 的 Web 端（`dsh web`）封装成可独立安装的桌面应用，让普通用户"下载 → 安装 → 双击打开 → 直接使用"，产品形态对齐 Codex / Claude Code 桌面端。

### 1.2 首期范围（In Scope）

1. 三端安装包：Windows（x64）、macOS（Apple Silicon + Intel）、Linux（x64）
2. 桌面壳：启动即拉起内嵌 Node 运行 `dsh web`，用系统 WebView 加载本地 URL，1:1 还原官方 web UI
3. 生命周期管理：关闭窗口 → 回收 Node 进程；进程异常 → 友好提示
4. Node 运行时内嵌：安装包自带 Node 22 LTS，不依赖用户已装 Node
5. 数据目录：与官方一致，默认 `~/.dsh`，会话/配置/凭据与官方 CLI 完全互通
6. 一个示范性自定义插件：演示"独立 cordis 插件 + 可跟随官方升级 + 功能不丢"
7. 版本同步机制：submodule 追踪官方源码，CI 检测新版本自动同步、跑兼容验证、开 PR

### 1.3 非目标（Out of Scope，首期明确不做）

- 修改 dsh 任何核心源码（架构约定，永久不做）
- 实现官方文档提到的 `file://` + IPC bridge 形态（官方尚未实现，等官方原生支持后再评估）
- 移动端 / 平板适配
- 多用户 / 团队协作 / 云端同步
- 自动更新（首期可留 TODO，二期用 tauri-updater 补）

---

## 2. 关键技术事实（源码分析结论，实施依据）

这些是从 `deepseek-harness` 源码中逐条确认的，后续所有设计都建立在这些事实上。

| # | 事实 | 来源 |
|---|---|---|
| F1 | 运行形态 = Node 进程 + 本地 HTTP + React SPA；`npx @deepseek-ai/dsh web` 启动 | README.md |
| F2 | `dsh web` 是 `dsh --profile web` 的硬编码别名 | apps/cli/src/args.ts |
| F3 | Node 引擎要求 `^22.19.0 \|\| >=24.0.0` | package.json |
| F4 | Web 默认监听 `http://127.0.0.1:3080`，仅回环 | web-app/README.md |
| F5 | **`--port 0` 让 OS 分配空闲端口**，`--port N` 指定端口 | web-app/src/startup.ts |
| F6 | **`--no-open` 禁止自动打开浏览器** | web-app/src/startup.ts |
| F7 | 禁止 `--host 0.0.0.0`（安全，防远程代码执行） | web-app/src/startup.ts |
| F8 | 启动时 stdout 打印 `dsh web: http://127.0.0.1:PORT/?token=xxx`，token 用于换签名 cookie | web-app 源码 + 测试 |
| F9 | 数据目录默认 `~/.dsh`，环境变量 `DSH_HOME` 可覆盖 | util/home-paths/src/index.ts |
| F10 | 自定义功能通过 `--patch <file.yml>` 挂载；插件是普通 npm 包，`dsh plugin add` 可安装 | args.ts、app-boot |
| F11 | 官方 npm 发布范围：dsh 家族 = `packages/!(experimental)/*` + `apps/*`（共享版本）；vendor 家族 = `vendor/*`（含 cordis，独立版本） | scripts/release/families.ts |
| F12 | `@deepseek-ai/dsh` 的 bin 入口 = `lib/bin.js` | families.ts `installedEntry` |
| F13 | 插件开发有完整指南：`defineTool`、`ctx.tools.register`、事件钩子等 | docs/cookbook/adding-a-tool.md |
| F14 | 官方版本当前 `0.1.2-alpha.1`，声明"会有破坏性变更" | package.json |
| F15 | 仓库**没有任何 electron/tauri 代码**；webserver README 提到 Electron IPC 形态但未实现 | 全仓搜索 |

**核心推论**：桌面壳只需做到"spawn `dsh web --port 0 --no-open`，解析出实际 URL，用 WebView 打开"，即可 100% 还原 web 端。这是最简单、最不易碎、与官方升级最解耦的方案。

---

## 3. 总体架构

### 3.1 运行时拓扑

```
┌─────────────────────────────────────────────────────┐
│  Tauri 桌面应用（用户双击启动）                        │
│                                                       │
│  ┌─────────────────────────────────────────────┐     │
│  │ Rust 主进程                                   │     │
│  │  1. 定位 resources/ 下的 node + dsh 运行时    │     │
│  │  2. spawn node <dsh-bin> web --port 0 --no-open --patch <自定义> │
│  │  3. 监听 stdout，解析 "dsh web: http://.../token" │     │
│  │  4. 创建 WebviewWindow 加载该 URL            │     │
│  │  5. 窗口关闭 / 应用退出 → 杀 node 进程        │     │
│  └─────────────────────────────────────────────┘     │
│           │ 加载本地 URL                              │
│  ┌────────▼────────────────────────────────────┐     │
│  │ 系统 WebView（WebView2 / WKWebView / WebKitGTK）│   │
│  │  = 官方 React SPA，1:1 还原                    │     │
│  └─────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────┘
                        │ HTTP/WS (127.0.0.1)
┌───────────────────────▼─────────────────────────────┐
│  内嵌 Node 进程（子进程）                              │
│  node <dsh>/lib/bin.js web --port 0 --no-open ...    │
│   = 完整的 DeepSeek Harness 官方运行时               │
│  数据目录：~/.dsh（与官方 CLI 共享）                   │
└─────────────────────────────────────────────────────┘
```

### 3.2 分层与解耦原则（核心）

桌面端仓库由三个正交的层组成，**层与层之间通过稳定的边界隔离**：

| 层 | 内容 | 与上游关系 | 升级时动作 |
|---|---|---|---|
| **L1 上游** | `deepseek-harness/`（submodule） | 官方源码，只读 | `submodule update` |
| **L2 桌面壳** | Tauri 工程（Rust + 打包配置） | 自有，不碰 L1 | 无变化（除非 dsh 启动协议变） |
| **L3 插件** | 独立 cordis 插件包 | 依赖 npm 上的 `@deepseek-ai/*` | 重新 `pnpm install` + 类型对齐 |

**不可违背的红线**：
1. L2 只通过"进程 + stdout + HTTP"与 L1 交互，**绝不 import L1 源码**
2. L3 只依赖 npm 发布的 `@deepseek-ai/*` 包，**绝不 import L1 的源码路径**
3. 任何对 L1 的补丁（patch）都记录在 L2 的 `patches/` 目录，用 `git apply` 在 CI 里打，**不进 submodule**

### 3.3 目录结构（目标仓库）

```
deepseek-harness-desktop/
├── deepseek-harness/                  # L1：官方源码（git submodule，只读）
├── tauri-app/                         # L2：Tauri 桌面壳
│   ├── src-tauri/
│   │   ├── src/main.rs                # 主进程：spawn/解析/生命周期
│   │   ├── tauri.conf.json            # 打包配置、resources 声明
│   │   └── Cargo.toml
│   ├── resources/
│   │   └── runtime/                   # 打包时注入：node 二进制 + dsh 运行时
│   └── (无前端源码，WebView 直接加载本地 URL)
├── plugins/                           # L3：自定义 cordis 插件（独立包）
│   └── dsh-plugin-session-export/     # 示范插件
│       ├── package.json
│       ├── tsconfig.json
│       ├── src/index.ts
│       └── cordis.patch.yml
├── scripts/
│   ├── sync-upstream.sh               # 拉取官方新版本
│   ├── build-runtime.sh               # 构建 dsh 运行时产物
│   └── bundle.sh                      # 组装 resources/runtime + 打安装包
├── .github/workflows/
│   ├── sync-upstream.yml              # 检测官方新版本 → 开 PR
│   └── release.yml                    # 三端安装包矩阵
└── docs/
    ├── dsh-desktop-analysis.md        # 项目分析（已完成）
    └── implementation-plan.md         # 本文件
```

---

## 4. 核心技术方案

### 4.1 桌面壳（L2）：Tauri

- 主进程用 Rust，仅依赖 `tauri` + `tauri-plugin-shell`（或直接用 `std::process::Command`，避免多余依赖）
- **无前端源码**：`WebviewWindow` 直接 `loadUrl("http://127.0.0.1:<port>/?token=...")`
- 窗口配置：默认 1400×900，可缩放，保留系统标题栏（首期不做自定义标题栏）

启动流程（`main.rs` 伪逻辑）：

```rust
// 1. 解析 resources 路径（跨平台处理 bundle 内路径）
let runtime = resolve_runtime_dir();

// 2. 组装启动参数
let dsh_bin = runtime.join("node_modules/@deepseek-ai/dsh/lib/bin.js");
let patch   = runtime.join("plugins/session-export.cordis.patch.yml"); // 有则加
let args    = ["web", "--port", "0", "--no-open", "--patch", patch];

// 3. spawn 内嵌 node
let mut child = Command::new(runtime.join("node"))
    .arg(dsh_bin).args(args)
    .stdout(Stdio::piped()).stderr(Stdio::piped())
    .spawn()?;

// 4. 逐行读 stdout，匹配 "dsh web: (http://...)"
let url = wait_for_startup_url(&mut child).await?; // 带超时，超时则读 stderr 报错

// 5. 打开窗口
WebviewWindow::builder(app, "main", WebviewUrl::External(url)).build()?;

// 6. 生命周期：监听窗口关闭 / app 退出 → kill child；child 退出 → 提示并关窗
```

### 4.2 Node 运行时内嵌

- **版本**：Node 22 LTS 最新（满足 `^22.19`），CI 从 `https://nodejs.org/dist/` 下载官方二进制
- **打包方式**：作为 Tauri `resources` 打包（`bundle.resources` 指向 `resources/runtime/`），主进程用 `std::process::Command` 直接调用，**不需要 sidecar 机制**（更简单、少一层抽象）
- **平台矩阵**：
  | 平台 | 二进制 | 安装包影响 |
  |---|---|---|
  | Windows x64 | `node.exe` | +~30MB（压缩后约 +20MB） |
  | macOS arm64/x64 | `node` | +~40MB |
  | Linux x64 | `node` | +~30MB |
- **备选**（若体积敏感）：运行时检测系统 Node ≥22，缺失则引导下载；首期按"内嵌"执行

### 4.3 dsh 运行时产物打包（关键决策）

这里有两种来源，**推荐方案 A**：

**方案 A（推荐）：npm 官方包作为运行时**

CI 打包时，在干净的目录 `resources/runtime/` 执行：

```sh
npm init -y
npm install @deepseek-ai/dsh@<锁定版本>   # 锁定到 submodule 当前版本
```

- 优点：体积小（只装 dsh + 运行时依赖）、产物是官方测试过的发布包、与"桌面版版本 = 官方版本"天然对齐
- 缺点：需要官方确实发布了对应 npm 版本（已确认：`@deepseek-ai/dsh` 是 dsh 家族成员，会发布）

**方案 B（备选）：submodule 源码自构建**

在 submodule 内 `pnpm install && pnpm run build`，把 `apps/web/dist` + 所有 `lib/` + `node_modules` 一起打包。

- 缺点：体积巨大（完整 monorepo node_modules 可达数百 MB）、构建链路复杂、产物未经官方打包校验
- 仅在"官方 npm 包缺某个运行时必需产物"时才启用

> 结论：首期用方案 A。方案 B 作为 fallback 写入风险清单。

### 4.4 启动 URL 与 token 处理

- `--port 0` 让 OS 分配端口 → 避免与用户已开的 3080 端口冲突
- 从 stdout 解析 `dsh web: http://127.0.0.1:<port>/?token=<token>` 拿到完整 URL（含 token）
- WebView 加载该 URL → 官方前端拿 token 换 cookie 后重定向到干净 `/`（全程复用官方机制，无自定义安全逻辑）
- 兜底：若 30s 内未解析到 URL，读取 stderr 展示诊断信息

**待 P0 验证**：Windows 下 Node stdout 管道缓冲是否会导致 URL 行延迟/不 flush。若有问题，备选方案是固定端口 + HTTP 健康检查轮询。

### 4.5 数据目录

- **不覆盖 `DSH_HOME`**，保持官方默认 `~/.dsh`
- 这样桌面端与官方 CLI 完全共享会话历史、模型配置、API Key、凭据
- 首期不做"应用专属沙箱目录"；若未来需要，通过启动脚本设置 `DSH_HOME` 即可，不改源码

### 4.6 自定义插件（L3）：独立 cordis 包

**示范插件选型：`dsh-plugin-session-export`（会话导出为 Markdown）**

选择理由：
1. 纯 cordis 插件，完全独立，最能验证"不碰源码 + 可升级 + 不丢"的核心诉求
2. 不依赖桌面壳 IPC，三端行为一致
3. 用户价值直观（一键把当前会话导出为 .md 文件）

插件结构（参考官方 `adding-a-package.md` 与 `adding-a-tool.md`）：

```ts
// plugins/dsh-plugin-session-export/src/index.ts
import type { Context } from '@deepseek-ai/cordis'

export const name = 'session-export'
export const inject = ['commands', 'sessions']   // 挂到命令系统

export function apply(ctx: Context) {
  // 注册一个命令 / 或监听 session 事件，把 SessionEvent log 投影成 Markdown 写入文件
}
```

挂载方式（通过 patch 文件，由桌面壳启动时注入）：

```yaml
# plugins/dsh-plugin-session-export/cordis.patch.yml
- id: session-export
  name: 'dsh-plugin-session-export'
  config: { outDir: '~/Documents/dsh-exports' }
```

桌面壳启动参数追加 `--patch <该 patch 的绝对路径>`（F10 已确认支持）。

**依赖关系**：插件 package.json 里 `peerDependencies: { "@deepseek-ai/cordis": "..." }`，从 npm 安装（F11 确认 cordis 会发布）。

### 4.7 桌面壳额外能力（IPC 桥，二期预留）

首期不需要，但架构预留：Tauri 主进程可通过 `tauri-plugin-shell` 或自定义 command 与 WebView 通信。未来系统级能力（系统通知、文件拖放、剪贴板、截屏推送给 agent）走这条链路，与 dsh 完全解耦。

---

## 5. 版本同步方案

### 5.1 submodule 策略

- `deepseek-harness` 作为 submodule，指向官方 `main` 分支
- 桌面端仓库用 `git` 记录 submodule 的 commit，作为"当前对齐版本"的唯一真值
- **版本对齐 = submodule 的 commit SHA + `deepseek-harness/package.json` 的 version 字段**

### 5.2 CI 自动同步（`sync-upstream.yml`）

```
触发：定时（每周）+ 手动 workflow_dispatch
步骤：
  1. git submodule update --remote --merge
  2. 读取 deepseek-harness/package.json 的 version，比对上次同步记录
  3. 若版本未变 → 跳过（no-op）
  4. 若版本变更：
     a. 更新锁定版本号（写入 .dsh-version 文件）
     b. 构建运行时（npm install @deepseek-ai/dsh@<new>）
     c. 重新安装插件依赖（对齐 cordis 版本）
     d. 跑兼容验证（见 5.3）
     e. 全部通过 → 开 PR（含版本 diff、验证报告）
     f. 有失败 → 开 issue 标记"需人工介入"
```

### 5.3 升级兼容性验证（同步 PR 必须通过的 gate）

| 检查 | 命令/方法 | 目的 |
|---|---|---|
| 运行时能装 | `npm install @deepseek-ai/dsh@<new>` 成功 | 官方包可发布可安装 |
| 能启动 | 跑 `node <dsh-bin> web --port 0 --no-open`，解析到 URL | 启动协议未变（F2/F5/F6/F8） |
| web 能打开 | headless 浏览器访问 URL，页面 200 | 前端产物完整 |
| 插件兼容 | 带 `--patch` 启动，插件正常加载 | 自定义功能不丢 |
| 类型对齐 | 插件 `pnpm install` 后 `tsc --noEmit` 通过 | cordis API 无破坏性变更 |
| 官方自检 | submodule 内 `pnpm run typecheck` | 官方自身健康 |

### 5.4 同步失败时的降级策略

- 若官方新版本破坏兼容（F14 明确警告会有破坏性变更），桌面端**停留在旧版本**，等待修复
- 自定义插件若依赖的 cordis API 变更，只需改插件源码（L3 层），**不动 L1/L2**

---

## 6. 分阶段实施计划

### 阶段 0：技术验证（P0，1 天）

**目标**：跑通最小闭环，消除所有"不确定"。

| 任务 | 产出 |
|---|---|
| 0.1 装 Node 22，验证 `npx @deepseek-ai/dsh web` 可启动 | 确认官方 npm 包可用 |
| 0.2 验证 `--port 0 --no-open` 组合，stdout 能解析到 URL | 确认 F5/F6/F8 |
| 0.3 验证 Windows/macOS 下 stdout 缓冲行为 | 决定 4.4 的解析方案 |
| 0.4 确认 `@deepseek-ai/cordis` 等依赖包的 npm 版本可用 | 决定插件能否独立开发（F11） |
| 0.5 验证 `--patch` 挂载一个最小插件 | 确认 F10 |

### 阶段 1：桌面壳骨架（P1，2-3 天）

| 任务 | 产出 |
|---|---|
| 1.1 初始化 Tauri 工程（无前端） | 可编译的空壳 |
| 1.2 实现 `main.rs`：spawn node + 解析 URL + 打开 WebView | 核心启动链路 |
| 1.3 实现生命周期管理：关窗杀进程、进程退出提示 | 完整闭环 |
| 1.4 本地用系统 Node 跑通（暂不内嵌） | 可手动演示 |

### 阶段 2：打包链路（P2，3-5 天）

| 任务 | 产出 |
|---|---|
| 2.1 `build-runtime.sh`：npm 安装 `@deepseek-ai/dsh@<锁定>` | 运行时产物 |
| 2.2 下载并内嵌各平台 Node 二进制 | resources/runtime/node |
| 2.3 `bundle.sh`：组装 resources + `tauri build` | 三端安装包 |
| 2.4 配置 `tauri.conf.json`（图标、窗口、bundle 配置） | 安装包可装可跑 |
| 2.5 macOS 签名/公证、Windows 代码签名（可用测试证书先跑通） | 可分发雏形 |

### 阶段 3：示范插件（P3，3-5 天）

| 任务 | 产出 |
|---|---|
| 3.1 创建 `dsh-plugin-session-export` 独立包 | 插件源码 |
| 3.2 实现"会话导出 Markdown"逻辑 | 可用功能 |
| 3.3 写 `cordis.patch.yml`，接入桌面壳启动参数 | 端到端打通 |
| 3.4 单元测试 + 手动验证 | 功能可用 |

### 阶段 4：同步机制（P4，3-4 天）

| 任务 | 产出 |
|---|---|
| 4.1 初始化 submodule，记录基线 commit | 仓库结构就位 |
| 4.2 `sync-upstream.sh` 脚本 | 一键同步 |
| 4.3 `sync-upstream.yml` workflow（5.2 全流程） | CI 自动化 |
| 4.4 兼容性 gate（5.3）接入 PR 检查 | 同步可信 |

### 阶段 5：打磨与发布（P5，2-3 天）

| 任务 | 产出 |
|---|---|
| 5.1 启动页/加载态/错误态 UI 兜底 | 体验完整 |
| 5.2 图标、应用名、版本号规范 | 品牌就绪 |
| 5.3 三端 release workflow + 发布 checklist | 正式发布 |

---

## 7. 工期估算

| 阶段 | 工期（人日） |
|---|---|
| P0 技术验证 | 1 |
| P1 桌面壳骨架 | 2-3 |
| P2 打包链路 | 3-5 |
| P3 示范插件 | 3-5 |
| P4 同步机制 | 3-4 |
| P5 打磨发布 | 2-3 |
| **合计** | **14-21 人日** |

按 1 人全职计算：**首版约 3-4 周**；加 1 周缓冲（应对 alpha 阶段的不确定性），**约 4-5 周出 1.0 公测**。

---

## 8. 风险与应对

| 风险 | 等级 | 应对 |
|---|---|---|
| 官方仍 alpha，API 破坏性变更（F14） | 高 | 同步机制留"停留旧版本"降级路径；插件层隔离变化 |
| Windows stdout 缓冲导致 URL 解析失败 | 中 | P0 验证；备选固定端口 + 健康检查 |
| Linux 沙箱（Landlock）在 Win/mac 缺失 | 中 | 首期不启沙箱，确认核心体验无影响；P0 验证 |
| npm 官方包缺运行时必需产物 | 中 | 备选方案 B（源码自构建） |
| macOS 公证/Windows 签名门槛 | 中 | P2 先用测试证书，正式发布前补正式证书 |
| 内嵌 Node 导致包体积 +20-40MB | 低 | 可接受；若敏感则切换系统 Node 检测方案 |
| 官方未来原生支持桌面端 | 低 | 属利好；届时桌面壳可退化为纯 wrapper 或直接迁移 |

---

## 9. 验收标准（首版 Definition of Done）

1. **三端可装可跑**：Windows/macOS/Linux 安装包下载后无需 Node、无需命令行即可打开使用
2. **1:1 还原**：桌面端 UI 与 `npx @deepseek-ai/dsh web` 在浏览器中的表现完全一致
3. **数据互通**：桌面端会话历史/配置与官方 CLI 共享（同一 `~/.dsh`）
4. **生命周期正确**：关闭窗口后无残留 node 进程；进程崩溃有友好提示
5. **示范插件可用且可升级**：会话导出功能在同步官方新版本后仍正常工作
6. **同步自动化**：官方发新版本后，CI 自动开 PR，PR 附带兼容性验证报告
7. **零源码侵入**：`git diff` 确认 submodule 无任何改动

---

## 10. 待确认问题（不阻塞，先按默认值推进）

| # | 问题 | 默认假设 |
|---|---|---|
| Q1 | 应用名称（产品名） | 暂定 `DeepSeek Harness Desktop` |
| Q2 | 是否首期就上自动更新 | 首期不做，留 TODO |
| Q3 | 代码签名证书（Win/mac） | 先用测试证书，发布前补齐 |
| Q4 | 是否需要中文优先的 UI 文案 | 跟随官方 i18n，不额外覆盖 |
| Q5 | 示范插件最终选型 | 会话导出 Markdown（可改） |

---

## 11. 下一步

评审确认后，从 **阶段 0（技术验证）** 开始动手。P0 是纯验证、零代码侵入，风险最低，能立刻消除方案中的关键不确定性。
