# ACP Timeline Chronology Rework

Date: 2026-04-03
Status: Completed
Scope:

- `src-tauri/src/services/acp/mod.rs`
- `src-tauri/src/services/acp/README.md`
- `src/features/launcher/components/SessionTimeline.tsx`
- `src/features/launcher/launcher.css`
- `src/features/launcher/README.md`
- `src/features/launcher/components/README.md`

## Problem

ACP Agent 交互界面有两个根问题：

1. 多工具调用时，前端把事件压成折行 pill 云，视觉上像 telemetry 面板，不像对话时间线。
2. 后端和前端都会把一轮 assistant 输出压成固定的 `thought / actions / content` 摘要，真实时序被抹平，导致 thought 无法按实际位置穿插在工具调用前后。

## Decisions

1. 后端不再把同类型 block 跨事件回填到更早位置；只在“紧邻且同类”时合并 chunk。
2. assistant message 的 block 顺序由运行时事件决定，前端不得再重排成固定 `answer-first`。
3. action trail 从 pill 聚合改成线性事件条目，优先服务“读懂顺序”，不是“扫分类标签”。
4. 工具详情继续保留显式展开，但改为弱内联展开区，不再做成独立 inspection panel。
5. 正文仍保持更高视觉权重，但这种优先级只能靠排版建立，不能靠篡改时间线顺序建立。

## Result

- thought / tool call / tool update / content 现在可以按真实顺序交错显示。
- 多工具调用场景不再退化成拥挤的折行 chip cloud。
- `correlationId` 继续保留，供后续需要时做更深层的 call/result 关联，但当前 UI 不再强制压成单个 pill。
- 相关架构与 feature 文档已同步，后续不应再把 ACP transcript 改回固定摘要结构。
