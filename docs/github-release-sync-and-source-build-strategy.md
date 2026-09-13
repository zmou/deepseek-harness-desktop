# DeepSeek Harness GitHub Release 同步与源码构建方案

> 状态：方案确认，待实施
>
> 适用版本：`dsh-v0.1.2-alpha.2` 及后续官方 GitHub Release
>
> 当前策略：人工指定 tag，同步官方源码，源码构建运行时，门禁通过后再打包

## 1. 目的

本方案用于解决官方 GitHub Release 与 npm 默认发布标签不同步的问题，并为桌面端提供一条不修改官方 `deepseek-harness` 源码、当前不依赖 npm 运行时包、后续可以重复升级的构建路径。

目标不是把官方仓库复制成一个长期维护的分支，也不是承诺未来所有 alpha 版本都无需适配。目标是把官方源码、桌面壳、自定义扩展和升级验证分成独立层，使每次升级都能明确回答以下问题：

- 当前桌面端跟随了哪个 GitHub tag 和 commit；
- 运行时是否由该 commit 的官方源码构建；
- 构建出的 Web、CLI、bundle 和依赖是否完整；
- 自定义功能是否仍然兼容；
- 新版本失败时是否可以继续使用旧版本并回滚。

## 2. 当前版本事实

截至本方案编写时，已核对到以下状态：

| 项目 | 当前值 |
|---|---|
| GitHub Release tag | `dsh-v0.1.2-alpha.2` |
| tag 对应 commit | `0a53fb55bea101816fa226bb964ae2bed71c343b` |
| 官方源码版本 | `0.1.2-alpha.2` |
| npm `latest` | `0.1.1-rc.2` |
| npm `next` | `0.1.1-rc.2` |
| npm `alpha` | `0.1.2-alpha.2` |
| 当前外层运行时脚本 | 通过 `pnpm add @deepseek-ai/dsh` 从 npm 安装 |

npm 已经存在 `alpha.2` 并不改变本方案的原则。当前阶段的版本真值仍然是 GitHub tag 和其 commit；npm 的 `latest`、`next` 或不带明确版本号的安装命令不能用于决定桌面端运行时版本。

## 3. 关键概念

### 3.1 Monorepo

官方仓库是单仓多包仓库（monorepo）。一个 Git 仓库中同时维护多个相互依赖的包，例如：

- `apps/cli`：`@deepseek-ai/dsh` 的命令入口；
- `packages/bundle/web-app`：Web profile 的 bundle 和启动 glue；
- `packages/client/*`：浏览器端模块和 UI；
- host、agent、session、tool、Cordis 等其他包；
- 根级 `pnpm-lock.yaml`、构建脚本和测试。

这些包在源码中通过 `workspace:^` 等 workspace 依赖连接。生产 Web 运行时需要的是这些包构建后的可执行闭包，而不是只拿 `apps/cli` 一个目录。

### 3.2 构建工作区与运行时闭包

两者必须分开：

| 名称 | 作用 | 是否进入安装包 |
|---|---|---|
| 构建工作区 | 完整官方 monorepo、开发依赖、源码、测试和构建工具 | 否 |
| 运行时闭包 | `dsh` 编译产物、Web 产物、官方 bundle、生产依赖和 Node | 是 |

完整 monorepo 可以在构建阶段存在，但不能直接作为桌面安装包的运行目录。最终安装包只携带运行 `dsh web` 必需的内容。

## 4. GitHub 源码与官方 npm 包的关系

### 4.1 官方 npm 发布链路

官方发布流程的核心步骤是：

```text
checkout dsh-vX.Y.Z tag
    ↓
pnpm install --frozen-lockfile
    ↓
pnpm run build:official
    ↓
构建 host、client、Web 和 bundle 产物
    ↓
pnpm run release:pack --family dsh
    ↓
为 dsh 发布家族生成多个 npm tarball
    ↓
验证打包后的干净安装
    ↓
npm publish
```

官方发布的不是一个包含整个 monorepo 的源码压缩包，而是多个已构建的 npm 包。`@deepseek-ai/dsh` 主要提供 `lib/*.js` 和 `lib/bin.js` 入口；它依赖的 `dsh-web-app`、`dsh-base`、client、host 等包也分别发布。用户安装 `@deepseek-ai/dsh` 时，npm 再解析这些依赖形成完整运行时。

### 4.2 同一个 tag 自行构建是否功能等价

如果满足以下条件，自行基于同一个 GitHub tag 构建出的运行时，在产品功能和运行作用上应与官方 npm 版本等价：

1. 使用相同的 tag 对应 commit；
2. 使用该 commit 中的官方 lockfile；
3. 执行官方 `pnpm run build:official`；
4. 构建完整的 dsh 发布家族，而不是只构建 CLI；
5. 收集完整的生产依赖闭包、官方 bundle 和 Web 前端产物；
6. 使用兼容的 Node 版本和等价启动参数；
7. 通过 CLI、Web、插件和安装闭包验证。

两者的差别主要在交付链，而不是目标功能：

| 对比项 | 官方 npm 版本 | GitHub tag 自行构建 |
|---|---|---|
| 输入 | 官方 tag 产出的发布包 | 官方 tag 的源码 |
| 构建者 | 官方 CI | 本地或自有 CI |
| 交付 | npm registry 中的多个 tarball | Tauri resources 中的运行时闭包 |
| 运行内容 | 编译后的 JS、Web 产物、生产依赖 | 编译后的 JS、Web 产物、生产依赖 |
| 当前适用性 | 受 npm 标签和发布时机影响 | 直接跟随 GitHub Release |
| 主要风险 | npm 发布滞后或依赖解析变化 | 自己漏构建、漏依赖或验证不足 |

因此，本方案不追求文件逐字节相同，而要求来源、版本、依赖闭包、启动行为和用户可见功能一致。自行构建后直接运行开发工作区，或者只复制一个 `bin.js`，都不能视为等价实现。

## 5. 总体架构

```text
GitHub tag / commit
        │
        ▼
官方 deepseek-harness submodule（只读）
        │
        ▼
pnpm install --frozen-lockfile
        │
        ▼
pnpm run build:official
        │
        ▼
官方本地包产物 + 生产依赖闭包
        │
        ├── 自定义插件和 cordis patch
        │
        ▼
tauri-app/resources/runtime/
        │
        ▼
Tauri 启动内嵌 Node
        │
        ▼
dsh web --port 0 --no-open
        │
        ▼
系统 WebView 加载官方 Web UI
```

外层仓库分为三层：

| 层 | 内容 | 升级时动作 |
|---|---|---|
| L1 上游 | `deepseek-harness/` Git submodule | 更新 commit 指针 |
| L2 桌面壳 | Tauri Rust 主进程和打包配置 | 通常不变，仅在启动协议变化时适配 |
| L3 自定义扩展 | 独立插件、patch、适配测试 | 重新构建并通过兼容性门禁 |

L2 不 import L1 源码，L3 不 import L1 源码路径。所有跨层交互通过进程启动、stdout URL、HTTP、公开插件接口和构建产物完成。

## 6. 上游 submodule 规范

### 6.1 目录和版本真值

外层仓库应将官方目录正式登记为 Git submodule：

```text
deepseek-harness-desktop/
├── .gitmodules
├── deepseek-harness/       # 官方仓库 submodule，只读
├── scripts/
├── tauri-app/
├── plugins/
└── docs/
```

外层 Git 只记录 submodule 的 commit 指针。版本记录同时包含：

- tag 名称，例如 `dsh-v0.1.2-alpha.2`；
- 完整 commit SHA；
- `deepseek-harness/package.json` 的版本；
- 官方 lockfile 摘要。

### 6.2 同步校验

每次同步必须先验证：

1. 远端 tag 存在；
2. tag 解析出的最终 commit SHA 已记录；
3. 本地 submodule HEAD 等于该 SHA；
4. 根 `package.json` 版本与 tag 版本一致；
5. dsh 发布家族的包版本一致；
6. submodule 工作区无未提交修改；
7. 外层仓库没有将补丁写入官方目录。

GitHub source archive 可以作为缓存输入，但必须在下载后用目标 commit 校验，不能使用 `main`、浮动分支或未校验的 archive。

## 7. 官方源码构建

构建必须在完整官方 submodule 中执行：

```bash
pnpm install --frozen-lockfile
pnpm run build:official
```

`build:official` 的含义是完成官方 host/client library 与 Web 前端构建，并写入官方构建记录。构建环境应固定并记录：

- Node 主版本和完整版本；
- pnpm 版本；
- 操作系统和 CPU 架构；
- `pnpm-lock.yaml` SHA-256；
- Git commit SHA；
- 构建 profile 和公开构建环境变量。

构建失败时不得继续组装 runtime，也不得覆盖当前稳定版本。

## 8. Runtime closure 组装

### 8.1 目标目录

建议的最终目录如下：

```text
tauri-app/resources/runtime/
├── node/
│   └── node.exe 或 node
├── dsh-runtime/
│   ├── package.json
│   ├── pnpm-lock.yaml
│   ├── node_modules/
│   └── runtime-manifest.json
└── plugins/
    ├── <plugin-package>/
    └── *.cordis.patch.yml
```

### 8.2 组装原则

- 运行时由当前 tag 构建出的官方包组成；
- 不从 npm 下载 `@deepseek-ai/dsh` 作为运行时来源；
- 优先使用官方 `pnpm pack` 生成本地 tarball，再在干净 staging 目录安装；
- 不让最终运行时继续解析 `workspace:*` 或 `workspace:^`；
- 只保留生产依赖、官方 bundle、client/Web 产物和运行入口；
- 不复制完整 monorepo、源码、测试、文档和开发依赖；
- 允许构建阶段使用 registry 获取第三方开源依赖，但官方 dsh 包必须来自当前 tag 的本地构建产物；
- Node 二进制继续由外层构建脚本下载并放入 runtime。

推荐使用 pnpm 的生产部署能力生成隔离目录；如果当前 pnpm 配置要求 legacy deploy，应显式使用兼容参数，并在日志中记录。最终目录必须经过干净安装验证，不能直接把官方工作区的 `node_modules` 原样复制进安装包。

### 8.3 运行入口

运行时入口固定为：

```text
node_modules/@deepseek-ai/dsh/lib/bin.js
```

Tauri 启动参数为：

```text
dsh web --port 0 --no-open
```

`--port 0` 让操作系统分配空闲端口，`--no-open` 禁止 dsh 自己打开默认浏览器。主进程从 stdout 解析 `dsh web: http://127.0.0.1:<port>`，再让系统 WebView 加载该地址。

## 9. 运行时版本清单

每次 runtime 构建必须生成 `runtime-manifest.json`。示例：

```json
{
  "formatVersion": 1,
  "upstream": {
    "repository": "https://github.com/deepseek-ai/deepseek-harness",
    "ref": "dsh-v0.1.2-alpha.2",
    "commit": "0a53fb55bea101816fa226bb964ae2bed71c343b",
    "version": "0.1.2-alpha.2"
  },
  "toolchain": {
    "node": "v24.14.0",
    "pnpm": "11.7.0"
  },
  "source": {
    "lockfileSha256": "..."
  },
  "runtime": {
    "entry": "@deepseek-ai/dsh/lib/bin.js",
    "artifactSha256": "..."
  },
  "extension": {
    "version": "0.1.0",
    "artifactSha256": "..."
  }
}
```

清单中的 `artifactSha256` 应覆盖排序后的运行时文件路径和内容，避免只记录目录大小。扩展没有启用时，`extension.version` 使用明确的空值或 `none`，不能省略字段造成多种解释。

## 10. 自定义 Web 扩展

### 10.1 目录边界

```text
plugins/
├── dsh-plugin-<feature>/
│   ├── package.json
│   ├── src/
│   ├── tests/
│   └── cordis.patch.yml
├── patches/
└── compatibility/
```

插件、patch、适配测试和构建脚本均属于外层仓库，不能放入 submodule。插件依赖公开的 Cordis、dsh API、bundle、client slot 或事件接口；不能依赖官方源码相对路径，也不能通过复制官方 UI 源码来实现长期分叉。

### 10.2 构建和安装

插件构建时应使用当前 tag 生成的本地官方包进行类型检查和验证。最终以独立 package 和 `cordis.patch.yml` 装入 runtime。插件的依赖可以使用本地文件包或当前构建生成的官方 tarball，不能隐式依赖开发机上残留的 workspace symlink。

### 10.3 升级影响

官方升级不会覆盖插件源码。若公开 API 发生破坏性变更，只修改 L3 插件和适配测试；如果启动协议、Web 资源布局或 Tauri 资源路径发生变化，再修改 L2。任何变化都必须经过门禁，不能直接替换稳定运行时。

## 11. 人工触发的升级流程

当前采用人工指定目标 tag，不自动追踪新版本。推荐入口形式：

```bash
node scripts/sync-upstream.mjs --ref dsh-v0.1.2-alpha.2
node scripts/build-runtime.mjs --ref dsh-v0.1.2-alpha.2
node scripts/check-compatibility.mjs --runtime <runtime-path>
```

实际脚本名称可以按仓库实现调整，但行为必须固定为以下顺序：

1. 接收明确 tag；
2. 查询远端 tag 和 commit；
3. 拒绝 submodule 脏工作区；
4. 更新 submodule 到目标 commit；
5. 读取和校验官方版本；
6. 执行 `pnpm install --frozen-lockfile`；
7. 执行 `pnpm run build:official`；
8. 生成官方本地包产物；
9. 组装隔离 runtime closure；
10. 构建自定义插件；
11. 生成 `runtime-manifest.json`；
12. 执行兼容性门禁；
13. 门禁通过后才允许生成 Tauri 安装包。

脚本应支持 `--dry-run`、构建缓存、失败日志保留和明确的输出目录。新版本必须先写入独立候选目录，不能直接覆盖 `tauri-app/resources/runtime` 中的稳定版本。

## 12. 兼容性门禁

### 12.1 必须阻断的检查

| 门禁 | 检查内容 |
|---|---|
| Tag | 远端 tag、解析 commit、本地 submodule HEAD 一致 |
| 版本 | tag、根 package、dsh 发布成员版本一致 |
| 源码 | submodule 无修改，外层无官方目录补丁 |
| Lockfile | 使用目标 tag 自带 lockfile，摘要记录一致 |
| 官方构建 | `pnpm run build:official` 成功 |
| 本地安装 | 从当前构建的本地 tarball 干净安装成功 |
| CLI | `dsh --version` 返回目标版本 |
| Web | `dsh web --port 0 --no-open` 在超时内打印 URL |
| HTTP | URL 返回 200，前端入口和关键静态资源可加载 |
| 插件 | 自定义 plugin 和 patch 正常加载 |
| 清单 | manifest 字段、版本、入口和摘要一致 |
| Tauri | 内嵌 Node、bin.js 和资源路径在目标平台存在 |
| 生命周期 | 关闭窗口回收 Node，Node 异常退出能诊断 |

### 12.2 官方检查

在构建时间和 CI 资源允许时，执行官方 typecheck、lint、相关 Web smoke test 和官方 packed-install 验证。不能因为桌面端最终只使用运行时闭包，就跳过能发现包依赖或构建错误的官方检查。

### 12.3 门禁结果

- 全部通过：生成候选安装包并进入人工验收；
- 任一阻断项失败：不替换稳定 runtime，不生成正式安装包；
- 失败报告必须保存目标 tag、commit、工具链、日志和失败检查；
- 旧版本仍然可启动。

## 13. 回滚与发布

建议按版本保留候选目录：

```text
runtime-releases/
├── 0.1.2-alpha.2-<commit>/
├── 0.1.3-alpha.1-<commit>/
└── current -> 通过门禁的版本
```

在候选版本全部通过后，再通过原子目录切换或完整重新打包替换当前版本。至少保留上一个可运行版本。失败回滚只切换运行时目录，不修改用户的 `DSH_HOME`、会话历史、凭据或工作区文件。

发布包和内部构建报告都应包含：

- 上游 tag 和 commit；
- dsh 版本；
- Node/pnpm 版本；
- lockfile 摘要；
- runtime 摘要；
- 自定义扩展版本和摘要；
- 门禁结果。

## 14. 安全与供应链要求

- 不执行未校验的远端脚本；
- tag 解析后记录完整 commit，不能只记录版本字符串；
- source archive 下载后必须校验目标 commit 或内容摘要；
- 构建日志中记录实际 Node、pnpm 和 lockfile；
- npm registry 不参与官方 dsh 运行时选版；
- 构建阶段需要的第三方依赖应由 lockfile 固定，并尽量使用受控缓存；
- 运行时不应包含开发工具、测试夹具、凭据或工作区私有文件；
- Tauri 只允许加载 `127.0.0.1` 的本地 dsh URL，不开放 `0.0.0.0`；
- 版本清单摘要校验失败时停止启动，避免静默运行混合版本。

## 15. 对当前仓库的后续改造清单

> **当前状态（2026-09）**：本方案仍为规划。现状是：运行时由
> `scripts/build-runtime.mjs` 通过 `npm install @deepseek-ai/dsh@<固定版本>` 从 npm
> 安装（npm 扁平布局 + overrides 钉死家族包版本 + glob 工具补丁），
> `deepseek-harness/` 仅为本地参考目录（gitignored，未登记 submodule）。
> 以下改造尚未实施。

当前 `scripts/build-runtime.mjs` 仍然执行：

```text
npm install @deepseek-ai/dsh
```

这与本方案（源码构建运行时）不一致，属有意为之的现状：npm 安装是当前唯一可用的
稳定路径。实施阶段应按以下顺序改造：

1. 正式登记 `deepseek-harness` submodule；
2. 新增 tag/commit 校验脚本；
3. 将 `build-runtime.mjs` 改为调用官方源码构建；
4. 用本地构建产物生成隔离 runtime closure；
5. 生成并校验 `runtime-manifest.json`；
6. 新增兼容性门禁脚本；
7. 为失败保留候选目录和日志；
8. 保持 Tauri 的进程启动协议不变，仅增加 manifest 和资源校验；
9. 将人工升级流程写入 CI 的手动 workflow 或本地统一入口。

这些改造不应修改官方 submodule 内的源码。若确实遇到官方缺陷，需要在外层以明确记录的 patch 文件表达，并在每次升级时重新应用和验证；长期优先推动官方提供公开扩展点。

## 16. 验收标准

### 基线版本

- `dsh-v0.1.2-alpha.2` 能从源码完成官方构建；
- runtime 不依赖安装时从 npm 获取 `@deepseek-ai/dsh`；
- Tauri 能用内嵌 Node 启动 Web；
- Web 页面、静态资源和核心交互可用；
- manifest 能准确描述 tag、commit、工具链和摘要；
- 自定义扩展加载成功；
- 关闭桌面应用后 Node 子进程被回收。

### 后续升级

- 只需指定新的官方 tag 即可生成候选版本；
- 外层自定义插件和 patch 不被覆盖；
- 官方源码目录保持原样，无外层业务代码混入；
- 新版本失败时旧版本仍可运行；
- 升级报告可以追溯到具体 tag、commit 和构建产物；
- 门禁结果足以支持人工决定是否发布。

## 17. 结论

推荐采用以下长期原则：

> GitHub tag 决定版本，官方源码决定构建输入，官方构建脚本决定产物生成方式，runtime closure 决定桌面包内容，独立插件决定自定义能力，兼容性门禁决定是否允许升级。

这样，官方 npm 发布滞后不会阻塞桌面端跟随 GitHub Release；完整 monorepo 只承担构建职责，不污染最终安装包；自定义 Web 功能不会因为同步官方源码而丢失；每次升级都有明确的版本证据、失败阻断和回滚路径。

“100% 升级无需适配”不是现实承诺。可以承诺的是：官方源码不被修改，自定义代码不被覆盖，升级经过可重复验证，破坏性变更不会绕过门禁直接进入稳定发行包。
