# v0.1.7-rc.2 桌面端系统托盘功能实施与验收记录

> 状态：已实施（Windows Tauri 主壳已通过开发运行验收；Electron 兼容壳已通过 Windows 开发运行验收，macOS 目标平台验收待 macOS 环境执行）
> 对应规格：`docs/versions/v0.1.7-rc.2/desktop-tray-feature-plan.md`
> 实施日期：2026-09-28

## 1. 变更文件

| 文件 | 变更内容 |
|---|---|
| `tauri-app/src-tauri/src/main.rs` | 新增线程安全退出状态（`AtomicBool`）、托盘初始化（`TrayIconBuilder` + 两菜单项 + 双击）、主窗口 close 拦截、统一恢复函数、child 异常退出协调、退出清理 |
| `tauri-app/src-tauri/Cargo.toml` | 为 `tauri` 依赖启用 `tray-icon` feature（依赖版本不变，Cargo.lock 未变化；tray-icon 0.24.2 / muda 0.19.3 原已在依赖树中） |
| `electron-app/src/main.js` | 新增 `Tray` 实例（两菜单项 + 双击）、`close` 拦截、`before-quit` / `window-all-closed` / `will-quit` 调整、统一恢复函数、child 异常退出置位 |
| `electron-app/electron-builder.yml` | `files` 增加 `icons/icon.png`（Tray 需要运行时图标文件；buildResources 只用于构建期，不会进 asar） |

明确未修改：`tauri-app/resources/runtime/`、dsh Web UI / 官方源代码、`~/.dsh` 用户数据、`tauri.conf.json` 窗口尺寸 / 资源路径 / 图标集合 / 运行时资源配置、capability / 权限文件、NSIS 安装逻辑、无关版本号与发布脚本。

## 2. 实现要点

### 2.1 Tauri 主壳（main.rs）

- **生命周期状态**：`AppState` 增加 `quitting: Arc<AtomicBool>`（`SeqCst`），由托盘菜单「退出」、child 异常退出、`RunEvent::Exit` 兜底置位；窗口事件、托盘回调、child 监听线程、RunEvent 可在不同上下文访问，全部通过原子读判断，不使用 setup 局部布尔。
- **托盘创建**（`init_tray`）：
  - 固定 ID `main-tray`，`TrayIconBuilder::with_id`；
  - 图标复用 `app.default_window_icon()`（Windows 取 bundle.icon 的 icon.ico，其他平台取 icon.png），不新增图标资源；
  - tooltip = `DeepSeek Harness Desktop`；
  - 菜单仅两项：`打开主界面`（`open-main`）、`退出`（`quit`），顺序固定、无分隔线；
  - `.show_menu_on_left_click(false)`：关闭 Windows 默认「左键单击弹菜单」，避免吞掉双击事件（右键弹菜单不受影响）；
  - `on_tray_icon_event` 处理 `DoubleClick{button: Left}` → `show_main_window`；
  - `on_menu_event` 按 MenuId 分发 → `show_main_window` / `request_quit`。
  - 托盘初始化失败 → 记录日志并 `std::process::exit(1)`（按启动失败处理，不静默创建「关闭后不可恢复」的隐藏窗口，规格 §8.1）。
- **窗口恢复**（`show_main_window`）：`get_webview_window("main")` → `unminimize` → `show` → `set_focus`；窗口不存在时记录调试日志并返回，不创建第二窗口 / 不重启 dsh。第二实例回调、托盘双击、菜单「打开主界面」三条路径复用同一函数。
- **close 拦截**：`on_window_event(WindowEvent::CloseRequested)`，未置位退出状态时 `api.prevent_close()` + `unminimize` + `hide()`；已置位时不拦截，正常关闭。
- **child 异常退出**：监听线程检测到 child 退出（无论退出码）→ 先置位 `quitting` → 关闭所有窗口 → 最后一个窗口销毁触发 `ExitRequested` → `Exit`，回收 child 并退出。不会留下只有托盘、没有后端的死 UI。
- **退出**（`request_quit`）：幂等（已置位直接返回），置位后 `app.exit(0)` → `RunEvent::ExitRequested`（窗口真正关闭）→ `RunEvent::Exit`（`state.set_quitting()` 兜底 + drop 托盘引用 + kill/wait child）。
- **单实例**：第二实例回调改为调用 `show_main_window`，与托盘恢复逻辑统一（规格 §8.4）。

### 2.2 Electron 兼容壳（main.js）

- **Tray 创建**（`createTray`）：模块级 `tray` 变量长期持有；`electron-app/icons/icon.png`；tooltip `DeepSeek Harness Desktop`；`Menu.buildFromTemplate` 仅两项 `打开主界面` / `退出`；`tray.on('double-click')` → `showMainWindow`；初始化幂等（`if (tray) return`）。
- **托盘初始化失败**：记录日志、置位 quitting、killChild、`app.quit()`（按启动失败处理，规格 §8.1）。
- **窗口恢复**（`showMainWindow`）：`isDestroyed` 安全返回 → `isMinimized` 先 `restore()` → `show()` → `focus()`。second-instance、托盘双击、菜单、macOS `activate` 复用。
- **close 拦截**：`win.on('close')` 未置位 quitting 时 `preventDefault()` + `hide()`。
- **生命周期**：`before-quit` 置位 quitting + killChild（在窗口 close 之前，避免主动关闭被再次隐藏）；`window-all-closed` 置位 + killChild + `app.quit()`（只可能在显式退出或 child 异常退出路径触发）；`will-quit` 销毁托盘图标；SIGINT/SIGTERM 保留原回收逻辑。
- **child 异常退出**（`onChildExit`）：先置位 `quitting` 再关窗，否则 close 拦截会把窗口再次隐藏（规格 §7.4 / §8.3）。
- **退出**（`requestQuit`）：幂等，置位后 `app.quit()`。

## 3. 验证过程与环境

测试环境：Windows 11（x64），真实桌面会话，任务栏 1920×1080。

- Tauri 主壳：`cargo check` / `cargo check --release` 均通过；`cargo fmt` 已执行。用 `TAURI_CONFIG` 覆盖 identifier 为 `com.deepseek.dsh-desktop-traytest` 构建 debug 二进制（避免与已安装的 DeepSeek Harness Desktop 单实例互斥），并复用仓库内真实 runtime 启动。
- Electron 壳：`node --check electron-app/src/main.js` 通过；`npm install`（按 package-lock 锁定版本）后在本机 Windows 用 Electron 43 直接运行 `main.js` 冒烟验证（macOS 目标包构建需在 macOS 环境执行）。

## 4. 验收矩阵结果

### 4.1 Tauri Windows（开发运行验收）

| # | 验收项 | 结果 | 说明 |
|---|---|---|---|
| 10.1 | 基础启动：桌面进程 + dsh child + 主窗口 + 托盘图标 | ✅ | 进程树确认：dsh-desktop + node(dsh) + WebView2 + `tray_icon_app` 窗口 |
| 10.2 | 关闭到托盘：主窗口隐藏、进程/child/托盘存活 | ✅ | 对主窗口发送 `WM_CLOSE`：窗口 `IsWindowVisible=False`，进程与 dsh child 仍运行；日志 `[tray] close requested; hiding to tray` |
| 10.3 | 托盘恢复（隐藏态）：第二实例启动恢复原窗口 | ✅ | 隐藏后启动第二实例：第二实例退出（单实例），原窗口 `visible=True`（经统一 `show_main_window`） |
| 10.4 | 菜单精确性 | ✅（代码层） | 菜单仅两项、顺序固定、文本精确匹配（`MenuItem::with_id` × 2，无 separator）；Windows 托盘菜单为 muda popup（HMENU 不直接可枚举），由代码定义保证 |
| 10.5 | 真正退出 | ✅（代码路径） | `request_quit` 幂等 → `app.exit(0)` → `RunEvent::Exit` 回收 child、drop 托盘 |
| 10.6 | 异常退出（kill dsh child） | ✅ | 强杀 dsh node child：日志 `[dsh] child exited abnormally: code=Some(-1)`，应用进程退出，无残留 node/托盘死 UI |
| 10.7 | 单实例回归 | ✅ | 显示态 / 隐藏态第二实例均不增加进程、child、托盘；隐藏态能恢复原窗口 |
| 10.8 | 静态与打包验证 | 部分完成 | Rust fmt/check 通过；Windows 生产安装包构建与托盘交互验证需在最终打包机执行 |

### 4.2 Electron（Windows 开发运行验收；macOS 待环境）

| # | 验收项 | 结果 | 说明 |
|---|---|---|---|
| 10.1 | 基础启动 | ✅ | Electron 主进程 + dsh node child + 窗口 + `Electron_NotifyIconHostWindow` |
| 10.2 | 关闭到托盘 | ✅ | `WM_CLOSE` → 窗口隐藏，主进程与 dsh child 存活 |
| 10.3 | 托盘恢复（隐藏态） | ✅ | 隐藏后第二实例启动：第二实例退出，原窗口恢复 `visible=True` |
| 10.6 | 异常退出（kill dsh child） | ✅ | Electron 主进程退出，无残留 |
| 10.8 | 静态检查 | ✅ | `node --check` 通过；macOS 包构建与托盘交互待 macOS 环境 |

## 5. 平台差异记录（规格 §7.5 / §11）

1. **macOS 托盘双击**：Electron 与 Tauri（tray-icon）在 macOS 上均不提供 Tray `double-click` 事件（仅 Windows）。恢复入口为右键菜单「打开主界面」+ macOS Dock `activate`（Electron 侧已实现；Tauri 侧由系统 Dock 点击触发 Reopen 语义）。已按规格要求保留右键「打开主界面」作为明确恢复入口。
2. **macOS 托盘图标模板**：本次使用 `electron-app/icons/icon.png`（512×512 彩色品牌图标）。macOS 菜单栏推荐 template 图标（黑 + alpha），但规格要求「不得改变用户看到的品牌」，故保留原图；若后续需适配 macOS 深浅色菜单栏，可在既有图标资源基础上生成 template 变体，本次不扩展。
3. **Windows 托盘图标尺寸**：Tauri 复用 `icons/icon.ico`（含多尺寸），Electron 复用 `icon.png`（512×512，系统会缩放）；均不新增品牌资源。
4. **托盘图标在任务栏溢出区**：新添加的托盘图标可能落在系统托盘溢出区（`^` 折叠区），`Shell_NotifyIconGetRect` 在图标不可见时返回 E_FAIL，属 Windows 正常行为；图标本身已正确注册（`tray_icon_app` / `Electron_NotifyIconHostWindow` 窗口存在），用户展开溢出区后可见并可交互。

## 6. 与规格 §6.5（依赖与权限边界）的核对

- 托盘完全由 Rust 主进程创建（`TrayIconBuilder`），不经过任何前端 JS command，因此**无需**修改 capability / ACL 权限文件；仓库内也不存在 `src-tauri/capabilities/` 目录。
- `tauri` 依赖仅启用既有 `tray-icon` feature，未升级任何 crate；`Cargo.lock` 无变化。
- `tauri.conf.json` 未修改。

## 7. 遗留与后续

- [ ] Windows Tauri **生产 NSIS 安装包**构建与安装后托盘交互验收（本机未执行完整打包，避免与正在运行的已安装版本冲突；需在打包机执行 `scripts/build-win.ps1`）。
- [ ] Electron **macOS 包**（`scripts/build-mac-electron.sh`）构建与目标 macOS 环境托盘验收。
- [ ] macOS 双壳（Tauri macOS / Electron）行为一致性对照（规格 §13 第 5 步）。
- [ ] 现有下载另存为、WebSocket、会话功能回归（本次未触碰相关代码，但完成定义要求打包后回归）。
