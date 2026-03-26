# Operation Trail Rebalance

Date: 2026-03-26
Status: Completed
Scope: `src/features/launcher/components/SessionTimeline.tsx`, `src/features/launcher/launcher.css`

## Goal

把问答结果中的“操作轨迹”从并列内容区降回正文后的次级辅助信息，落实既有约束：

- 答案正文优先
- 操作轨迹次级
- thought 更次级
- 角色元信息最低

## Decisions

1. assistant message 内的归并顺序调整为 `thought -> content -> actions`，渲染时正文先于轨迹出现。
2. action trail 去掉独立标题栏和计数栏，避免在消息卡里再造一个 telemetry 子模块。
3. tool detail 从 hover 即展开改为显式点击展开，减少无意抢焦点。
4. action trail 的背景、边框、文字和分类色收回全局 token，避免继续靠局部硬编码维持视觉层级。

## Result

- 操作轨迹仍保留可见性和可追溯性，但不再先于答案进入阅读链路。
- 详情区不再使用深色高对比 inspection panel，而是作为消息内部弱展开区存在。
- launcher 文档和架构文档已同步更新，后续不应再把 action trail 实现回独立主面板。
