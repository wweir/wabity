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

- [clipboard-history-design-2026-04-01.md](/Users/wweir/Sites/Mine/wabity/docs/clipboard-history-design-2026-04-01.md): 历史剪贴板范围、数据模型和回贴链路
- [desktop-notification-design-2026-03-27.md](/Users/wweir/Sites/Mine/wabity/docs/desktop-notification-design-2026-03-27.md): 桌面通知完成语义与平台边界
- [kill-command-design-2026-04-03.md](/Users/wweir/Sites/Mine/wabity/docs/kill-command-design-2026-04-03.md): `/kill` 的补全与执行语义
- [shortcut-discoverability-design-2026-04-06.md](/Users/wweir/Sites/Mine/wabity/docs/shortcut-discoverability-design-2026-04-06.md): 快捷键失败反馈、设置页总览和 launcher 空态提示
- [system-opener-design-2026-03-28.md](/Users/wweir/Sites/Mine/wabity/docs/system-opener-design-2026-03-28.md): 统一系统打开能力与安全边界

### LLM / 设置

- [llm-model-settings.md](/Users/wweir/Sites/Mine/wabity/docs/llm-model-settings.md): provider 分组 / model 最小单元建模与兼容迁移
- [llm-builtin-provider-catalog.md](/Users/wweir/Sites/Mine/wabity/docs/llm-builtin-provider-catalog.md): 内置 provider 模板目录

### RAG / MCP / ACP

- [rag-qa-design.md](/Users/wweir/Sites/Mine/wabity/docs/rag-qa-design.md): launcher 轻量问答链路
- [rag-document-ingestion-design-2026-03-26.md](/Users/wweir/Sites/Mine/wabity/docs/rag-document-ingestion-design-2026-03-26.md): 文档摄取、切分和索引边界
- [rag-metadata-cache.md](/Users/wweir/Sites/Mine/wabity/docs/rag-metadata-cache.md): RAG 元数据缓存与 staged/active 切换语义
- [rag-retrieval-optimization-2026-04-07.md](/Users/wweir/Sites/Mine/wabity/docs/rag-retrieval-optimization-2026-04-07.md): 启发式 query rewrite 与多查询召回优化
- [rag-mcp-server.md](/Users/wweir/Sites/Mine/wabity/docs/rag-mcp-server.md): 内置 MCP server 的模块化设计
- [rag-answer-refactor-2026-03-30.md](/Users/wweir/Sites/Mine/wabity/docs/rag-answer-refactor-2026-03-30.md): 问答后端重构记录
- [acp-timeline-chronology-rework-2026-04-03.md](/Users/wweir/Sites/Mine/wabity/docs/acp-timeline-chronology-rework-2026-04-03.md): ACP transcript 按真实时序渲染的实现决策

### 工程化

- [large-file-splitting-2026-04-03.md](/Users/wweir/Sites/Mine/wabity/docs/large-file-splitting-2026-04-03.md): 大文件拆分的边界收敛记录
- [memory-budget-optimization-2026-04-07.md](/Users/wweir/Sites/Mine/wabity/docs/memory-budget-optimization-2026-04-07.md): 常驻缓存、RAG 批次与业务级内存观测的收敛决策
- [release-version-sync-2026-04-11.md](/Users/wweir/Sites/Mine/wabity/docs/release-version-sync-2026-04-11.md): 发版版本源同步与 tag 顺序约束

## 维护规则

- 系统边界、分层职责、关键数据流统一写在根目录 [ARCHITECTURE.md](/Users/wweir/Sites/Mine/wabity/ARCHITECTURE.md)
- feature 局部约束写到对应目录 `README.md`
- 新增 `docs/` 文档前，先判断它是否真的会在一周后仍值得保留；如果只是某次检查或修样式过程，别写进这里
