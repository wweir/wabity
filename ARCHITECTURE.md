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
- macOS 截图采集、ScreenCaptureKit backend 与 Screen Recording 权限
- ACP agent 进程
- USearch 索引文件与 SQLite

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

结果展示进一步约束为：

- 翻译和问答都只能消费后端明确拆好的主内容与次级 thought；当前后端若返回 `ExecutionResult.structured_payload.kind = translation_result` 且附带 `reasoning`，前端只能把它渲染成折叠 disclosure，不能兜底替代 `primaryText`

前端不负责：

- 直接读写配置文件
- 推断系统能力
- 拼接平台协议
- 复制后端业务规则

补充约束：

- 根级 `src/` 不保留没有实际内容的预留目录；空的 `components`、`assets` 一类目录应删除，而不是提前占位
- feature 内部优先直接依赖 `src/lib/tauri/client/*` 职责子模块；`src/lib/tauri/client.ts` 只保留少量稳定公共入口，不作为默认的大一统导出层
- 不新增 `shared`、`common`、`utils` 这类语义空洞的根级目录；跨 feature 复用未形成稳定边界前，代码先留在所属 feature 内
- launcher 页面入口只保留页面级装配和事件分发；纯展示推导放在 `launcherPageModel.ts`，RAG 运行状态订阅和候选/剪贴板选择副作用分别放在专用 hook，样式入口只负责按稳定顺序导入 `styles/` 子文件
- settings 样式入口只负责导入 `styles/` 子文件；具体 frame、section、LLM 和响应式规则按职责拆分，避免继续把整个设置页 cascade 堆在单个 CSS 文件

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

补充约束：

- OpenAI-compatible 传输与 payload 兼容逻辑沉到 workspace crate `src-tauri/crates/openai-compatible`
- 该 crate 内部固定按 `client`、`parsing`、`streaming`、`extract`、`models` 分层；宿主 `src/infrastructure/openai_compatible.rs` 只保留兼容导出与本地类型转换
- RAG storage 使用 `storage/` 目录承接持久化边界；`mod.rs` 保留 chunk/vector store 和恢复编排，`metadata.rs` 承接 `rag_files`、FTS lexical index 和相关 SQLite 投影逻辑

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
- RAG chunk 元数据、文本与暂存向量缓存：SQLite
- RAG ANN 索引文件：USearch
- ACP session 可恢复快照：应用状态存储

原则：

- 配置读写集中在配置模块，不在业务流程里到处拼路径
- 前后端通信只使用结构化模型，不透出底层文件格式
- active chunk 的 ANN 向量长期驻留在 USearch；SQLite 只在 staged 写入、复用命中或恢复窗口内短暂持有 `vector_blob`
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

### 6.4 截图 OCR 与 Review

`Alt+D` 的稳定入口语义保持为“优先翻译当前应用选中文本；没有选中文本时再进入截图 OCR”。无选中文本时当前进入 Screenshot Review 确认边界；截图 backend 主路径是 Wabity 自己的透明 overlay 收集 region，再交给 macOS `ScreenCaptureKit` 保存截图，不再调用 shell `screencapture`。

无选中文本时的数据流为：

1. 隐藏当前 Wabity 快捷窗口，避免把 launcher / clipboard panel 截进图片。
2. 通过 `infrastructure/screen_capture` 打开透明 overlay 采集用户确认的 region；Rust 用 ScreenCaptureKit 拿到截图图像，再用本地 ImageIO 写入 Wabity 专用临时目录。取消选择会结束本次流程并恢复此前隐藏的快捷窗口。
3. 通过当前 OCR provider 识别文本和 blocks；本地 provider 仍是 macOS Vision，远程 provider 仍是 OpenAI-compatible `responses` 多模态 OCR。若 provider 是 `llm_ocr`，截图会在 Review 出现前发送给远程 OCR provider，设置页和 Review UI 必须明确提示。
4. 前端展示 Screenshot Review UI，显示截图预览、OCR blocks、可编辑文本和确认动作。OCR blocks 用来快速生成可编辑文本：默认全选并填入 textarea；用户切换 blocks 时同步覆盖 textarea；一旦用户手动编辑 textarea，blocks 退化为参考，不再隐式覆盖，除非用户显式点击“填入选中”。
5. 用户确认后才进入翻译或复制；取消、重试和空 OCR 都不能被当成空文本翻译。重新截图会先结束旧 review state，新的截图成功后再打开新 session；用户取消系统截图时回到普通 launcher。未来 vision prompt 必须走独立显式入口。

职责边界：

- `infrastructure/screen_capture` 只封装平台截图能力、权限错误和 capture metadata，不执行 OCR，不决定翻译。
- `services/ocr` 只识别给定图片中的文字，不再长期拥有截图交互逻辑。
- `services/screenshot_review` 属于用例编排层：它负责 screenshot -> review session -> OCR -> review -> 用户确认动作，并拥有临时文件生命周期；当前只处理翻译和复制。OCR 失败不是 fatal event，必须保留截图预览并允许重试。
- 前端 review state 是独立交互态；普通 launcher suggestions、动作匹配和问答状态不能在 review 打开时继续抢输入。

当前明确不做高级 OCR、屏幕解析和 vision prompt：不引入 RapidOCR、PaddleOCR、OmniParser、UI-TARS 或自动桌面 agent；OCR blocks 和 confidence 只能辅助用户确认，不能让程序自动猜测应该点哪里或发送什么。未来 `/img` / `/vision` 这类“图片 + prompt”能力必须走显式用户动作，不能和 `Alt+D` 自动翻译路径混在一起，也不能因为 OCR 为空自动发送整图。

### 6.5 设置保存

设置页只维护草稿、字段级错误展示和用户操作。

真实保存链路：

1. 前端提交某个分组的结构化草稿
2. `commands/settings` 做边界转换
3. `AppState` / settings 相关 service 先做跨分组引用归一化，再做校验和落盘
4. 落盘成功后把变更投影到运行时能力

补充约束：

- 设置页可以在具体分组内展示保存状态和操作入口，但保存语义必须绑定到该分组的结构化草稿提交，不能上提到窗口头部或跨分组入口
- 模型接入分组保存时，允许沿用当前已落盘的翻译 / 问答 / OCR / RAG 模型引用；但 Rust 侧必须先按最新 `providers/models` 目录修复或清空这些引用，再执行跨分组校验，不能先拿旧引用直接判错

典型投影包括：

- 快捷键重注册
- macOS Dock 展示策略更新
- 自启动状态与系统登录项对账
- OCR / LLM / RAG / ACP / MCP 运行时配置更新
- 截图 backend 与 Screen Recording 权限状态变化后的运行时反馈

其中 LLM 配置明确拆成两层：

- provider 层：`base_url`、`api_key`、`protocol`
- model 层：`id`、`model_type`、`model`、`model_identity_hint`、`builtin_preset_model_id`、`supports_multimodal`、`supports_stateful`

约束：

- provider 层只表达“连到哪里、用什么协议”，不再混入具体模型能力
- provider 是分组容器，一个 provider 下可以维护多个 `models[]`
- model 层才是最小可引用单元；翻译、OCR、RAG 问答和 RAG embedding 都只保存 `modelId`
- 设置页编辑顺序固定为“先选 provider 组，再在同一条主流程中编辑 provider 层连接信息和当前 model 层能力”；界面可以合并展示，但不能把两层数据边界混写
- 模型接入页的前端结构必须保留 provider 目录与当前编辑表单的边界；宽窗口可用双栏 master-detail，窄窗口回退单列；有条目时不显示顶部 quick jump，页头之后直接进入双栏工作区。右侧编辑区外层卡片必须保留稳定内边距，状态条和分段标题不能贴到或越过外框。组内模型列表使用纵向单选目录展示模型名、调用方式和可用功能，并支持方向键切换；新增和删除动作不能混进模型目录项。模型名远端候选使用可手填 combobox + listbox 语义，候选面板在字段内占位并由列表自身滚动，筛选/刷新动作不能混进 option 列表语义。设置页初始化 LLM 草稿时必须结合内置模板目录清理无效 `builtin_preset_model_id`，Rust 保存链路也必须在校验前做同样归一化
- 配置读取只接受当前两层结构和 `*ModelId` 引用；旧版平铺模型字段、`*ProviderId` 路由字段和 `responsesModel + embeddingModel` 拆分逻辑已移除

### 6.6 Launcher 固定窗口

Launcher 固定状态是会话级运行态，只保存在 `ShortcutRuntimeState`，不写入 `config.toml`。它只改变失焦自动隐藏判断：`launcher_pinned = true` 时窗口层跳过 blur auto-hide，并保留 macOS panel 可见性补偿；`Esc`、`Alt+Space`、关闭按钮、执行结果要求关闭窗口等显式隐藏路径仍然生效。

前端入口必须保持上下文相关：

- 标准 launcher 空态不展示固定按钮
- 下方交互区出现结果、问答、预览或 ACP session 时展示固定按钮
- 设置页 header 在关闭按钮旁展示同一个固定按钮
- 所有入口控制同一个 `launcherPinned` 状态，不区分 launcher 和设置页

### 6.7 RAG 文档摄取与状态反馈

RAG 建索引固定分两层：

1. `document_extract` 先把原文件归一化成可分块文本和结构锚点
2. `rag/indexing` 再做切块、向量复用、embedding 和元数据持久化

当前 PDF 约束：

- 只支持文本型 PDF，不做 OCR
- 使用 `lopdf` 按页抽取，但不是“整页成功/失败”二值语义；页内 text chunk 允许部分失败，保留可读片段并把失败原因记成 warning
- 抽取后会经过轻量文本质量闸门，明显控制字符污染或可疑乱码页不会进入 embedding
- warning 会沿 `RagRuntimeStatus` 和手动重建结果向上暴露，前端可见最近若干条，而不是只剩一个笼统的“跳过文件”

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
- `bun run tauri dev` 必须经由仓库脚本包装；开发态默认写入 `src-tauri/target`，并通过仓库脚本按需清理 `debug/deps`、`debug/incremental` 等旧缓存，避免清理口径分散
- macOS 发布包分两条链路：本地构建走 `src-tauri/tauri.macos.conf.json` + `scripts/patch-bundle-dmg.mjs` 的 Finder 美化 DMG 链路；CI 构建退化为 `.app` + `hdiutil create` 的简化 DMG，避免无 GUI runner 上的 Finder 自动化随机失败
- `application` 和 `process` 缓存不在启动时预热，也没有固定轮询刷新；首次命中时同步建快照，后续只在过旧时异步补刷新，避免把常驻扫描成本摊到空闲态

## 8. 关键设计决策

当前长期成立的设计决策如下：

1. 前端负责展示和输入，Rust 负责业务调度与运行时约束
2. IPC 边界显式建模，不依赖隐式字符串协议
3. ACP session、轻量问答、普通 launcher 执行是三条不同链路
4. 平台能力统一下沉到 `infrastructure`，业务语义统一收敛到 `services`
5. RAG 使用 USearch + SQLite 组合：SQLite 作为真相源，USearch 只负责 ANN 检索
6. 配置条目先表达“接入点和能力”，运行时用途资格由后端统一投影，不让前端各自猜
7. 内置 MCP server 是统一 loopback endpoint + 模块注册，不伪装成多条普通外部 server
8. 常驻索引和缓存默认优先收紧内存占用，再考虑额外吞吐；预热轮询、重复字符串和大批次中间态都不是默认选项
9. PDF 摄取优先保留可验证的可读文本，再决定是否索引；“抽到了非空字符串”不等于“可用于 embedding 的文本”
10. 截图 OCR 主路径是 ScreenCaptureKit region capture 并由本地编码层落盘 + 用户确认边界；截图落盘由本地 ImageIO 完成，高级 OCR、vision prompt 或屏幕解析仍必须作为后续显式入口处理；没有 review UI 的自动推断不是可接受的主路径

## 9. 文档地图

- 根目录 [README.md](/Users/wweir/Sites/Mine/wabity/README.md): 项目简介、开发命令、打包与发布说明
- [docs/README.md](/Users/wweir/Sites/Mine/wabity/docs/README.md): `docs/` 文档索引与保留规则
- [docs/screencapturekit-ocr-workflow-design-2026-05-14.md](/Users/wweir/Sites/Mine/wabity/docs/screencapturekit-ocr-workflow-design-2026-05-14.md): Screenshot Review、ScreenCaptureKit 截图 backend 与后续 vision prompt 边界设计
- [src/features/settings/README.md](/Users/wweir/Sites/Mine/wabity/src/features/settings/README.md): 设置页局部约束与模块职责
- [src/features/launcher/README.md](/Users/wweir/Sites/Mine/wabity/src/features/launcher/README.md): launcher feature 约束
- [src-tauri/src/services/README.md](/Users/wweir/Sites/Mine/wabity/src-tauri/src/services/README.md): Rust service 层职责
- [src-tauri/src/domain/README.md](/Users/wweir/Sites/Mine/wabity/src-tauri/src/domain/README.md): 领域模型边界
- [src-tauri/src/infrastructure/README.md](/Users/wweir/Sites/Mine/wabity/src-tauri/src/infrastructure/README.md): 基础设施边界
