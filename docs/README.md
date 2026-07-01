# docs

`docs/` 只保留两类文档：

1. 仍然有效的方案设计和关键决策
2. 对当前代码结构仍有解释价值的实施记录

不再保留的内容：

- 一次性 UI 审计报告
- 已完成且没有长期复用价值的整改流水账
- 和 `ARCHITECTURE.md`、feature `README.md` 重复表达同一约束的文档

## 当前保留文档

### Launcher / 交互

- [screencapturekit-ocr-workflow-design-2026-05-14.md](/Users/wweir/Sites/Mine/wabity/docs/screencapturekit-ocr-workflow-design-2026-05-14.md): 截图 OCR review、ScreenCaptureKit 迁移和未来图片 + prompt 多模态入口边界设计
- [clipboard-history-design-2026-04-01.md](/Users/wweir/Sites/Mine/wabity/docs/clipboard-history-design-2026-04-01.md): 历史剪贴板范围、数据模型和回贴链路
- [desktop-notification-design-2026-03-27.md](/Users/wweir/Sites/Mine/wabity/docs/desktop-notification-design-2026-03-27.md): 桌面通知完成语义与平台边界
- [kill-command-design-2026-04-03.md](/Users/wweir/Sites/Mine/wabity/docs/kill-command-design-2026-04-03.md): `/kill` 的补全与执行语义
- [shortcut-discoverability-design-2026-04-06.md](/Users/wweir/Sites/Mine/wabity/docs/shortcut-discoverability-design-2026-04-06.md): 快捷键失败反馈、设置页总览和 launcher 空态提示
- [system-opener-design-2026-03-28.md](/Users/wweir/Sites/Mine/wabity/docs/system-opener-design-2026-03-28.md): 统一系统打开能力与安全边界

### LLM / 设置

- [llm-model-settings.md](/Users/wweir/Sites/Mine/wabity/docs/llm-model-settings.md): provider 分组 / model 最小单元建模与兼容迁移
- [llm-builtin-provider-catalog.md](/Users/wweir/Sites/Mine/wabity/docs/llm-builtin-provider-catalog.md): 内置 provider 模板目录
- [src/features/settings/SETTINGS_IA.md](/Users/wweir/Sites/Mine/wabity/src/features/settings/SETTINGS_IA.md): 设置页信息架构、导航与分组职责
- [src/features/settings/SETTINGS_BEHAVIOR.md](/Users/wweir/Sites/Mine/wabity/src/features/settings/SETTINGS_BEHAVIOR.md): 设置页保存语义、跨页依赖与后端命令
- [src/features/settings/SETTINGS_UI.md](/Users/wweir/Sites/Mine/wabity/src/features/settings/SETTINGS_UI.md): 设置页视觉层级、布局与组件模式

### RAG / MCP / Agent

- [rag-qa-design.md](/Users/wweir/Sites/Mine/wabity/docs/rag-qa-design.md): launcher 轻量问答链路
- [rag-document-ingestion-design-2026-03-26.md](/Users/wweir/Sites/Mine/wabity/docs/rag-document-ingestion-design-2026-03-26.md): 文档摄取、切分和索引边界
- [rag-metadata-cache.md](/Users/wweir/Sites/Mine/wabity/docs/rag-metadata-cache.md): RAG 元数据缓存与 staged/active 切换语义
- [rag-retrieval-optimization-2026-04-07.md](/Users/wweir/Sites/Mine/wabity/docs/rag-retrieval-optimization-2026-04-07.md): 启发式 query rewrite 与多查询召回优化
- [agent-tools.md](/Users/wweir/Sites/Mine/wabity/docs/agent-tools.md): Agent 内置工具模块与 MCP 配置边界
- [rag-answer-refactor-2026-03-30.md](/Users/wweir/Sites/Mine/wabity/docs/rag-answer-refactor-2026-03-30.md): 问答后端重构记录
- [pi-agent-single-runtime-migration-2026-06-29.md](/Users/wweir/Sites/Mine/wabity/docs/pi-agent-single-runtime-migration-2026-06-29.md): 移除 ACP client、收敛到 Pi SDK 单运行时的执行方案
- [acp-timeline-chronology-rework-2026-04-03.md](/Users/wweir/Sites/Mine/wabity/docs/acp-timeline-chronology-rework-2026-04-03.md): 旧 ACP transcript 按真实时序渲染的历史实现决策；Pi Agent timeline 只继承“真实时序”原则，不继承 ACP transport

### 工程化

- [large-file-splitting-2026-04-03.md](/Users/wweir/Sites/Mine/wabity/docs/large-file-splitting-2026-04-03.md): 大文件拆分的边界收敛记录
- [memory-budget-optimization-2026-04-07.md](/Users/wweir/Sites/Mine/wabity/docs/memory-budget-optimization-2026-04-07.md): 常驻缓存、RAG 批次与业务级内存观测的收敛决策
- [release-version-sync-2026-04-11.md](/Users/wweir/Sites/Mine/wabity/docs/release-version-sync-2026-04-11.md): 发版版本源同步与 tag 顺序约束

## 维护规则

- 系统边界、分层职责、关键数据流统一写在根目录 [ARCHITECTURE.md](/Users/wweir/Sites/Mine/wabity/ARCHITECTURE.md)
- feature 局部约束写到对应目录 `README.md`
- 新增 `docs/` 文档前，先判断它是否真的会在一周后仍值得保留；如果只是某次检查或修样式过程，别写进这里
