# Wabity 架构说明

## 1. 文档目标

`ARCHITECTURE.md` 只描述稳定架构，不承载易漂移的实现细节。

这份文档回答四个问题：

1. `Wabity` 现在到底是什么系统
2. 代码按什么边界分层
3. 核心运行链路如何穿过这些层
4. 当前有哪些明确设计约束

不应该继续写进这里的内容：

- CSS token、颜色、间距、浮层位置这类 UI 微决策
- 某个页面上一个按钮的排列顺序
- 某个 feature 当前有哪些临时文案或交互修饰
- 过长的功能清单和历史变更记录

这些内容应下沉到对应目录的 `README.md`，大型方案和阶段推进记录放到 `docs/`。

## 2. 产品定义

`Wabity` 是一个基于 `Tauri v2 + React + TypeScript + Rust` 的桌面 launcher。它当前不是“通用桌面平台”，而是围绕下面几条主链路组织：

1. 全局快捷键唤起 launcher
2. 在当前 workspace 中输入并执行本地动作、文件补全或应用启动
3. 在 launcher 内执行轻量 RAG 问答
4. 创建并管理本地 `stdio` ACP agent session
5. 通过截图 OCR 或选中文本翻译，把结果回填到 launcher
6. 通过设置页维护快捷键、外观、AI 功能、LLM、RAG、ACP agent 和全局 MCP 配置

明确非目标：

- 不做插件市场
- 不做远程 ACP transport 编排
- 不做全盘文件索引
- 不让前端接管主业务调度
- 不把 ACP session 和 launcher 轻量问答混成同一种会话模型

## 3. 总体架构

系统可以概括为一条固定链路：

`快捷键 / UI 输入 -> 前端状态编排 -> Tauri IPC -> Rust AppState -> 领域服务 -> 基础设施 / 外部系统`

其中外部系统包括：

- 操作系统窗口与全局快捷键
- 本地配置文件与 workspace 历史
- 本地应用索引、文件系统、目录监听
- ACP agent 进程
- OpenAI 兼容 LLM / OCR / Embedding 服务
- LanceDB 与 SQLite

核心原则：

- 前端负责展示、输入状态和安全 UI 副作用，不负责核心业务编排
- Rust 侧负责调度、约束和运行时状态
- 前后端只通过结构化模型通信
- 平台能力统一经 `infrastructure` 接入，业务语义统一经 `services` 暴露
- 能用类型表达的约束，不留给字符串协议和运行时猜测

## 4. 代码分层

### 4.1 前端

| 路径                    | 职责                                           | 约束                           |
| ----------------------- | ---------------------------------------------- | ------------------------------ |
| `src/app`               | 应用壳、视图切换、全局外观应用                 | 不承载业务规则                 |
| `src/features/launcher` | launcher UI、输入状态、补全浮层、问答/ACP 展示 | 匹配、执行、搜索以后端结果为准 |
| `src/features/settings` | 设置页 UI 与分组草稿态                         | 不直接处理配置落盘细节         |
| `src/lib/tauri`         | IPC client、事件订阅、浏览器 fallback          | 前端与 Rust 的唯一通信边界     |

前端当前只有两个真正的产品视图：`launcher` 和 `settings`。`src/app/App.tsx` 只负责这两个视图的装配与切换。

`settings` 里凡是显式提交的分组，都把保存、恢复已保存版本和定位问题动作放在主编辑区内的草稿操作卡，不再挂在页面头部。

其中 `AI 功能` 分组按任务边界组织为“翻译配置”和“文档问答配置”两个编辑卡，每张卡同时维护任务级模型路由和系统提示词；`LLM` 分组只维护可复用条目本身，不承载任务默认选择。

### 4.2 Rust 后端

| 路径                           | 职责                                       | 约束                         |
| ------------------------------ | ------------------------------------------ | ---------------------------- |
| `src-tauri/src/main.rs`        | 薄入口，只调用 `wabity_lib::run()`         | 不写业务逻辑                 |
| `src-tauri/src/lib.rs`         | 模块装配与桌面入口导出                     | 保持极薄                     |
| `src-tauri/src/app.rs`         | 启动顺序、插件注册、快捷键绑定、窗口初始化 | 只做应用装配，不沉淀具体业务 |
| `src-tauri/src/commands`       | Tauri IPC 分发与参数解析                   | 只做边界转换                 |
| `src-tauri/src/state`          | `AppState` 运行时总装配                    | 协调服务，不变成巨型业务类   |
| `src-tauri/src/domain`         | 稳定领域模型与前后端共享结构               | 不依赖 Tauri UI 细节         |
| `src-tauri/src/services`       | 用例级服务和核心流程                       | 纯逻辑优先，平台调用后置     |
| `src-tauri/src/infrastructure` | 配置、窗口、快捷键等系统能力               | 不承载业务语义               |

### 4.3 分层依赖方向

允许的依赖方向：

`frontend -> tauri client -> commands -> state -> services -> domain / infrastructure`

禁止的方向：

- 前端直接拼系统协议或读取配置文件
- `domain` 反向依赖 `services`
- `infrastructure` 承担动作匹配、问答调度等业务决策
- `main.rs` / `lib.rs` / `app.rs` 堆满流程细节

## 5. 运行时核心

### 5.1 启动序列

启动入口是：

`src-tauri/src/main.rs -> src-tauri/src/lib.rs::run -> src-tauri/src/app.rs::run`

当前固定顺序：

1. 初始化 `tracing`
2. 注入构建时版本与日期信息
3. 注册 Tauri plugin 和全局快捷键 handler
4. 创建 `AppState`
5. 对账 `general.autoStart` 与系统登录启动项状态
6. 启动 ACP event loop
7. 恢复可恢复 session
8. 初始化应用索引后台任务
9. 配置主窗口
10. 读取并注册快捷键
11. 启动后默认隐藏 launcher

约束：

- `setup` 只做装配
- 启动阶段需要等待的异步初始化统一通过 `tauri::async_runtime::block_on(...)` 接入
- 开机自启动属于基础设施能力：保存设置时必须即时同步到系统登录项，启动时还要再做一次 best-effort 对账，避免配置与系统状态漂移
- `main.rs` 保持薄，业务逻辑下沉到模块

### 5.2 AppState

`AppState` 是 Rust 侧的运行时总装配点，不是领域模型。

它负责：

- 持有 matcher、executor、application、file search、ACP、RAG、OCR 等服务实例
- 持有配置存储与当前 workspace 运行时状态
- 在需要时把配置投影成运行时依赖
- 对外提供统一的查询、执行、设置更新和会话管理入口

它不应该负责：

- 维护前端展示细节
- 混合低层文件格式处理与高层业务规则
- 直接承载庞杂的分支逻辑而不继续下沉到 `services`

### 5.3 IPC 边界

`src-tauri/src/commands/mod.rs` 当前保留了显式命令分发，而不是依赖宏生成的大一统入口。

这样做的原因：

- IPC 边界更清楚
- 参数解析集中
- `rust-analyzer diagnostics` 不依赖 Tauri 宏展开结果

命令按职责拆到：

- `launcher`
- `settings`
- `rag`
- `skills`
- `workspace`
- `acp`

## 6. 核心业务链路

### 6.1 Launcher 输入链路

主链路：

1. 前端采集输入并判断上下文
2. 通过 IPC 调用匹配、文件搜索、应用搜索或执行命令
3. Rust 侧根据 `QueryPayload` 和输入模式分流
4. `MatcherService` / `FileSearchService` / `ApplicationService` 返回结构化候选
5. `ExecutorService` 或特定服务执行动作
6. 前端按结构化结果渲染反馈、问答或会话流

当前输入分流是显式的：

- `@token` 走 workspace 内文件搜索
- 显式 `/` 命令和强信号输入走动作匹配
- 普通文本优先应用搜索
- 应用候选不成立时，主动作可回退到轻量 RAG 问答

### 6.2 设置链路

主链路：

1. 设置页读取当前配置
2. 各分组维护前端草稿态
3. 保存时通过独立命令写回后端
4. `ConfigStore` 负责 TOML 读写、兼容默认值和原子落盘
5. 需要即时生效的配置同步回运行时状态

关键约束：

- 配置模型集中在 `domain/settings.rs`
- 前端不解释配置文件路径和落盘方式
- 不同分组的草稿不要互相污染
- 配置新增字段必须向后兼容
- `general.autoStart` 不是“只落盘不生效”的静态字段；Rust 侧保存成功后必须继续同步系统登录启动项
- 内置 RAG MCP server 的运行状态和是否写入全局 MCP 清单必须分离：前者来自运行时状态投影，后者仍由设置页草稿显式控制

### 6.3 OCR / 翻译快捷键链路

当前有三类全局快捷键：

- launcher 唤起
- 截图 OCR 回填
- 优先翻译选中文本，否则截图 OCR 并翻译

这条链路跨越窗口、平台能力和远程模型，因此刻意放在 Rust 侧编排。

关键原因：

- 读取选中文本、截图、窗口恢复都属于平台能力
- OCR provider 和翻译 provider 都依赖运行时配置
- 失败时需要统一把错误投影回 launcher

约束：

- 截图流程当前只有 macOS 可用
- 选中文本读取必须留在快捷键处理线程，不可随意丢到 Tokio worker
- 远程 OCR 当前只替换识别器，不代表截图能力已经跨平台

### 6.4 RAG 索引与问答链路

RAG 分成两块：

1. 索引构建与增量维护
2. 运行时问答

索引侧：

- 输入来自设置页里的 source directories、ignore globs 和 embedding provider
- ignore globs 默认预置常见第三方依赖目录和编译产物目录，降低把 `node_modules`、`target`、`dist` 一类噪音文档误入索引的概率；用户仍可在设置页显式覆盖
- `RagIndexService` 维护 LanceDB 向量索引和 SQLite 元数据；文件级索引目标以 embedding fingerprint 表达，而不是只记模型名。fingerprint 先尝试从模型自身的稳定身份推导，例如显式 digest、`/models` 返回项里的 digest/fingerprint hint，或官方托管模型 ID；只有无法稳定确认模型空间时才回退到 endpoint 绑定
- watcher 只监听显式配置目录
- schema 或 embedding fingerprint 变化会触发重建语义
- 运行态单独暴露 `phase/scanned/completed/total/pending` 这组结构化计数，launcher 状态栏直接消费，不靠字符串猜重建进度

问答侧：

- launcher 触发 `rag_answer`
- 服务按 AI 功能页里显式选择的问答 LLM 协议分流到 `responses` 或 `chat/completions`
- 内置工具至少包括 `wabity.rag.query` 和 `wabity.read_file_lines`
- 前端只消费结构化结果、citation 和 action 轨迹
- OpenAI-compatible 的 URL 归一化、错误体提取、`responses`/`chat` 文本提取、SSE 兜底解析和 `/models` 提取统一收口到基础设施层适配模块，避免问答、翻译、OCR、模型列表各自复制一份脆弱解析逻辑

关键设计：

- RAG 不自动把检索结果偷偷塞进 prompt
- 模型必须显式调用工具获取证据
- 续链状态由显式 `conversation_state` 承载，而不是靠前端猜测
- 只有当前 provider + workspace scope 匹配时才允许续链
- `responses stateful` 的继续追问如果因为 provider 预算或累计上下文过大被拒绝，后端会丢弃旧 `response_id`，改用最近历史重试一次，避免长 response chain 直接把问答链路打死

### 6.5 ACP Session 链路

ACP 是独立于 launcher 轻量问答的第二条交互链。

主链路：

1. 前端选择 agent 并创建 session
2. 后端启动本地 `stdio` agent
3. session 生命周期由 `AcpService` 管理
4. 后端通过 Tauri Channel/API 将有序更新推送到前端
5. 前端按 `AcpSessionSummary` / `AcpSessionDetail` 渲染摘要与消息流

边界：

- 当前只支持本地 `stdio`
- session 创建时绑定 workspace
- 全局 MCP server 清单在建会话时透传给 agent
- ACP session 的恢复依赖 agent 自己的 `session/load` 能力，不伪装恢复成功

为什么 ACP 不和 launcher 问答复用一套模型：

- ACP 有独立会话生命周期
- ACP 响应是持续流式、可恢复、带 agent 状态的
- launcher 问答更像一次轻量工具调用链，不需要完整 agent runtime

### 6.6 浏览器 fallback

前端 `src/lib/tauri/client.ts` 内置了浏览器 fallback，用于非桌面端预览和基础 UI 开发。

它的作用只是：

- 允许在浏览器里加载页面
- 提供默认配置和少量 fallback 行为
- 避免前端开发完全依赖桌面 runtime

它不是产品运行架构，也不应反向决定桌面端边界。

## 7. 关键领域模型

当前最重要的模型如下：

| 模型                                      | 作用                                         |
| ----------------------------------------- | -------------------------------------------- |
| `QueryPayload`                            | 描述输入模式、原始文本和来源元数据           |
| `ActionDescriptor` / `ActionMatch`        | 描述可执行动作及匹配结果                     |
| `ExecutionRequest` / `ExecutionResult`    | 描述一次执行请求及结构化返回                 |
| `WorkspaceState`                          | 当前 workspace 与最近目录状态                |
| `AppSettings`                             | 通用、外观、提示词、LLM、OCR、RAG 的聚合配置 |
| `AcpAgentCatalog` / `AcpMcpServerCatalog` | ACP agent 与全局 MCP server 配置目录         |
| `AcpSessionSummary` / `AcpSessionDetail`  | ACP session 摘要与消息流详情                 |
| `RagRuntimeStatus` / `RagScanResult`      | RAG 运行状态、文件级进度与扫描结果           |
| `BuiltinRagMcpServerStatus`               | 内置 RAG MCP server 状态投影                 |
| `PublicSkillCatalog`                      | 公共 skill 目录浏览模型                      |

这些模型的职责是稳定前后端边界，而不是为了省事塞一个弱类型 JSON 大包。

## 8. 设计决策

### 8.1 Rust 持有主调度权

这是桌面应用，不是纯前端页面。窗口、快捷键、OCR、配置、外部进程、索引维护都带平台成本。把主调度留在 Rust，可以避免：

- 前端堆积平台分支
- UI 状态和系统状态相互穿透
- 字符串协议失控

### 8.2 领域模型优先于流程胶水

先建模 `query`、`execution`、`settings`、`acp`、`rag`，再写流程代码。否则所有复杂性都会退化成：

- 大量匿名 JSON
- 命令分支不断扩张
- 前后端对同一字段含义各自理解

### 8.3 Services 尽量保持用例语义

`services` 不是“杂物间”。它应该按能力边界组织：

- 匹配
- 执行
- 文件搜索
- 应用搜索
- OCR
- 翻译
- RAG
- ACP

如果某块逻辑只能靠注释解释，多半说明分层或抽象已经失败。

### 8.4 Workspace 是一等边界

当前很多能力都故意绑定 workspace：

- 文件补全
- RAG 读取白名单
- ACP session 绑定上下文

原因很简单：launcher 不是全盘搜索器。先把“当前上下文内可靠工作”做稳，比做一个无边界系统更有价值。

### 8.5 配置集中持久化

配置统一经 `ConfigStore` 管理，而不是各 feature 各自落盘。

这样做保证：

- TOML 格式兼容演进
- 快捷键、外观、提示词、LLM、OCR、RAG、ACP 配置有单一真相源
- 运行时状态和持久化配置的映射关系明确

### 8.6 构建信息必须注入

应用启动日志会显式记录：

- `WABITY_APP_VERSION`
- `WABITY_BUILD_DATE`

这不是装饰，而是为了排查“当前到底跑的是哪个构建”这种低级问题。

## 9. 当前限制与风险

### 9.1 平台偏置仍然存在

当前应用搜索、截图 OCR、窗口行为都有明显 macOS 偏置。文档必须承认这一点，不能把“未来可扩展”写成“现在已支持”。

### 9.2 AppState 容易膨胀

`AppState` 现在是合理的总装配点，但它天然有继续膨胀成 God object 的风险。新增能力时，优先下沉到：

- 新的领域模型
- 新的 service
- 更收紧的配置或基础设施模块

### 9.3 RAG 与 ACP 都在增加状态复杂度

当前系统已经同时存在：

- 前端局部 UI 状态
- Rust 运行时状态
- ACP session 状态
- RAG conversation state
- 配置持久化状态

如果边界不清楚，最先坏的不是功能，而是状态一致性。

### 9.4 文档漂移风险

`ARCHITECTURE.md` 以前的问题不是遗漏，而是越权。以后继续把 feature 微决策写到这里，结果只会再次失真。

## 10. 文档分工

| 文档                                     | 应写内容                                  | 不该写内容                       |
| ---------------------------------------- | ----------------------------------------- | -------------------------------- |
| `ARCHITECTURE.md`                        | 系统边界、分层、核心链路、设计决策        | 页面微交互、样式常量、实现流水账 |
| `src/features/launcher/README.md`        | launcher 交互结构、组件职责、前端局部约束 | 全局系统架构                     |
| `src/features/settings/README.md`        | 设置页分组、草稿态、保存语义              | Rust 运行时总装配                |
| `src-tauri/src/domain/README.md`         | 领域模型设计与边界                        | UI 细节                          |
| `src-tauri/src/services/README.md`       | 各服务职责与用例边界                      | 页面布局细节                     |
| `src-tauri/src/infrastructure/README.md` | 平台能力、配置存储、窗口约束              | 业务规则细节                     |
| `docs/`                                  | 大型方案、决策过程、阶段进度              | 稳定架构总览的替代品             |

## 11. 当前目录概览

```text
.
├── src
│   ├── app
│   ├── features
│   │   ├── launcher
│   │   └── settings
│   └── lib/tauri
├── src-tauri
│   └── src
│       ├── commands
│       ├── domain
│       ├── infrastructure
│       ├── services
│       └── state
└── docs
```

这个结构本身已经表达了当前架构：前端只保留 UI feature，Rust 侧按边界清晰分层，`docs/` 承接大型方案，而不是把所有信息继续压进一个总文档。
