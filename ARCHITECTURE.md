# Wabity 架构说明

## 1. 目标

`Wabity` 是一个基于 `Tauri v2` 的桌面 launcher，同时承载一个轻量 ACP agent 面板。当前范围收敛为下面这条复合主链路：

1. 全局快捷键唤起 launcher
2. 顶部地址栏展示并切换当前 workspace
3. 输入文本，匹配本地动作，或用 `@` 在当前 workspace 内补全文件
4. 需要时创建本地 `stdio` ACP session，并把输入发送给当前 session
5. 用 session 点切换会话、观察后台更新和通知状态
6. 为后续 OCR、历史、配置和系统动作扩展预留稳定边界

“像 Alfred / uTools / Bob” 只是产品灵感，不是可执行需求。当前实现只承认一个现实目标：先把 `workspace -> 输入 -> 本地动作或 ACP session -> 结果` 内核做稳。

## 2. 架构原则

### 2.1 产品范围收紧

- 首版只做 launcher 核心，不做插件市场
- 首版只做本地动作注册，不做远程扩展生态
- ACP 当前只支持本地 `stdio` transport，不做远程连接和多协议混接
- ACP session 在创建时绑定一个 workspace；后续切换全局 workspace 不会漂移旧 session
- 当前不声明 ACP `fs.readTextFile`、`fs.writeTextFile`、`terminal` 等 client capability；如果 agent 仍请求，会记录系统消息并拒绝
- OCR 当前支持两类 provider：macOS 本地 `VNRecognizeTextRequest`，以及引用设置页里 OpenAI 兼容 LLM provider 的远程多模态 OCR；当前打通的是“全局快捷键 -> 交互式截图 -> OCR -> 回填 launcher 输入框 -> 弹出 launcher”闭环，但交互式截图本身仍只在 macOS 下可用
- 系统动作优先做低风险能力，危险能力先占位，不伪装成功能
- 文件搜索范围当前收敛到当前 workspace，不碰全盘索引

### 2.2 技术原则

- 桌面壳使用 `Tauri v2`
- 前端固定使用 `React + TypeScript + Vite`
- 核心业务建模放在 Rust 侧，前端不承载主调度逻辑
- 前后端只通过结构化模型通信，不依赖字符串拼接协议
- 系统能力通过 `infrastructure` 或前端 effect 边界接入，避免 UI 横穿平台细节
- UI 优先极简输入器形态，不做高信息密度面板

## 3. 系统分层

### 3.1 UI 层

职责：

- 渲染 launcher 主窗口
- 维护 workspace 地址栏、session 点、输入内容、候选选中态和结果展示
- 处理键盘导航、候选点击和少量前端副作用
- 对 `open_url`、`copy_text` 这类安全效果做 UI 侧补完
- 当前交互收敛为”顶部 workspace bar + session 点 + 主输入区 + 内联结果/会话流”
- ACP 会话流在前端按“最新在上、历史在下”展示；最新一次用户提交会在流顶部额外弱化回显一份，避免 agent 流式输出增长后当前请求脱离视线
- assistant 消息和 launcher 内联 markdown 预览统一复用共享 `MarkdownRenderer`：基于 `react-markdown` + `rehype-highlight`，并补 GFM、Mermaid、MDX 安全兼容、Obsidian 风格 wiki link / callout / highlight；user 消息保持纯文本 `<pre>` 渲染
- 全局样式基线收敛在 `src/app/global.css`：集中维护字体、颜色、圆角、阴影、glass frame、浮层、按钮、输入框等 design tokens 和共享外观；`launcher.css`、`settings.css` 只保留页面特有布局、状态和局部覆盖，避免在 feature 内继续散落近似常量
- agent 的 tool call、plan update 等操作事件在 assistant 消息内部显示为操作条，保持 ACP 响应原始顺序；tool call / tool result 优先按 ACP `tool_call_id` 融合成单个 pill，避免前端靠标题猜配对；有详情的 pill 默认通过 hover 在 action bar 下方展开大面板展示 `raw_input`/`raw_output`，尽量吃满当前窗口可用宽度，并在展示层尽量解码 `\n`、`\t`、`\uXXXX` 等常见转义，点击作为触屏兜底
- 设置页沿用与主 launcher 相同的面板宽度规格，避免页面切换时窗口宽度突变
- 设置页窗口高度不再随表单内容增长；页面进入设置视图时按当前显示器可用高度收敛到固定上限，超出内容统一交给页内滚动区承接
- 设置页顶栏和 launcher 地址栏一样保留显式拖拽空白区，窗口移动不依赖标题栏原生装饰
- 设置页主体改成一级导航分组：通用、快捷键、外观、LLM、RAG、ACP Agent、MCP、Skills、关于，不再混成单个长表单；OCR 配置并入通用页，ACP 相关配置拆成两个独立一级层级，减少长表单切换成本
- 设置页表单默认优先使用输入框、下拉框、开关等直接控件；除非确实需要承载高密度说明或复杂选择语义，否则不要轻易引入卡片式交互
- 设置页中的 LLM 配置支持维护多个 provider 条目；每个条目显式保存 `baseUrl`、`apiKey`、`model`、协议 `openai_chat` / `openai_responses` / `openai_embedding` 以及 `supportsMultimodal`
- LLM、OCR、RAG 页面都使用草稿态；保存某个分组时只提交该分组和其他分组的已落盘值，避免未保存草稿被跨分组保存意外带入配置文件
- 设置页中的 ACP Agent 配置支持维护多个 agent 条目；页面以编辑表单为主，预设只作为名称和启动命令的下拉填表入口，不再在设置页暴露默认 agent 入口
- 设置页中的 Skills 页面只读扫描 `~/.agents/skills`，展示公共 skill 的 frontmatter meta、目录/文件统计和目录树；它不参与 `config.toml` 持久化，也不读取文件正文
- ACP Agent 页面在“选择 Agent”卡片右侧保留一条极简安装提示，并让右侧安装提示显著宽于左侧选择区；窄屏时回落成上下布局，减少“命令已预填但本机没装”的断层
- ACP Agent 安装卡片除了展示安装命令，还需要补一条一句话介绍和官方链接，降低“知道命令但不知道对应产品和文档入口”的理解成本
- ACP Agent 预设的安装提示必须对齐各 agent 官方安装文档；未在官方文档出现的安装方式不能冒充推荐路径，像 Codex 这类同时提供 `npx` 试运行和长期安装入口的预设，需要在文案里明确区分
- “选择 Agent” 下拉项直接展示 agent 名称、启动命令和已配置状态，避免用户只看名称还要反推会填什么
- “选择 Agent” 下拉框移除空白占位项，默认回落到首项“自定义 · 空白表单”
- “选择 Agent” 左侧采用“选择框 + 辅助文案 + 提交按钮”的单列输入模块，弱化工具栏感，保持表单主视角
- 设置页中的 MCP 配置是全局共享目录，所有 agent 共用同一份 `stdio/http/sse` server 清单；这些配置不会由 Wabity 本地代理执行，而是在 `session/new` / `session/load` 时直接作为 ACP `mcp_servers` 交给选中的 agent
- ACP Agent 和 MCP 页面都改成表单优先：移除顶部独立概览卡，在已配置列表上方用一句短说明解释配置用途，下面直接编辑当前表单；MCP 的 transport 选择收敛为和 ACP 快速填充一致的双栏创建入口，右侧说明卡解释字段和示例
- 设置页中的 ACP Agent 预设不承载默认 agent 选择；真正创建 session 用哪个 agent，由 launcher 顶部 Agent 菜单决定。重复预设不会静默失败，而是明确提示已存在并跳到对应 agent
- ACP 草稿区在详情面板底部固定粘底保存栏，持续显示未保存状态、问题数、放弃入口和保存入口；存在校验错误时优先定位到第一个问题字段
- ACP 草稿在前端先做即时校验：agent 名称、启动命令、MCP 必填字段以及 `KEY=VALUE` 文本格式有误时，不允许触发保存
- 设置页中的通用、外观、LLM、OCR、RAG、快捷键、ACP agent 配置和全局 MCP 清单都写入用户配置目录下的 `wabity/config.toml`；设置页打开时读当前值，修改后立即回写
- OCR provider 表单按 provider 条件显示字段：只有选中 `llm_ocr` 时才展示 LLM provider 选择器，避免把无关凭据输入暴露成常驻噪音
- `config.toml` 反序列化必须对旧版本缺失字段保持向后兼容；新增配置项默认通过 `serde(default)` 回填，不能因为用户本地残留旧配置而阻断启动
- 光标所在的 `@token` 触发文件搜索模式，搜索根目录固定为当前 workspace
- 左上角通过固定文案 `Workspace` 的选择器统一承载“选择新目录”和“最近目录切换”，顺序上先新建后最近
- workspace 选择框、session 列表与补全框一样使用外层绝对定位浮层，避免被 launcher 面板内部裁剪
- 顶部 workspace 地址栏刻意做弱化处理，只保留必要可点击性，不抢主输入区注意力
- 地址栏剩余空白宽度显式作为拖拽区，避免窗口移动和按钮点击区域混在一起
- 地址栏拖拽区依赖 Tauri capability `core:window:allow-start-dragging`，不能只改前端光标或 DOM 属性
- macOS / Linux 下，涉及用户 HOME 的路径统一用 `~` 表达，缩短顶部与会话区路径显示
- launcher 启动时优先恢复上次成功保存的当前 workspace；缺失或无效时才回退到用户 `HOME`
- 最近目录列表只保留最近 3 个有效目录，避免 workspace 选择器无限增长
- 最近目录历史独立写入 `wabity/workspace-history.toml`，避免和主配置文件耦合
- launcher 默认在失焦时自动隐藏，但原生目录选择器打开期间会通过后端运行时状态临时暂停这条策略
- launcher 从全局快捷键、OCR 回填或其他显示路径重新出现时，前端会在窗口重新获得焦点或重新可见后主动把光标聚焦回主输入框；不能只依赖首次挂载时的 `autoFocus`
- launcher 主输入框显式关闭浏览器原生 `autocomplete`、`autocorrect`、`autocapitalize` 和 `spellcheck`，避免系统历史候选或拼写建议和自定义补全浮层叠出双层列表
- 当其他应用的选中文本被注入 launcher 输入框时，前端会显式把光标放到输入开头，并直接切到多行输入模式，而不是沿用浏览器默认的文本尾部选区或继续停留在单行模式
- 主输入框聚焦态收敛为底边提亮和轻微背景过渡，不使用全尺寸粗 outline，避免输入时边框权重突增
- 激活某个 ACP session 后，主输入区切到会话发送模式；未激活 session 时仍走本地 launcher 动作模式
- 配置好 ACP agent 后，底部操作条在“未激活 session”场景下提供显式 `Agent 执行` 按钮；该按钮会自动创建或复用 ACP session，并复用会话面板展示输出；同时绑定 `Alt+Enter` 作为直接触发快捷键
- 底部操作条保留明确主次层级：`Agent 执行` 和主动作按钮作为强化主操作组，但只暴露当前状态真正可执行的操作；主按钮会在 `执行` / `发送` / `插入路径` 之间切换，没有有效目标时禁用，当前 session 的关闭统一收敛到顶部会话区和会话面板
- 输入框下方、按钮左侧定义为左对齐状态栏，默认承载最后一次用户提交内容；长文本不折行，改为在可视窗口内横向滚动，避免继续占高
- launcher 多行输入框按内容自动增高，但会按屏幕可用高度设置上限；超出后切换为输入框内滚动，避免窗口跟着长 prompt 持续增高
- `/format` 作为 JSON pretty format 主命令，`/fmt` 为短别名并保留 `/json` 兼容；输入命中该命令且后续内容是合法 JSON 时，前端会在输入框下方直接渲染带复制按钮和语法高亮的格式化预览，而不必先执行一次动作
- `/md` 作为 Markdown 渲染主命令，`/markdown` 为长别名；输入命中该命令时，前端会在输入框下方直接渲染 Markdown 预览，并安全兼容 Mermaid、MDX 和 Obsidian 风格扩展语法
- `/base64` 作为文本编解码命令，执行时会先尝试把载荷识别为 UTF-8 Base64 文本；命中则解码，否则按普通文本编码；执行结果与其他快捷命令统一走可复制的内联结果卡片
- launcher 内联结果卡和 ACP 消息流统一收敛到固定高度上限，并使用内部滚动承接长输出，避免原生窗口被单次执行结果拉出屏幕
- 顶部会额外显示当前“新建 session 所用”的 agent 选择器；可以并存多个不同 agent 的 session，但每个 session 只绑定一个 agent
- 顶部新建 session 的 `+` 只在已经配置 agent 时显示；未配置时不再保留无意义的禁用按钮
- session 点是轻量会话切换器，而不是完整标签页；状态语义固定为：绿色慢闪=`running`，快闪=有新通知，灰色=已断开，黄色=可恢复错误，红色=不可恢复错误
- 顶部 `会话` 按钮不是纯文案开关，而是会话入口摘要：同时展示当前会话/运行态摘要与总数，降低用户对右上角小圆点语义的记忆负担
- 顶部支持展开全部 session 列表，查看标题、状态、绑定 workspace，并直接切换或关闭指定 session；该列表挂在右上角触发器下方的附着式浮层里，支持外部点击关闭、`Esc` 收起和 `+N` 溢出展开，并通过固定高度上限避免过度拉长 launcher 主体
- `SettingsPage` 和 `SessionTimeline` 都按真实进入时机懒加载：设置页不占 launcher 启动主包；共享 markdown 渲染器进入 launcher 主包后，Mermaid 这类更重的依赖仍按代码块命中时动态导入，避免普通输入也下载整套图表运行时
- 普通动作补全只有在显式 `/` 命令、`http`/`{` 这类强信号输入，或至少输入 2 个英文字符 / 1 个非英文字母后才显示，避免任意非空文本都弹候选
- 文件搜索只有在输入满足 2 个英文字符或 1 个非英文字母（如中文）后才触发，并追加 50ms 防抖，避免过早抖动查询
- 补全提示改为跟随 `textarea` 光标的浮动候选框，查询和补全文本都只基于光标前内容实时刷新，并在候选刷新时默认选中第一项；候选项内容压成更宽的单行布局，动作候选统一显示“主 `/` 命令 + 简短说明”，不再暴露内部评分理由；主动作按钮显式展示当前快捷键，单行输入用 `Enter`、多行输入用 `Ctrl/Cmd+Enter`；slash 动作一旦确认，不再把完整命令文本留在输入框里，而是切到一次性的“待执行 slash 动作”状态：输入框只保留 payload，下一次 `Enter` 或主按钮直接执行该动作；如果确认时已经带 payload，则同一次确认直接执行，并在成功后仍只保留 payload；前缀解析会接受已选动作的最短 slash 前缀并允许 payload 紧跟命令后面，例如 `/uhello` 会按 `/upper hello` 处理，避免 UI 归一化丢失时把命令前缀误当正文；当用户在候选列表里显式选中某个 slash 动作时，执行逻辑以该动作本身为准，不再要求当前输入 token 已经完全匹配它，避免键盘选中 `upper` 后仍把前导 `/` 留进 payload；slash 执行后输入框保留 payload，光标映射到剥离命令前缀后的对应位置，不强制跳到文本末尾；`Esc` 默认隐藏整个补全框；选中项变化时列表自动上下滚动，尽量让高亮项保持在可视区域中线附近；文件补全只替换当前 `@token`，不会覆盖光标前的整段 prompt

当前实现位置：

- `src/app/global.css`
- `src/app/App.tsx`
- `src/features/launcher/LauncherPage.tsx`
- `src/features/launcher/query.ts`
- `src/features/launcher/layout.ts`
- `src/features/launcher/workspace.ts`
- `src/features/launcher/sessions.ts`
- `src/features/launcher/components/LauncherHeader.tsx`
- `src/features/launcher/components/LauncherComposer.tsx`
- `src/features/launcher/components/RestoreNoticeList.tsx`
- `src/features/launcher/components/LauncherFeedback.tsx`
- `src/features/launcher/components/MarkdownRenderer.tsx`
- `src/features/launcher/components/SessionTimeline.tsx`
- `src/features/launcher/components/CompletionPopup.tsx`
- `src/features/launcher/components/SessionPanel.tsx`
- `src/features/launcher/components/WorkspacePickerPanel.tsx`
- `src/features/launcher/components/AgentPickerPanel.tsx`
- `src/lib/tauri/client.ts`

### 3.2 应用编排层

职责：

- 接收前端查询请求
- 接收 workspace 和 ACP session 管理请求
- 普通文本流在达到应用搜索阈值后才调用应用搜索服务生成候选应用；显式 `/` 或 `http`/`{` 强信号才调用动作匹配服务
- `@` 文件流调用文件搜索服务生成候选文件
- 应用启动请求直接调用应用服务，不把“选中的应用对象”塞回 action 协议
- 调用执行服务返回结构化执行结果
- 启动本地 `stdio` ACP agent，维护 session 生命周期，并通过 Tauri Channel API 把后台更新有序推送到前端
- 初始化并热更新 RAG watcher 运行时；前端显式触发重建时暴露独立 `scan_rag_sources` 命令
- 对 ACP `session/update` 和对应 `session/prompt` response，后端基于 SDK 的原始有序 stream 统一收口，再投影到前端消息列表，避免 SDK 回调层并发派发导致的 turn 内乱序
- 在 `session/new` / `session/load` 时，把全局 MCP server 清单直接透传给 ACP agent；如果 agent 初始化返回的 `mcp_capabilities` 不支持 `http` 或 `sse`，则在会话创建前显式失败
- 统一暴露 Tauri command 给前端
- 启动时初始化 `tracing`，日志时间戳显式使用本地时区而不是默认 UTC
- 全局快捷键当前分为两类：普通 launcher 唤起，以及 macOS 交互式截图 OCR 唤起；后者在后台阻塞式完成截图和当前配置的 OCR provider 识别，再回到主窗口展示结果
- 为了让 `rust-analyzer diagnostics src-tauri --severity error` 可用，Tauri IPC 分发和 app context 额外保留一条仅在 `cfg(rust_analyzer)` 下生效的无宏诊断路径；正式构建仍走 Tauri 官方生成宏与 build script 产物

当前实现位置：

- `src-tauri/src/commands/mod.rs`
- `src-tauri/src/state/mod.rs`
- `src-tauri/src/app.rs`

启动约束：

- `setup` 钩子保持同步装配边界，不直接假设当前线程已经进入 Tokio 上下文
- 启动阶段需要等待的异步初始化统一通过 `tauri::async_runtime::block_on(...)` 接入 Tauri runtime，避免用 `tokio::runtime::Handle::current()` 在无 runtime 上下文里触发 panic

### 3.3 领域核心层

核心对象：

- `QueryPayload`：统一描述输入模式、原始文本、文本段和来源元数据
- `ActionDescriptor`：动作元数据，声明标题、简短说明、别名、关键词、输入模式约束、分类和优先级
- `ActionMatch`：动作候选与匹配分数
- `InstalledAppMatch`：已安装应用候选结果，包含应用名、bundle 路径和匹配分数；内存索引记录保持在服务内部，不暴露给前端
- `FileSearchMatch`：文件搜索结果，包含完整路径、文件名、父目录和匹配分数
- `ExecutionRequest`：一次动作执行请求
- `ExecutionResult`：执行状态、展示文本、结构化副作用提示、后续动作建议
- `WorkspaceState`：当前 workspace 根目录与最近目录
- `GeneralSettings` / `AppearanceSettings` / `LlmProviderConfig` / `LlmSettings` / `OcrSettings` / `RagSettings` / `RagScanResult` / `AppSettings`：设置页通用/外观/LLM/OCR/RAG 配置模型
- `AcpAgentConfig` / `AcpAgentCatalog`：本地 ACP agent 的预设目录；单个 agent 既支持直接 `program + args`，也支持 `shellCommand`；catalog 里仍保留默认 agent 字段，作为后端兼容和兜底
- `AcpMcpServerConfig` / `AcpMcpServerCatalog`：全局 MCP server 目录，覆盖 `stdio/http/sse` 三类 transport，以及 `args`、`env`、`headers` 等连接参数
- `AcpSessionSummary` / `AcpSessionDetail`：ACP session 的状态投影与消息流，摘要里显式携带 `errorLevel`、绑定的 `agentName`
- `AcpMessageBlock`：ACP 消息块，支持 `thought`、`actions`、`content` 三种类型，每条消息按时间线顺序存储多个块
- `AcpActionEvent`：assistant `actions` 块里的操作事件；除 `kind`、`title`、`detail` 外，还携带可选 `correlation_id`，当前主要用于 tool call / tool result 配对展示
- `AcpSessionMessage`：单条 ACP 消息，包含 `id`、`role`、`blocks`（消息块列表）和 `pending` 状态
- `PublicSkillCatalog` / `PublicSkillEntry` / `SkillTreeNode`：公共 skill 浏览模型，覆盖 `.agents/skills` 根目录、单个 skill 的 meta 信息、目录/文件统计和目录树
- `OcrProvider`：OCR provider 抽象；当前请求模型显式支持 `image_path + focus_point`，结果模型返回完整文本、文本块 bounding box，以及可选的命中块

当前实现位置：

- `src-tauri/src/domain/query.rs`
- `src-tauri/src/domain/actions.rs`
- `src-tauri/src/domain/application.rs`
- `src-tauri/src/domain/file_search.rs`
- `src-tauri/src/domain/execution.rs`
- `src-tauri/src/domain/workspace.rs`
- `src-tauri/src/domain/acp.rs`
- `src-tauri/src/domain/skills.rs`
- `src-tauri/src/services/ocr.rs`

### 3.4 基础设施层

职责：

- 全局快捷键注册
- 平台对应的 launcher 唤起快捷键
- 主窗口显示、隐藏、聚焦和失焦自动隐藏
- 目录选择器等原生瞬时交互期间，临时抑制失焦自动隐藏，避免 launcher 被系统对话框误收起
- 透明窗口与 CSS 伪异形圆角配合
- workspace / ACP agent / 全局 MCP 配置持久化
- 设置页通用/外观/LLM/OCR/RAG 配置持久化
- 本地目录选择器
- 公共 skill 目录扫描与 `SKILL.md` frontmatter 解析
- 后续承接截图、存储、配置和其他桌面能力

当前实现位置：

- `src-tauri/src/infrastructure/hotkey.rs`
- `src-tauri/src/infrastructure/config.rs`
- `src-tauri/src/infrastructure/window.rs`
- `src-tauri/tauri.conf.json`

公共 skill 约束：

- 只读扫描用户主目录下的 `~/.agents/skills`
- 每个子目录视为一个公共 skill 条目；若存在 `SKILL.md`，则解析 YAML frontmatter 的 `name`、`description`、`argument-hint`、`license` 和 `metadata`
- 目录树只返回名称、相对路径和层级，不把文件内容透传给前端

配置约束：

- 配置文件固定写入 `dirs::config_dir()/wabity/config.toml`
- workspace 最近目录历史单独写入 `dirs::config_dir()/wabity/workspace-history.toml`
- `AppState::new` 启动时先经由 `ConfigStore::load()` 读取配置，再初始化 workspace 与其他运行时状态
- 当前已进入持久化的设置包括：通用、外观、LLM provider 目录、OCR、快捷键、workspace、ACP agent、全局 MCP、ACP session 恢复快照
- 配置写入统一通过 `safe_write` 走“同目录临时文件 + rename”原子替换，避免部分写入留下坏文件
- macOS 下凡是命中 `NSPanel` 的运行时窗口操作，都必须通过 Tauri `run_on_main_thread` 派发；后台 OCR 任务回到 launcher 时也不例外

## 4. 已实现的关键子系统

### 4.1 输入模型

当前输入模型已经显式分型，支持：

- `inline`
- `multiline`
- `ocr`
- `clipboard`
- `selection`

当前领域模型仍保留 `inline` 与 `multiline` 等输入模式；当前 UI 默认先以 `inline` 启动，检测到多行文本、用户显式插入换行，或外部应用选中文本注入时，切到 `multiline`。其余模式已进入领域模型，但尚未接入真实来源。

### 4.1.1 快捷键设置约束

- 设置页录制快捷键时，纯修饰键不落盘；只有“修饰键 + 实际按键”或单个实际按键才会形成可保存值
- Rust 侧在保存前再次解析和校验快捷键字符串，不信任前端输入
- 当前支持两类全局快捷键：`toggle_launcher` 和 `ocr_capture`
- 默认快捷键当前为：`toggle_launcher = Cmd+Shift+Space`、`ocr_capture = Cmd+Shift+O`（非 macOS 分别对应 `Ctrl+Shift+Space`、`Ctrl+Shift+O`）
- 启动读取配置时，如果发现 `ocr_capture` 仍等于历史默认值，会自动迁移到新默认值；自定义值不改
- 配置更新成功后会立即注销旧全局快捷键并注册新快捷键；如果新快捷键注册失败或配置写入失败，则回滚到旧快捷键，避免出现“设置已保存但运行时无效”的假状态

### 4.2 动作系统

当前 `/` 动作列表只暴露已可执行命令：

- `/open` -> `open_url`
- `/upper` -> `uppercase_text`
- `/title` -> `title_case_text`
- `/lower` -> `lowercase_text`
- `/camel` -> `camel_case_text`
- `/snake` -> `snake_case_text`
- `/words` -> `word_count`
- `/lines` -> `line_count`
- `/trim` -> `trim_whitespace`
- `/unique` -> `unique_lines`
- `/sort` -> `sort_lines`
- `/format` / `/fmt` / `/json` -> `json_pretty_print`
- `/md` / `/markdown` -> `markdown_render`
- `/base64` -> `base64_text`

未接通的高风险或高波动能力当前不进入 `/` 列表，避免把“占位”伪装成可用命令。

### 4.3 文件搜索

当前文件搜索链路如下：

1. 光标所在 token 命中合法 `@` 前缀
2. 前端切换到文件搜索模式
3. 前端先判断输入是否达到触发阈值：至少 2 个英文字符，或至少 1 个非英文字母（如中文）
4. 达到阈值后等待 50ms，再发起搜索，避免输入过程中频繁请求
5. Rust 侧根据当前操作系统选择搜索后端：macOS 优先查询 workspace 内的 Spotlight 文件索引，其他平台直接走本地兜底索引
6. 如果原生搜索后端不可用、失败或没有结果，则回退到当前 workspace 的一次性索引
7. 兜底索引用 `ignore` 做目录遍历与常见噪音目录过滤
8. 候选结果统一再用 `skim` 底层采用的 `fuzzy-matcher` 对完整路径和文件名做模糊匹配，并额外偏置文件名命中
9. 用户选择文件后，只替换当前 `@token`，保留前后文

当前没有做全盘索引，也没有做实时文件系统监听。这是刻意收缩，不是遗漏。

### 4.4 应用搜索与启动

当前 launcher 的最小应用链路如下：

1. 用户输入普通文本，且当前不在 `@` 文件模式，也没有 `/`、`http`、`{` 这类动作强信号
2. 前端切到应用搜索模式，并请求后端应用服务
3. Rust 侧在启动后后台预热应用索引，当前只扫描 macOS 的 `/Applications`、`/System/Applications` 和 `~/Applications`
4. 索引只收录 `.app` bundle，命中 bundle 后直接跳过其内部目录，避免把应用包内容误扫进索引
5. 索引快照常驻内存，查询阶段只对当前内存快照做排序和匹配，不再同步碰文件系统或 `mdls`
6. 候选排序优先看应用显示名 fuzzy 匹配，并保留 bundle 目录名作为别名参与匹配；因此像 `WeChat.app -> 微信` 这类本地化应用同时支持中英文检索
7. 后台每 5 分钟刷新一次应用索引；launcher 打开或查询时如果发现快照过旧，会异步补一次刷新，但当前查询仍然继续使用旧快照
8. 用户确认候选后，后端通过 `open <bundle-path>` 启动应用，并返回标准 `ExecutionResult`

当前刻意没有做：

- Windows Start Menu / Linux `.desktop` 发现逻辑
- 应用图标读取
- 最近使用排序和使用频率学习
- 全盘 Spotlight/LaunchServices 混合索引

### 4.5 OCR

当前 OCR 链路已经具备 provider 可切换的第一版实现：

1. 全局 `ocr_capture` 快捷键触发交互式截图
2. 截图落到临时文件，不走剪贴板
3. Rust 侧根据配置选择 provider：`system` 走 macOS `Vision`，`llm_ocr` 通过设置页里选中的 OpenAI 兼容多模态 provider 调用 `chat/completions`
4. `system` provider 仍返回全文、逐块 bounding box、平均置信度，以及可选的 `focus_point` 命中块；远程多模态 provider 当前只稳定返回全文
5. 成功时把识别文本作为 launcher 输入内容注入，并自动弹出 launcher
6. 失败或无文本时，也会弹出 launcher 并给出明确错误提示，而不是静默吞掉
7. 设置页允许保存多个 OpenAI 风格 LLM provider、默认 provider，以及 OCR 引用的 provider；LLM 表单先输入 `baseUrl + apiKey`，再通过 `/models` 拉候选模型，下拉选择后仍保留手填未列出模型的入口，最后再声明协议类型和多模态支持。保存时会校验所选 OCR provider 是否存在、协议为 `openai_chat` 且声明支持多模态，启动时遇到坏配置则把 provider 降级成明确不可用状态

当前刻意没有做：

- 非 macOS 交互式截图
- 截图后的图像持久化历史
- 前端显式传入 `focus_point`
- OCR 文本块的 UI 可视化高亮

### 4.6 RAG

当前 RAG 子系统实现了第一版本地索引闭环：

1. 设置页单独提供 RAG 分组，保存扫描目录、忽略 glob 和 embedding provider 引用
2. RAG 只接受协议为 `openai_embedding` 的 provider，沿用统一的 `baseUrl`、`apiKey` 和 `model`
3. `scan_rag_sources` 命令可以显式触发全量重建，便于用户在设置页立即验证配置
4. 应用启动和设置保存都会把当前 RAG 配置下发到 `RagIndexService`
5. `RagIndexService` 会先做一次全量扫描，再为每个选中目录注册递归 watcher
6. 当前只向量化后缀为 `.md`、`.mdx`、`.txt`、`.markdown`、`.rst`、`.adoc` 的文件；文件还必须是可读 UTF-8 文本、不含 NUL 字节、大小不超过 50 MB，且不命中 ignore glob
7. 设置页里的扫描目录示例和目录选择默认位置固定收敛为 `~/Documents`，不再写死开发机路径
8. 文本切分统一基于 `text-splitter`：Markdown 类文件使用 `MarkdownSplitter`，其余文本文件使用 `TextSplitter`
9. 向量数据写入内嵌 LanceDB，并保存 `source_root`、`absolute_path`、`relative_path`、`chunk_index` 和原始文本等元数据
10. 文件变更时先删除旧向量，再写入新向量；目录级变更则退回全量重建，避免局部状态漂移

当前刻意没有做：

- PDF、Office、图片等非 UTF-8 文本抽取
- 查询链路和召回排序；当前先把索引构建与维护边界做稳
- 多 embedding provider 混合索引

### 4.7 ACP session

当前 ACP 子系统实现了第一版最小闭环：

1. 只支持本地 `stdio` agent
2. 通过 ACP Rust SDK 建立 client-side connection
3. 启动时发送 `initialize`
4. 基于当前 workspace 创建 `newSession`
5. 新建 session 时显式绑定所选 agent；激活 session 后通过 `prompt` 发送输入
6. 通过 ACP SDK 原始有序 stream 观察 `session/update` 与 `session/prompt` response，再投影为前端消息列表
7. `prompt` 提交后前端会立即看到”用户消息 + pending assistant + running 状态”，不再等第一段流式返回才更新
8. session 更新通过 Tauri Channel API 推送，保证前后端传输顺序；同时后端不会直接信任 SDK `session_notification()` 回调时序，而是用原始 stream 保证同一 turn 内的 chunk / action / finished 边界顺序
9. 支持 `cancel` 和关闭 session
10. 后台 session 收到更新时，对应通知点进入快闪通知态；运行中 session 维持绿色慢闪
11. 本地持久化当前 live session 的最小快照，启动时优先尝试通过 agent 的 `session/load` 恢复
12. 恢复快照时继续使用创建该 session 的 agent 信息，不受设置页预设或后端默认 agent 兜底值变更影响

当前刻意不做：

- ACP 文件工具能力
- ACP 终端能力
- 远程 transport
- session 持久化恢复
- 不支持恢复时的“假 session”；恢复失败只显示明确提示，不伪装成已恢复
- 一个 session 同时挂多个 agent

恢复策略：

- 只恢复我们自己保存过的 live session 快照
- 恢复能力完全依赖 agent 自身是否支持 `session/load`
- agent 不支持、session 已失效、workspace 无效或 agent 启动失败时，前端显示恢复失败提示
- 对明确不支持或已失效的快照会从本地持久化中移除，避免每次启动重复报同一条废提示

### 4.8 匹配与排序

当前普通动作匹配服务实现了第一版轻量规则：

1. 输入模式过滤
2. 前端先做补全显示阈值过滤：显式 `/` 命令、`http`/`{` 强信号输入，或至少 2 个英文字符 / 1 个非英文字母
3. 标题前缀命中
4. 标题包含命中
5. 别名和关键词命中
6. 特定输入加权（如 URL、JSON、多行文本）

文件搜索会先走平台优先级更高的原生索引能力；当前 macOS 优先 Spotlight，失败或无结果时再回退到 `skim` 底层采用的 `fuzzy-matcher` 排序的本地索引结果。

### 4.9 执行链路

当前执行链路采用两段式：

1. Rust 返回结构化 `ExecutionResult`
2. 前端根据 `structuredPayload.effect` 执行安全副作用

这样做的原因很直接：

- `open_url` 更适合在前端通过 `opener` 插件收尾
- `copy_text` 更适合由前端剪贴板能力收尾
- 危险能力暂不开放，避免过早引入 shell 等高成本能力

### 4.10 窗口形态

当前窗口策略是：

- 尽量使用透明窗口承载圆角 launcher
- 如果平台不支持真正的窗口圆角，则退化为透明窗口 + CSS 伪异形白色面板
- 圆角、阴影和高斯感主要由前端样式承担，而不是赌平台原生一致性
- launcher 主页面尺寸由前端内部页面内容实时回写；`setSize` 直接设置窗口内容区尺寸，不额外补偿 macOS 圆角或外沿
- launcher 的尺寸测量不再只盯着圆角面板，而是统一读取整页可见内容盒；这样附着式补全面板等浮层也能把真实边界回写给原生窗口；设置页单独使用固定高度上限策略，不复用这套内容驱动高度逻辑
- 页面壳层保留极小透明安全边，尺寸测量使用完整包围盒而不是只看右下角，避免负 margin 输入区或透明圆角边缘被窗口裁切
- 原生 resize 链路收口到 Rust 命令；macOS 上若主窗口已转换成 `NSPanel`，则直接调用 `setContentSize`，不再假设通用 `WebviewWindow.set_size` 一定能同步到 panel
- 原生窗口阴影关闭，避免 macOS 透明窗口在视觉上额外凸出一圈
- 不再依赖 `windowEffects` 提供原生背景和圆角层，窗口可见边界完全由前端面板控制，避免原生效果层与网页面板叠出双层边界
- macOS 下显式启用 `macOSPrivateApi` 维持透明窗口能力，但不再把原生圆角当成主视觉来源；代价仍是失去 App Store 上架兼容性
- macOS 下 launcher 不再依赖普通 `NSWindow` 的层级补丁，而是在启动时把主窗口转换成 `NSPanel`；面板样式使用 `nonactivating panel`，并显式设置 `can_join_all_spaces + full_screen_auxiliary + stationary` 与高层级，目标是稳定显示在全屏应用所在的 Space 之上
- macOS 下 `NSPanel` 的显示、隐藏、置前和尺寸调整统一走主线程派发，避免 Tokio worker 线程直接触发 AppKit 的线程断言崩溃
- 全局快捷键切换按“Pressed 只响应第一次，Released 才重新解锁”的状态机处理，不再靠时间去抖碰运气
- 主输入框允许横向突破容器内边距，保证文本输入区域与窗口外框视觉对齐
- 窗口不再依赖固定宽高；前端测量输入区、辅助信息、候选列表和当前页面根节点后，把结果实时回写给原生窗口，多行输入则按内容自动扩高
- 窗口 resize 只改尺寸，不再在每次内容变化后重新 `center`；否则页面一更新窗口就抖动或跳位
- launcher 不再使用原生窗口 `center`；每次显示前都会优先按当前鼠标所在显示器选择目标屏幕，拿不到时再退回窗口当前所在屏或主屏，并基于该显示器 `work_area` 计算默认位置：水平方向居中，垂直方向把窗口中心放在可用高度的 `0.382` 处；这样比机械正中更靠上，也能避开 Dock / 菜单栏保留区
- 但前端启动后首轮内容测量会立刻回写真实窗口宽高；为了避免初始宽度变化把默认横向居中打偏，后端会在 launcher 第一次真正显示前，跟随这些首轮 resize 重新计算默认位置；一旦用户见到窗口，就不再借 resize 抢回位置

这不是完美原生窗口，只是当前跨平台成本下更可控的折中。

### 4.11 OCR 边界

当前 OCR 只完成：

- `OcrProvider` trait
- `OcrResult` 结果模型
- `MacOsVisionOcrProvider` 本地实现
- `OpenAiCompatibleOcrProvider` 远程实现
- 运行时按配置选择 provider
- 截图 OCR 结果回填主输入流

当前未完成：

- 非 macOS 交互式截图
- 图片缓存
- 前端 `focus_point` 命中交互
- OCR 文本块高亮与历史

## 5. 当前目录结构

当前仓库已落地的目录结构如下：

```text
wabity/
├── ARCHITECTURE.md
├── docs/
├── IMPLEMENTATION_PLAN.md
├── README.md
├── .gitignore
├── src/
│   ├── app/
│   ├── components/
│   ├── features/
│   │   └── launcher/
│   └── lib/
└── src-tauri/
    ├── capabilities/
    ├── tauri.conf.json
    ├── tauri.macos.conf.json
    ├── src/
    │   ├── app.rs
    │   ├── commands/
    │   ├── domain/
    │   ├── infrastructure/
    │   ├── services/
    │   ├── state/
    │   ├── lib.rs
    │   └── main.rs
    └── Cargo.toml
```

仓库卫生约束：

- 根目录 `.gitignore` 统一忽略本地构建产物与工具状态，例如 `node_modules/`、`dist/`、`src-tauri/target/`
- 本地 agent 配置目录 `.agents/`、`.claude/`、`.qwen/` 不进入版本库；这些目录属于个人运行环境，不是产品源码
- `.vscode/` 默认不提交，只保留 `extensions.json` 这类对团队协作有稳定价值的最小共享配置

## 6. 当前设计决策

### 6.1 为什么主链路先做 workspace + 本地动作 + ACP session

因为这是当前最容易验证架构边界的三条低风险入口。你如果一开始就冲截图、OCR、ACP 工具、shell 和应用启动，只会同时引入权限、平台差异和安全面，等于主动抬高失败概率。

### 6.2 为什么文件搜索走“workspace 作用域 + 原生索引优先，`ignore + skim matcher` 兜底”

因为目标不是发明模糊搜索算法，而是尽快得到足够可用的 workspace 级文件搜索：

- macOS 上 Spotlight 已经维护了系统级文件索引，在 workspace 范围内优先复用更便宜
- `ignore` 负责遍历与过滤，作为跨平台稳定兜底
- `skim` 的匹配算法由 `fuzzy-matcher` crate 提供，能直接嵌进当前 Rust 服务里，不必把 TUI 交互层带进来

### 6.3 为什么 ACP 先只做 `stdio` 和无工具 capability

因为当前产品定义是“轻量 agent 面板”，不是完整 IDE 宿主：

- `stdio` transport 最容易验证生命周期、日志和回收语义
- 不声明文件和终端 capability，可以先把 session、消息流和通知模型做稳
- 一旦工具能力开放，权限模型、workspace 边界和副作用回收会立刻复杂一个数量级

这条组合比手写评分器更稳，也比一开始就为每个平台分别维护全量搜索后端更收敛。

### 6.3 为什么系统动作只有一部分真实可用

`open_url` 的收尾成本低、边界清楚，适合作为显式 `/` 动作暴露；`copy_text` 只保留为前端副作用和结果复制的内部能力，不再单独暴露 `/copy`。`open_app` 和 `run_shell` 则不是。当前实现直接不把后两者放进 `/` 列表，避免为了“看起来功能很多”把安全和维护成本一起引爆。

### 6.4 为什么 OCR 要保持 provider 边界

因为 OCR 是高波动子系统。先把 provider 边界和结果模型立住，后续接入本地 OCR、云 OCR 或图像预处理时才不会把执行链路一起拖垮。现在把远程 OCR 收敛到“设置页管理 OpenAI 兼容 provider + OCR 只引用 provider id”这条边界后，也证明了这个判断是对的：协议、凭据和模型选择都留在配置层，截图链路本身不用知道第三方是谁。

### 6.5 为什么 macOS 打包配置单独拆到 `tauri.macos.conf.json`

因为 `dmg` 是 macOS 专属产物，不应该直接写进跨平台默认配置里：

- 默认 `src-tauri/tauri.conf.json` 继续承载通用 Tauri 配置
- 通用配置里的 `productName` 固定为 `Wabity`，统一约束打包产物名称首字母大写，例如 `Wabity.dmg`、`Wabity.exe`
- `src-tauri/tauri.macos.conf.json` 只覆盖 macOS 的 bundle 目标，显式收口为 `app + dmg`
- 这样做能把平台差异限制在配置边界内，而不是把其他平台构建路径也绑到苹果产物语义上

### 6.6 为什么 ACP 事件传输使用 Channel API 而非 emit/listen

因为 Tauri `app.emit()` + 前端 `listen()` 模式存在 IPC 层乱序风险：

- `emit()` 是广播模式，不保证同一 session 的多个事件按发送顺序到达前端
- 前端虽然可以用时间戳、消息数、权重等启发式规则缓解乱序，但无法从根本上解决问题
- Channel API 是 Tauri 2 专为流式有序数据设计的传输层，保证消息顺序
- Channel API 无需额外 HTTP 服务器（对比 SSE 方案），直接复用 Tauri IPC 层
- Channel API 提供编译期类型安全，避免手动序列化/反序列化错误

## 7. 当前风险点

- `Tauri v2` 的透明窗口、快捷键和窗口行为仍需真实手测验证
- 输入法兼容、多显示器、高 DPI 还没有开始验证
- 用户目录首次建索引可能有明显延迟
- OCR 已有可用能力，但远程 provider 仍依赖外部网络、第三方额度、模型的多模态真实能力以及用户配置正确性
- 历史排序、配置持久化、错误日志持久化尚未接入
- macOS `dmg` 目前只验证本地 unsigned 打包链路；签名、公证和 stapling 仍属于后续发布流程问题

## 8. 当前阶段产物

当前仓库已经完成第一批可运行骨架：

- `Tauri v2 + React + TypeScript` 项目初始化
- 全局快捷键切换主窗口
- 窗口失焦自动隐藏
- 透明窗口 + 圆角 launcher 退化方案
- 单输入框、少量控制按钮的极简 launcher UI
- `@` 用户目录模糊文件搜索
- Rust 领域模型、匹配服务、文件搜索服务、执行服务和 OCR provider 抽象/实现
- Rust 单元测试与构建基线

下一阶段应该继续推进：

- 非 macOS 截图流程
- 历史排序
- 更多可执行系统动作
