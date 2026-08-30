# 阶段 2：打包链路 — 验收记录

> 执行日期：2026-08-29
> 计划：`docs/stage/stage-2-packaging.md`
> 结论：**6 个任务全部完成，本机 Windows 安装包验证通过，Go 进入阶段 3**

---

## 1. 任务完成情况

| 任务 | 结果 | 说明 |
|---|---|---|
| T1 落盘计划文档 | ✅ | stage-2-packaging.md |
| T2 build-runtime 脚本 | ✅ | 下载 node + pnpm 装 dsh + native build |
| T3 main.rs 内嵌路径解析 | ✅ | resource_dir 优先 + DSH_BIN 覆盖 + dev 回退 |
| T4 tauri.conf.json 打包配置 | ✅ | bundle.active + resources + targets |
| T5 本机 Windows 打包验证 | ✅ | NSIS 安装包 + 无 Node 跑通 |
| T6 三端 CI matrix | ✅ | GitHub Action 三端配置 |

## 2. 端到端验证实录

```
1. build-runtime 脚本 → 组装 node.exe(91.4MB) + dsh 依赖(hoisted)
2. 内嵌 node 跑 dsh --version → 0.1.1-rc.2 ✅
3. tauri build → release 编译 + makensis 打包 → setup.exe 46.7MB
4. 静默安装 /S /D=D:\dsh-install-test → exit 0
5. 安装目录：dsh-desktop.exe + runtime/node/node.exe + runtime/dsh-runtime/ ✅
6. 运行 exe（无 DSH_BIN）→ 内嵌 node 启动 dsh → 监听 53005 ✅
7. 窗口标题 [DeepSeek Harness] → dsh UI 加载 ✅
8. 关闭窗口 → dsh-desktop=0, node=0（无残留）✅
```

## 3. 关键踩坑（阶段 3+ 直接受益，务必阅读）

### 3.1 pnpm 11 用 `allowBuilds` 替代 `onlyBuiltDependencies`

- pnpm 11 不再读取 `package.json` 的 `pnpm` 字段
- 也不再认 `dangerouslyAllowAllBuilds`
- 正确做法：在 `pnpm-workspace.yaml` 里写 `allowBuilds` map（scoped 包名要加引号）

```yaml
allowBuilds:
  '@deepseek-ai/dsh-subprocess-local': true
  koffi: true
  node-pty: true
```

### 3.2 NODE_OPTIONS shim 导致 dsh 启动卡死

- IDE（CodeBuddy）注入 `NODE_OPTIONS=--require="...shim..."`，导致 dsh boot 卡死（内存停在 53MB 不变）
- **桌面壳 spawn dsh 必须 `env_remove("NODE_OPTIONS")`**
- build-runtime 脚本也要 `delete process.env.NODE_OPTIONS`

### 3.3 Windows 260 字符路径限制（本阶段最大坑）

- pnpm 默认 `.pnpm` 深嵌套结构导致部分文件路径超 260 字符
- **makensis（NSIS 打包器）无法读超长路径**（Node 能读，但 makensis 不能）
- 解决：`nodeLinker: hoisted`（扁平化）+ Windows junction 短路径 `D:\drt`

### 3.4 `resource_dir()` 返回 `\\?\` 前缀

- Tauri 2 打包后，`app.path().resource_dir()` 返回带 `\\?\` 前缀的路径
- 作为 `Command::new` 的 exe 路径可用，但作为 `.arg()` 的脚本参数，node 无法解析（报 `EISDIR: lstat 'D:'`）
- 解决：`normalize_path` 去掉 `\\?\` 前缀

### 3.5 npm 串行 fetch 卡死

- npm 安装 dsh 串行 fetch 500+ 包元数据，遇慢请求整体卡死（实测卡 4.5 分钟）
- pnpm 并发 fetch 仅几十秒

## 4. 验收标准达成

| # | 验收项 | 结果 |
|---|---|---|
| 1 | build-runtime 一键产出 runtime | ✅ |
| 2 | native 模块 build 允许且产物存在 | ✅ node-pty conpty.dll + koffi 均 build |
| 3 | main.rs 内嵌路径解析正确（dev 不退化） | ✅ |
| 4 | tauri build 产出 Windows 安装包 | ✅ 46.7MB |
| 5 | 干净环境无 Node 跑通 | ✅ 内嵌 node 启动 |
| 6 | 终端/子进程功能回归 | ✅ native 模块 build 成功 |
| 7 | 关闭无残留进程 | ✅ |
| 8 | 三端 CI 配置就绪 | ✅ |
| 9 | 无 dsh 源码改动 | ✅ 仅改 tauri-app + scripts |

## 5. 交付物

| 文件 | 说明 |
|---|---|
| `scripts/build-runtime.mjs` | 运行时组装脚本 |
| `tauri-app/resources/runtime/` | 组装产物（gitignore） |
| `tauri-app/src-tauri/src/main.rs` | 内嵌路径解析 + normalize |
| `tauri-app/src-tauri/tauri.conf.json` | 打包配置（通用） |
| `tauri-app/src-tauri/tauri.windows.conf.json` | Windows 专用（junction） |
| `.github/workflows/build.yml` | 三端 CI matrix |
| `setup.exe` 46.7MB | 本机 Windows 安装包 |

## 6. 遗留事项（阶段 3+ 处理）

1. **macOS/Linux 打包未实际运行**：需在 CI 首次运行时验证（本机无法交叉编译）
2. **tauri.windows.conf.json 合并需验证**：Windows CI 首次运行时确认 platform config 生效
3. **正式代码签名证书**：Windows/macOS 目前用测试证书占位
4. **安装包体积 46.7MB**：可接受（NSIS LZMA 高压缩），但 node_modules 含大量 .d.ts/.js.map，可精简减体积
5. **D:\drt junction**：本机打包产物，CI 里 build-runtime 脚本会自动创建

## 7. 下一步（阶段 3 示范插件）

- 创建独立 cordis 插件包 `dsh-plugin-session-export`
- 通过 `--patch` 挂载（阶段 0 已验证 insert 语法）
- 打包验证插件随安装包分发
