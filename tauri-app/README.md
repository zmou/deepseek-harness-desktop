# dsh-desktop（Tauri 桌面壳）

DeepSeek Harness 的桌面壳，用 Tauri 2 把 dsh web 封装成本地桌面应用。普通用户安装后无需 Node、无需命令行即可使用。

## 原理

桌面壳启动后，spawn 一个 Node 子进程（内嵌）运行 `dsh web --port 0 --no-open`，从 stdout 解析出实际监听地址，再用系统 WebView 加载该地址，实现 1:1 还原官方 Web UI。

## 开发模式运行

```powershell
# 1. 指定 dsh 的 bin.js 路径（dev 模式用系统 node + .stage-p0）
$env:DSH_BIN = "d:/Codes/deepseek-harness-desktop/.stage-p0/node_modules/@deepseek-ai/dsh/lib/bin.js"

# 2. 启动
Set-Location "d:/Codes/deepseek-harness-desktop/tauri-app/src-tauri"
cargo run
```

## 打包（生产模式）

```powershell
# 1. 组装运行时（下载 node + npm 装 dsh，产物在 tauri-app/resources/runtime/）
node scripts/build-runtime.mjs

# 2. 打包（产出安装包到 src-tauri/target/release/bundle/）
$env:NODE_OPTIONS = ""
Set-Location "d:/Codes/deepseek-harness-desktop/tauri-app/src-tauri"
npx -y @tauri-apps/cli build
```

> 一键脚本：`scripts/build-win.ps1`（Windows）/ `scripts/build-mac.sh`（macOS）。
> Windows 若因工作区路径过深触发 NSIS 260 字符错误，可设 `DSH_RUNTIME_DIR` 指向短路径（如 `D:\rt`）。

## 路径解析优先级

| 优先级 | node | dsh bin.js |
|---|---|---|
| 1 | — | `DSH_BIN` 环境变量 |
| 2 | `resource_dir/runtime/node/`（内嵌） | `resource_dir/runtime/dsh-runtime/...`（内嵌） |
| 3 | 系统 PATH 的 node | `../../.stage-p0/...`（dev 回退） |

## 关键实现细节（踩坑记录）

1. **NODE_OPTIONS 清除**：spawn dsh 必须 `env_remove("NODE_OPTIONS")`，否则 IDE 注入的 `--require` shim 会卡死 dsh boot
2. **`\\?\` 前缀**：Tauri 打包后 `resource_dir()` 返回带 `\\?\` 前缀的路径，需 `normalize_path` 去掉，否则 node 无法解析 bin.js 参数
3. **Windows 长路径**：runtime 现用 npm 扁平布局（无 pnpm junction），实测最长路径 < 260 字符，可放仓库内；早期用 pnpm + junction `D:\drt` 规避 260 字符限制，现仅作为 `DSH_RUNTIME_DIR` 逃生舱保留（历史见 `docs/stage/stage-2-packaging.md`）

## 依赖

- Rust stable（msvc 工具链）
- Visual Studio Build Tools（C++ 工作负载）
- Node.js ≥ 22.19（仅构建期；`build-runtime.mjs` 负责下载内嵌运行时）
- WebView2 运行时（Win10/11 一般已预装）

## 目录结构

```
tauri-app/
├── frontend/index.html          # 编译期占位页
├── resources/runtime/           # build-runtime 产物（gitignore）
│   ├── node/                    # node 二进制
│   └── dsh-runtime/             # npm 安装的 dsh
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json          # 通用打包配置
    ├── tauri.windows.conf.json  # Windows 专用（nsis 目标）
    ├── tauri.linux.conf.json    # Linux 专用（deb/appimage 目标）
    ├── tauri.macos.conf.json    # macOS 专用（app/dmg 目标）
    ├── build.rs
    ├── icons/
    └── src/main.rs              # spawn + 解析 + WebView + 生命周期
```
