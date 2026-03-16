# launcher feature

职责：

- 管理 launcher 主窗口 UI
- 协调 workspace 地址栏、session 点、输入内容、候选动作和执行结果
- 处理键盘导航与前端副作用（打开 URL、复制文本）
- `LauncherPage.tsx` 只保留状态编排、命令调用和事件处理；纯函数和展示块拆到同目录模块与 `components/`

当前代码组织：

- `LauncherPage.tsx`：页面级状态、effect、键盘流和 Tauri 命令编排；顶部栏、输入区、反馈区都只负责装配组件，不再内联大段 JSX
- `launcher.css`：只保留 launcher 特有布局、状态和消息流样式；按钮、输入框、浮层、glass frame 等共享外观基线统一回收到 `src/app/global.css`
- `query.ts`：输入分类、`@token` 解析、补全文本和搜索阈值判断
- `layout.ts`：输入测量、宽度约束和补全浮层定位基础工具
- `workspace.ts`：workspace 路径格式化和面包屑构建
- `sessions.ts`：session 摘要合并、状态文案和 dot class 计算
- `components/SessionTimeline.tsx`：ACP 消息流与 action bar
- `components/LauncherHeader.tsx`：顶部 workspace bar、agent 选择器、session 摘要按钮和 session dot 带
- `components/LauncherComposer.tsx`：主输入区和底部操作条
- `components/RestoreNoticeList.tsx` / `components/LauncherFeedback.tsx`：恢复提示、内联结果卡片、JSON / Markdown 预览和 session 状态反馈
- `components/MarkdownRenderer.tsx`：共享 markdown 渲染管线，统一处理 GFM、Mermaid、MDX 安全兼容和 Obsidian 风格扩展
- `components/CompletionPopup.tsx` / `SessionPanel.tsx` / `WorkspacePickerPanel.tsx` / `AgentPickerPanel.tsx`：launcher 外层浮层组件
- `components/SessionTimeline.tsx`：仅在激活 ACP session 后才懒加载；会话 markdown 仍复用共享渲染器，但 mermaid 等较重依赖继续按需动态导入

当前 UI 决策：

- 顶部增加 workspace 面包屑地址栏和 session 点带
- 左上角增加固定文案 `Workspace` 的选择器按钮，展开后顶部优先提供“选择新的工作目录”，其后再列最近目录
- workspace 选择框、session 列表和补全框都挂在外层浮层，不受圆角面板内部裁剪限制
- workspace 地址栏故意压低字号、底色和间距，降低存在感，避免抢主输入区注意力
- 地址栏末尾的空白区显式作为窗口拖拽带，允许直接拖动 launcher 位置
- 该拖拽带走 Tauri `startDragging()`，并依赖 capability `core:window:allow-start-dragging`
- macOS / Linux 下，涉及 HOME 的路径统一用 `~` 缩写
- launcher 启动时优先恢复上次保存的当前 workspace；无效时回退到用户 `HOME`，最近目录只保留 3 个
- launcher 默认失焦自动隐藏，但打开原生目录选择器时会临时抑制自动隐藏，避免页面看起来“闪退”
- launcher 通过快捷键、OCR 回填或其他显示路径重新出现时，主输入框会主动恢复焦点，不能只依赖首次挂载时的 `autoFocus`
- launcher 主输入框显式关闭浏览器原生 `autocomplete`、`autocorrect`、`autocapitalize` 和 `spellcheck`，避免系统历史候选或拼写建议和自定义补全浮层叠出双层列表
- 外部应用选中文本注入 launcher 时，主输入框会把光标显式定位到文本开头，并自动切到多行模式，避免默认落在末尾或继续停留在单行输入
- 主输入框聚焦态只保留柔和的底边提亮和浅背景过渡，不再叠全尺寸粗 outline，避免输入时视觉重心突然跳变
- 多行输入框按内容自动增高，但会基于屏幕可用高度收敛到固定上限；超出部分交给输入框内滚动，避免 Tauri 窗口被长文本继续撑高
- 主输入区在“本地 launcher 模式”和“ACP session 模式”之间切换
- 配置好 ACP agent 后，底部操作条会提供 `Agent 执行` 入口；仅在未激活 session 时显示，点击会自动创建或复用 ACP session，并在下方会话面板展示 agent 输出；该入口额外绑定 `Alt+Enter`
- 顶部支持为“下一次新建 session”选择当前 agent；同一时刻可以并存多个不同 agent 的 session
- 结果改为内联反馈；ACP 激活时下方改为消息流面板
- 内联结果卡和 ACP 消息流都带固定高度上限与内部滚动，不允许单次长输出把 launcher 主窗口顶出屏幕
- `/format`、`/fmt`、`/json` 的预览、`/md` / `/markdown` 的 markdown 预览，以及 `/base64` 等快捷命令执行结果统一落在内联结果卡片；结果卡片自带复制按钮，JSON 走语法高亮，Markdown 走共享渲染器
- 文本处理 slash 命令当前内建 `/upper`、`/title`、`/lower`、`/camel`、`/snake`、`/trim`、`/unique`、`/sort`、`/words`、`/lines`；其中 `/title` 会把每个词的首字母转为大写，`/unique` 和 `/sort` 都按行处理
- 底部操作条只保留当前状态真正可执行的动作：主按钮会在 `执行` / `发送` / `插入路径` 之间切换，并在没有有效目标时禁用；当前 session 的关闭统一收敛到顶部会话区和会话面板，不再在底部重复放一个“关闭会话”
- 输入框下方、按钮左侧定义为左对齐状态栏；默认展示最后一次用户提交的内容，文本溢出时自动横向滚动，不再额外在消息区顶部重复回显一块“最后一次提交”
- `/format` 成为 JSON 格式化主命令，`/fmt` 为短别名，保留 `/json` 兼容；当输入命中该命令且后续内容是合法 JSON 时，输入框下方直接显示 pretty format 预览
- `/md` 成为 Markdown 渲染主命令，`/markdown` 为长别名；命中后输入框下方直接显示 Markdown 预览，支持 GFM、Mermaid、MDX 安全兼容，以及 Obsidian 风格 `[[wiki link]]`、`> [!note]` callout、`==highlight==`
- `/base64` 作为文本编解码命令；执行时会自动尝试把载荷识别为 UTF-8 Base64 文本，命中则解码，否则编码
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
- 补全提示改为跟随输入光标的浮动候选框，位置和内容都基于光标前文本实时刷新，默认高亮第一项；动作候选固定显示“主 `/` 命令 + 简短说明”，应用候选显示应用名和 bundle 路径；主动作按钮显式展示快捷键：单行输入用 `Enter`，多行输入用 `Ctrl/Cmd+Enter`；候选框可见时 `Enter` 默认执行当前模式的主动作，`Esc` 默认隐藏整个补全框；slash 动作确认后不会再把完整命令文本留在输入框里，而是进入一次性的待执行状态：输入框只保留 payload，下一次 `Enter` 或主按钮直接执行；如果确认时已经带 payload，则该次确认直接执行，并在成功后仍只保留 payload；已选 slash 动作还会接受最短命令前缀和紧贴 payload 的写法，例如 `/uhello` 会按 `/upper hello` 处理；通过上下键显式选中某个 slash 动作后，执行也会以该动作为准，不会把前导 `/` 残留进 payload；slash 执行后会保留 payload 内原来的光标逻辑位置，不会强制跳到末尾；高亮项变化时列表会自动滚动，尽量保持高亮项居中
- `/` 候选列表只保留已可执行命令，不把未接通的占位能力混进来制造噪音
- 使用透明窗口 + CSS 圆角伪异形，尽量模拟原生圆角窗口
- 原生窗口尺寸直接由内部页面内容尺寸实时驱动；`setSize` 设置的是窗口内容区，不需要再额外补偿 macOS 圆角或外沿
- 尺寸同步改为以整页可见内容盒为准，不只看圆角面板本身；绝对定位的补全面板、设置页等页面切换也会参与测量，避免窗口和页面边界错位
- 页面根节点额外保留极小透明安全边，并按完整包围盒测量左右溢出，避免透明窗口把圆角边缘直接裁掉
- 原生窗口 resize 不再由前端直接调用 `WebviewWindow.setSize`；统一走 Rust 命令，macOS 下显式写入 `NSPanel.setContentSize`
- 禁用原生窗口阴影，避免 macOS 透明窗口在视觉上比内部页面多出一圈外沿
- 不再依赖 `windowEffects` 的原生背景和圆角层，窗口外观完全由前端圆角面板控制，避免 macOS 原生效果层和网页面板叠出双层边界
- macOS 下仍启用 `macOSPrivateApi` 维持透明窗口能力，但不再把原生圆角当成窗口外观来源；这仍然放弃 App Store 兼容性
- 主输入框横向与窗口外框对齐，不再受内容容器水平内边距二次收缩
- 窗口宽高由内容实时测量驱动：多行输入自动增高，外框宽度在可用范围内随实际内容收缩或扩展；窗口 resize 不再顺手重新居中，避免页面一变窗口就跳位置
- 光标所在 `@token` 切换到文件搜索模式；文件查询达到 2 个英文字符或 1 个非英文字母（如中文）后，再延迟 50ms 发起搜索
- 文件搜索根目录固定为当前 workspace，不再默认为用户目录
- 文件补全只替换当前 `@token`，不会覆盖光标前其他 prompt 内容
- 普通文本应用搜索当前只做 macOS `.app` bundle 查找；应用查询至少需要 2 个英文字符或 1 个非英文字母，并额外追加 80ms 防抖，避免任意单字符输入就触发搜索；候选展示名优先取系统本地化名称，bundle 目录名仍作为中英文双向检索别名；不做图标、不做最近使用排序、不做跨平台兜底
- 应用搜索的后端索引会在启动后后台预热，并维持常驻内存快照；前端查询命中的永远是当前快照，不等待刷新任务
- session 点只承担轻量切换和通知，不承担完整标签页语义
- session 光点语义固定为：绿色慢闪=`running`，快闪=有新通知，灰色=会话断开，黄色=可恢复错误，红色=不可恢复错误
- `prompt` 发出后，前端立即插入用户消息和 pending assistant 占位，避免会话看起来“没反应”
- ACP 会话流展示改为最新 turn 在上、历史 turn 在下；页面顶部会额外弱化回显最后一次用户提交，便于在 agent 流式输出时保持当前请求上下文
- 时间线上相邻的 `thought` 块会在前端合并显示，避免 agent 连续推送 reasoning chunk 时被拆成多个折叠块制造视觉噪音
- assistant action bar 会把带相同 `correlationId` 的 `tool-call` / `tool-update` 融合成一个 tool pill；详情默认通过 hover 在 action bar 下方展开大面板，并在展示层尽量解码常见转义文本，点击作为触屏兜底
- session 更新合并以 `lastUpdatedAtMs` 和消息权重单调收敛，避免旧快照覆盖异步事件流
- session 恢复完全依赖 agent 自身能力；agent 不支持 `session/load` 时，只提示，不伪装恢复成功
- 全局快捷键恢复为平台对应方案：macOS 使用 `⌘⇧Space`，非 macOS 使用 `Ctrl+Shift+Space`

约束：

- 领域匹配与执行逻辑以后端 Rust 命令为准
- 前端只做展示状态和安全的 UI 侧效果补全
