# launcher feature

职责：

- 管理 launcher 主窗口 UI
- 协调 workspace 地址栏、session 点、输入内容、候选动作和执行结果
- 处理键盘导航与前端副作用（打开 URL、复制文本）
- `LauncherPage.tsx` 只保留状态编排、命令调用和事件处理；纯函数和展示块拆到同目录模块与 `components/`

当前代码组织：

- `LauncherPage.tsx`：页面级状态、effect、键盘流和 Tauri 命令编排；顶部栏、输入区、反馈区都只负责装配组件，不再内联大段 JSX
- `launcher.css`：只保留 launcher 特有布局、状态和消息流样式；按钮、输入框、浮层和冷静中性色 design tokens 等共享外观基线统一回收到 `src/app/global.css`
- ACP 会话时间线里的 `user / assistant / system` 消息卡片底色必须基于全局 token 组合，禁止在 `launcher.css` 里直接写死只适合浅色主题的消息背景
- `MarkdownRenderer` 衍生出来的 Mermaid 状态文本、错误文案和 highlight.js 语法色同样必须走全局 code token；浅色和深色都不能继续保留私有 code palette 或只适合浅底块的 hex 色
- session panel、restore notice、Markdown 辅助元素和轻量问答元信息区都必须复用全局 surface / text / status token；不要再靠 feature 私有的乳白半透明面和浅描边硬编码制造层级
- ACP 输出区的阅读优先级固定为“答案正文 > 操作轨迹 > 思考过程 > 角色元信息”；assistant message 内的 block 顺序必须先渲染正文，再渲染操作轨迹，最后才是 thought disclosure；tool detail 只作为消息内部次级展开区，不能继续做成比答案更抢眼的大黑面板；`chat/completions` 返回的 reasoning 也必须沿用同一原则，不能再直接冒充正文；兼容层若把 thinking 混进 `message.content`，也必须先在后端归一化拆出次级 thought，再交给前端
- `Agent` 行只保留身份标识；thought 入口降到正文后的消息元信息区，保持紧凑次级，不得继续占据 assistant 卡片的首个视觉落点；assistant 正文的字号和前景权重必须显著高于 thought / action 元信息
- `actionCatalog.ts`：集中维护 launcher/browser fallback 共用的动作描述符和 slash alias，避免页面层与 fallback 重复声明动作元数据
- 透明窗口外沿只保留极小安全边给圆角裁切；launcher frame 的真实底色、提亮边和 overlay 必须基于全局 surface token 推导，不能再硬编码浅色玻璃层覆盖暗色主题
- 顶部 picker、workspace crumb、session toggle 和底部动作按钮都属于自定义按钮外观，必须先移除浏览器原生 `appearance`，否则 WebKit 会把深色 token 冲回浅色系统按钮
- focus ring、selected/hover/disabled 状态都必须走语义 token；不要在局部按钮上继续靠统一 `opacity` 或白色高光兜暗色可读性
- `query.ts`：输入分类、`@token` 解析、补全文本和搜索阈值判断
- `layout.ts`：输入测量、宽度约束和补全浮层定位基础工具
- `workspace.ts`：workspace 路径格式化和面包屑构建
- `sessions.ts`：session 摘要合并、状态文案和 dot class 计算
- `useFloatingPanel.ts`：浮层附着定位和外部点击关闭的共享 hook，避免 `LauncherPage` 重复堆叠近似 effect
- `components/SessionTimeline.tsx`：ACP 消息流与 answer-first 的 action trail
- `components/LauncherHeader.tsx`：顶部 workspace bar、agent 选择器、session 摘要按钮和 session dot 带
- `components/LauncherComposer.tsx`：主输入区和底部操作条
- `components/RestoreNoticeList.tsx` / `components/LauncherFeedback.tsx`：恢复提示、内联结果卡片、JSON / Markdown 预览和 session 状态反馈
- `components/MarkdownRenderer.tsx`：共享 markdown 渲染管线，统一处理 GFM、Mermaid、MDX 安全兼容和 Obsidian 风格扩展
- `MarkdownRenderer.tsx` 里的 `remark-mdx` / `rehype-highlight` 改为异步加载，避免把整套 MDX 和语法高亮依赖静态塞进同一个懒加载 chunk
- `components/CompletionPopup.tsx` / `SessionPanel.tsx` / `WorkspacePickerPanel.tsx` / `AgentPickerPanel.tsx`：launcher 外层浮层组件
- `components/SessionTimeline.tsx`：仅在激活 ACP session 后才懒加载；会话 markdown 仍复用共享渲染器，但 mermaid 等较重依赖继续按需动态导入；action trail 只作为正文后的弱辅助区，不再保留独立标题栏或 hover 即抢焦点的详情面板

当前 UI 决策：

- 顶部增加 workspace 面包屑地址栏和 session 点带
- 左上角增加固定文案 `Workspace` 的选择器按钮，展开后顶部优先提供“选择新的工作目录”，其后再列最近目录
- workspace 选择框、session 列表和补全框都挂在外层浮层，不受圆角面板内部裁剪限制
- workspace 地址栏故意压低字号、底色和间距，降低存在感，避免抢主输入区注意力
- 地址栏末尾的空白区显式作为窗口拖拽带，允许直接拖动 launcher 位置
- 该拖拽带走 Tauri `startDragging()`，并依赖 capability `core:window:allow-start-dragging`
- macOS / Linux 下，涉及 HOME 的路径统一用 `~` 缩写
- launcher 启动时优先恢复上次保存的当前 workspace；无效时回退到用户 `HOME`，最近目录只保留 3 个
- launcher 外观跟随设置页的外观配置：启动时和设置保存后都会同步应用 `theme` / `fontSize`
- launcher 默认失焦自动隐藏，但打开原生目录选择器时会临时抑制自动隐藏，避免页面看起来“闪退”
- 问答结果落地不是普通文本更新；`LauncherPage.tsx` 在写入 QA message 前必须显式调用 `armLauncherBlurAutoHideSuppression(...)`，并在结果初次落地与后续 resize 稳定期内临时关闭 blur auto-hide，同时暂停 `window focus`、`visibilitychange`、`onFocusChanged` 这类被动 refocus 链；稳定期结束后这两条保护要自动恢复，显式退出 QA 展示态时也要立即恢复，不能再把“等待下一次用户输入”当成唯一恢复路径，否则失焦隐藏会被长期关死。macOS 窗口层已经放弃 `nonactivating panel`，改成“可激活、只抢 key 不争 main”的 floating panel；同时结果展示期如果仍发生原生失焦，窗口层默认不会再自动抢回 key 焦点，而是只保留 suppression，等待用户显式重新聚焦；只有在原生 panel 已经掉出可见层时，窗口层才允许做一次不抢焦点的 `show/orderFrontRegardless` 补偿，避免结果面看起来像“自动隐藏”
- QA 结果展示期不能简单粗暴地把原生 auto-resize 全关掉；当前策略是继续保留尺寸观察，但进入 QA 后切到“只增不减”的窗口同步。这样首屏回答、懒加载的 Markdown / 语法高亮 / citation 仍能把窗口继续撑开，而短时测量抖动不会把窗口又缩回去截断下半截内容；离开 QA 展示态后才恢复正常的可增可减 resize
- launcher 通过快捷键、OCR 回填、快捷翻译结果回填或其他显示路径重新出现时，主输入框会主动恢复焦点，不能只依赖首次挂载时的 `autoFocus`
- `Alt+Space` 现在只负责显示或隐藏 launcher；这条热路径不再同步读取外部应用选中文本，否则显隐会被模拟复制和剪贴板轮询拖慢
- 纯 OCR 回填会通过独立事件把识别文本写回主输入框，并把输入模式显式标成 `ocr`；外部选中文本仍保留 `selection` 模式，不能再统一退化成 `multiline`
- `Alt+D` 快捷翻译在拿到原文后会先立即弹出 launcher，并把原文注入输入框；前端进入独立 pending 状态、临时抑制应用/动作建议，等后台 LLM 返回后再把译文渲染到结果卡
- launcher 主输入框显式关闭浏览器原生 `autocomplete`、`autocorrect`、`autocapitalize` 和 `spellcheck`，避免系统历史候选或拼写建议和自定义补全浮层叠出双层列表
- 通过快捷翻译链路注入的外部选中文本，主输入框会把光标显式定位到文本开头，并自动切到 textarea 形态，避免默认落在末尾或继续停留在单行输入
- 主输入框聚焦态只保留柔和的底边提亮和浅背景过渡，不再叠全尺寸粗 outline，避免输入时视觉重心突然跳变
- 主输入框必须带稳定的程序化名称；单行模式下补全关系按 `combobox + listbox + option` 暴露，active option 通过 `aria-activedescendant` 跟随选中项
- 多行输入框按内容自动增高，但会基于屏幕可用高度收敛到固定上限；超出部分交给输入框内滚动，避免 Tauri 窗口被长文本继续撑高
- 主输入区在“本地 launcher 模式”和“ACP session 模式”之间切换
- 配置好 ACP agent 后，底部操作条会提供 `Agent 执行` 入口；仅在未激活 session 时显示，点击会自动创建或复用 ACP session，并在下方会话面板展示 agent 输出；该入口额外绑定 `Alt+Enter`
- 顶部支持为“下一次新建 session”选择当前 agent；同一时刻可以并存多个不同 agent 的 session
- 结果改为内联反馈；ACP 激活时下方改为消息流面板
- 内联结果卡和 ACP 消息流都带固定高度上限与内部滚动，不允许单次长输出把 launcher 主窗口顶出屏幕
- `/format`、`/fmt`、`/json` 的预览、`/md` / `/markdown` 的 markdown 预览，以及 `/base64` 等快捷命令执行结果统一落在内联结果卡片；结果卡片自带复制按钮，JSON 走语法高亮，Markdown 走共享渲染器
- 文本处理 slash 命令当前内建 `/upper`、`/title`、`/lower`、`/camel`、`/snake`、`/trim`、`/unique`、`/sort`、`/words`、`/lines`，以及走默认 LLM provider 的 `/translate`、`/fy`、`/tr`；其中 `/title` 会把每个词的首字母转为大写，`/unique` 和 `/sort` 都按行处理，翻译默认在未指定目标语言时按“简中->英文、英文->简中、其他语言->简中”处理，并保留原文风格与格式
- 底部操作条只保留当前状态真正可执行的动作：未激活 session 时主链路顺序固定为 `设置 -> 翻译 -> Agent 执行 -> 主按钮`；主按钮的文案、色调、禁用态和 Enter 行为共用同一套状态机：普通场景默认显示 `执行`，RAG 回退显示 `问答`，session 内显示 `发送`，文件 token 选中候选后显示 `插入路径`；显式 slash 动作优先显示动作本名（例如 `翻译`、`转大写`），`@token` 尚无候选时显示 `搜索路径` 且保持禁用。`补全` 只作为尾部辅助按钮出现；当前 session 的关闭统一收敛到顶部会话区和会话面板，不再在底部重复放一个“关闭会话”
- ACP `Agent` 按钮和主按钮共用同一套会话状态，但不能复用“候选搜索加载”这类辅助态；补全查询、普通动作执行、Agent prompt 发送必须分别建模，否则按钮文案和禁用态会被串错
- 底部 `设置` 入口继续保持 icon-only 次级按钮，但图标语义改成三滑杆调参 glyph，而不是密集的实心齿轮；这种小尺寸下的识别度更高，也更贴近“配置当前行为”的语义
- 动作条在深色主题下不再额外包一层浅底胶囊容器；主按钮、翻译按钮和设置按钮直接用语义 token 区分强弱状态，避免“按钮上再贴按钮”的贴纸感
- 输入框下方、按钮左侧定义为左对齐状态栏；空闲时默认展示最近一次本地提交的内容，但当 RAG 后台索引、模型请求或 Agent 长任务在运行时，状态栏会切到 `运行状态` 并优先展示对应进度文案；RAG 重建态除了阶段外，还要显示当前 `completed/total` 文件进度，避免只剩“正在扫描 / 剩余 N 个”这种无法判断进度的弱提示；ACP / 问答态的会话标题、agent、workspace 和错误也统一显示在这里，不再在输出区底部重复补一条状态线；单条长文本自动横向滚动，多条状态项则在状态栏内部纵向滚动
- `/format` 成为 JSON 格式化主命令，`/fmt` 为短别名，保留 `/json` 兼容；当输入命中该命令且后续内容是合法 JSON 时，输入框下方直接显示 pretty format 预览
- `/md` 成为 Markdown 渲染主命令，`/markdown` 为长别名；命中后输入框下方直接显示 Markdown 预览，支持 GFM、Mermaid、MDX 安全兼容，以及 Obsidian 风格 `[[wiki link]]`、`> [!note]` callout、`==highlight==`
- `/base64` 作为文本编解码命令；执行时会自动尝试把载荷识别为 UTF-8 Base64 文本，命中则解码，否则编码；当前与其他纯文本 slash 动作一样，支持 `inline`、`multiline`、`ocr`、`clipboard`、`selection`
- 顶部 `会话` 按钮同时承担入口和状态摘要：显示当前会话/运行态摘要与总数，点击展开全部 session 列表
- session 面板改为挂在右上角触发按钮下方的附着式浮层，支持外部点击关闭、`Esc` 收起、`+N` 溢出按钮展开全部列表，并通过固定高度上限避免把 launcher 主体继续拉长
- 展开的 session 列表会显示当前会话标签、状态胶囊和绑定 workspace，并支持直接切换或关闭指定 session
- session 列表和状态栏都会显式展示该 session 绑定的 agent，避免把“顶部选择器当前值”和“会话实际 agent”混为一谈
- 顶部新建 session 的 `+` 只在已经配置 agent 时显示；未配置时不再放一个纯禁用按钮制造噪音
- 启动时会读取后端恢复提示，并以内联提示块展示哪些 session 没能通过 agent 恢复
- 候选项收敛为附着式建议列表
- 本地 launcher 模式下，输入分三类：`@token` 走 workspace 文件搜索，显式 `/` 或 `http`/`{` 强信号走动作候选，其余文本只有在达到应用搜索阈值后才走应用搜索
- 应用搜索当前只面向已安装桌面应用；前端只负责展示候选和触发启动，不自己拼本地索引
- 移除输入框下方的常驻 workspace 描述，只保留必要的错误或执行反馈
- 补全提示改为跟随输入光标的浮动候选框，位置和内容都基于光标前文本实时刷新，默认高亮第一项；动作候选固定显示“主 `/` 命令 + 简短说明”，应用候选显示应用名和 bundle 路径；主动作按钮显式展示快捷键：单行和多行输入都用 `Enter`，多行模式换行改为 `Ctrl/Cmd+Enter`；候选框可见时 `Enter` 默认执行当前模式的主动作，`Esc` 默认隐藏整个补全框；slash 动作确认后不会再把完整命令文本留在输入框里，而是进入一次性的待执行状态：输入框只保留 payload，下一次 `Enter` 或主按钮直接执行；如果确认时已经带 payload，则该次确认直接执行，并在成功后仍只保留 payload；已选 slash 动作还会接受最短命令前缀和紧贴 payload 的写法，例如 `/uhello` 会按 `/upper hello` 处理；通过上下键显式选中某个 slash 动作后，执行也会以该动作为准，不会把前导 `/` 残留进 payload；slash 执行后会保留 payload 内原来的光标逻辑位置，不会强制跳到末尾；高亮项变化时列表会自动滚动，尽量保持高亮项居中
- `/` 候选列表只保留已可执行命令，不把未接通的占位能力混进来制造噪音
- 使用透明窗口 + CSS 圆角伪异形，尽量模拟原生圆角窗口
- 原生窗口尺寸直接由内部页面内容尺寸实时驱动；`setSize` 设置的是窗口内容区，不需要再额外补偿 macOS 圆角或外沿
- 尺寸同步改为以整页可见内容盒为准，不只看圆角面板本身；绝对定位的补全面板、设置页等页面切换也会参与测量，避免窗口和页面边界错位
- 页面根节点需要为圆角和 frame 外阴影一起预留透明安全边，尤其是底部和左右两侧；否则透明窗口会把 CSS 阴影裁成直角
- 原生窗口 resize 不再由前端直接调用 `WebviewWindow.setSize`；统一走 Rust 命令，macOS 下显式写入 `NSPanel.setContentSize`
- 禁用原生窗口阴影，避免 macOS 透明窗口在视觉上比内部页面多出一圈外沿
- 不再依赖 `windowEffects` 的原生背景和圆角层，窗口外观完全由前端圆角面板控制，避免 macOS 原生效果层和网页面板叠出双层边界
- macOS 下仍启用 `macOSPrivateApi` 维持透明窗口能力，但不再把原生圆角当成窗口外观来源；这仍然放弃 App Store 兼容性
- 主输入框横向与窗口外框对齐，不再受内容容器水平内边距二次收缩
- 窗口宽高由内容实时测量驱动：多行输入自动增高，外框宽度在可用范围内随实际内容收缩或扩展；前端测量只观察 shell 和直接浮层子节点，避免全树扫描布局盒造成的持续重排；隐藏期间和每次重新显示后的短暂稳定窗口内，后端仍会按默认规则纠正首轮定位，稳定后才停止借 resize 抢位置，避免页面一变窗口就持续跳位
- launcher 窄窗口下优先保住输入区和主按钮：frame 最小宽度降到 `440px`，顶部 workspace/session 区和底部动作条允许折行，所有主按钮触达尺寸保持 `44px`
- 光标所在 `@token` 切换到文件搜索模式；文件查询达到 2 个英文字符或 1 个非英文字母（如中文）后，再延迟 50ms 发起搜索
- 文件搜索根目录固定为当前 workspace，不再默认为用户目录
- 文件补全只替换当前 `@token`，不会覆盖光标前其他 prompt 内容
- 普通文本应用搜索当前只做 macOS `.app` bundle 查找；应用查询至少需要 2 个英文字符或 1 个非英文字母，并额外追加 80ms 防抖，避免任意单字符输入就触发搜索；候选展示名优先取系统本地化名称，bundle 目录名仍作为中英文双向检索别名；不做图标、不做最近使用排序、不做跨平台兜底
- 应用搜索的后端索引会在启动后后台预热，并维持常驻内存快照；前端查询命中的永远是当前快照，不等待刷新任务
- session 点只承担轻量切换和通知，不承担完整标签页语义
- session 光点语义固定为：绿色慢闪=`running`，快闪=有新通知，灰色=会话断开，黄色=可恢复错误，红色=不可恢复错误
- `prompt` 发出后，前端立即插入用户消息和 pending assistant 占位，避免会话看起来“没反应”
- ACP 会话流展示改为最新 turn 在上、历史 turn 在下；时间线按 message 边界渲染，同一条 assistant turn 内的 `thought`、`actions`、正文只作为该消息内部块显示，不再先拆成多条独立 item
- 时间线上相邻的 `thought` 块会在前端合并显示，避免 agent 连续推送 reasoning chunk 时被拆成多个折叠块制造视觉噪音
- 时间线中的“思考”折叠必须使用真实按钮并暴露展开态，不能再用 click-only `div`
- thought 展开内容使用面向阅读的普通排版，并更接近辅助注释而不是引用块/日志摘录；折叠入口必须给出简短预览，避免只剩一个无信息量的小标签
- thought 开关放在正文与 action 之后的消息元信息区；只有展开后的 thought 内容才进入该元信息区下方堆叠，避免把辅助信息抬到答案前面
- assistant action bar 会把带相同 `correlationId` 的 `tool-call` / `tool-update` 融合成一个 tool pill；详情默认只在显式点击后于 action bar 内作为次级展开区展示，并在展示层尽量解码常见转义文本；hover 只保留轻量提示，不再直接展开详情
- thought block 默认折叠，但用户手动展开后，在同一条消息继续流式追加时必须尽量保留展开状态；不要每次增量更新就把用户已打开的内容重新折回去
- session 更新合并以 `lastUpdatedAtMs` 和消息权重单调收敛，避免旧快照覆盖异步事件流
- session 恢复完全依赖 agent 自身能力；agent 不支持 `session/load` 时，只提示，不伪装恢复成功
- 全局快捷键默认使用 `Alt+Space` 唤起 launcher，`Alt+R` 做截图 OCR 回填，`Alt+D` 会先翻译当前应用选中文本；若没有选中内容，再回退到截图 OCR 并翻译

约束：

- 领域匹配与执行逻辑以后端 Rust 命令为准
- 前端只做展示状态和安全的 UI 侧效果补全

当前已实现的 RAG 问答：

- RAG 问答采用双入口：显式 slash 动作 `/ask` / `/qa` / `/docs` 可以强制进入；普通文本模式下，如果应用搜索没有弹出补全框，则默认主动作回退到 RAG 问答
- 只要应用搜索存在可见候选，主动作仍保持应用启动优先级；不要把所有普通自然语言都无条件送进问答
- launcher 会在本地同时保留最近几轮问答的 user/assistant 文本，以及一份显式 `conversationState`：其中包含上一轮 `responses` 的 `response_id`、续链 scope、累计 citation、累计 action 轨迹和累计工具摘要。后端只在 scope 与当前 provider + workspace 一致时继续沿用它；当当前问答协议是 `responses stateful` 时，继续追问优先把 `previous_response_id` 交给后端，只发送当前问题；如果首轮续问被 provider 以预算或上下文过大拒绝，后端会自动丢弃旧 `response_id`，改用最近历史重试一次；`responses stateless` 和 `chat/completions` 则统一回退到显式回传最近历史。这仍只是轻量多轮上下文，不是 ACP session
- `Esc` 在收起 launcher 本体时会显式清空这份轻量问答上下文；其它隐藏路径例如 blur auto-hide、打开引用前的临时隐藏、执行结果要求关闭 launcher 或全局快捷键 toggle 隐藏都保留当前问答上下文，避免把“临时收起窗口”和“主动结束本轮问答”混成同一个动作
- 轻量问答的结构化 payload 除了 `conversationState`、citation、action 和 tool 摘要外，还允许带一段可选 `reasoning`；只有在 provider 同时给出明确正文和 reasoning 时，前端才把这段 reasoning 接成次级 thought 折叠块，绝不能再把 reasoning 当主答案兜底展示；`chat/completions` 兼容层若把 thinking 作为 `content` 数组项或 `<think>...</think>` 片段返回，后端也必须先拆成 `primary_text + reasoning`
- 问答成功后，反馈区优先切到轻量对话历史，而不是只显示最后一张结果卡；assistant 内容仍走 Markdown 渲染，便于继续追问
- 问答请求会显式携带工具列表：无论 `responses` 还是 `chat/completions`，都会注入内置 `wabity.read_file_lines` 和 `wabity.rag.query`；只有 `responses` 额外注入全局 MCP 里的 HTTP/SSE server。模型可以在单轮里并发调用多个工具，页面不再只显示压缩摘要，而是直接复用 `SessionTimeline` 的 action bar 渲染“第 N 步”、tool call 输入和 tool result 输出
- 本地文件引用点击打开需要单独命令；不要把桌面 opener 逻辑混进 `MarkdownRenderer`
