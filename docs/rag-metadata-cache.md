# RAG Metadata Cache

## 背景

原有 RAG 实现虽然已经有 SQLite 文件级元数据缓存，但语义仍是单版本：

- SQLite 只表达一个文件当前是不是 `pending/indexed`
- 文件一旦进入重建分支，就会先删旧向量，再写新向量
- watcher 去抖后仍按路径逐个开关索引存储 / SQLite 连接
- 文件内容变化时默认整文件所有 chunk 重新 embedding

这会带来三个直接问题：

- 查询窗口会出现“旧版本已删、新版本未写完”的空洞
- 大批量文件改动时，本地数据库和 embedding 调用都有明显额外开销
- 长文档只改小段时，embedding 成本被整文件重建放大

## 决策

继续使用一份本地 chunk/向量索引 + 一份 SQLite，但把语义改成显式版本切换。

职责划分：

- 本地索引：保存 chunk 文本、向量和检索所需片段级元数据
- SQLite：保存文件级“当前活动版本 + 待切换版本”状态

### SQLite 记录字段

- `absolute_path`
- `source_root`
- `relative_path`
- `embedding_fingerprint`
- `active_version_id`
- `active_content_md5`
- `active_modified_at_ms`
- `active_size_bytes`
- `active_chunk_count`
- `active_indexed_at_ms`
- `pending_version_id`
- `pending_content_md5`
- `pending_modified_at_ms`
- `pending_size_bytes`
- `pending_chunk_count`
- `pending_started_at_ms`

### 本地 chunk 记录字段

- `id`
- `source_root`
- `absolute_path`
- `relative_path`
- `path`
- `version_id`
- `embedding_fingerprint`
- `chunk_state`
- `chunk_index`
- `line_start`
- `line_end`
- `paragraph_line_start`
- `heading_path`
- `chunk_reuse_key`
- `text_fingerprint`
- `text`
- `vector`

其中：

- `chunk_state` 当前只区分 `staged` 和 `active`
- 查询侧只读取 `active` chunk
- `chunk_reuse_key` 用于在同一文件的新旧版本之间复用未变化 chunk 的向量
- `embedding_fingerprint + text_fingerprint + text` 共同承担全局向量缓存；同文本只要命中同一 embedding fingerprint，就可以跨文件直接复用旧向量

## 版本切换语义

### 全量扫描

旧逻辑：

- 先清空本地索引
- 全量重算 embedding

现逻辑：

- 先读取 SQLite 元数据，并校验本地索引布局
- 扫描当前目录，和历史元数据对账
- `size + mtime` 未变化的文件：直接视为未变，跳过文件读取和 embedding
- `size/mtime` 变化但 `md5` 不变的文件：只刷新 `active` 元数据，不重算 embedding
- 如果 `active` 版本仍匹配当前文件，但本地残留了未完成的 `pending` 版本：清掉残留 `pending` 元数据和 `staged` 向量，不把文件再次送去 embedding
- 新文件、内容变更文件或 embedding fingerprint 变更文件：扫描线程一旦发现就立即生成新的待索引版本计划；实际开始 indexing 时再把该文件写成 `pending`，避免整批排队文件在中断后一起残留脏状态
- writer 串行把新版本 chunk 以 `staged` 写入本地索引，成功后切到 `active`
- 等所有待重建文件都写成功后，最后再统一清理旧 `active` 版本和整轮扫描里确认 stale 的路径

### 增量更新

- watcher 去抖后不再逐路径独立处理，而是先汇总路径，再批量查 metadata、批量删写本地索引、批量提交 SQLite
- 只有目录 rename 等明确可能影响整棵子树路径映射的事件才升级成全量对账；普通目录 create / metadata / other 噪音优先按增量事件处理
- 文件删除或失效：删向量、删元数据
- 文件内容未变：不重算 embedding
- 文件内容变更：保留旧 `active` 版本可查询，新版本写成 `staged`；切换成功后再清理旧 `active`

### 兼容性处理

- 如果本地索引布局或 SQLite metadata schema 已过期，直接清空本地索引和 SQLite 后重建
- 如果 SQLite 记录的 `embedding_fingerprint` 与当前 provider 不一致，同样清空后重建
- 如果运行时配置暂时无效，例如扫描目录不可解析或 provider 配置有误，只上报错误并保留现有索引，避免把“坏配置”误处理成“缓存必须失效”
- 当前不做历史 schema 迁移，目标是保证语义简单和实现收敛

## Chunk 复用与 Embedding 节流

### Markdown 切块

- Markdown 不再直接按统一字符窗口切整篇文本
- 索引前会先按标题、列表项、fenced code block 和普通段落做语义预切
- 之后只在同一 `heading_path` 内按目标字符预算打包多个语义块
- 当前打包目标是 `350 chars`，硬上限是 `550 chars`，相邻 chunk 之间保留约 `80 chars` 重叠
- 如果单个语义块本身已经超过硬上限，才回退到 `MarkdownSplitter` 在块内继续拆分

这样做的目的不是追求“块越小越好”，而是避免读书摘记、列表式笔记这类 Markdown 被整篇并成一个 chunk，同时又不把技术文档和代码块切得过碎

### Chunk 复用

- 文件重建前，先读取该文件当前 `active` 版本的 chunk 向量
- 新切块会为每个 chunk 生成 `chunk_reuse_key`
- 若新旧版本存在相同 `chunk_reuse_key`，则直接复用旧向量
- 若文件内复用仍未命中，则继续按 `原文文本 + embedding_fingerprint` 在现有 chunk 行中查找已算好的向量
- 只有文件内复用和全局文本缓存都未命中的 chunk 才会重新调 embedding

这不是严格意义上的“任意文档结构差异都能做最优最小重建”，但已经把“整文件所有 chunk 全重算”收敛为“先复用文件内旧块，再复用全局同文本旧向量，最后只为真正缺失的文本请求 embedding”

### Embedding 批次

- embedding 请求使用更长 HTTP 超时
- 批次大小从安全下限起步
- 成功整批则继续放大
- 超时或显式 OOM / 413 则拆小重试

目标不是保守稳定优先，而是在不把 watcher 打死的前提下尽量吃满 provider 吞吐

## 低风险优化

- SQLite 连接默认启用 `WAL` 和批量事务，减少高频小写入的锁竞争
- watcher 批次内复用索引存储 / SQLite 连接，不再按文件反复开关
- 文件读取路径减少不必要的字节复制
- 元数据批量查询使用分批 `IN (...)`，避免逐路径单查
- 纯 metadata watcher 事件在进入索引规划前直接过滤，避免把 write-time / xattr 噪音升级成整文件读取、`md5` 和分片
- 手动全量重建与后台 watcher 增量维护通过同一把存储锁串行化，避免两条链路并发改写同一份本地索引 / SQLite
- 当前语义索引已经切到“SQLite 真相源 + USearch 派生文件”；只要 active chunk 集合变化，就在该批次结束前重建 USearch，不能再沿用旧的阈值延迟重建语义
- active chunk 成功写入 USearch 后，会回收 SQLite 行上的 `vector_blob`；SQLite 不再长期保留 active 向量副本
- 如果只残留 `rag-chunks.dirty` 但 USearch 仍可用，启动时只补做 blob 回收并清掉 dirty marker，不应误触发全量重建
- 如果 active 向量 blob 只回收到一半且 USearch 又缺失/损坏，这属于不可恢复的半残状态；启动时直接重置本地索引，避免拿不完整 blob 强行重建
- active 向量集合现在额外维护一份 SQLite 单行摘要元数据：`active_vector_count`、`vector_dimensions`、`key_xor`、`key_sum`、`key_hash_xor`、`key_hash_sum`；`rag_chunks` 行本身也会持久化每条向量的稳定 `vector_hash`。USearch 旁边同步写 sidecar manifest，记录同一份摘要加上索引文件长度、mtime、整文件 `md5`，以及少量确定性 probe 向量的 `vector_key + vector_hash`。稳定启动路径先做“SQLite 摘要 vs manifest probe vs USearch 导出探针”快速对账，再用整文件摘要兜底同大小/同 mtime 的非 probe 污染；只有 manifest 缺失、旧版 manifest 升级或缺 digest 字段时才回退一次完整向量覆盖校验，并在通过后补写 manifest。注意：legacy `rag_chunks` 行如果既缺 `vector_hash` 又已经回收了 `vector_blob`，启动阶段默认不会把当前 USearch 直接反写成新的真相；只有旧 manifest probe 仍能验证当前 USearch 时，才允许受控补齐缺失 hash

## 状态

- 2026-03-17：首版 SQLite 文件级元数据缓存接入全量扫描和 watcher 增量更新
- 2026-03-17：补充“先写 SQLite、后写向量”的恢复语义，修复首轮慢扫描时 metadata 为空导致的误报与索引自清空问题
- 2026-03-17：embedding 请求改为更长超时，并在索引过程中自适应调整批次大小；成功整批会继续放大，超时或显式 OOM/413 再拆小重试
- 2026-03-17：元数据新增 `embedding_model`；切换 embedding 模型后，旧向量会被判定为失效并重新生成
- 2026-03-17：索引语义从单版本 `pending/indexed` 升级为 `staged/active` 版本切换；新版本写成功后再切换并清理旧版本，同时接入 watcher 批处理、连接复用、chunk 指纹复用和低风险内存优化
- 2026-03-17：全量重建改成流式并行流水线；扫描过程中一旦发现待重建文件就立即进入读取、分片、embedding 和落盘，stale 清理延后到所有新向量落盘完成后统一执行
- 2026-03-21：文件级索引目标从 `embedding_model` 升级为 `embedding_fingerprint`，冷启动也会对账当前 embedding 目标身份；fingerprint 优先使用模型自身稳定身份（显式 digest、`/models` 返回项里的 digest/fingerprint hint），普通兼容模型则默认绑定 `provider endpoint + normalized model name`，避免跨 endpoint 误复用向量。chunk 行新增 `embedding_fingerprint` / `text_fingerprint`，会在调用 embedding 前先按 `原文文本 + embedding_fingerprint` 查找全局已算好的向量再决定是否远程计算
- 2026-03-23：watcher 现在会直接忽略纯 metadata 事件，手动全量重建与后台增量维护共享存储互斥
- 2026-04-07：向量存储切换为“SQLite 真相源 + USearch 派生索引”；USearch 不再按脏阈值延迟刷新，而是在 active chunk 变化后按批次重建，避免语义索引陈旧
- 2026-04-10：active chunk 在 USearch 落盘成功后立即清空 SQLite `vector_blob`，减少本地磁盘占用；若启动时 USearch 缺失且 active blob 已回收，则重置本地索引并走全量重建
- 2026-04-10：补充恢复语义细化；dirty marker 残留但 USearch 可用时只完成善后，不再误清空索引；active blob 处于部分回收状态且 USearch 不可用时，直接判为不可恢复并重置
- 2026-04-14：为 active 向量集合引入 SQLite 增量摘要元数据、行级 `vector_hash` 和 USearch sidecar manifest；启动完整性校验从“逐条导出 active 向量”收敛成“元数据摘要 + probe 向量 + 索引文件摘要”的快速对账，缺 manifest、旧版 manifest 或缺 digest 字段的场景才回退一次完整覆盖校验并回填 manifest；若 legacy active 行缺 `vector_hash` 且 blob 已回收，只有旧 manifest probe 仍能验证当前 USearch 时才允许补齐 hash
- 2026-03-25：Markdown 切块升级为“两阶段切块”：先按标题 / 列表项 / 代码块 / 段落做语义预切，再按目标字符预算在同一标题路径下打包；单个语义块超限时才回退到 `MarkdownSplitter`，以改善读书摘记和列表式笔记的检索粒度
