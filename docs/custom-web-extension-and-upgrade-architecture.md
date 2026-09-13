# DeepSeek Harness Web 定制与官方升级隔离规范

> 文档性质：架构分析与长期维护规范
>
> 目标：在不修改官方 `deepseek-harness` 源码的前提下定制 Web 界面，并让官方版本可以持续升级。
>
> 本文只定义方案、边界和验收规则，不代表已经完成这些工程改造。

## 1. 结论

推荐采用“官方上游只读 + 独立扩展插件 + 独立 Bundle/Profile Patch + Tauri 系统壳”的四层隔离架构：

```text
官方 deepseek-harness（只读上游）
              |
              v
独立 Host Plugin / Client Plugin
              |
              v
独立 Bundle + cordis.patch.yml
              |
              v
Tauri 桌面壳（只负责系统能力）
```

这套方案能够保证：

| 目标 | 结论 |
|---|---|
| 官方源码不被修改 | 可以做到 |
| 官方升级时不产生源码合并冲突 | 可以做到 |
| 自定义代码不会因上游更新丢失 | 可以做到 |
| 官方每次升级后完全无需适配 | 不能保证 |
| 升级影响可以自动发现并阻断发布 | 可以做到 |

“100% 升级”必须准确表述为：**自定义代码和官方源码完全分离，升级不会丢失自定义实现；如果官方公开契约发生破坏性变化，兼容性门禁会发现问题，并要求修改自定义适配层。**

DeepSeek Harness 当前仍处于 alpha 阶段，官方插件、Slot、Remote、事件类型和启动协议都可能发生破坏性变化。因此不能承诺“永远零适配”，但可以把适配范围控制在独立扩展层内。

## 2. 当前项目事实

DeepSeek Harness 的 Web 运行形态不是一个需要被 Fork 的单体前端，而是多个 Cordis Client Plugin 组合形成的浏览器应用：

```text
Tauri
  └── Node 子进程
        └── dsh web
              ├── Cordis Host 插件树
              ├── 本地 HTTP API
              ├── WebBootGraph
              └── React Web Client
                    ├── ui-layout
                    ├── ui-sidebar
                    ├── ui-conversation
                    ├── ui-chat
                    ├── ui-settings
                    └── 其他 Client Plugin
```

官方 Web Client 的关键扩展机制如下：

1. `ctx.slots.register()` 和 `ctx.slots.inject()`：向已经声明的 UI 位置增加或替换内容。
2. `ConversationNodeDefinition`：把 Session 事件转换成 Chat、Trajectory 或自定义会话节点。
3. `dsh.client`：声明浏览器端插件，并通过 `./client` 导出浏览器构建产物。
4. Client Module Graph：由 Host 生成 WebBootGraph，浏览器按图加载动态插件。
5. Host Plugin + Remote/API Controller：为浏览器提供自定义业务能力和持久化能力。
6. `cordis.patch.yml`：在官方 Bundle 之后追加自定义插件和配置。

官方相关规范：

- [Web Client 架构](../deepseek-harness/docs/subsystems/web-client.md)
- [Slots 规范](../deepseek-harness/docs/subsystems/slots.md)
- [Conversation 规范](../deepseek-harness/docs/subsystems/conversation.md)
- [Client Plugin 规则](../deepseek-harness/packages/client/AGENTS.md)
- [Client Module Graph](../deepseek-harness/packages/client/modules/README.md)
- [新增 Package 指南](../deepseek-harness/docs/cookbook/adding-a-package.md)

## 3. 四层隔离架构

### 3.1 Layer 1：官方上游层

建议将官方仓库正式纳入外层仓库的 Git submodule：

```text
deepseek-harness/
```

这一层只承担以下职责：

- 保存官方源码；
- 固定官方 commit 或 tag；
- 同步上游；
- 构建官方 runtime；
- 作为自定义插件的类型和构建输入。

这一层禁止：

- 修改官方源码；
- 添加自定义组件；
- 添加自定义业务逻辑；
- 修改官方 UI 包；
- 在官方文件中直接写桌面适配代码。

当前工作区中的 `deepseek-harness/` 是一个嵌套 Git 仓库，但外层仓库仍将其显示为未跟踪目录，而不是正式 submodule。长期维护前必须确定一种可复现的上游锁定方式，推荐使用 submodule。

上游版本的唯一真值应由以下组合构成：

```text
官方 commit SHA
+ deepseek-harness/package.json version
+ pnpm-lock.yaml
+ Node.js 版本
```

不要只跟踪 `main`、`master` 或一个浮动 npm 版本。

### 3.2 Layer 2：自定义扩展层

所有 Web 定制都放在官方仓库之外：

```text
extensions/
├── packages/
│   ├── dsh-client-ui-custom/
│   ├── dsh-host-custom/
│   └── dsh-extension-contract/
└── package.json
```

#### 纯界面插件

只包含 Browser Client Plugin，适用于：

- 新增按钮；
- 新增侧边栏入口；
- 新增设置项；
- 新增工具卡片；
- 替换一个局部展示；
- 注册主题或样式扩展。

#### 双端插件

同时提供 Host 半和 Browser 半，适用于：

- 访问本地文件；
- 执行本地操作；
- 保存额外配置；
- 提供新的状态流；
- 调用自定义服务；
- 新增需要权限控制的功能。

Host 负责权威状态、持久化、权限和并发顺序；Client model 负责浏览器侧的 React-free 镜像；React 组件只负责展示。

### 3.3 Layer 3：Bundle/Profile Patch 层

自定义插件的加载和配置通过自己的 Patch 完成：

```text
extensions/
└── bundles/
    └── desktop-web/
        └── cordis.patch.yml
```

官方 Bundle 的组合关系是：

```text
官方 base bundle
  -> 官方 web bundle
  -> profile patch
  -> 用户 patch
  -> --patch overlay
```

自定义 Patch 应只维护增量：

- 插入自定义 Host/Client Plugin；
- 为自定义插件提供配置；
- 必要时有针对性地覆盖某个官方 row。

不要把官方完整 Web roster 复制到自己的 Patch 中。复制完整 roster 会把官方每次增删插件、调整顺序和修改配置都变成你的维护负担。

### 3.4 Layer 4：Tauri 桌面壳

Tauri 只负责系统级能力：

- 定位内嵌 Node 和 dsh runtime；
- 启动和停止 Node 子进程；
- 分配端口；
- 读取启动地址；
- 创建 WebView；
- 文件下载和另存为；
- 单实例；
- 窗口和系统菜单；
- 异常退出和日志。

Tauri 不负责：

- React 页面结构；
- Session 状态；
- Chat 渲染；
- 工具卡片业务；
- Web UI 的 DOM 重写；
- 官方组件的复制版本；
- 业务数据持久化。

当前 [tauri-app/src-tauri/src/main.rs](../tauri-app/src-tauri/src/main.rs) 已采用“启动 Node，再用 WebView 加载本地 URL”的基本方向。它目前还直接启动 `dsh web --port 0 --no-open`，自定义 Patch 的正式挂载应作为独立启动配置接入，而不是把插件逻辑写入 Rust 主流程。

## 4. UI 需求与扩展点映射

| 需求 | 首选实现 | 说明 |
|---|---|---|
| 侧边栏新增入口 | `sidebar.footer.action` 等 Sidebar Slot | 追加一个独立 entry |
| 会话头部新增操作 | `conversation.session.header.actions` | 通过 `ctx.slots.inject()` 等待声明 |
| Composer 增加按钮 | `conversation.composer.bar`、`conversation.input.left/right` | 不复制 Composer |
| 设置页面增加项目 | `settings.general.item`、`settings.section`、插件设置 Slot | 设置状态归属自己的 namespace |
| 新增工具卡片 | `tool.call.toolview` 或 keyed Tool Slot | 保留通用 fallback |
| 新增消息展示 | `ConversationNodeDefinition` + `conversation.chat.node` | 不重新解析 Session 日志 |
| 新增会话事件 | 自己定义 Session Event，再由 Definition 处理 | 事件必须可重放、可分页 |
| 修改主题 | `ctx.theme`、`--dsw-*` 设计 token | 不覆盖所有官方 CSS |
| 增加 Host 能力 | Host Plugin + Remote/API Controller | 浏览器不直接接触 Host 内部实现 |
| 替换一个官方局部组件 | single/keyed Slot 的明确优先级覆盖 | 只在确实需要时使用 |
| 完整重做整个页面 | 自己接管高层 Slot 或 root | 高风险，应单独维护兼容适配层 |
| 下载、菜单、窗口能力 | Tauri | 仅系统能力 |

### Slot 使用规则

官方 Slot 是首选扩展接口，必须遵守以下规则：

1. 向其他插件声明的 Slot 贡献内容时使用 `ctx.slots.inject()`。
2. `ctx.slots.register()` 只在确定目标 Slot 已经声明时使用。
3. 普通新增内容使用新的 list `id` 或未占用的 keyed key。
4. single/keyed Slot 的同名覆盖代表替换，不是追加。
5. 组件 Props 必须从 `PropsRuntime`、`PropsRenderSlots`、`PropsStore` 和 inject face 推导，组件不能拿到 Cordis `ctx`。
6. Slot store 只保存选择、草稿、面板宽度等视图状态，不能替代业务数据源。
7. Slot 的声明生命周期、卸载和重新挂载必须能够正确清理自定义 entry。

### Conversation 使用规则

需要在会话中展示新内容时：

1. 先为业务对象确定稳定 ID。
2. 定义可重放的 Session 事件或使用已有标准事件。
3. 实现一个 `ConversationNodeDefinition`。
4. 在 Chat 或其他目标中注册自己的 node data 类型。
5. 通过 keyed `conversation.chat.node` 注册渲染器。

禁止在自定义插件中重新打开 Session 历史、扫描全部事件或复制官方 Chat/Trajectory 状态管理。官方 Conversation 层已经负责事件窗口、分页、重连、Context 和目标快照。

## 5. Client Plugin 的规范包结构

一个完整的自定义 Client Plugin 至少应具备：

```text
dsh-client-ui-custom/
├── package.json
├── tsconfig.json
├── tsdown.config.ts
├── src/
│   ├── index.ts
│   ├── invariant.ts
│   └── client/
│       ├── index.ts
│       ├── contract/
│       ├── locales.ts
│       └── *.tsx
├── README.md
└── tests/
```

`package.json` 至少应保证：

- 导出 `.`、`./invariant`、`./client` 和需要的类型入口；
- 声明 `dsh.client`；
- `platform` 为 `web`；
- 构建产物包含 `lib/client.js`；
- `files` 覆盖所有运行时 JavaScript、CSS 和类型文件；
- 依赖、peerDependencies、devDependencies 按官方 Client Plugin 规则划分。

`dsh.client.external` 只用于真正的浏览器模块图依赖，不能用来绕过“功能插件之间不能 runtime-import”的边界。React、Cordis 和官方基线模块应使用 Web Client 的共享模块表，避免重复打包。

官方的 `dsh.client.inject` 是插件关系描述，不应被误认为可以代替 Cordis service 注入或控制所有激活顺序。真正的运行时服务依赖由 Cordis service `inject` 解决；同步模块依赖由 Client Module Graph 解决。

## 6. 明确禁止的长期方案

以下方式可以短期改出效果，但不符合可升级架构：

1. 直接修改 `apps/web`。
2. 直接修改 `packages/client/ui-*`。
3. Fork 官方仓库后长期手工 rebase。
4. 复制官方 Chat、Session、Layout 或 Settings 代码。
5. 通过大范围 CSS 覆盖官方页面。
6. 通过 DOM 查询和 MutationObserver 替换主要页面结构。
7. 在 Tauri 初始化脚本中重写整个 Web UI。
8. 依赖 CSS Module hash、私有 class 名或具体 DOM 层级。
9. 直接 runtime-import 官方功能插件内部实现。
10. 在自定义 Patch 中复制完整官方 Web roster。
11. 把业务状态放入 Tauri Rust 层。
12. 用本地临时修改的官方源码构建生产包。

当前 Tauri 中基于 ARIA 文案隐藏 Session 导出弹窗，属于桌面适配层的临时兼容措施。它可以保留为窄范围的兜底，但不能成为业务 UI 的主要扩展方式，因为它依赖官方 DOM、ARIA 属性、文案和 Portal 结构。

## 7. 版本与依赖策略

### 7.1 版本真值

每个可发布版本都应记录：

```text
desktop release
├── upstream commit SHA
├── dsh version
├── pnpm-lock.yaml digest
├── Node.js version
├── React/Cordis versions
├── custom plugin versions
└── custom bundle/patch version
```

不要仅依据文档中的版本号。当前工作区已经出现官方 `package.json` 版本和既有分析文档版本不一致的情况，这正是需要建立版本真值和 CI 校验的原因。

### 7.2 依赖范围

自定义插件应优先依赖官方发布的公开包入口，不依赖官方仓库的源码路径。建议维护兼容矩阵：

| 官方版本/commit | 自定义插件版本 | 兼容状态 | 备注 |
|---|---|---|---|
| upstream A | extension 1.x | supported | 基线版本 |
| upstream B | extension 1.x | tested | 无需适配 |
| upstream C | extension 1.x | blocked | Slot/Remote 破坏性变更 |
| upstream C | extension 2.x | supported | 适配层更新 |

当官方接口不稳定时，把差异收敛到自定义适配层，不要把上游兼容代码散落在所有 UI 组件中。

## 8. 官方升级流程

每次升级都必须作为一个可审查的升级变更完成。

### 阶段 1：更新上游

1. 更新 submodule 到明确的官方 commit 或 tag。
2. 记录官方版本、commit、锁文件摘要和 Node 版本。
3. 保留上一个已发布版本作为回滚基线。

### 阶段 2：验证官方自身

在自定义插件之前，先确认官方构建和测试本身正常：

- Host 构建；
- Client 构建；
- Web 构建；
- 官方 typecheck；
- 官方 GUI 测试；
- 官方 Web replay/e2e 测试。

### 阶段 3：构建自定义扩展

1. 使用当前官方公开包和类型重新构建自定义插件。
2. 检查 `./client` 导出和 `dsh.client` 声明。
3. 检查依赖和外部模块请求。
4. 构建自己的 Bundle 和 Patch。
5. 将插件和 Patch 安装到生产 runtime 的可解析位置。

### 阶段 4：检查 Client Module Graph

必须验证：

- 每个动态插件都有 `./client` 导出；
- 所有动态模块请求都有唯一供应方；
- 没有自依赖或同步循环；
- React、Cordis 等共享模块没有重复副本；
- 构建产物和资源文件没有遗漏；
- Browser half 和 Host half 的版本一致；
- WebBootGraph 可以完成加载和激活。

### 阶段 5：运行兼容性门禁

至少覆盖以下测试：

| 门禁 | 目标 |
|---|---|
| 官方 typecheck/build | 上游自身健康 |
| 自定义插件 typecheck/build | 扩展代码仍符合公开类型 |
| Client package 校验 | 包结构、依赖和模块图合法 |
| Slot contract 检查 | Slot 名、scope、cardinality、Props 未失配 |
| Conversation replay | 自定义消息/工具节点可重放 |
| Web replay/e2e | 官方 UI 与自定义 UI 可启动和交互 |
| 关键 UI snapshot | 防止扩展入口静默消失 |
| Tauri 启动测试 | Node、端口、URL 和 WebView 启动正常 |
| 子进程生命周期测试 | 窗口退出后 Node 正确回收 |
| 打包产物 smoke test | 安装包内 runtime、插件和 Patch 都可解析 |

任何一个关键门禁失败，都不应直接发布新版本，而应停留在上一版可用 runtime。

## 9. 推荐的升级自动化

建议建立以下脚本或 CI 工作流：

```text
scripts/
├── sync-upstream.*
├── build-runtime.*
├── build-extensions.*
├── verify-client-compatibility.*
├── verify-desktop-smoke.*
└── package-release.*
```

自动升级任务的逻辑应是：

```text
发现上游新 commit
        |
        v
更新 submodule 指针
        |
        v
构建官方 runtime
        |
        v
构建自定义插件和 Patch
        |
        v
运行类型、模块图、Slot、Web、Tauri 门禁
        |
        +-- 失败：阻断升级，保留旧版本
        |
        `-- 成功：生成可审查升级变更
```

升级 PR 中应能清楚看到：

- 上游 commit 从哪里变到哪里；
- 官方版本如何变化；
- 哪些公开契约发生变化；
- 自定义插件是否需要适配；
- 测试和快照是否变化；
- 最终安装包使用的完整版本组合。

## 10. Tauri 与官方启动协议的隔离

当前 Tauri 通过 stdout 解析类似以下内容的启动地址：

```text
dsh web: http://127.0.0.1:PORT
```

这种方式目前可用，但它依赖官方启动输出格式。为了降低升级影响：

1. 将 Node 启动、stdout 解析、健康检查封装在一个很小的适配模块中。
2. 为端口分配、URL 解析、超时和异常退出建立单元测试。
3. 不要让其他 Tauri 代码依赖 dsh 内部实现。
4. 如果官方未来提供机器可读的启动协议，优先切换到该协议。
5. 将启动协议变更作为兼容性门禁的一部分。

Tauri 中的下载接管、另存为和窗口处理属于系统能力，可以继续保留；但 Web 业务仍应通过官方 Host/Client 扩展机制完成。

## 11. 目标目录结构

建议最终形成如下逻辑结构：

```text
deepseek-harness-desktop/
├── deepseek-harness/                 # 官方 submodule，只读
├── extensions/
│   ├── packages/
│   │   ├── dsh-client-ui-custom/
│   │   ├── dsh-host-custom/
│   │   └── dsh-extension-contract/
│   ├── bundles/
│   │   └── desktop-web/
│   │       └── cordis.patch.yml
│   └── package.json
├── tauri-app/                        # 只负责系统壳
├── scripts/
│   ├── sync-upstream.*
│   ├── build-runtime.*
│   ├── verify-compatibility.*
│   └── package-desktop.*
├── tests/
│   ├── custom-ui/
│   ├── compatibility/
│   └── desktop-smoke/
└── docs/
    ├── custom-web-extension-and-upgrade-architecture.md
    └── upgrade-policy.md
```

当前目录名称可以暂时保持不变，关键不是目录名字，而是四个所有权边界必须成立：

```text
官方源码：只读输入
自定义插件：独立产物
Patch：独立组合层
Tauri：系统能力层
```

## 12. 分阶段演进建议

### 阶段 A：建立边界

- 将官方仓库固定为 submodule 或等价的精确锁定源；
- 把官方 commit、版本和锁文件设为版本真值；
- 明确外层仓库不允许出现官方源码修改。

### 阶段 B：建立最小自定义插件

- 先实现一个只追加 Slot 的小插件；
- 验证 `dsh.client`、`./client`、Bundle、Patch 和 WebBootGraph 全链路；
- 验证升级后插件仍能加载。

### 阶段 C：增加业务能力

- 需要后端时增加 Host Plugin 和 Remote；
- 需要会话节点时增加 Conversation Definition；
- 为每个插件添加自己的测试、README 和兼容矩阵。

### 阶段 D：建立发布门禁

- 接入官方构建和测试；
- 接入自定义 Client 检查；
- 接入 Web replay/e2e；
- 接入 Tauri 启动、退出和打包 smoke test；
- 升级失败自动停留在旧版本。

### 阶段 E：处理高风险整体改版

如果未来确实要完全重做官方页面，不应偷偷从 DOM 层替换，而应明确把它作为“自有 Shell”项目：

- 只消费官方公开 Client model、Remote 和 Conversation 数据；
- 自己定义高层 Slot 和页面布局；
- 将官方 UI roster 和自有 roster 分开；
- 接受更高的维护成本；
- 用适配层隔离官方模型变化。

整体重做并不是不能做，但它不再是低成本 UI 扩展，而是维护一套独立的 Web Presentation Layer。

## 13. 验收标准

方案实施完成后，应满足：

1. 外层仓库可以独立查看和更新官方 submodule 指针。
2. 官方 submodule 内没有自定义源码修改。
3. 自定义插件可以脱离官方源码目录单独构建。
4. 自定义 Web UI 只通过公开 Slot、Conversation、Remote 或 Theme 接口接入。
5. 自定义 Bundle/Prompt 只维护增量 Patch。
6. Tauri 不包含业务 UI 实现。
7. 上游升级不会产生官方源码合并冲突。
8. 上游升级会触发完整兼容性门禁。
9. 门禁失败时不会覆盖上一版可运行发布物。
10. 官方破坏性变更只需要修改自定义适配层、插件或 Patch，不需要手工合并官方 UI 源码。

## 14. 最终判断

这套架构是当前项目最合理的长期方案。它利用了 DeepSeek Harness 已经提供的插件化、Slot、Conversation 和 Client Module Graph 机制，把“定制 Web UI”放在官方支持的扩展边界上，同时让 Tauri 保持为一个尽量薄的系统壳。

真正应该追求的不是“官方升级后永远不用看代码”，而是：

```text
升级可重复
升级可验证
失败可阻断
问题可定位
修复范围可控
自定义代码不丢失
```

这才是能够长期维护并持续跟随官方版本的工程化目标。
