# 阶段 1：桌面壳骨架 — 实施计划

> 状态：待执行
> 日期：2026-08-29
> 上级计划：`docs/implementation-plan.md`（阶段 1，P1）
> 前置依赖：`docs/stage/stage-0-results.md`（阶段 0 已全部通过，结论 Go）
> 目标：用 Tauri 2 搭建桌面壳骨架，在本机（Windows）跑通"双击启动 → 自动拉起 dsh web → WebView 显示官方 UI → 关闭回收进程"的最小闭环。

---

## 0. 概述

### 0.1 目标

在不修改 dsh 源码、不引入自定义插件、不做三端打包的前提下，产出可运行的 Tauri 桌面壳骨架，验证桌面端封装的核心链路成立。

### 0.2 完成标准（Definition of Done）

以下 5 个任务（T1~T5）全部完成，且满足第 5 节验收标准：

- 本机 `cargo run` 能完整跑通最小闭环
- WebView 显示的是 dsh 官方 React UI（1:1 还原）
- 关闭窗口后无残留 node 进程
- 计划文档 + 运行说明留存

### 0.3 时间预算

| 项 | 预算 |
|---|---|
| Rust 工具链安装 | 0.5 天（含下载/编译等待） |
| Tauri 工程骨架 + 启动链路 + 生命周期 | 1.5 天 |
| 本机验收与调优 | 0.5 天 |
| **合计** | **2.5 天**（1 人） |

### 0.4 明确不在本阶段范围

- ❌ Node 内嵌（阶段 2）
- ❌ 自定义 cordis 插件（阶段 3）
- ❌ 三端打包 / 签名（阶段 2）
- ❌ 自定义 UI 界面（WebView 直接加载 dsh 现有 React SPA）
- ❌ git submodule 化、CI 同步（阶段 4）

---

## 1. 环境现状与前置条件

### 1.1 环境现状（2026-08-29 本机实测）

| 项 | 状态 | 说明 |
|---|---|---|
| OS | Windows（win32）+ PowerShell | — |
| Node | v24.14.0 ✅ | 阶段 1 用系统 node |
| pnpm | 11.7.0 ✅ | 备用 |
| **Rust** | ❌ **未安装**（cargo/rustc/rustup 均 CommandNotFoundException） | **T1 必须解决** |
| WebView2 | 待检查 | Win10/11 一般已预装 |
| dsh 运行时 | `.stage-p0/node_modules/@deepseek-ai/dsh/lib/bin.js` ✅ | 阶段 0 已装，直接复用 |

### 1.2 前置检查清单（T1 之前先跑）

```powershell
# 1. 确认 dsh 运行时仍在（阶段 0 工作区是否保留）
Test-Path "d:/Codes/deepseek-harness-desktop/.stage-p0/node_modules/@deepseek-ai/dsh/lib/bin.js"

# 2. 确认 WebView2 运行时（Windows Tauri 硬依赖）
Get-ItemProperty "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -ErrorAction SilentlyContinue | Select-Object -Property name, version

# 3. 确认 rustup-init 下载地址可达
# https://win.rustup.rs/x86_64
```

> 若 `.stage-p0` 已清理（阶段 0 报告建议 P1 前清理），T1 前需重新安装 dsh 运行时（命令见 1.3）。

### 1.3 dsh 运行时恢复（仅当 .stage-p0 已被清理时执行）

```powershell
$runtime = "d:/Codes/deepseek-harness-desktop/.stage-p0"
New-Item -ItemType Directory -Force -Path $runtime | Out-Null
Set-Location $runtime
pnpm add @deepseek-ai/dsh --reporter=append-only
# 完成后验证：
node "$runtime/node_modules/@deepseek-ai/dsh/lib/bin.js" --version
```

---

## 2. 技术方案与关键决策

### 2.1 运行时拓扑（阶段 1 形态）

```
┌─────────────────────────────────────────┐
│  Tauri 应用（cargo run 启动）             │
│  Rust 主进程                              │
│   1. resolve dsh 路径（DSH_BIN 环境变量）  │
│   2. spawn node <bin.js> web --port 0 --no-open │
│   3. BufReader 逐行读 stdout，正则解析 URL  │
│   4. WebviewWindowBuilder 加载 URL         │
│   5. 退出钩子 kill child；线程监听 child 退出│
└──────────────┬──────────────────────────┘
               │ 加载本地 URL
┌──────────────▼──────────────────────────┐
│  系统 WebView（WebView2）                 │
│  = dsh 官方 React SPA（1:1）              │
└──────────────┬──────────────────────────┘
               │ HTTP 127.0.0.1:<随机端口>
┌──────────────▼──────────────────────────┐
│  Node 子进程（系统 node）                  │
│  dsh web --port 0 --no-open              │
│  数据目录 ~/.dsh（与官方 CLI 互通）        │
└─────────────────────────────────────────┘
```

### 2.2 关键决策（依据阶段 0 验证结论）

| # | 决策 | 依据（阶段 0 实测） |
|---|---|---|
| 1 | 用 `std::process::Command` 而非 tauri-plugin-shell | 只需 spawn+读 stdout+kill，标准库足够，避免依赖 |
| 2 | `--port 0` 而非固定端口 | OS 分配空闲端口，彻底规避冲突（冲突会 EADDRINUSE 退出） |
| 3 | stdout 正则解析而非健康检查轮询 | URL 行 15s 内无缓冲 flush，格式稳定 `dsh web: http://127.0.0.1:PORT` |
| 4 | **无 token 处理** | 阶段 0 实测 npm 版 dsh web 无 token，根路径直接 200 |
| 5 | dsh 路径从 `DSH_BIN` 环境变量读取 | 阶段 2 切内嵌路径只改默认值，main.rs 逻辑不变 |
| 6 | 30s 启动超时兜底 | 阶段 0 观察到一次冷启动卡住，防止桌面壳永久挂起 |

### 2.3 关键代码结构（main.rs 职责划分）

```rust
// 全局子进程句柄（线程安全）
struct AppState {
    child: Mutex<Option<std::process::Child>>,
}

// —— 职责函数 ——
fn resolve_node() -> String                          // 阶段1返回"node"，阶段2改内嵌绝对路径
fn resolve_dsh_bin() -> PathBuf                      // 读 DSH_BIN，缺省回退 .stage-p0 路径
fn spawn_dsh() -> Result<Child>                      // spawn node，stdout/stderr piped
fn wait_for_url(child: &mut Child) -> Result<String> // BufReader 逐行读，正则解析，30s 超时
fn kill_child(state: &AppState)                      // 退出时杀子进程（含 kill 失败兜底）
fn watch_child_exit(child: Child, app: AppHandle)    // 独立线程 child.wait() 监听异常退出
```

URL 解析正则（阶段 0 已确认格式）：

```rust
let re = Regex::new(r"dsh web: (http://127\.0\.0\.1:\d+)")?;
```

---

## 3. 目录结构（本阶段产出）

```
deepseek-harness-desktop/
├── tauri-app/                          # 阶段 1 新建：Tauri 桌面壳
│   ├── src-tauri/
│   │   ├── Cargo.toml                  # 依赖 tauri 2、regex
│   │   ├── tauri.conf.json             # 窗口标题/尺寸，阶段 1 最小化
│   │   ├── build.rs                    # tauri_build::build()
│   │   ├── src/
│   │   │   └── main.rs                 # spawn + 解析 + WebView + 生命周期
│   │   └── icons/
│   │       └── icon.ico               # 占位图标
│   └── .gitignore
└── docs/
    └── stage/
        └── stage-1-desktop-shell.md    # 本文件
```

---

## 4. 任务分解

---

### T1 安装 Rust 工具链

**目的**：补上阶段 1 唯一的环境缺口，让 `cargo run` 可用。

**步骤**：

```powershell
# 方式 A（推荐）：winget 一键安装
winget install --id Rustlang.Rustup -e

# 方式 B：官网 rustup-init 安装（默认 msvc 工具链）
# 下载 https://win.rustup.rs/x86_64 得到 rustup-init.exe 并运行
# 交互提示选 1) Proceed with installation (default)

# 关键：Windows 上 rustup 默认装 msvc 工具链，需要 Visual Studio Build Tools 的 C++ 工作负载
# 若未装 Build Tools，先装（Tauri 编译 Rust 依赖必需 C++ 链接器）：
winget install --id Microsoft.VisualStudio.2022.BuildTools -e `
  --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive --norestart"

# 重开终端使 PATH 生效，验证：
rustc --version
cargo --version
rustup show
```

**预期结果**：
- `rustc --version` 返回 stable 版本（如 1.8x.x）
- `cargo --version` 正常
- 默认工具链为 `stable-x86_64-pc-windows-msvc`

**通过标准**：
- [ ] `cargo --version` 可执行
- [ ] 能通过 `rustup default stable` 确认默认工具链
- [ ] 记录 rustc 版本

**失败排查**：
- 若编译时报 `link.exe not found` → MSVC Build Tools 未装或未含 C++ 工作负载，重装 Build Tools
- 若 winget 不可用 → 走方式 B 官网下载 rustup-init

**产出物**：Rust 版本记录（写入运行说明）

---

### T2 手写最小 Tauri 2 工程骨架

**目的**：建立无前端的最小 Tauri 工程（WebView 直接加载外部 URL，不需要 `src/` 前端目录）。

**步骤**：

1. 创建目录结构：

```powershell
Set-Location "d:/Codes/deepseek-harness-desktop"
New-Item -ItemType Directory -Force -Path "tauri-app/src-tauri/src" | Out-Null
New-Item -ItemType Directory -Force -Path "tauri-app/src-tauri/icons" | Out-Null
```

2. 编写 `src-tauri/Cargo.toml`：

```toml
[package]
name = "dsh-desktop"
version = "0.1.0"
description = "DeepSeek Harness Desktop"
edition = "2021"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = [] }
regex = "1"

[features]
custom-protocol = ["tauri/custom-protocol"]
```

3. 编写 `src-tauri/tauri.conf.json`（阶段 1 最小化）：

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "dsh-desktop",
  "version": "0.1.0",
  "identifier": "com.deepseek.dsh-desktop",
  "build": {
    "beforeDevCommand": "",
    "devUrl": "",
    "beforeBuildCommand": "",
    "frontendDist": ""
  },
  "app": {
    "windows": [
      {
        "title": "DeepSeek Harness",
        "width": 1400,
        "height": 900,
        "resizable": true,
        "url": "about:blank"
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/icon.ico"
    ]
  }
}
```

> 说明：`url` 先用 `about:blank` 占位（编译期需要窗口存在）；真实 URL 在运行时由 main.rs 通过 `WebviewWindowBuilder` 重建窗口加载（见 T3/T4）。`devUrl`/`frontendDist` 留空，因为我们无前端源码。

4. 编写 `src-tauri/build.rs`：

```rust
fn main() {
    tauri_build::build()
}
```

5. 编写占位 `src-tauri/src/main.rs`（先放最小可编译入口，T3/T4 再填充完整逻辑）：

```rust
fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

6. 准备占位图标 `icons/icon.ico`（可用 Tauri 官方默认图标，或 `tauri icon` 命令生成，阶段 5 再定制）。

7. 编写 `tauri-app/.gitignore`：

```
target/
src-tauri/target/
```

8. 首次编译验证（**首次编译会下载并编译约 300+ crate，耗时 5-20 分钟，属正常**）：

```powershell
Set-Location "d:/Codes/deepseek-harness-desktop/tauri-app/src-tauri"
cargo build
```

**预期结果**：`cargo build` 成功，生成 `target/debug/dsh-desktop.exe`。

**通过标准**：
- [ ] `cargo build` 无错误（warning 可接受）
- [ ] 产物 `target/debug/dsh-desktop.exe` 存在
- [ ] 运行 exe 能弹出空白窗口（`about:blank`）

**失败排查**：
- `link.exe` 缺失 → T1 的 MSVC Build Tools 问题
- tauri.conf.json schema 报错 → 对照 Tauri 2 schema 修正字段
- 图标文件缺失报错 → 补 `icons/icon.ico`

**产出物**：可编译的最小 Tauri 工程

---

### T3 实现启动链路（spawn + 解析 URL）

**目的**：实现主进程拉起 dsh web 并解析出实际 URL 的核心逻辑。

**步骤**：

1. 在 `main.rs` 中实现核心函数（完整逻辑，直接落地）：

```rust
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, RunEvent};

// 全局子进程句柄
struct AppState {
    child: Mutex<Option<Child>>,
}

/// 阶段1返回系统 node，阶段2改为内嵌 node 绝对路径
fn resolve_node() -> String {
    "node".to_string()
}

/// dsh bin 路径：优先 DSH_BIN 环境变量，缺省回退 .stage-p0
fn resolve_dsh_bin() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("DSH_BIN") {
        let pb = PathBuf::from(&p);
        if pb.exists() { return Ok(pb); }
        return Err(format!("DSH_BIN 指向的文件不存在: {p}"));
    }
    // 缺省回退到阶段0工作区（相对仓库根）
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let default = cwd
        .join("../.stage-p0/node_modules/@deepseek-ai/dsh/lib/bin.js");
    if default.exists() { return Ok(default); }
    Err("未找到 dsh bin.js，请设置 DSH_BIN 环境变量".to_string())
}

/// spawn node 运行 dsh web
fn spawn_dsh() -> Result<Child, String> {
    let node = resolve_node();
    let bin = resolve_dsh_bin()?;
    Command::new(node)
        .arg(&bin)
        .arg("web")
        .arg("--port").arg("0")
        .arg("--no-open")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn dsh 失败: {e}"))
}

/// 逐行读 stdout，正则解析 URL，30s 超时
fn wait_for_url(child: &mut Child) -> Result<String, String> {
    let re = regex::Regex::new(r"dsh web: (http://127\.0\.0\.1:\d+)").unwrap();
    let stdout = child.stdout.take().ok_or("无法获取 stdout")?;
    let reader = BufReader::new(stdout);
    let deadline = Instant::now() + Duration::from_secs(30);

    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        println!("[dsh] {line}");
        if let Some(cap) = re.captures(&line) {
            return Ok(cap[1].to_string());
        }
        if Instant::now() > deadline {
            break;
        }
    }
    Err("30s 内未解析到 dsh URL".to_string())
}
```

> 说明：正则解析到 URL 后立即返回，无需等 stdout 关闭。`for line in reader.lines()` 会持续读直到 URL 出现或超时（超时靠 loop 内 `deadline` 判断 + 后续 kill）。

2. 在 `main` 中串联启动（T4 完成后统一整合，此处先聚焦 spawn + URL 解析可独立测试）。

**通过标准**：
- [ ] `resolve_dsh_bin()` 能定位到 dsh bin.js
- [ ] `spawn_dsh()` 能启动子进程
- [ ] `wait_for_url()` 能在 15s 内解析出 `http://127.0.0.1:<port>`
- [ ] 超时场景返回明确错误

**失败排查**：
- 解析不到 URL → 检查 stdout 是否被重定向丢失（阶段 0 已验证无缓冲，应正常）
- spawn 失败 → 检查 node 在 PATH、bin.js 路径正确

**产出物**：可独立验证的 spawn + 解析逻辑

---

### T4 实现生命周期管理

**目的**：确保"窗口关闭→杀进程"和"进程异常退出→提示关窗"两条链路可靠。

**步骤**：

1. 实现 `kill_child` 与 `watch_child_exit`：

```rust
fn kill_child(state: &AppState) {
    let mut guard = state.child.lock().unwrap();
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// 独立线程监听 child 退出，异常退出时通知前端并关闭窗口
fn watch_child_exit(mut child: Child, app: AppHandle) {
    std::thread::spawn(move || {
        let status = child.wait();
        // 进程已退出（正常或异常）
        if let Ok(status) = status {
            if !status.success() {
                let _ = app.emit("dsh-exited", status.code().unwrap_or(-1));
                // 关闭所有窗口
                if let Ok(windows) = app.webview_windows() {
                    for (_, w) in windows.iter() {
                        let _ = w.close();
                    }
                }
            }
        }
    });
}
```

2. 在 `main` 中整合完整生命周期：

```rust
fn main() {
    let app = tauri::Builder::default()
        .manage(AppState { child: Mutex::new(None) })
        .setup(|app| {
            let handle = app.handle().clone();

            // 1. spawn dsh
            let mut child = spawn_dsh().map_err(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            })?;

            // 2. 解析 URL
            let url = match wait_for_url(&mut child) {
                Ok(u) => u,
                Err(e) => {
                    eprintln!("[dsh] 启动失败: {e}");
                    // 读取 stderr 诊断
                    if let Some(err) = child.stderr.take() {
                        let r = BufReader::new(err);
                        for l in r.lines().flatten() { eprintln!("[dsh-err] {l}"); }
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    std::process::exit(1);
                }
            };

            // 3. 保存 child 句柄到全局状态
            {
                let state = app.state::<AppState>();
                *state.child.lock().unwrap() = Some(child);
            }

            // 4. 用解析到的 URL 重建主窗口（替代 about:blank）
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.navigate(&url);
            }

            // 5. 启动 child 退出监听线程（需要重新 take child 或共享状态）
            //    注：watch_child_exit 需要一个 Child 所有权，
            //    因此采用"取出句柄给线程，退出时再 kill"的共享设计，
            //    具体以实现为准（见下方实现注意）。

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // 退出事件：杀子进程
    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            let state = app_handle.state::<AppState>();
            kill_child(&state);
        }
    });
}
```

**实现注意（所有权与线程安全）**：

- `Child` 句柄只能有一个所有者。设计上需在"主进程退出时 kill"和"线程 wait 监听退出"之间共享。推荐方案：**把 `Child` 所有权交给监听线程**，全局 `AppState` 存 `Child` 的进程 id（`u32`）+ 一个可跨线程的 kill 手段；或者用 `Arc<Mutex<Option<Child>>>` 包在 `AppState` 里，监听线程从 Mutex 里 `take()` 拿到 Child 后 `wait()`，主进程退出时若 Mutex 里还有 Child 就 kill。
- 落地时优先用最直接可靠的方案：**监听线程 take 所有权 + wait**，退出钩子 kill 时若已被 take 则用记录的 pid 兜底 kill（Windows 可用 `taskkill /pid <pid> /T /F`，或保存 pid 后主进程再 spawn 时已持有 handle，退出时优先 kill handle）。

**通过标准**：
- [ ] 关闭窗口 → node 子进程被回收（`Get-Process node` 无残留）
- [ ] 手动 kill node 子进程 → 窗口自动关闭（异常退出链路）
- [ ] 正常退出与异常退出均无孤儿进程

**失败排查**：
- 关窗后 node 残留 → 检查退出钩子是否触发（`RunEvent::Exit` 在 tauri 2 的语义）、kill 是否成功（Windows 下可能需 `taskkill /T` 杀子进程树）
- 子进程退出未关窗 → 检查 `watch_child_exit` 线程是否正确持有 child、`app.emit` 事件是否被前端接收

**产出物**：完整可运行的生命周期逻辑

---

### T5 本机跑通验收

**目的**：端到端验证最小闭环，产出运行说明。

**步骤**：

```powershell
# 1. 设置 DSH_BIN 指向 dsh（若未用默认 .stage-p0 路径）
$env:DSH_BIN = "d:/Codes/deepseek-harness-desktop/.stage-p0/node_modules/@deepseek-ai/dsh/lib/bin.js"

# 2. 启动桌面壳
Set-Location "d:/Codes/deepseek-harness-desktop/tauri-app/src-tauri"
cargo run
```

**验收操作**：
1. 观察终端日志：应打印 `dsh web: http://127.0.0.1:<port>` 且窗口自动加载出 dsh 官方 UI
2. 确认 UI 与浏览器访问 `npx @deepseek-ai/dsh web` 一致（1:1）
3. 关闭窗口，执行 `Get-Process node` 确认无残留
4. 再次运行，启动后手动 `Stop-Process` 杀掉 node 子进程，确认窗口自动关闭

**通过标准**：
- [ ] `cargo run` 一键启动完整闭环
- [ ] WebView 显示 dsh 官方 React UI
- [ ] 关闭窗口无残留 node 进程
- [ ] 子进程异常退出窗口自动关闭
- [ ] 终端无致命错误日志

**产出物**：运行说明（写入 `tauri-app/README.md` 或本计划附录）

---

## 5. 验收标准汇总（DoD）

| # | 验收项 | 判定 |
|---|---|---|
| 1 | `cargo run` 能启动完整闭环 | 必需 |
| 2 | WebView 显示 dsh 官方 UI（1:1） | 必需 |
| 3 | 关闭窗口无残留 node 进程 | 必需 |
| 4 | 子进程异常退出窗口自动关闭 | 必需 |
| 5 | dsh 路径可通过 DSH_BIN 切换 | 必需 |
| 6 | 启动超时 30s 有兜底错误处理 | 必需 |
| 7 | 无 dsh 源码改动（git diff 验证） | 必需 |

---

## 6. 风险与应对

| 风险 | 等级 | 应对 |
|---|---|---|
| Rust/MSVC 工具链安装失败或编译报错 | 中 | 走 winget 标准流程；link.exe 缺失重装 Build Tools C++ 工作负载 |
| 首次 cargo build 极慢（300+ crate） | 低 | 属正常，耐心等待；用 `cargo build` 而非 `cargo run` 分离编译与运行 |
| Tauri 2 窗口 URL 运行时切换（about:blank → 本地 URL）有坑 | 中 | 用 `window.navigate(url)`；若不行则改为 setup 阶段直接用 `WebviewWindowBuilder` 新建窗口 |
| Child 句柄所有权导致退出竞态 | 中 | 用 Mutex 保存 + 监听线程 take 所有权 + pid 兜底 kill（见 T4 实现注意） |
| Windows 杀进程需杀进程树（node 有子进程） | 中 | 退出时 `child.kill()` 若不够，补 `taskkill /pid <pid> /T /F` |
| .stage-p0 已被清理导致找不到 dsh | 低 | 1.3 节有恢复命令；DSH_BIN 可显式指定 |

---

## 7. 阶段出口标准（Go / No-Go）

**Go（进入阶段 2 打包链路）**：
- 5 个任务全部完成，第 5 节 7 项验收全部通过
- 生命周期两条链路（关窗杀进程、异常退出关窗）均验证可靠

**No-Go**：
- T1 失败（Rust 环境装不上）→ 重新评估技术栈（退回 Electron）
- T3 失败（无法解析 URL）→ 回到阶段 0 结论排查，或改健康检查轮询方案
- T4 失败（孤儿进程无法根治）→ 暂停，优先解决进程回收可靠性再进入阶段 2

---

## 8. 交付物清单

| 文件 | 说明 |
|---|---|
| `tauri-app/src-tauri/` | 完整 Tauri 工程（Cargo.toml、tauri.conf.json、build.rs、main.rs、icons） |
| `tauri-app/.gitignore` | 忽略 target |
| `tauri-app/README.md` | 运行说明（如何 cargo run、DSH_BIN 配置） |
| 本计划文档 | `docs/stage/stage-1-desktop-shell.md` |
| 验收记录 | 可选：`docs/stage/stage-1-results.md`（验收结果留存） |
