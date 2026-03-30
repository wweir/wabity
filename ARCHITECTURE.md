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
5. 通过快捷翻译当前选中文本，或在未选中时回退到截图 OCR 后再翻译，并把结果回填到 launcher
6. 通过设置页维护快捷键、通知、外观、AI 功能、LLM、RAG、ACP agent 和全局 MCP 配置

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
- 操作系统默认 opener 与外部应用
- 操作系统通知中心
- 本地配置文件与 workspace 历史
- 本地应用索引、文件系统、目录监听
- ACP agent 进程
- OpenAI 兼容 LLM / OCR / Embedding 服务
- LanceDB 与 SQLite（含 RAG 元数据与 BM25/FTS 词法索引）

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

`launcher` 内部虽然同时承载本地执行结果、轻量 RAG 问答和 ACP session 时间线，但这些输出面都遵守同一个前端展示原则：主答案或主结果永远优先于调试性辅助信息。像 action trail、tool detail、thought 这类过程信息只能作为消息内部的次级 disclosure，不能和主答案并列成独立主面板，也不应占据答案的首个视觉落点。浅色/深色主题都必须复用同一套稳定 token 语义，feature CSS 不应继续保留只适用于单一主题的私有颜料。对于 `chat/completions` 返回的 reasoning，后端必须先把最终答案和 reasoning 分离；如果兼容层把 thinking 混进 `message.content`，后端也必须在归一化阶段拆出 `reasoning`。前端只能把 reasoning 当次级 thought 展示，不能再把它无差别塞进主答案文本。

launcher 进入轻量 RAG 问答结果展示态后，前端会把原生窗口 resize 切到“只增不减”的受限模式，并只在结果落地与后续 resize 稳定期内临时关闭 blur auto-hide 与被动输入框 refocus 链；稳定期结束后会自动恢复正常 blur 行为，而显式退出 QA 展示态则会立即恢复 shrink 与 refocus。这条链路属于窗口稳定性约束，不是普通 UI 动画细节。

macOS 下 launcher 不是普通文档窗口，而是服务于全屏覆盖场景的高层级 `NSPanel`。它会叠加 `nonactivating_panel`、全屏辅助 collection behavior 和 `Status` 窗口层级，以覆盖全屏 Space。这个约束直接服务于全局唤起和短时输入，不允许前端或业务流程把它当成长期主窗口去驱动。

`settings` 里凡是显式提交的分组，都把保存、恢复已保存版本和定位问题动作收敛在主编辑区入口，不再挂在页面头部。像 RAG 这类输入密集分组优先使用紧凑状态/操作条，而不是再叠一张独立大卡去占首屏。

`settings` 里凡是“多条目里选当前编辑对象”的目录，都必须暴露真实互斥选择语义，例如 `radiogroup/radio` 或等价模式；不能继续拿普通按钮状态去冒充单选关系。自定义模型选择器同样必须补齐 `combobox/listbox/option` 语义和键盘流，而不是只保留视觉下拉效果。

`settings` 里的短说明型表单优先靠组件级响应式布局消化横向空间：像 `input`、`select`、`combobox` 这类短字段默认并排“左侧说明 / 右侧控件与帮助”，容器变窄时再回退堆叠；`textarea`、目录列表和长说明块才继续保持纵向展开，避免把本可横向解决的信息继续压成长滚动。这个规则不是一刀切模板，像通用页的通知、外观、OCR 这类线性设置仍应保持单列顺排，避免把依赖关系横向打散。

`settings` 里的配置卡片内部字段列表允许组件级双列，但不是一刀切：RAG、LLM、ACP、MCP 这类编辑卡里的多个短字段可以左右分布，用卡片内部横向空间缩短纵向长度；header、动作区、模型选择器、`textarea`、多行目录和长说明块必须自动跨满整行。像 `AI 功能` 这种按任务组织、依赖渐进展开的任务卡，则继续保持单列任务流，避免被误拉成细长工作台。

`settings` 里的次级说明、用途摘要和安装提示优先平铺为普通区块，不再默认额外包一层渐变次卡；块级跳转也要尽量使用更短标签和更轻的 pill，避免把辅助信息重新堆成首屏噪声。

`settings` 的输入密集型页面默认走单列主流程：摘要、状态、主表单、次级说明和结果区按阅读顺序顺排，不再把能力说明、安装指南、扫描结果这类次级块挂成长期并排侧栏。只有像快捷键这类局部短行说明，才保留组件级双列并在容器变窄时回退堆叠。

`settings` 左侧分组导航只在真正窄到无法稳定容纳双列时，才允许退到主内容上方。像 `720px` 这类仍能维持“左侧分组 / 右侧编辑”主骨架的窗口宽度，必须继续优先让位给当前编辑区，不能让导航先占掉首个可编辑字段的首屏空间。

`settings` 里的摘要卡也必须维持单层信息结构：允许一张摘要卡承载主状态和扁平事实列表，但不允许在摘要卡内部再堆一排同构小卡去伪造层级；主次优先靠排版、留白和分组建立，而不是再加第二层边框容器。

`settings` 里的说明型侧栏块也应保持同一原则：只要风险、规则、建议动作或最近一次操作结果本身就是对“当前配置”的解释，就直接并入对应摘要卡；只有真正独立于当前配置的结果或状态，才保留单独板块。RAG 的最近一次手动重建结果也属于这一类，不再单独挂一个“扫描结果”卡。即便需要解释，也优先用分段、definition list 或短规则列表建立层级，不再把两三条说明拆成并列同构小卡。

`settings` 的主内容区只保留一层 section header：分组标题、简短说明和块级跳转共用同一行或同一区块，不再额外叠一层“当前分组”舞台或独立 jump rail 去消耗首屏。块级跳转只保留真正会离开当前视口的编辑区或结果区，不重复给首屏已可见的摘要块再挂一层导航；如果当前分组实际上只有 1 个可跳目标，或像通用页这样首屏已经能直接建立结构，就直接隐藏 jump，不为了形式完整再留一张无意义跳转卡。

像 `ACP Agent` 这类需要先建草稿再编辑的分组，空状态也必须直接落到可操作主任务上：模板选择、创建空白草稿和后续基础字段属于同一条流程，不允许先堆一张不可提交的保存状态卡或把入口拆成多个互相竞争的空面板。

进入分组内“当前草稿”编辑态后，共享双列规则也必须服从主任务可编辑性：只要当前字段或动作区被压到影响输入和扫读，就应退回单列主表单或整行动作区，不能为了复用统一布局继续把活跃表单挤成窄列。

其中 `AI 功能` 分组按任务边界组织为“翻译配置”和“文档问答配置”两个编辑卡，每张卡同时维护任务级模型路由和系统提示词；`LLM` 分组只维护可复用条目本身，不承载任务默认选择。

长文本型设置默认走渐进展开而不是整页常驻全高输入框。像 `AI 功能` 里的提示词编辑，首屏优先展示任务状态和核心路由，长文本编辑器只在用户明确展开时出现。

`LLM` 分组允许先创建普通 OpenAI-compatible 条目，再在同一张编辑卡里按需套用只读内置供应商模板。

`LLM` 条目的主编辑区遵守“模式先显式、字段后展开”的顺序：模板、配置类型和模型来源要先说明它们各自会改写什么边界，再进入名称、接入点、密钥和模型字段。用途与能力说明只保留约束性信息，不再重复目录区已经可见的身份摘要。版式上要与 `AI 功能`、`RAG` 这些输入密集分组保持同一节奏：状态条之后拆成多段主编辑卡，不允许再退回成单块细长长表单。

内置模板目录属于应用发布物，不写入用户配置；用户最终保存的仍然是普通 `LlmProviderConfig` 条目和本地 API Key。这样可以把“供应商引导元数据”和“用户私有密钥配置”分开，避免升级和持久化边界混淆。

对于从内置模板创建的条目，模型选择不是开放输入，而是受模板白名单目录约束；白名单模型带一句话说明，帮助用户在不理解供应商全量产品线的前提下完成选择。

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
3. 注册 Tauri plugin、系统通知能力和全局快捷键 handler
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
- 透明 launcher 的圆角和外阴影都由前端 CSS 控制；内容根节点必须显式预留透明安全边，否则窗口内容区会把底角阴影裁成直角
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
- `/open` 这类有副作用的本地动作由 Rust 运行时直接调系统 opener 执行，不把平台命令拼接和路径解析下放到前端
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
- 内置 LLM 供应商模板是只读目录数据，不直接替代用户已保存的 LLM 条目；模板更新随应用版本走，真实调用配置仍由用户保存和选择
- `general.autoStart` 不是“只落盘不生效”的静态字段；Rust 侧保存成功后必须继续同步系统登录启动项
- 轻量问答和 ACP prompt 完成后的系统通知同样属于运行时调度，不由前端根据 event 流自行推断；前端只维护触发器开关、内容粒度和系统设置引导，不提供未验证的伪权限按钮
- 内置 RAG MCP server 的运行状态和是否写入全局 MCP 清单必须分离：前者来自运行时状态投影，后者仍由设置页草稿显式控制

### 6.3 OCR / 翻译快捷键链路

当前有两类全局快捷键：

- launcher 唤起
- 优先翻译选中文本，未选中时截图 OCR 并翻译

这条链路跨越窗口、平台能力和远程模型，因此刻意放在 Rust 侧编排。

关键原因：

- 读取选中文本、截图、窗口恢复都属于平台能力
- OCR provider 和翻译 provider 都依赖运行时配置
- 失败时需要统一把错误投影回 launcher
- 当 OCR provider 选中远程多模态模型时，截图文件会在 Rust 侧编码成 data URL，并通过内置 OCR prompt 作为 `responses` 的 `input_text + input_image` 组合发给模型；这条提示词不下放到前端临时拼装
- 翻译链路严格按所选 LLM 条目声明的协议请求对应 endpoint，不再跨协议兜底；运行时会把翻译请求统一视为低复杂度任务，对所有翻译模型都显式注入 `thinking: { type: "disabled" }`，并优先以流式响应消费 SSE，避免兼容层把短文本翻译拖进高延迟路径或整包缓冲

约束：

- `launcher 唤起` 这条热路径现在只负责显隐窗口，不再同步尝试读取外部应用选中文本；读取选区会触发模拟复制与剪贴板变更确认，把它塞进 toggle 会直接拉高快捷键感知延迟
- 截图流程当前只有 macOS 可用
- 选中文本读取必须留在快捷键处理线程，不可随意丢到 Tokio worker
- macOS 下 launcher 不能只依赖 `Focused(false)` 自动隐藏；`NSPanel` 的焦点语义在回答渲染、窗口重排和空闲阶段仍可能出现无用户操作的抖动，继续把 `blur` 当关闭信号只会制造误隐藏。当前实现已放弃 `nonactivating panel`，改为可激活的 borderless `NSPanel` 来保证问答结果落地后的稳定输入，但窗口层仍必须继续用“显示后短暂抑制 + 失焦延迟确认 + 重新获焦取消”的状态机过滤假离焦，而不是继续深入探测 AppKit 当前事件这类高风险运行时细节
- macOS 下 launcher 的 toggle 语义也必须区分“已聚焦可见”和“仅运行时状态仍标记可见但窗口已经掉出前台”；后者再次触发快捷键时应重新置前，不应先走一次无感知的隐藏
- 全局快捷键门闩不能假设 `Released` 一定可靠到达；macOS 在抢焦点或激活 panel 的阶段可能丢失 release。运行时可以继续防抖同一轮按键，但若 release 长时间缺失，必须自动解锁，否则 launcher、OCR 和翻译快捷键都会被永久卡死
- 其他平台若保留失焦自动隐藏，同样必须区分“用户明确切走焦点”和“窗口刚显示时的瞬时焦点抖动”；显示后的短暂稳定窗口内，首个 `blur` 必须直接忽略，稳定窗口结束后的后续 `blur` 也要经过很短的确认延迟，若期间重新获焦则取消隐藏
- 前端自动测量并回写窗口尺寸不属于“用户交互”，这类内部 resize 必须显式标记为 transient window interaction；否则问答结果、OCR 回填或长输出渲染阶段产生的焦点抖动会被窗口层错误收敛成自动隐藏
- 问答结果展示期的原生窗口 auto-resize 不能简单停掉，也不能继续允许自由 shrink；当前做法是保留持续观察，但把窗口同步切到“只增不减”的受限模式。这样回答首屏和后续异步内容仍能把窗口继续撑开，而短时测量回退不会把 `NSPanel` 又缩回去截断内容；离开结果展示态后才恢复正常 shrink
- `transient window interaction` 不是万能兜底；当问答结果真正落地并开始重排消息流时，前端必须额外通过 IPC 重置一小段 launcher blur suppression，并在结果落地与 resize 稳定期内暂停 `window focus`、`visibilitychange`、`onFocusChanged` 这类被动输入框 refocus 链。稳定期结束后，blur auto-hide 与被动 refocus 都要自动恢复；显式退出结果展示态时也要立即恢复。否则窗口层刚把 `NSPanel` 拉回 key，前端又会立刻重新触发 DOM focus，形成新的 `blur/focus` 自激震荡。当前实现不再把 launcher 设成 `nonactivating panel`，因为它会在问答结果展示后持续掉 key 并直接打断输入；同时，结果展示期一旦前端显式关闭 blur auto-hide，窗口层默认不再自动重建 key 焦点或重复抢焦点，只保留 suppression 并等待稳定期结束或用户显式重新聚焦，避免把原生失焦打成反复 `blur/focus` 循环
- 但如果 macOS 原生 panel 在 blur-disabled 阶段已经自己掉出可见层，窗口层仍不能继续假设“没有 `hide()` 日志就代表用户还能看到它”；运行时必须补采 `NSPanel.isVisible / isKeyWindow / occlusionState / NSApp.isActive` 这组原生状态，并只允许做一次无焦点的可见性补偿（例如 `show + orderFrontRegardless`），禁止顺手重新 `activate` 或 `makeKey`
- 远程 OCR 当前只替换识别器，不代表截图能力已经跨平台
- OCR 纯回填和快捷翻译回填都必须把来源模式显式投影回前端：`ocr` 保留 OCR 语义，`selection` 保留外部选中文本语义，不能再一律退化成模糊的 `multiline`

### 6.4 RAG 索引与问答链路

RAG 分成两块：

1. 索引构建与增量维护
2. 运行时问答

索引侧：

- 输入来自设置页里的 source directories、ignore globs 和 embedding provider
- ignore globs 分成“固定内置规则 + 用户追加规则”两层：固定规则默认覆盖 `.git`、`node_modules`、`vendor`、`Pods`、`target`、`dist`、`build`、`out`、`.next`、`.nuxt`、`.svelte-kit`、`.turbo`、`.cache`、`coverage`、`.venv`、`venv`，后端归一化时会强制补回，前端不提供取消入口；用户只能在此基础上继续追加
- `RagIndexService` 维护 LanceDB 向量索引和 SQLite 元数据；SQLite 除了文件级 metadata，还承载 active chunk 的 `FTS5 + bm25()` 词法索引，供运行时混合召回使用。文件级索引目标以 embedding fingerprint 表达，而不是只记模型名。fingerprint 先尝试从模型自身的稳定身份推导，例如显式 digest、`/models` 返回项里的 digest/fingerprint hint，或官方托管模型 ID；只有无法稳定确认模型空间时才回退到 endpoint 绑定
- `rag` 的 Rust 实现已经从单个 `rag.rs` 收口为目录模块：`service` 负责 watcher 生命周期和入口，`indexing` 负责全量/增量计划与 staged/active 切换，`storage` 负责 LanceDB/SQLite，`embedding` 负责 fingerprint 和批处理请求，`chunking` 负责文档切块，`config` / `status` / `model` 负责共享规则、运行态状态和类型边界；后续修改默认沿这条边界落位，不再把不同层职责重新堆回单文件
- `document_extract` 是索引入口前的显式抽取层：纯文本和 Markdown 仍按 UTF-8 文本处理，`docx` 先规范化成 Markdown 风格文本，文本型 `pdf` 先按页抽取成 block，再进入后续 chunk 打包
- Markdown 文档切块不是简单按固定字符窗口切。索引侧会先按标题、列表项、代码块和普通段落做语义预切，再在同一标题路径内按字符预算打包；只有单个语义块本身过大时，才回退到通用 splitter 在块内继续拆分
- PDF chunk 的主定位锚点是 `page_start/page_end`；文本类文件继续保留 `line_start/line_end/paragraph_line_start` 强语义，citation 不再对 PDF 伪造行号
- watcher 只监听显式配置目录；纯 metadata 噪音不会升级成整文件重读或重分片
- 手动全量重建和后台 watcher 增量维护共享同一套存储互斥，避免并发改写同一份 LanceDB / SQLite
- schema、embedding fingerprint 或 extractor fingerprint 变化会触发重建语义
- 运行态单独暴露 `phase/scanned/completed/total/pending` 这组结构化计数，launcher 状态栏直接消费，不靠字符串猜重建进度

问答侧：

- launcher 触发 `rag_answer`
- `AppState` 只负责装配运行时依赖并转调独立的问答后端模块；问答核心通过库导出的稳定函数接口暴露，允许在不启动 Tauri UI 和 `AppState` 的前提下单独做集成测试
- `rag_answer` 本身不再继续膨胀成单文件总控；当前按 `conversation_state`、`result`、`parsing`、`tool_catalog`、`tool_execute`、`protocol_responses`、`protocol_chat` 拆成子模块，根模块只保留共享类型、入口编排和统一回合循环
- 服务按 AI 功能页里显式选择的问答 LLM 协议分流到 `responses` 或 `chat/completions`，不做跨协议 fallback
- 内置工具至少包括 `wabity.rag.query`、`wabity.read_file_lines`、`wabity.read_document_excerpt` 和有副作用的 `wabity.system.open`
- `wabity.system.open` 虽然会随问答工具列表一起注入，但只有当前问题明确要求“打开链接 / 文件 / 目录”时才允许真正执行；本地路径仍只允许落在当前 workspace 根目录和显式配置的 RAG source roots 内
- `wabity.system.open` 的 tool description 不是静态文案；后端会在构建请求时动态拼入当前宿主机的操作系统、版本，以及 PATH 上实际检测到的包管理器列表，减少模型对运行环境的臆测
- `responses` 链路会优先注入全局 HTTP/SSE MCP server；若某个兼容层对工具支持不完整，在带 `type=mcp` 或普通 `function` tools 时首轮返回 5xx，或请求长时间挂起后超时/取消，后端会逐级收缩到“仅内置 function tools”，必要时再收缩到“无工具请求”重试当前轮，并把这次兼容结果缓存在当前进程里，后续同一 provider/model 直接使用已知可用级别
- 前端只消费结构化结果、citation 和 action 轨迹
- citation 必须稳定携带 `document_kind`，并在页码锚点可用时优先展示页码；前端不能继续把所有命中都渲染成“文件行号”
- RAG 索引构建与检索同样通过库导出的稳定函数接口暴露最小测试入口，允许集成测试直接验证“建库 -> 向量/词法混合召回 -> 命中裁剪”的端到端行为，而不需要先启动 `AppState`
- RAG 检索不会再把召回完全绑死在单一路径向量 top-k 上；运行时会并行执行 LanceDB 向量候选和 SQLite `FTS5 + bm25()` 词法候选，按 `absolute_path + chunk_index` 去重合并后，再做轻量 rerank，并结合显式 `min_score`、默认高置信门槛、相对首命中的尾部截断、强实体 query 的锚点词硬过滤，以及标题-only / base64 类低质量 chunk 剔除，只保留高关联候选
- OpenAI-compatible 的 URL 归一化、鉴权注入、请求发送、错误体提取、`responses`/`chat` 文本提取、SSE 流式消费与归并、`/models` 提取统一收口到基础设施层薄 client，避免问答、翻译、OCR、embedding、模型列表各自复制一份脆弱传输逻辑

关键设计：

- RAG 不自动把检索结果偷偷塞进 prompt
- 模型必须显式调用工具获取证据
- 有副作用的 `wabity.system.open` 不属于“证据获取”，不能被模型主动猜测式调用；运行时和系统提示都会重复施加这条限制
- 续链状态由显式 `conversation_state` 承载，而不是靠前端猜测
- 只有当前 provider + workspace scope 匹配时才允许续链
- `responses stateful` 的继续追问如果因为 provider 预算或累计上下文过大被拒绝，后端会丢弃旧 `response_id`，改用最近历史重试一次，避免长 response chain 直接把问答链路打死
- launcher 对这份轻量问答上下文的退出语义必须和窗口显隐解耦：`Esc` 显式收起 launcher 时前端会清空当前问答续链状态，表示“结束这一轮轻量问答”；而 blur auto-hide、打开引用前的临时隐藏、执行结果要求关闭 launcher 或全局快捷键 toggle 隐藏只改变窗口可见性，不得顺手销毁问答上下文，更不能波及 ACP session

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
| `public/`                                | 前端静态资源与共享品牌源图                | 平台打包后的派生二进制图标       |
| `docs/`                                  | 大型方案、决策过程、阶段进度              | 稳定架构总览的替代品             |

## 11. 当前目录概览

```text
.
├── public
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

静态品牌源图当前集中在 `public/wabity.svg`；桌面端打包所需的 `png/icns/ico` 派生图标统一落在 `src-tauri/icons/`，避免前端和打包链各自维护一份不同语义的图标。
