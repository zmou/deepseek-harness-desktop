<p align="center">
  <img src="tauri-app/src-tauri/icons/icon.png" width="120" alt="DeepSeek Harness Desktop logo" />
</p>

<h1 align="center">DeepSeek Harness Desktop</h1>

<p align="center">
  <strong>DeepSeek Harness 的桌面壳</strong> —— 在桌面上一键运行
  <a href="https://github.com/deepseek-ai/deepseek-harness">DeepSeek Harness</a>，
  无需 Node.js、无需 pnpm、无需命令行，下载即用。
</p>

<p align="center">
  <samp><strong>简体中文</strong> · <a href="./README.en.md">English</a></samp>
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/License-MIT-4D6BFE?style=flat-square" /></a>
  <img alt="Platform" src="https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-black?style=flat-square" />
  <img alt="dsh version" src="https://img.shields.io/badge/dsh-0.1.2--rc.1-4D6BFE?style=flat-square" />
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-4D6BFE?style=flat-square" />
</p>

> [!IMPORTANT]
> **非官方项目**：本项目与 DeepSeek 官方无关，未获得官方背书。应用图标衍生自官方 logo，
> 仅用于标识与 DeepSeek Harness 的兼容性，详见 [NOTICE](NOTICE)。
>
> 当前为**早期预览版**，基于快速演进中的 `@deepseek-ai/dsh@0.1.2-rc.1`。
> 上游自述其尚未经过安全审计（见下方「安全与数据」）。

<!-- 截图区：发布前请补充真实产品截图（建议 2-3 张：主界面、会话、设置）。
     将截图放入 docs/images/ 后取消下方注释：

<p align="center">
  <img src="docs/images/hero.png" width="100%" alt="DeepSeek Harness Desktop 主界面" />
</p>
<table>
  <tr>
    <td><a href="docs/images/preview-1.png"><img src="docs/images/preview-1.png" alt="对话界面" /></a></td>
    <td><a href="docs/images/preview-2.png"><img src="docs/images/preview-2.png" alt="会话管理" /></a></td>
  </tr>
</table>
-->

---

## 这是什么

[DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness)（`dsh`）是 DeepSeek AI 官方的开源
智能体框架：一个 Node 进程 + 本地 HTTP 服务 + React Web 界面。本项目把它封装成本地桌面应用：

- **1:1 还原**：用系统 WebView（Windows WebView2 / macOS WKWebView）加载官方 Web UI，没有任何功能裁剪与魔改
- **零环境要求**：安装包内嵌 Node 运行时，双击即用
- **数据互通**：会话 / 配置 / 凭据与官方 CLI 完全共享（默认 `~/.dsh`）
- **生命周期干净**：关闭窗口自动回收 Node 子进程，无后台残留

## 下载

<!-- TODO: 发布后在此放入 Windows 安装包直链（推荐 latest 链接），例如：
  <a href="https://github.com/<owner>/deepseek-harness-desktop/releases/latest/download/DeepSeek-Harness-Desktop-Setup_v0.1.2-rc.1_x64.exe">
    <img src="https://img.shields.io/badge/Download-Windows_x64-4D6BFE?style=for-the-badge" alt="Download Windows x64" />
  </a>
-->

从 [Releases](../../releases) 页面按平台下载安装包：

| 平台 | 安装包文件名 | 状态 |
|---|---|---|
| Windows x64 | `DeepSeek-Harness-Desktop-Setup_v<version>_x64.exe` | ✅ 已提供（本机构建） |
| macOS 13+（Apple Silicon） | `DeepSeek-Harness-Desktop_v<version>_aarch64.dmg` | 🔧 由 CI 构建 |
| macOS 13+（Intel） | `DeepSeek-Harness-Desktop_v<version>_x64.dmg` | 🖥️ 本机 Intel Mac 构建 |
| macOS 12 及以下（Apple Silicon） | `DeepSeek-Harness-Desktop_v<version>_aarch64-electron.dmg` | 🔧 由 CI 构建 |
| macOS 12 及以下（Intel） | `DeepSeek-Harness-Desktop_v<version>_x64-electron.dmg` | 🖥️ 本机 Intel Mac 构建 |
| Linux x64 | `DeepSeek-Harness-Desktop_v<version>_amd64.deb` / `.AppImage` | 🔧 由 CI 构建 |

### macOS 选哪个包

macOS 有 **两个内核**（默认版 / 兼容版）× **两种芯片**（Apple Silicon / Intel）共 4 个安装包，
按「芯片 + 内核」两个维度选择：

**第一步，看芯片**（点左上角  →「关于本机」→「处理器/芯片」）：

| 芯片 | 架构后缀 |
|---|---|
| Apple Silicon（M1/M2/M3/M4…） | `aarch64` |
| Intel | `x64` |

**第二步，看内核**（靠文件名后缀区分）：

| | 默认版（无后缀） | 兼容版（`-electron` 后缀） |
|---|---|---|
| 渲染内核 | 系统 WebView（WKWebView） | 内嵌 Electron 43 / Chromium 150 |
| 系统要求 | macOS 13 及以上 | **macOS 12.0 及以上**（含 12） |
| 安装包体积 | 更小 | 约 187 MB（含内嵌 Chromium） |
| 适用场景 | 推荐，系统能升级就用它 | 系统停留在 macOS 12 时的选择 |

> **怎么选**：先按芯片选对架构，再先试默认版；若在 macOS 12 上打开后**白屏**
> （系统 WebView 太旧，解析不了官方前端 bundle），再换带 `-electron` 后缀的兼容版。
> 两版数据完全互通（会话 / 配置 / 凭据都在 `~/.dsh`），桌面层能力一致，可在两者之间直接切换。

> **兼容版的长期建议**：Electron 43 是官方支持 macOS 12 的最后一档，Chromium 安全更新会逐步停止。
> 它定位是**过渡方案**——系统可升级时请迁回默认版。

> **Windows 首次运行提示**：当前安装包尚未进行代码签名，SmartScreen 可能弹出蓝色警告。
> 点击 **「更多信息」→「仍要运行」** 即可继续。

> **macOS**：当前为 ad-hoc 签名，首次打开若被 Gatekeeper 拦截，请右键应用 → 打开。

## 桌面壳的增值能力

在官方 Web UI 之外，桌面壳提供这些**系统层能力**（不含任何 UI 魔改，Web 界面 100% 来自官方）：

- **单实例**：重复启动时自动恢复并聚焦已有窗口，不产生第二个实例
- **下载接管**：会话导出等下载走系统「另存为」对话框，记住上次下载目录
- **安全加固**：日志中的一次性 token 全量脱敏；`dsh web` 仅监听 `127.0.0.1` 随机端口
- **代理抗性**：WebView2 强制不走系统代理，避免本机代理（如 Clash）把 localhost 流量挟持导致 WebSocket 卡死
- **进程守护**：Node 异常退出时自动关闭窗口；stdout/stderr 持续泵送，防止管道缓冲塞满导致 dsh 假死
- **风格统一**：隐藏官方前端重复的 Session 导出弹窗（与真实「另存为」流程去重）

> 这些是当前已实现的桌面层能力。后续差异化功能的规划见 [Roadmap](docs/implementation-plan.md)。

## 构建

### Windows

```powershell
# 依赖：Node.js ≥ 22.19、Rust (msvc)、Visual Studio Build Tools（C++ 工作负载）
powershell -ExecutionPolicy Bypass -File scripts/build-win.ps1
# 指定 dsh 版本（自动同步 build-runtime / tauri.conf / Cargo.toml 三处版本号）
powershell -ExecutionPolicy Bypass -File scripts/build-win.ps1 -DshVersion 0.1.3-alpha.2
```

### macOS

```bash
# 依赖：Node.js ≥ 22.19、Rust、Xcode Command Line Tools
bash scripts/build-mac.sh
```

### macOS 12 兼容版（Electron）

```bash
# 依赖：Node.js ≥ 22.19、npm（hdiutil 随 macOS 内置）
bash scripts/build-mac-electron.sh
# 产物：electron-app/release/DeepSeek-Harness-Desktop_v<version>_<arch>-electron.dmg
```

- 与默认版**完全独立**的一条链路（Electron 43 + 内嵌 Chromium），只复用 `scripts/build-runtime.mjs` 的运行时产物
- 版本号自动同步：从 `scripts/build-runtime.mjs` 读 `DSH_VERSION`，写入 `electron-app/package.json`（拼 `-electron` 后缀）；
  升级 dsh 用 `DSH_VERSION=0.1.6 bash scripts/build-mac-electron.sh`
- 打包后自动断言 `LSMinimumSystemVersion <= 12.0`，防止 `electron` 依赖被解析到更高大版本把 macOS 12 用户挡在门外
- 不要放宽 `electron` 的版本锁（`package.json` 里是精确版本，不是 `^`）

开发模式（不用打包，直接用系统 node + 仓库内已构建的 runtime）：

```bash
cd electron-app && npm start
# 也可用 DSH_BIN 指到其它 dsh 产物
```

### 仅构建运行时（三端通用）

```bash
node scripts/build-runtime.mjs
# 产物：tauri-app/resources/runtime/{node/, dsh-runtime/}（已 gitignore）
```

### CI

`.github/workflows/build.yml` 提供三条链路：Windows / macOS / Linux 的 Tauri 构建矩阵，以及独立的
`build-macos-electron`（Electron 兼容版 dmg + 门槛断言）。推送 `v*` tag 或手动
`workflow_dispatch` 触发，安装包产物上传到 Actions artifacts。

> **Windows 深路径逃生舱**：若工作区路径过深导致 NSIS 打包报 260 字符错误，可设
> `$env:DSH_RUNTIME_DIR = "D:\rt"` 把运行时放到短路径（历史踩坑见
> `docs/stage/stage-2-packaging.md`）。

## 开发模式

```powershell
# dev 模式用系统 node + .stage-p0 的 dsh（无需内嵌运行时）
$env:DSH_BIN = "d:/Codes/deepseek-harness-desktop/.stage-p0/node_modules/@deepseek-ai/dsh/lib/bin.js"
cargo run --manifest-path tauri-app/src-tauri/Cargo.toml
```

> 首次运行前先执行一次 `node scripts/build-runtime.mjs` 或保留占位目录，否则 `cargo run` 会因缺少
> `resources/runtime` 目录报错（仓库已内置 `.gitkeep` 占位）。

## 安全与数据

**请在使用前了解风险**：本应用会在你的电脑上运行一个本地 AI 智能体进程，该智能体
**可以执行命令、读写文件**。上游 DeepSeek Harness [明确声明](https://github.com/deepseek-ai/deepseek-harness/blob/master/SAFETY.md)：

> 该项目尚未经过安全审计，沙箱、审批与权限控制不能保证隔离。

- ⚠️ 只在可信环境中使用；不推荐以高权限账户运行
- ✅ Web 服务仅监听 `127.0.0.1` 随机端口，不向局域网开放
- ✅ 启动一次性 token 全程脱敏处理，下载鉴权复用官方会话 cookie 机制
- ✅ 数据目录与官方 CLI 共享（默认 `~/.dsh`），升级 / 卸载不清空数据
- ℹ️ 内嵌 Node 运行时可能被杀毒软件误报（AI 工具链常见现象），添加信任即可

## 版本策略

- 桌面版版本号 = 内置的 `@deepseek-ai/dsh` 版本号（当前 `0.1.2-rc.1`）
- 升级 = 改 `scripts/build-runtime.mjs` 的 `DSH_VERSION` + 同步 `tauri.conf.json` / `Cargo.toml` 的
  version，再重新构建（`build-win.ps1 -DshVersion` 可一键同步三处）
- `deepseek-harness/` 目录仅作上游源码本地参考（已 gitignore）；当前运行时通过 npm 官方包构建，
  源码构建路径的规划见 `docs/github-release-sync-and-source-build-strategy.md`

## 文档

| 文档 | 内容 |
|---|---|
| [`tauri-app/README.md`](tauri-app/README.md) | 默认版桌面壳原理、路径解析优先级、踩坑记录 |
| [`docs/electron-macos-compat-spec.md`](docs/electron-macos-compat-spec.md) | macOS 12 兼容版（Electron）实施方案规格 |
| [`docs/electron-macos-compat-implementation.md`](docs/electron-macos-compat-implementation.md) | macOS 12 兼容版实施计划与验收记录 |
| [`docs/dsh-desktop-analysis.md`](docs/dsh-desktop-analysis.md) | dsh 架构分析与桌面封装方案选型 |
| [`docs/implementation-plan.md`](docs/implementation-plan.md) | 总体实施计划与阶段划分 |
| [`docs/custom-web-extension-and-upgrade-architecture.md`](docs/custom-web-extension-and-upgrade-architecture.md) | Web 定制与官方升级隔离规范 |
| [`docs/github-release-sync-and-source-build-strategy.md`](docs/github-release-sync-and-source-build-strategy.md) | GitHub Release 同步与源码构建方案（规划） |
| [`docs/stage/`](docs/stage/) | 各阶段实施计划与验收记录 |

## 社区与反馈

- 问题与建议 → [Issues](../../issues)
- 贡献 → [CONTRIBUTING.md](CONTRIBUTING.md)
- 变更记录 → [CHANGELOG.md](CHANGELOG.md)

## 许可证

本项目以 [MIT](LICENSE) 协议开源 © DeepSeek Harness Desktop contributors。
上游 DeepSeek Harness 及其依赖保持各自的许可证与商标政策，详见 [NOTICE](NOTICE)。