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
# 1. 组装运行时（下载 node + pnpm 装 dsh + Windows 创建 junction）
node scripts/build-runtime.mjs

# 2. 打包（产出安装包到 src-tauri/target/release/bundle/）
$env:NODE_OPTIONS = ""
npx -y @tauri-apps/cli build
```

## 路径解析优先级

| 优先级 | node | dsh bin.js |
|---|---|---|
| 1 | — | `DSH_BIN` 环境变量 |
| 2 | `resource_dir/runtime/node/`（内嵌） | `resource_dir/runtime/dsh-runtime/...`（内嵌） |
| 3 | 系统 PATH 的 node | `../../.stage-p0/...`（dev 回退） |

## 关键实现细节（踩坑记录）

1. **NODE_OPTIONS 清除**：spawn dsh 必须 `env_remove("NODE_OPTIONS")`，否则 IDE 注入的 `--require` shim 会卡死 dsh boot
2. **`\\?\` 前缀**：Tauri 打包后 `resource_dir()` 返回带 `\\?\` 前缀的路径，需 `normalize_path` 去掉，否则 node 无法解析 bin.js 参数
3. **Windows 长路径**：pnpm 用 `nodeLinker: hoisted` + junction `D:\drt` 规避 makensis 的 260 字符限制

## 依赖

- Rust stable（msvc 工具链）
- Visual Studio Build Tools（C++ 工作负载）
- pnpm（组装运行时）
- WebView2 运行时（Win10/11 一般已预装）

## 目录结构

```
tauri-app/
├── frontend/index.html          # 编译期占位页
├── resources/runtime/           # build-runtime 产物（gitignore）
│   ├── node/                    # node 二进制
│   └── dsh-runtime/             # pnpm 安装的 dsh
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json          # 通用打包配置
    ├── tauri.windows.conf.json  # Windows 专用（junction 短路径）
    ├── build.rs
    ├── icons/
    └── src/main.rs              # spawn + 解析 + WebView + 生命周期
```
