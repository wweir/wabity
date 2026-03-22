# launcher components

职责：

- 承载 `LauncherPage` 拆出的无状态或弱状态展示组件
- 保持样式类名和交互语义稳定，把状态编排继续留在页面层

当前模块：

- `LauncherHeader.tsx`：渲染顶部 workspace bar、agent picker 触发器和 session dot 带
- `LauncherComposer.tsx`：渲染主输入区、底部状态栏和操作按钮，包括设置、补全、翻译、Agent 执行和主动作；状态栏文案与色调由页面层注入
- `RestoreNoticeList.tsx`：渲染 session 恢复失败提示列表
- `LauncherFeedback.tsx`：渲染带复制按钮的内联执行结果卡片、`/format` JSON 高亮预览、`/md` Markdown 预览、ACP session 时间线和状态栏
- `MarkdownRenderer.tsx`：统一承接 launcher 内的 Markdown 渲染，补 GFM、Mermaid、MDX 安全兼容和 Obsidian 风格 wiki link / callout / highlight
- `SessionTimeline.tsx`：渲染 ACP 会话消息流与 launcher 轻量问答历史；对 assistant message 会先把同一 turn 内零散的 `thought` / `actions` / `content` block 归并成三条规范化流，再承接 thought 折叠、action pill 详情和 Markdown 内容，避免工具事件把同一轮输出切成两段
- `CompletionPopup.tsx`：渲染文件、动作、应用建议列表
- `SessionPanel.tsx`：渲染全部 session 浮层
- `WorkspacePickerPanel.tsx`：渲染 workspace 选择浮层
- `AgentPickerPanel.tsx`：渲染 agent 选择浮层

约束：

- 不接入 Tauri API，不直接管理 launcher 核心状态
- 英文注释只在非显然逻辑需要时出现；展示组件默认不写解释性废话
- 输入、选择和操作按钮行为都通过 props 回调回传给 `LauncherPage`
