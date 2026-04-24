# launcher components

职责：

- 承载 `LauncherPage` 拆出的无状态或弱状态展示组件
- 保持样式类名和交互语义稳定，把状态编排继续留在页面层

当前模块：

- `LauncherHeader.tsx`：渲染顶部 workspace bar、agent picker 触发器和 session dot 带；不再承载历史剪贴板入口
- `LauncherComposer.tsx`：渲染主输入区、底部状态栏和操作按钮，包括设置、补全、翻译、Agent 执行和主动作；状态栏文案与色调由页面层注入
- `RestoreNoticeList.tsx`：渲染 session 恢复失败提示列表
- `LauncherFeedback.tsx`：继续承接 launcher 主反馈区，但内部已经按 `session / result / QA` 三类展示拆成更细粒度子区块，减少高频输入时对整块内容的重算
- `LauncherSuggestionsSection.tsx`：收口文件、动作、应用补全浮层，作为 launcher suggestion layer 的独立渲染边界
- `LauncherSessionSection.tsx`：收口 session 浮层，避免 session 面板装配逻辑继续堆在 `LauncherPage`
- `LauncherClipboardSection.tsx`：收口历史剪贴板面板，隔离 `clipboard` 面板 props 和事件边界
- `MarkdownRenderer.tsx`：统一承接 launcher 内的 Markdown 渲染，补 GFM、Mermaid、MDX 安全兼容和 Obsidian 风格 wiki link / callout / highlight
- `ThoughtDisclosure.tsx`：提取 launcher 内通用的 thought 折叠展示；ACP 时间线和翻译结果都复用同一套预览、展开语义和阅读排版，避免同一类次级信息出现两套行为
- `SessionTimeline.tsx`：渲染 ACP 会话消息流与 launcher 轻量问答历史；assistant message 必须按后端提供的 block 原始顺序渲染，保留 `thought / actions / content` 的真实交错时序，不能再为了“摘要化”重排成固定答案优先结构；正文块仍保持主阅读排版，action trail 和 thought 继续作为次级信息，但只能靠视觉权重降低，而不能靠改写顺序；thought 默认折叠，只提供简短预览，展开内容使用普通阅读排版而不是日志式 `pre`；ACP 流式输出时只在用户仍跟随当前 turn 尾部时自动追随新增文本，用户手动滚离后不得抢回滚动位置
- `CompletionPopup.tsx`：渲染文件、动作、应用建议列表
- `ClipboardHistoryPanel.tsx`：渲染由全局快捷键打开的历史剪贴板面板，按 `Pinned / Recent` 两段显示文本条目；当 launcher 已在前台时，确认条目会插入输入框，否则回贴到外部应用；同时提供 pin/unpin、删除动作
- `SessionPanel.tsx`：渲染全部 session 浮层；列表项靠单一激活高亮、语义状态标签和明确关闭按钮建立层级，不重复堆叠“当前”提示
- `WorkspacePickerPanel.tsx`：渲染 workspace 选择浮层
- `AgentPickerPanel.tsx`：渲染 agent 选择浮层

约束：

- 不接入 Tauri API，不直接管理 launcher 核心状态
- 英文注释只在非显然逻辑需要时出现；展示组件默认不写解释性废话
- 输入、选择和操作按钮行为都通过 props 回调回传给 `LauncherPage`
