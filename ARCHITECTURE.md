# Wabity 架构说明

## 1. 文档边界

`ARCHITECTURE.md` 只描述稳定事实：

- 系统是什么
- 代码如何分层
- 核心数据流如何穿过这些层
- 当前明确坚持的设计约束

不应该写进这里的内容：

- 页面排版、按钮层级、CSS token 之类易漂移 UI 细节
- 某次审计、整改或重构的过程记录
- 具体字段文案、临时交互补丁、样式修修补补

这些内容分别下沉到：

- `docs/`：大型方案、关键决策、仍有长期价值的实施记录
- feature `README.md`：局部约束和模块职责

## 2. 产品边界

`Wabity` 是一个基于 `Tauri v2 + React + TypeScript + Rust` 的桌面 launcher。它围绕以下主链路组织：

1. 用全局快捷键唤起 launcher 或历史剪贴板面板
2. 在当前 workspace 中搜索文件、执行本地动作、启动应用或终止进程
3. 在 launcher 内执行轻量 RAG 问答
4. 创建并管理本地 `stdio` ACP agent session
5. 翻译当前选中文本；无选中时回退到截图 OCR 后再翻译
6. 维护少量文本剪贴板历史，并支持回贴到外部应用
7. 在设置页维护快捷键、外观、通知、AI、RAG、ACP agent 和全局 MCP 配置

明确非目标：

- 不做插件市场
- 不做远程 ACP transport 编排平台
- 不做全盘文件索引
- 不让前端接管核心业务调度
- 不把 ACP session 和 launcher 轻量问答混成同一种会话模型

## 3. 系统上下文

总体链路固定为：

`快捷键 / UI 输入 -> 前端状态编排 -> Tauri IPC -> Rust AppState -> services -> infrastructure / 外部系统`

外部系统包括：

- 操作系统窗口、全局快捷键、Dock/任务栏与通知中心
- 系统剪贴板、前台应用切换与跨应用粘贴
- 本地配置文件、workspace 历史与文件系统
- 应用索引、目录监听、运行中进程枚举
- OpenAI 兼容 LLM / OCR / Embedding 服务
- ACP agent 进程
- LanceDB 与 SQLite

## 4. 分层结构

### 4.1 前端

| 路径                    | 职责                                                           | 约束                       |
| ----------------------- | -------------------------------------------------------------- | -------------------------- |
| `src/app`               | 应用壳、视图切换、全局主题应用                                 | 不承载业务规则             |
| `src/features/launcher` | launcher 输入、补全、结果面板、ACP transcript、clipboard panel | 只消费结构化后端结果       |
| `src/features/settings` | 设置页分组 UI、草稿态、字段级校验展示                          | 不直接决定落盘与运行时装配 |
| `src/lib/tauri`         | IPC client、事件订阅、浏览器 fallback                          | 前端与 Rust 的唯一通信边界 |

前端只负责：

- 视图状态
- 表单草稿
- 键盘交互
- 结构化结果展示

前端不负责：

- 直接读写配置文件
- 推断系统能力
- 拼接平台协议
- 复制后端业务规则

### 4.2 Rust 后端

| 路径                           | 职责                                     | 约束                 |
| ------------------------------ | ---------------------------------------- | -------------------- |
| `src-tauri/src/main.rs`        | 薄入口                                   | 只做启动转发         |
| `src-tauri/src/lib.rs`         | 桌面运行入口导出                         | 保持极薄             |
| `src-tauri/src/app.rs`         | 启动顺序、插件注册、窗口初始化           | 只做应用装配         |
| `src-tauri/src/commands`       | IPC 命令边界、参数解析                   | 不沉淀业务逻辑       |
| `src-tauri/src/state`          | `AppState` 总装配与运行时依赖分发        | 不变成巨型业务类     |
| `src-tauri/src/domain`         | 稳定领域模型、配置模型、前后端共享结构   | 不依赖 Tauri UI 细节 |
| `src-tauri/src/services`       | 用例级业务流程                           | 纯逻辑优先           |
| `src-tauri/src/infrastructure` | 系统能力、存储、窗口、快捷键、外部客户端 | 不承载业务语义       |

允许的依赖方向：

`frontend -> tauri client -> commands -> state -> services -> domain / infrastructure`

禁止的依赖方向：

- 前端直接触达配置文件或系统 API
- `domain` 反向依赖 `services`
- `infrastructure` 决定业务分流
- `main.rs` / `app.rs` 堆积业务流程

## 5. 运行时核心对象

### 5.1 AppState

`AppState` 是 Rust 侧的运行时总装配点，不是领域模型，也不是万能 service。

它负责：

- 持有 matcher、executor、file search、application、process、RAG、OCR、ACP 等服务实例
- 持有配置存储和当前 workspace 运行时状态
- 把持久化配置投影成运行时依赖
- 对外暴露统一的查询、执行、设置更新和会话管理入口

它不负责：

- 维护前端展示细节
- 把低层文件格式处理和高层业务规则揉成一团
- 直接塞进长分支流程而不继续下沉到 `services`

### 5.2 持久化边界

当前持久化分为几类：

- 应用配置：`config.toml`
- workspace 历史：`workspace-history.toml`
- 剪贴板历史：`clipboard-history.toml`
- RAG 元数据与词法索引：SQLite
- RAG chunk 与向量：LanceDB
- ACP session 可恢复快照：应用状态存储

原则：

- 配置读写集中在配置模块，不在业务流程里到处拼路径
- 前后端通信只使用结构化模型，不透出底层文件格式
- 运行时状态和持久化配置分离，不能把配置对象直接当运行时真相源

## 6. 核心数据流

### 6.1 Launcher 输入与执行

主链路：

1. 前端采集输入并判断当前模式
2. 通过 IPC 请求匹配、文件搜索、应用搜索、进程搜索或动作执行
3. Rust 按 `QueryPayload` 与输入上下文分流到对应 service
4. service 返回结构化候选或执行结果
5. 前端只负责渲染结构化反馈

显式分流规则长期保持：

- `@token` 走 workspace 文件搜索
- slash 命令走动作匹配
- 普通文本优先保留本地动作/应用搜索机会
- 问答只在显式动作或默认回退条件满足时触发

### 6.2 轻量 RAG 问答

问答链路和 ACP session 分离：

1. launcher 触发 `rag_answer`
2. 后端组装问答上下文、RAG 工具、受限文件读取与 opener 能力
3. 兼容 `responses` / `chat/completions` 调用路径
4. 按需执行本地 function tools 与外部 HTTP/SSE MCP server
5. 返回结构化答案、引用和进度事件

约束：

- 这是轻量多轮问答，不是第二套长期 agent session
- 本地读文件和 opener 都受 workspace / source roots 白名单约束
- 问答运行时可以使用 MCP，但不会把 ACP session 模型强塞进 launcher

### 6.3 ACP Session

ACP 链路是独立运行时：

1. 用户在 launcher 中创建或切换 session
2. Rust 启动本地 `stdio` agent 进程
3. `AcpService` 维护 session 生命周期、事件流和快照恢复
4. 前端按真实时序渲染 transcript

约束：

- ACP session 与轻量问答的状态、模型和输出面完全分开
- transcript 顺序由运行时事件决定，前端不重排成摘要模板
- session 级 mode / model / runtime option 只属于当前 session，不回写全局 AI 设置

### 6.4 设置保存

设置页只维护草稿、字段级错误展示和用户操作。

真实保存链路：

1. 前端提交某个分组的结构化草稿
2. `commands/settings` 做边界转换
3. `AppState` / settings 相关 service 做校验、归一化和落盘
4. 落盘成功后把变更投影到运行时能力

典型投影包括：

- 快捷键重注册
- macOS Dock 展示策略更新
- 自启动状态与系统登录项对账
- OCR / LLM / RAG / ACP / MCP 运行时配置更新

## 7. 启动与平台约束

启动顺序固定为：

1. 初始化 `tracing`
2. 注入构建时版本与日期信息
3. 注册单实例守卫与 Tauri plugins
4. 创建 `AppState`
5. 初始化通知、窗口和快捷键
6. 恢复可恢复 session 与后台任务
7. 启动后默认隐藏 launcher

关键约束：

- `main.rs` 保持薄，只负责启动转发
- 同一用户登录会话内只允许一个原生实例
- launcher 是短时交互窗口，不是长期主工作台
- 历史剪贴板使用独立原生窗口，而不是 launcher 内部视图切换
- 构建产物必须注入版本和日期信息

## 8. 关键设计决策

当前长期成立的设计决策如下：

1. 前端负责展示和输入，Rust 负责业务调度与运行时约束
2. IPC 边界显式建模，不依赖隐式字符串协议
3. ACP session、轻量问答、普通 launcher 执行是三条不同链路
4. 平台能力统一下沉到 `infrastructure`，业务语义统一收敛到 `services`
5. RAG 使用 LanceDB + SQLite 组合，而不是把全部状态塞进单一存储
6. 配置条目先表达“接入点和能力”，运行时用途资格由后端统一投影，不让前端各自猜
7. 内置 MCP server 是统一 loopback endpoint + 模块注册，不伪装成多条普通外部 server

## 9. 文档地图

- 根目录 [README.md](/Users/wweir/Sites/Mine/wabity/README.md): 项目简介、开发命令、打包与发布说明
- [docs/README.md](/Users/wweir/Sites/Mine/wabity/docs/README.md): `docs/` 文档索引与保留规则
- [src/features/settings/README.md](/Users/wweir/Sites/Mine/wabity/src/features/settings/README.md): 设置页局部约束与模块职责
- [src/features/launcher/README.md](/Users/wweir/Sites/Mine/wabity/src/features/launcher/README.md): launcher feature 约束
- [src-tauri/src/services/README.md](/Users/wweir/Sites/Mine/wabity/src-tauri/src/services/README.md): Rust service 层职责
- [src-tauri/src/domain/README.md](/Users/wweir/Sites/Mine/wabity/src-tauri/src/domain/README.md): 领域模型边界
- [src-tauri/src/infrastructure/README.md](/Users/wweir/Sites/Mine/wabity/src-tauri/src/infrastructure/README.md): 基础设施边界
