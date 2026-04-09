# 内存预算收敛记录（2026-04-07）

## 背景

这次调整针对的是“空闲态和重建态都偏重，但进程级 RSS 又难以直接归因”的问题。

结论先固定：

- 空闲态大头仍然是 `Tauri/AppKit/WebKit` 底座，不是单点 Rust 泄漏
- 但业务层仍存在几类明确可收敛的常驻内存成本
- 后续优化必须优先依赖业务观测字段，而不是继续只盯进程总 RSS

## 本轮长期决策

1. `application` 和 `process` 缓存不再在启动时预热，也没有固定轮询刷新
2. `application` 首次命中时同步建索引；旧快照超过 10 分钟后只异步补刷新
3. `process` 首次命中时同步建快照；旧快照超过 15 秒后只异步补刷新
4. `file_search` 本地索引记录只保留 `path`，`file_name` 和 `parent` 按需投影
5. RAG 摄取按文档类型收紧单文件上限：纯文本/Markdown 20 MB、`docx` 16 MB、`pdf` 8 MB
6. RAG 全量重建收紧并发和中间态：文件级并发 2、embedding 默认批次 4、最大 32、扫描到写入之间的缓冲缩小
7. `PreparedRagFile` 不再长期持有整份 `PreparedRagChunk` 列表；进入写入阶段前再重建 chunk 中间态
8. `application`、`process`、`file_search` 和 RAG 文件摄取都必须补齐条目数、路径字节数、抽取文本字节数、chunk 数、向量字节估算等业务观测字段

## 不接受的回退

- 为了“首查更快”重新加启动预热或固定定时刷新
- 在索引记录里长期保留可由 `path` 派生的重复字符串
- 为了追求吞吐默认放大 RAG 并发和 embedding 批次
- 只看 RSS 就声称发现了业务热点，却没有业务观测字段支撑

## 后续观测重点

- `application cache refreshed`
- `process cache snapshot replaced`
- `workspace file index ready`
- `prepared RAG file for indexing`

如果后续仍然出现内存峰值，先看这些日志里的条目数、字节数和 chunk 数，再决定是调预算、拆模型，还是继续改数据结构。
