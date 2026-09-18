# 阶段 3 验收记录：macOS 12 兼容版（Electron）

- 规格：[`docs/electron-macos-compat-spec.md`](../electron-macos-compat-spec.md)
- 实施计划：[`docs/electron-macos-compat-implementation.md`](../electron-macos-compat-implementation.md)
- 验收日期：2026-09-17
- 验收机器：macOS 12.7.6（Monterey，x86_64，BuildVersion 21H1320）——即规格指定的目标机
- 内置运行时：`@deepseek-ai/dsh@0.1.5-rc.1` + node v24.14.0（213.8 MB）
- Electron：43.7.1（Chromium 150 / Node 24.21.0）

## 1. 结论

**通过**（仅「真人在另存为对话框点击保存」「实际发送对话消息」两项留作发布前手动确认）。

Tauri 版在 macOS 12 上的白屏问题已由独立的 Electron 兼容版解决：官方 UI 在 macOS 12.7.6 上完整
渲染，桌面层能力（单实例 / 下载接管 / 日志脱敏 / 弹窗隐藏 / 进程守护）与默认版对齐，打包链路
产出的 dmg 可安装可运行。

## 2. 交付物

| 产物 | 路径 |
|---|---|
| 桌面壳源码 | `electron-app/src/{main.js,runtime.js,redact.js,hide-session-dialog.js}` |
| 打包配置 | `electron-app/electron-builder.yml`、`electron-app/package.json`（electron 精确锁 43.7.1） |
| 一键打包脚本 | `scripts/build-mac-electron.sh` |
| CI 任务 | `.github/workflows/build.yml` → `build-macos-electron`（含门槛断言） |
| 安装包 | `electron-app/release/DeepSeek-Harness-Desktop_0.1.5-rc.1_x64-electron.dmg`（187 MB） |
| 文档 | `README.md` / `README.en.md` 选版指引与构建章节、`CHANGELOG.md` |

## 3. 验收矩阵

### 3.1 功能验收（规格 §12.1）

| # | 验收项 | 结果 | 证据 |
|---|---|---|---|
| 1 | macOS 12.7.6 实机无白屏、官方 UI 完整渲染 | ✅ | 打包产物启动后 CDP 探针：`title=DeepSeek Harness`、`#root.children=1`、界面文案（新会话 / 工作区 / 设置 / 探索未至之境）完整 |
| 2 | 会话/消息链路、WebSocket 稳定 | ✅（间接） | CDP `Network` 域实测：`ws://127.0.0.1:<port>/api/remote.mux` 建立，12s 内收发各 4 帧；页面 reload 后仍正常渲染。未实际发送对话消息（会消耗模型额度），留作手动项 |
| 3 | 会话导出走「另存为」+ 记住上次目录 + 写入 zip | ✅（对话框可见性待人工确认） | 触发 `/api/session.export` 下载：Chromium 自身下载被 `preventDefault()` 取消（`~/Downloads` 无静默落盘）；`dialog.showSaveDialog` 的 promise 持续挂起等待用户操作（不自动 resolve、无异常）；写入路径实测 `saved 507 bytes`，产物为合法 zip（含 `session.v3.jsonl`）；`last-download-dir.txt` 正确记录目录 |
| 4 | 日志 token 全部 `***` | ✅ | `~/.dsh/dsh-desktop.log` 中 `token=` 出现 8 次，全部为 `token=***`，未脱敏行数 0 |
| 5 | 官方 Session 导出乐观弹窗被隐藏、遮罩不残留 | ✅ | 合成同构 DOM 实测：`[role=dialog][aria-modal=true][aria-label*="Session"]` 计算样式 `display:none`，其遮罩 wrapper（`:has()` 规则）同样 `display:none`；实机 `sessionDialogs: 0` |
| 6 | 重复启动 → 恢复已有窗口、无第二实例 | ✅ | 第二实例立即以 exit 0 退出；主进程数仍为 1、dsh 子进程仍为 1、`ready url` 仅出现 1 次 |
| 7 | 关窗无残留 node；kill node → 窗口自动关闭 | ✅ | 关闭窗口后 Electron 与内嵌 node 进程数均为 0；`kill <node pid>` 后日志 `[dsh] child exited: code=0 signal=null`，Electron 主进程自动退出，无残留 |
| 8 | 数据与官方 CLI / Tauri 版互通（同一 `~/.dsh`） | ✅ | 工作区与会话列表与 CLI/Tauri 版一致、导出的是同一份会话日志；日志同写 `~/.dsh/dsh-desktop.log` |

### 3.2 自动化检查（规格 §12.2）

| 检查 | 结果 |
|---|---|
| `node --check electron-app/src/*.js` | ✅ 4 个文件全部通过 |
| 打包脚本门槛断言 `LSMinimumSystemVersion <= 12.0` | ✅ 产物实测 `12.0` |
| CI 门槛断言（正反用例） | ✅ 真实产物通过；把 plist 改成 `13.0` 后按预期失败退出 |
| 打包脚本端到端 | ✅ `bash scripts/build-mac-electron.sh` 7 步全绿，产 dmg 187 MB |
| dmg 内容与 plist | ✅ 含 `.app` + `Applications -> /Applications`；`CFBundleIdentifier=com.deepseek.dsh-desktop-electron`（与 Tauri 版 `com.deepseek.dsh-desktop` 不冲突）、`CFBundleShortVersionString=0.1.5-rc.1-electron`、`LSMinimumSystemVersion=12.0` |
| 产物运行时路径解析 | ✅ 使用 `Contents/Resources/runtime/node/node` + `.../dsh-runtime/.../bin.js`（内嵌运行时，非系统 node） |

### 3.3 老机器实测清单（规格 §12.3）

| 机器 | 结果 |
|---|---|
| macOS 12.7.6（目标机） | ✅ 已完成实机验证 |
| macOS 13+ 一台 | ⏳ 未做（本机即目标机，无第二台设备）。CI 在 `macos-latest` 上构建，建议发布前在任意 macOS 13+ 机器补一次冒烟 |

## 4. 与规格/计划的偏差（实测发现，已在代码中修正）

| # | 计划/规格写法 | 实测结果 | 最终实现 |
|---|---|---|---|
| 1 | §11.3 断言 `Electron Framework.framework/Resources/Info.plist` 的 `LSMinimumSystemVersion` | Electron 43.7.1 的该键**不在 Framework 内**，而在 `Contents/Info.plist` | 断言同时检查两个 plist 并取更严格者；都找不到即 fail（避免静默通过） |
| 2 | §7.6 用 `item.setSavePath()`（并假设默认会弹「另存为」） | Electron **默认静默写入「下载」目录**，不弹任何对话框；且 `setSavePath()` 要求在 `will-download` 回调内**同步**调用，无法等异步对话框返回 | `event.preventDefault()` 取消原生下载 → `dialog.showSaveDialog()` 弹「另存为」→ 确认后用 `net.request` 写盘（与 Tauri 版「取消 + 壳侧自取」结构一致） |
| 3 | §7.5「Electron 下载 cookie **自动携带**」 | `net.request` 的 `useSessionCookies` **默认为 false**，实测返回 **401**（cookie jar 里确实有 `dsh-auth-*`） | 显式 `net.request({ session: defaultSession, useSessionCookies: true })`；仍无需 Tauri 版的手动 token 兑换 |
| 4 | T8「子进程**非 0** 退出 → 关闭窗口」 | dsh 会捕获 SIGTERM 并以 `code=0` 优雅退出。按「非 0 才关窗」实现，`kill <node pid>` 后会留下一个后端已死的空窗口，与 §12.1.7 冲突 | 非主动退出路径下，子进程**任何**退出都关窗（后端没了 UI 必然不可用） |
| 5 | — | 宿主/IDE 注入的 `NODE_OPTIONS` shim 会干扰 Electron 等子进程 | 沿用项目既有约定：spawn dsh 时 `delete env.NODE_OPTIONS` |

## 5. 已知限制

- 「另存为」对话框的**可见性**与「点击保存后落盘」需要真人在 GUI 上确认一次（自动化环境无辅助功能
  权限，无法操作原生对话框）；代码路径、默认路径、取消分支与写入函数均已单独验证。
- Electron 43 是官方支持 macOS 12 的最后一档（规格 §13 R1/R4），Chromium 安全更新会逐步停止，
  兼容版定位为过渡方案。
- 产物为 ad-hoc 签名（与 Tauri 版一致），首次打开可能被 Gatekeeper 拦截，需右键 → 打开。

## 6. 复现命令

```bash
# 打包（含 runtime 构建、版本号同步、门槛断言）
bash scripts/build-mac-electron.sh

# 开发模式（系统 node + 仓库内已构建 runtime）
cd electron-app && npm start

# 仅语法检查
for f in electron-app/src/*.js; do node --check "$f"; done
```
