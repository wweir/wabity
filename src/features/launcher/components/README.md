# launcher components

职责：

- 承载 `LauncherPage` 拆出的无状态或弱状态展示组件
- 保持样式类名和交互语义稳定，把状态编排继续留在页面层

当前模块：

- `LauncherHeader.tsx`：渲染顶部 workspace bar、agent picker 触发器和 session dot 带；不再承载历史剪贴板入口
- `LauncherComposer.tsx`：渲染主输入区、底部状态栏和操作按钮，包括设置、补全、翻译、Agent 执行和主动作；状态栏文案与色调由页面层注入
- `RestoreNoticeList.tsx`：渲染 session 恢复失败提示列表
- `LauncherFeedback.tsx`：渲染带复制按钮的内联执行结果卡片、`/format` JSON 高亮预览、`/md` Markdown 预览、ACP session 时间线和状态栏
- `MarkdownRenderer.tsx`：统一承接 launcher 内的 Markdown 渲染，补 GFM、Mermaid、MDX 安全兼容和 Obsidian 风格 wiki link / callout / highlight
- `SessionTimeline.tsx`：渲染 ACP 会话消息流与 launcher 轻量问答历史；对 assistant message 会先把同一 turn 内零散的 `thought` / `actions` / `content` block 归并成稳定结构，但阅读顺序必须保持 answer-first：正文先于 action trail，thought 继续作为更次级的折叠信息；thought 入口位于正文后的消息元信息区，只提供简短预览，展开内容使用普通阅读排版而不是日志式 `pre`
- `CompletionPopup.tsx`：渲染文件、动作、应用建议列表
- `ClipboardHistoryPanel.tsx`：渲染由全局快捷键打开的历史剪贴板面板，按 `Pinned / Recent` 两段显示文本条目；当 launcher 已在前台时，确认条目会插入输入框，否则回贴到外部应用；同时提供 pin/unpin、删除动作
- `SessionPanel.tsx`：渲染全部 session 浮层；列表项靠单一激活高亮、语义状态标签和明确关闭按钮建立层级，不重复堆叠“当前”提示
- `WorkspacePickerPanel.tsx`：渲染 workspace 选择浮层
- `AgentPickerPanel.tsx`：渲染 agent 选择浮层

约束：

- 不接入 Tauri API，不直接管理 launcher 核心状态
- 英文注释只在非显然逻辑需要时出现；展示组件默认不写解释性废话
- 输入、选择和操作按钮行为都通过 props 回调回传给 `LauncherPage`
