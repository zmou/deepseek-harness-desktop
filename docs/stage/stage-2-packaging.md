# 阶段 2：打包链路 — 实施计划

> 状态：待执行
> 日期：2026-08-29
> 上级计划：`docs/implementation-plan.md`（阶段 2，P2）
> 前置依赖：`docs/stage/stage-1-results.md`（阶段 1 已跑通桌面壳骨架）
> 目标：把 dsh 完整运行时（Node 二进制 + dsh 依赖树）内嵌进 Tauri 安装包，让普通用户安装后无需装 Node、无需命令行即可使用，产出可分发安装包。

---

## 0. 概述

### 0.1 目标

把桌面壳从"依赖系统 Node + 外部 dsh 路径"升级为"完全自包含"：构建期脚本组装运行时 → 打包期 `bundle.resources` 整体带入安装包 → 运行期 `resource_dir` 解析内嵌路径。

### 0.2 完成标准（Definition of Done）

以下 6 个任务（T1~T6）完成，且满足第 5 节验收标准：

- 本机产出 Windows 安装包，安装到干净环境后**无需 Node** 也能跑通
- 终端/子进程功能（native 模块）不因编译产物缺失而崩
- 三端 CI 配置就绪（macOS/Linux 产物由 CI 产出）

### 0.3 时间预算

| 项 | 预算 |
|---|---|
| build-runtime 脚本 + 内嵌路径改造 + 打包配置 | 2 天 |
| 本机 Windows 打包验证 + native 回归 | 1.5 天 |
| 三端 CI 配置 | 0.5 天 |
| **合计** | **4 天**（1 人） |

### 0.4 明确不在本阶段范围

- ❌ 自定义 cordis 插件（阶段 3）
- ❌ 版本同步机制 / submodule 化（阶段 4）
- ❌ 自动更新（阶段 5）
- ❌ 正式代码签名证书（先用测试证书占位）

---

## 1. 环境现状与前置条件

### 1.1 环境现状（基于阶段 0/1 实测）

| 项 | 状态 |
|---|---|
| OS | Windows（win32）+ PowerShell |
| Rust | 1.98.0（msvc），VS Community 2026 提供链接器 ✅ |
| Node | v24.14.0（系统，阶段 2 起不再依赖） |
| pnpm | 11.7.0 ✅ |
| 桌面壳 | 阶段 1 已跑通（`tauri-app/src-tauri/main.rs`） |
| dsh 运行时 | `.stage-p0/node_modules/@deepseek-ai/dsh`（192.9MB，供开发回退） |

### 1.2 关键实测数据（阶段 0 结论，决定阶段 2 设计）

1. node_modules 体积 **192.9MB**（446 包）；node 二进制 zip 34.5MB（解压约 80-100MB）→ 安装包预估 **250-300MB**
2. **必须用 pnpm**：npm 串行 fetch 500+ 包元数据卡死，pnpm 并发仅几十秒
3. pnpm 默认忽略 5 个 native 模块 build scripts：`node-pty`（有全平台 prebuild）、`koffi`（无编译产物）、`@deepseek-ai/dsh-subprocess-local`、`protobufjs`、`@google/genai`
4. dsh 启动：`node <bin.js> web --port 0 --no-open`；bin.js 位于 `node_modules/@deepseek-ai/dsh/lib/bin.js`
5. Node engines：`^22.19 || >=24`

---

## 2. 技术方案与关键决策

### 2.1 打包流程架构

```
build-runtime 脚本
  └─> resources/runtime/{node/ + dsh-runtime/}
        └─> tauri build（bundle.resources 打包）
              └─> NSIS / dmg / deb 安装包
                    └─> 用户安装
                          └─> resource_dir 解析内嵌 node + dsh bin.js
                                └─> spawn dsh web 跑通
```

### 2.2 关键决策与理由

| # | 决策 | 理由 |
|---|---|---|
| 1 | 用 `bundle.resources` 而非 externalBin/sidecar | dsh node_modules 是 446 包目录树，resources 支持整目录递归打包保留结构；sidecar 面向单二进制 |
| 2 | resources 用**映射形式**精确控制 | `{ "resources/runtime/": "runtime/" }` 固定到安装包内 `runtime/` 相对路径，避免绝对路径被改写为 `_root_` 的坑 |
| 3 | build-runtime 用 pnpm | 阶段 0 实测 npm 卡死 |
| 4 | native 模块用 `--config.dangerouslyAllowAllBuilds`（或 approve-builds）| 否则 node-pty/koffi 静默失效，终端功能崩 |
| 5 | dev/prod 双路径解析 | `main.rs` 用 `cfg!(debug_assertions)` 或探测 `resource_dir` 区分开发（系统 node + .stage-p0）与生产（内嵌 node + runtime），保证阶段 1 `cargo run` 调试不退化 |
| 6 | `DSH_BIN` 保留为最高优先级覆盖 | 便于排查打包后路径问题 |

### 2.3 运行时目录结构（脚本产物）

```
tauri-app/resources/runtime/
├── node/                              # node 二进制（各平台）
│   └── node.exe                       # Windows；其他平台为 node
└── dsh-runtime/                       # pnpm 安装的 dsh
    ├── package.json
    └── node_modules/
        └── @deepseek-ai/dsh/lib/bin.js
```

---

## 3. 目录结构（本阶段产出）

```
deepseek-harness-desktop/
├── tauri-app/
│   ├── resources/runtime/             # [NEW] build-runtime 产物（gitignore）
│   ├── src-tauri/
│   │   ├── src/main.rs                # [MODIFY] 内嵌路径解析
│   │   ├── tauri.conf.json            # [MODIFY] bundle 配置
│   │   └── icons/                     # [MODIFY] 多尺寸图标
│   └── .gitignore                     # [MODIFY] 忽略 resources/runtime/
├── scripts/
│   └── build-runtime.mjs              # [NEW] 运行时组装脚本
├── .github/workflows/
│   └── build.yml                      # [NEW] 三端 CI matrix
└── docs/stage/
    └── stage-2-packaging.md           # 本文件
```

---

## 4. 任务分解

---

### T1 落盘计划文档

**目的**：产出本计划文档（本任务已完成）。

**产出物**：`docs/stage/stage-2-packaging.md`

---

### T2 编写 build-runtime.mjs 运行时组装脚本

**目的**：一键组装 `resources/runtime/`，作为构建期唯一交接点。

**脚本职责**（Node `.mjs`，跨平台）：

```javascript
// scripts/build-runtime.mjs 伪结构
// 1. 解析目标平台/架构：process.platform + process.arch
// 2. 下载 node 官方二进制到 resources/runtime/node/
//    - Windows x64: node-v24.14.0-win-x64.zip
//    - macOS arm64: node-v24.14.0-darwin-arm64.tar.gz
//    - macOS x64:   node-v24.14.0-darwin-x64.tar.gz
//    - Linux x64:   node-v24.14.0-linux-x64.tar.xz
// 3. 在 resources/runtime/dsh-runtime/ 执行：
//    pnpm add @deepseek-ai/dsh --config.dangerouslyAllowAllBuilds
// 4. 校验 bin.js 与 node 二进制存在，打印 node/dsh 版本 + runtime 体积
```

**关键要求**：
- **增量复用**：node 二进制和 node_modules 已存在时跳过重复下载/安装（缩短 CI 时间）
- 打印关键产物信息：node 版本、dsh 版本、runtime 总体积、native 模块 build 结果
- 下载失败/校验失败时明确报错退出

**通过标准**：
- [ ] 脚本在 Windows 本机成功产出 `resources/runtime/node/node.exe` + `resources/runtime/dsh-runtime/node_modules/@deepseek-ai/dsh/lib/bin.js`
- [ ] native 模块（node-pty/koffi）build scripts 已执行（koffi 有 `.node` 产物）
- [ ] 重复运行脚本能增量跳过

**失败排查**：
- native 模块 build 失败 → 检查该平台是否有 C++ 编译工具链；或改用 `pnpm approve-builds` 交互确认
- 下载慢 → 可配置镜像（如 `NODE_MIRROR` 环境变量）

---

### T3 改造 main.rs 内嵌路径解析

**目的**：让 `resolve_node` / `resolve_dsh_bin` 优先解析安装包内嵌路径。

**改造后的核心接口**：

```rust
// 优先级：DSH_BIN 环境变量 > resource_dir 内嵌 > dev fallback .stage-p0
fn resolve_dsh_bin(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    // 1. DSH_BIN 环境变量（最高优先级，排障/调试用）
    if let Ok(p) = std::env::var("DSH_BIN") { /* ... */ }

    // 2. 内嵌：resource_dir/runtime/dsh-runtime/node_modules/@deepseek-ai/dsh/lib/bin.js
    let resource_dir = app.path().resource_dir()?;
    let embedded = resource_dir.join("runtime/dsh-runtime/node_modules/@deepseek-ai/dsh/lib/bin.js");
    if embedded.exists() { return Ok(embedded); }

    // 3. dev fallback：../../.stage-p0/...
    // ...
}

// prod: resource_dir/runtime/node/node.exe（Windows）/ node（其他）
// dev : 系统 PATH 的 "node"
fn resolve_node(app: &tauri::AppHandle) -> PathBuf { /* ... */ }
```

**关键要求**：
- 启动时**打印实际解析到的 node 路径与 dsh 路径**，便于安装包环境排障
- 失败时给出可操作错误（提示 DSH_BIN 或检查 runtime 是否完整），不静默 panic
- 保留阶段 1 的 `cargo run` 调试路径（dev fallback 仍可用）

**通过标准**：
- [ ] dev 模式（cargo run）仍能跑通（回退到 .stage-p0 + 系统 node）
- [ ] 打包后能解析到内嵌 node + dsh 路径

---

### T4 配置 tauri.conf.json 打包

**目的**：启用打包并正确配置 resources 与 targets。

**关键配置**：

```json
{
  "bundle": {
    "active": true,
    "resources": {
      "resources/runtime/": "runtime/"
    },
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.ico"
    ]
  }
}
```

**图标要求**：
- 用 `tauri icon` 命令（需 `cargo install tauri-cli`）从一张 1024x1024 源图生成全尺寸图标
- 或继续用占位图标（阶段 5 再定制）

**通过标准**：
- [ ] `bundle.active: true` 生效
- [ ] `bundle.resources` 映射正确（打包后 runtime 在安装包内 `runtime/` 相对路径）
- [ ] 图标配置完整

---

### T5 本机 Windows 打包验证

**目的**：产出 Windows 安装包并在干净环境验证"无 Node 也能跑通"。

**步骤**：

```powershell
# 1. 组装运行时
node scripts/build-runtime.mjs

# 2. 打包（产出 NSIS 安装包到 src-tauri/target/release/bundle/nsis/）
$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
& $cargo tauri build   # 需先 cargo install tauri-cli，或用 cargo run -- build
```

**验证**：
1. 安装包安装到干净目录（或虚拟机/沙箱）
2. 确认安装后**不依赖系统 Node**（移除 PATH 中的 node 或用全新环境）
3. 启动应用 → 窗口加载 dsh UI
4. **回归 native 终端功能**：在 UI 中执行一个 shell/终端命令，确认 node-pty/koffi 正常（而非只看 web UI 能开）
5. 关闭无残留进程

**通过标准**：
- [ ] 产出 NSIS 安装包（记录体积）
- [ ] 干净环境安装后无需 Node 跑通
- [ ] 终端/子进程功能正常（native 模块可用）
- [ ] 关闭无残留进程

---

### T6 三端 CI matrix

**目的**：用 GitHub Action 三 runner 构建三端包。

**关键点**：
- `matrix`: `windows-latest` / `macos-latest` / `ubuntu-latest`
- 每端：装 pnpm + 跑 `build-runtime.mjs`（按平台下载对应 node）+ `tauri build`
- Windows 代码签名：测试证书占位（正式证书后补）
- macOS 公证：`APPLE_CERTIFICATE` 等 secrets 占位，先用 `--signing-identity "-"` 跳过
- Linux 依赖：安装 `libwebkit2gtk-4.1-dev` 等 Tauri 系统依赖

**通过标准**：
- [ ] workflow 文件就绪，matrix 三端
- [ ] 本机可验证的部分（Windows）已通过；macOS/Linux 配置完整、可待 CI 运行

---

## 5. 验收标准汇总（DoD）

| # | 验收项 | 判定 |
|---|---|---|
| 1 | build-runtime 脚本一键产出 runtime | 必需 |
| 2 | native 模块 build 已允许且产物存在 | 必需 |
| 3 | main.rs 内嵌路径解析正确（dev 不退化） | 必需 |
| 4 | tauri build 产出 Windows 安装包 | 必需 |
| 5 | 干净环境无 Node 跑通 | 必需 |
| 6 | 终端/子进程功能回归正常 | 必需 |
| 7 | 关闭无残留进程 | 必需 |
| 8 | 三端 CI 配置就绪 | 必需 |
| 9 | 无 dsh 源码改动（git diff 验证） | 必需 |

---

## 6. 风险与应对

| 风险 | 等级 | 应对 |
|---|---|---|
| native 模块（koffi）在 Windows 无 C++ 工具链时 build 失败 | 中 | 本机已有 VS Community 2026；CI 各 runner 预装工具链；必要时用 prebuild 兜底 |
| 安装包 250-300MB 偏大 | 低 | 可接受；README 标注；未来可精简 node_modules |
| resources 打包 446 包目录树耗时 | 中 | 首次打包耗时纳入预期；增量复用脚本缩短 CI |
| 打包后 native 模块路径变化加载失败 | 中 | T5 用真实终端功能回归，而非只看 web UI |
| macOS 公证 / Windows 签名证书缺失 | 中 | 测试证书占位，正式证书后补 |
| 本机无法交叉编译 macOS/Linux | — | 由 CI matrix 解决，本机只验 Windows |

---

## 7. 阶段出口标准（Go / No-Go）

**Go（进入阶段 3 示范插件）**：
- Windows 安装包本机验证通过（无 Node 跑通 + native 回归）
- 三端 CI 配置就绪

**No-Go**：
- native 模块无法在打包后正常工作 → 暂停，优先解决 node-pty/koffi 的打包兼容性
- 内嵌路径解析在打包后失效 → 回到 T3 排查 resource_dir 行为

---

## 8. 交付物清单

| 文件 | 说明 |
|---|---|
| `scripts/build-runtime.mjs` | 运行时组装脚本 |
| `tauri-app/resources/runtime/` | 组装产物（gitignore） |
| `tauri-app/src-tauri/src/main.rs` | 内嵌路径解析改造 |
| `tauri-app/src-tauri/tauri.conf.json` | 打包配置 |
| `tauri-app/src-tauri/icons/` | 多尺寸图标 |
| `.github/workflows/build.yml` | 三端 CI matrix |
| `docs/stage/stage-2-packaging.md` | 本计划 |
| Windows 安装包 | 本机验证产物 |
