# Changelog

本文件记录用户可见的变更。/ Notable user-facing changes.

格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)（自 0.1.2-rc.1 起）。
版本号 = 内置的 `@deepseek-ai/dsh` 版本号。

## [Unreleased]

- 开源发布准备：LICENSE、NOTICE、CONTRIBUTING、CHANGELOG
- README 重构：中文主版（`README.md`）+ 独立英文版（`README.en.md`），新增徽章、下载与平台支持表、安全声明、桌面壳增值能力章节
- 构建可移植性：移除 Windows 硬编码 `D:\rt` 运行时路径，默认使用仓库内 `tauri-app/resources/runtime/`
- 修复 CI：tauri CLI 工作目录、Linux 打包目标、产物上传路径
- 新增 `tauri.linux.conf.json`（Linux 打包目标 deb + AppImage）
- 修复 `cargo check` 依赖占位目录的问题（`resources/runtime/.gitkeep`）
- 命名规范化：`productName` → `DeepSeek Harness Desktop`（全名，用于安装目录/开始菜单/卸载列表/任务管理器），`mainBinaryName` 保持 `dsh-desktop`（exe 文件名），安装包文件名统一为 `DeepSeek-Harness-Desktop-Setup-v<version>-<arch>.exe`

## [0.1.2-rc.1] - 2026-09-07

- 内置运行时升级至 `@deepseek-ai/dsh@0.1.2-rc.1`
- 单实例：重复启动时恢复并聚焦已有窗口
- 下载接管：WebView2 下载改为「另存为」对话框 + Rust 侧下载，记住上次目录
- 隐藏 dsh 前端的 Session 导出弹窗（与自定义「另存为」流程去重）
- 适配 dsh 0.1.2+ 的会话 cookie 鉴权：启动时用 token 预换 cookie 供 Rust 侧下载使用
- 日志脱敏：所有日志中的 token 参数统一替换为 `***`
- 修复：stdout/stderr 管道泵送（防止 64KB 缓冲塞满导致 dsh 假死）

## [0.1.2-alpha.3] - 2026-09-05

- 内置运行时升级至 `@deepseek-ai/dsh@0.1.2-alpha.3`
- 打包链路全面落地：Node 内嵌、npm 扁平布局、家族包版本钉死、短路径规避 NSIS 260 字符限制
- 安装包/快捷方式图标修复（自定义 NSIS 模板补丁）
- 任务栏图标可见性优化（浅蓝底黑鲸）

## [0.1.2-alpha.2] - 2026-08-31

- 首个可分发版本：Tauri 2 桌面壳 + 内嵌 Node 运行时
- 启动协议：spawn `dsh web --port 0 --no-open`，stdout 解析 ready URL
- 生命周期管理：关窗杀进程、异常退出关窗
- 三端 CI matrix 就绪

> 更早的内部阶段记录（阶段 0-2）见 [docs/stage/](docs/stage/)。
