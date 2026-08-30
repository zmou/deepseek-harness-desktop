# DeepSeek Harness 项目分析与桌面端封装方案

> 本报告基于 `d:/Codes/deepseek-harness-desktop/deepseek-harness` 当前源码的实地阅读（README、ARCHITECTURE、dev guide、Web bundle/package 文档、`apps/cli` 源码、构建脚本、版本号等）。

---

## 一、DeepSeek Harness 是什么

DeepSeek Harness（`dsh`）是 DeepSeek AI 官方发布的开源 **Agent Harness / 智能体框架**，类似一个 CLI + Web 形态的 "AI 编程代理宿主机"，内置一套工具（读/写文件、执行命令、子 Agent、终端、文件系统沙箱、计划、记忆、会话日志…），可挂多个 LLM 适配器，默认指向 DeepSeek 自家 API，也能接其它 OpenAI 兼容端点。

**版本**：当前仓内 version 是 `0.1.2-alpha.1`（一处重要信息——正式版尚未发布，官方目前声明 *"THERE WILL BE COMPATIBILITY-BREAKING CHANGES"*，早期阶段、API 会大幅变动）。

**License**: MIT。

---

## 二、技术栈（影响后续封装方案的关键事实）

| 维度 | 实际内容 |
|---|---|
| 运行时 | **Node.js** ^22.19 \|\| >=24（**必须有 Node 环境**） |
| 包管理 | pnpm@11.7.0（Corepack），monorepo |
| 框架内核 | **Cordis** 插件框架（一切皆插件：模型、工具、会话、Agent loop 都可被替换） |
| 前端 | **Vite 6 + React 18**（`apps/web`，纯 SPA，构建后是 `dist/`） |
| 包入口 CLI | `apps/cli`：执行 `node bin.ts` 等价于 `dsh web`，启 `http://127.0.0.1:3080` |
| 分发方式 | 默认通过 `npx @deepseek-ai/dsh web` 跑（官方目前只发布到 npm），源码分发要 `git clone + pnpm install + pnpm build` |
| 已有的"服务端" | `dsh-host-webserver` + `dsh-host-frontend-static`，fallback seat 静态服务 |
| 安全性 | 默认只 bind 127.0.0.1，禁止 `--host 0.0.0.0`；启动 URL 带 token，浏览器通过 token 换签名 cookie |
| 沙箱 | Linux 上用 Landlock（`native/landlock-run`），代码里没有 Windows 沙箱位 |
| 桌面相关 | **仓库里完全没有 electron / tauri / electron-builder**；`native/` 只是 Linux 沙箱 |

事实结论：**dsh 本质就是一个 Node 进程 + 一个本地 HTTP 服务 + 一个 Vite 构建的 React SPA**，没有任何"桌面端"的现成壳。所以"封装成桌面 APP" = 选一种壳把它装起来。

---

## 三、整体架构（简化版）

```
┌──────────────────────────┐
│  apps/web (Vite + React) │  ← 构建产物 dist/（被静态托管）
└──────────┬───────────────┘
           │ HTTP / WebSocket
┌──────────▼───────────────┐
│  apps/cli (Node 进程)    │  ← 入口 `dsh web`，启动 Cordis Loader
│  dsh-host-webserver      │  ← HTTP server（token + cookie + CORS）
│  dsh-host-frontend-static│ ← 兜底托管 SPA dist/
└──────────┬───────────────┘
           │
┌──────────▼───────────────┐
│  dsh-base bundle（共享层）│  ← 模型适配、工具、持久化、沙箱、凭据、设置…
│  + dsh-web-app bundle    │  ← Web 表层补丁
└──────────┬───────────────┘
           │
┌──────────▼───────────────┐
│  packages/* (40+ 子包)   │  ← Cordis 插件树
│  core/*, llm/*, tool/*,  │
│  host/*, client/*, sdk/…
└──────────────────────────┘
```

启动流程（用户视角）：
1. 执行 `dsh --profile web`（默认 profile = web）
2. Node 启动 Cordis，按顺序 apply bundles（dsh-base → dsh-web-app）→ profile patch → 用户 patch → `--patch` 覆盖
3. HTTP 服务起来在 `127.0.0.1:3080`，打印 `dsh web: http://127.0.0.1:3080/?token=xxx`
4. 默认浏览器被打开（`--no-open` 可关），浏览器拿 token 换 cookie，跳到 `/`
5. React SPA 走两段启动（`dsh-client-web` 的 `AppWebEntry`）：先 framework-free boot 页 + 进度，再 hydrate 完整 UI

---

## 四、怎么"1:1 封装成桌面 APP"（技术方案）

### 方案 A —— Tauri（**推荐**）

**为什么推荐**：包小（10MB 级别安装包）、启动快、安全模型天然对齐（Rust 后端 + WebView）、能直接控制子进程生命周期、对中文环境（Windows 7/10/11 / macOS 12+）友好。

**原理**：
```
Tauri 主进程（Rust）
   └─ 启动时 spawn `node dsh web --port=3081 --no-open`（offscreen）
   └─ 等 127.0.0.1:3081 上有健康响应后
   └─ `WebviewWindow` 打开一个 WebView，加载 http://127.0.0.1:3081
   └─ WebView 看见的就是 dsh 的 React SPA（1:1）
   └─ 关闭主窗口时 kill Node 子进程
```

注意需要 `--port=3081`（**避开**官方默认 3080），因为冲突检测机制存在；`--no-open` 一定要加，否则 Node 进程会尝试再开一次浏览器。

**前端打包**：dsh 的 `apps/web/dist/` 已经是纯静态 SPA，你不用碰它；Tauri 只负责"打开它"。

**Node 内嵌**：通过 `tauri-plugin-shell` 或 `tauri.conf.json` 的 `externalBin` 把 `node` 二进制随安装包分发；或者更省事，让用户在系统里装 Node ≥22，桌面端启动时检测 → 引导安装。

### 方案 B —— Electron（**最省事**）

**原理**：
```js
main.js (Electron 主进程)
  spawn `npx @deepseek-ai/dsh web --port=3081 --no-open`
  new BrowserWindow({ width: 1440, height: 900 })
    .loadURL('http://127.0.0.1:3081')
```

- 优点：生态最熟（开发快、测试方便）
- 缺点：安装包 80-150MB，内存偏大，Node 重复打包（Electron 自带一份 + 外面再跑 dsh 一份）
- 既然你的目的是 1:1 还原 web 端 → Electron 自带的 Chromium 看 React SPA 是真 1:1，跟"在浏览器里开官方页面"等价

### 方案 C —— 直接用系统浏览器 + 桌面快捷方式（**最轻**）

**原理**：打包一个 .exe / .app / .deb，启动脚本里面跑：
- mac/win: `node dsh web --port=3081` + `xdg-open http://127.0.0.1:3081`
- 关闭时回收进程

就是发个 wrapper 脚本。**你给普通用户装的是脚本 + Node + dsh 包 + 一个图标**，没有 WebView 嵌入；这是最低成本路径，缺点是"打开的不是 APP，是浏览器标签页"，跟 codex/claude 桌面端的产品形态不一致。

### 我的建议

| 维度 | Tauri | Electron | 脚本 |
|---|---|---|---|
| 安装包体积 | 10-20MB | 80-150MB | 30-60MB（Node + 依赖） |
| 启动速度 | ★★★★ | ★★ | ★★★ |
| 与 dsh 解耦（便于同步官方版本） | ★★★（主进程只 spawn node）| ★★★ | ★★★★★ |
| 开发工作量 | ★★★（要写 Rust）| ★★ | ★★★★★ |
| 1:1 还原 web 端 | 完全一致 | 完全一致 | 一致（但不是 APP 形态）|

**首选 Tauri**：契合你要的"普通用户安装即用"，包也小，迭代起来干净。如果你 / 团队不懂 Rust → 退回 Electron，代码量更少。

---

## 五、怎么对接官方升级（"100% 同步 + 自定义功能不丢"）

这是你最关心的第 2 点，关键在于 **"桌面端壳" vs "dsh 核心" 必须完全解耦**。

### 5.1 解耦分层的目录结构（推荐）

```
你的仓库  deepseek-harness-desktop/
├── deepseek-harness/          ← 官方仓库（git submodule / git subtree / 定期 rebase）
│
├── packages/desktop-shell/    ← 你自己的"桌面壳"
│   ├── src/                   ← Tauri/Electron 主进程代码
│   ├── patches/               ← 你自己的小功能代码（独立目录）
│   ├── build/                 ← 打包脚本
│   ├── sync.sh                ← 同步官方版本的脚本
│   └── README.md
│
└── docs/                       ← 工程文档
```

两种 git 子仓库整合姿势：
- **`git submodule`**：推荐。`deepseek-harness` 用 submodule 引入，Shell 主目录干净；升级就是 `git submodule update --remote deepseek-harness && cd deepseek-harness && pnpm install && pnpm build`，然后你重新打包。
- **`git subtree`**：如果你想单仓管理；升级用 `git subtree pull --prefix=deepseek-harness https://github.com/deepseek-ai/deepseek-harness.git main --squash`。

### 5.2 同步脚本 `sync.sh` 设计思路

```sh
#!/usr/bin/env bash
set -euo pipefail

# 1) 拉官方最新
git submodule update --remote --merge deepseek-harness

# 2) 校验兼容：dsh 版本号变了 → 跑 typecheck/build 验证你的 shell 假设的端口、启动方式、token 交换是否还成立
cd deepseek-harness
pnpm install
pnpm run build           # 关键！它会构建 web dist/ 和 host 包

# 3) 走 dsh 自带的发布自检（pack-install 校验）
pnpm run release:pack

# 4) 把新构建产物（apps/web/dist/）复制到 shell 期望的目录
cd ..
./packages/desktop-shell/scripts/copy-built-artifacts.sh

# 5) 你自己的"小功能"模块：与 dsh 核心零侵入——只走 dsh 的 cordis patch、附加独立 plugin
#    升级时这里要做的只是重新 pnpm install + build
```

### 5.3 你的"额外小功能"怎么写才能不丢？

dsh 是 "everything-is-a-plugin" + profile + cordis.patch.yml，所以你的"额外功能"有 4 种植入位置，**从侵入性低到高**：

| 植入位置 | 适合什么 | 升级冲突风险 |
|---|---|---|
| **A. 独立 cordis plugin 包（推荐）** | 你的所有新功能 | ★ 最低。dsh 升级基本不碰你 |
| **B. 用户的 `cordis.patch.yml`** | 给 row 加配置、调换 bundle 顺序 | 低 |
| **C. 修改 dsh 现成包源码** | 不得已才用 | **高**，建议放弃 |
| **D. 桌面壳加 IPC 层**（Tauri/Electron 特有能力） | 系统级：截屏推送给 dsh、文件拖放、剪贴板增强、GPU 加速等 | 与 dsh 完全解耦，最稳 |

**最稳的实践**：
- 你自己建一个 cordis plugin 包 `@your-org/dsh-plugin-xxx`（参考 [extension-cookbook](docs/architecture.md) 文档，那里明确写了"add a package"指南）
- 它依赖 `@deepseek-ai/cordis-plugin-group` 等官方接口，跟随版本升级即可
- 在 desktop-shell 的启动包装里 `dsh web --patch ./your-plugin.cordis.patch.yml`
- 这样 dsh 升 `0.1.2 → 0.1.3`，你只需要 `pnpm install && pnpm build`，自定义功能丝毫无损

### 5.4 自动化同步的 CI 模板（之后会让你做）

- GitHub Action：每周 / 每次 upstream tag → 跑 `sync.sh` → 出 PR
- PR 必须通过 dsh 的内置 `pnpm run check:ci` 与你的 e2e（启动 → 打开 → 聊天）
- 出包由单独的 release workflow 触发（用 GH Action matrix 打 Windows / macOS / Linux 三端安装包）

---

## 六、工作量与工期估算（一个全职工程师）

| 阶段 | 工作 | 工期（人日） |
|---|---|---|
| **P0：方案选型 + 验证环境** | 跑通"机器 1：Node 22，机器 2：装 dsh，npx dsh web 能开"，确认端口/token/重定向闭环 | 1 |
| **P1：骨架 —— 桌面壳 v0** | Tauri/Electron 项目初始化，spawn node + 打开 WebView + 关闭时 kill | 2-3 |
| **P2：构建产物打包** | 在 shell 里跑 `pnpm install && pnpm run build` → 抽 `apps/web/dist/` 到资源目录；处理 Node 跟随发布（tauri externalBin 或 Node 安装引导） | 3-5 |
| **P3：安装包与签名** | Windows：MSI/NSIS；macOS：dmg + 公证；Linux：deb/AppImage；自动更新走 tauri-updater | 3-5 |
| **P4：第 1 个"额外小功能"** | 用独立 plugin 的方式演示打一遍（验证同步流程） | 3-5 |
| **P5：升级同步机制** | git submodule + sync.sh + CI；含回归测试（typecheck + 启动 + 简单 e2e） | 3-4 |
| **P6：打磨（图标/启动页/错误页/离线态）** | boot 页失败兜底；进程异常退出的提示 | 2-3 |
| **总计** | | **约 17-26 人日** ≈ **3.5–5 周**（1 人全职） |

工期基数建议给 **4 周首版 + 2 周打磨**，即 **6 周能出 1.0 公测**。这是基于 dsh 0.1.2 还在 alpha 的乐观估计；一旦官方在 Windows/macOS 上有兼容性破坏，需要 +1 周应急。

风险点（会拉长工期）：
1. **Node 跟随发布**：Windows 安装包把 Node 22 打进去会让 MSI 体积 +30MB，且要处理 UAC / PATH；可选折中：检测到系统没 Node 时引导用户去装。
2. **Linux 沙箱（Landlock）**：在 Windows/macOS 不存在，需要确认 sandbox 关闭时不影响核心体验（看 Landlock 是怎么被启用的，是 fallback 实现还是直接要它）。
3. **dsh 还在 alpha**：0.1.2 alpha.1 → 1.0 之间大概率改 API，建议每 2 周同步一次而不是死磕大版本。
4. **中文路径 / Windows 性能**：跑 Vite 产物的 chrome 内核在低端 Windows 上偶有卡顿，需要 QA。

---

## 七、关键决策清单（建议在你动代码前确认）

| 问题 | 我的默认假设（可改）|
|---|---|
| 桌面端框架 | Tauri（如果团队有 Rust 经验），否则退 Electron |
| Node 怎么提供 | 安装包内嵌（更"开箱即用"），约 +25-30MB |
| 自定义功能位置 | 全部放独立 cordis plugin 包，桌面壳只做系统能力 |
| 升级策略 | git submodule + CI 自动 PR |
| 首期范围 | 1:1 复刻 web + 1 个示范性自定义插件 + Win/Mac/Linux 三端包 |

---

## 八、一句话总结

dsh 本质是 "Node 进程 + 本地 HTTP + React SPA"，完全可以用 Tauri/Electron 把它的 `127.0.0.1:3080` SPA 装进壳里；与官方保持同步的关键是 `git submodule` + cordis 独立插件做扩展，shell 与 dsh 核心零侵入；首版 6 周内可发。

---

*准备好之后告诉我用 Tauri 还是 Electron（或者你想先做最轻量"脚本式"原型），我就可以开始搭骨架了。*
