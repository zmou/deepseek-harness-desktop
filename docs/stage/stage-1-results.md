# 阶段 1：桌面壳骨架 — 验收记录

> 执行日期：2026-08-29
> 计划：`docs/stage/stage-1-desktop-shell.md`
> 结论：**5 个任务全部完成，7 项验收全部通过，Go 进入阶段 2**

---

## 1. 任务完成情况

| 任务 | 结果 | 说明 |
|---|---|---|
| T1 安装 Rust 工具链 | ✅ | cargo/rustc 1.98.0，msvc 工具链，VS Community 2026 提供链接器 |
| T2 Tauri 2 工程骨架 | ✅ | 编译通过，exe 11.6MB，窗口正常弹出 |
| T3 启动链路 | ✅ | spawn node + 解析 URL + 30s 超时 |
| T4 生命周期管理 | ✅ | 关窗杀进程 + 异常退出关窗 |
| T5 本机跑通验收 | ✅ | 完整闭环验证通过 |

## 2. 验收标准达成情况

| # | 验收项 | 结果 |
|---|---|---|
| 1 | `cargo run` 能启动完整闭环 | ✅ exe 启动 → spawn node → dsh web 监听 → 窗口加载 |
| 2 | WebView 显示 dsh 官方 UI | ✅ 窗口标题 "DeepSeek Harness"，dsh web 就绪监听随机端口 |
| 3 | 关闭窗口无残留 node 进程 | ✅ CloseMainWindow 后 node count=0 |
| 4 | 子进程异常退出窗口自动关闭 | ✅ kill node 后 exe 自动退出 |
| 5 | dsh 路径可通过 DSH_BIN 切换 | ✅ 环境变量生效 |
| 6 | 启动超时 30s 有兜底 | ✅ wait_for_url 30s 超时逻辑 |
| 7 | 无 dsh 源码改动 | ✅ 仅新增 tauri-app/ 目录 |

## 3. 端到端验证实录

```
1. 启动 exe → exe alive=True
2. spawn node 子进程 → node pid=38888 WS=56MB
3. dsh 启动中 → node WS 涨到 100MB → 124.7MB
4. dsh 就绪 → node 监听端口 65424（--port 0 随机端口生效）
5. 窗口创建 → title=[DeepSeek Harness]
6. 关闭窗口 → CloseMainWindow=True → exe count=0, node count=0（无残留）
7. 重新启动 → 窗口就绪
8. kill node → 3s 后 exe count=0（异常退出自动关窗）
```

## 4. 踩坑记录（阶段 2+ 直接受益）

1. **tauri.conf.json 的 `devUrl: ""` 空字符串** → 报 `relative URL without a base`，需移除该字段
2. **Windows 下 tauri-build 始终需要 `icons/icon.ico`**（即使 bundle.active=false），需准备占位图标（纯 Python 生成，无 Pillow 依赖）
3. **窗口 URL 用 `WebviewWindowBuilder` 动态创建**（`WebviewUrl::External`）而非 config 静态定义 + navigate，避免 about:blank→navigate 的坑
4. **Child 句柄用 `Arc<Mutex<Option<Child>>>`**，退出钩子 take+kill，监听线程 try_wait 轮询（500ms），避免所有权竞态
5. **首次 cargo build 极慢**（300+ crate，约 8 分钟），后续增量编译仅 46s

## 5. 交付物

| 文件 | 说明 |
|---|---|
| `tauri-app/src-tauri/` | 完整 Tauri 工程 |
| `tauri-app/frontend/index.html` | 占位页 |
| `tauri-app/README.md` | 运行说明 |
| `tauri-app/.gitignore` | 忽略 target |

## 6. 下一步（阶段 2 打包链路）

- dsh 运行时内嵌（替换系统 node + DSH_BIN）
- 三端安装包 + 签名
- 图标定制
