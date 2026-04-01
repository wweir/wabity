# AI Settings Remediation 2026-04-01

## Scope

按 `2026-04-01` 的 AI 功能页审计结论，只修 `AI 功能` 分组当前最显著的交互层级问题：

- 任务卡底部按钮太抢镜
- 次级恢复动作和主保存动作长成同一层级
- “展开编辑”入口过轻，和正文链接混在一起
- 窄宽度下操作区容易退化成按钮墙

## Decisions

1. 只改 `AI 功能` 任务卡，不顺手重排 `LLM`、`RAG` 或其它 settings 分组。
2. 每张任务卡只保留一个主保存按钮；恢复类动作降为 quiet action。
3. “展开编辑 / 收起编辑”从弱文本链接升级为可感知的轻量按钮，但仍保持次级层级。
4. 窄宽度下优先允许动作换行，不再把三颗按钮硬堆成整列全宽按钮。
5. 同步更新 `src/features/settings/README.md` 和 `ARCHITECTURE.md`，把 AI 功能任务卡的动作层级约束写清楚。

## Status

- Phase start: 已开始
- Phase end: 已完成

## Verification Baseline

- 运行格式化
- 运行前端 lint
- 运行 `rust-analyzer diagnostics src-tauri --severity error`
- 复查 AI 功能页动作层级和窄宽度回退
