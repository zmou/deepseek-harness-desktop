# Contributing to DeepSeek Harness Desktop

感谢你考虑为这个项目做贡献！

## 行为准则

- 尊重他人，善意沟通
- 讨论聚焦技术问题本身

## 如何贡献

### 报告问题

在 [Issues](../../issues) 中报告 Bug 时，请附上：

1. 操作系统与版本、dsh-desktop 版本
2. 复现步骤
3. 期望行为 vs 实际行为
4. 相关日志（Windows：`%LOCALAPPDATA%\dsh-desktop\dsh-desktop.log`；macOS/Linux：`~/.dsh/dsh-desktop.log`）。日志中的一次性 token 已被脱敏，可直接粘贴

### 提交代码

1. Fork 仓库并创建分支
2. 遵循现有代码风格：Rust 用 `cargo fmt`，脚本保持与现有文件一致
3. 说明改动的动机与影响
4. 涉及构建链路的改动请本地验证：
   - `node --check scripts/build-runtime.mjs`（语法）
   - `cargo check --manifest-path tauri-app/src-tauri/Cargo.toml`（编译）
   - 如有条件，跑通 `node scripts/build-runtime.mjs`
5. 提交 PR

### 修改上游源码的边界（红线）

`deepseek-harness/` 是官方上游（本地仅作参考，已 gitignore），**不要修改其源码，也不要提交它的内容**。
自定义功能请通过 dsh 的 cordis 插件机制实现，详见
[docs/custom-web-extension-and-upgrade-architecture.md](docs/custom-web-extension-and-upgrade-architecture.md)。

### 文档

- `README.md` 为中文主版，`README.en.md` 为英文版；修改功能时请同步两份
- 各阶段计划与验收记录见 `docs/stage/`

## 本地开发

- 桌面壳：`tauri-app/`（Rust + Tauri 2），入口 `tauri-app/src-tauri/src/main.rs`
- 运行时组装：`scripts/build-runtime.mjs`
- 一键打包：`scripts/build-win.ps1`（Windows）/ `scripts/build-mac.sh`（macOS）
- CI：`.github/workflows/build.yml`（三端矩阵，产物上传到 Actions artifacts）

## 新贡献者常见问题

**Q: 构建产物（runtime/、target/）需要提交吗？**
不需要，均已 gitignore。

**Q: 升级 dsh 版本要改哪些文件？**
`scripts/build-runtime.mjs` 的 `DSH_VERSION` + `tauri.conf.json` / `Cargo.toml` 的 version
（`build-win.ps1 -DshVersion` 可一键完成）。

**Q: 我可以基于此项目做二次发行吗？**
可以（MIT），但请遵守 [NOTICE](NOTICE) 中的商标声明：不要声称是 DeepSeek 官方出品。