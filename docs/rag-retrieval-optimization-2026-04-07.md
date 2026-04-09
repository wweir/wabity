# RAG Retrieval Optimization 2026-04-07

## 背景

当前 RAG 查询链路已经具备：

- USearch 向量召回
- SQLite `FTS5 + bm25()` 词法召回
- 轻量 rerank
- 单文件配额裁剪

但仍然存在两个高频弱点：

1. 用户问题常带有明显的问句包装语，例如“哪里 / where / 帮我找 / documented”，直接拿原问题做 embedding 和 FTS 会稀释真正的约束词。
2. 现有检索虽然已经做“向量 + 词法”混合，但本质上仍偏单查询召回；一次 query 如果表达噪音太大，后续 rerank 没有足够好的候选可排。

## 目标

在不引入额外 LLM 请求、不改变现有设置模型、不推翻当前 USearch + SQLite 检索布局的前提下，先落地两项高优先级优化：

- 启发式 query rewrite
- 两阶段多查询召回 + 统一 rerank

## 方案

### 1. 结构化 QueryPlan

查询入口先把原始问题归一化成 `QueryPlan`：

- `normalized_query`：原问题标准化文本
- `focus_query`：去掉明显问句包装语后的聚焦查询
- `required_terms`：必须尽量保留的实体词 / 关键术语
- `semantic_queries`：供 embedding 用的 1 到 3 个查询变体
- `lexical_queries`：供 FTS 用的 1 到 4 个查询变体

当前 rewrite 是启发式的，不再调一次 LLM。原因很简单：

- 为检索前置再加一轮 LLM 调用，等于把“不稳定推理”移到更前面，不是优化，是扩散不确定性
- launcher 内的轻量问答链路对时延敏感，不值得为了召回先多付一次网络 RTT
- query rewrite 的目标是保留约束，不是生成答案；规则化处理更可控

### 2. 多查询召回

向量侧：

- 对 `semantic_queries` 批量请求 embedding
- 对每个 query 变体分别做向量召回
- 合并候选时保留更强的 query 变体权重

词法侧：

- 不再只跑一条 `OR` 查询
- 优先构建 `AND` 查询、聚焦 phrase 查询、路径词查询，再回退到 `OR`
- 多条 FTS 查询 union 后再按 chunk 去重

### 3. 统一 rerank

原有 rerank 继续保留，但额外引入：

- `focus_query` 命中
- `required_terms` 覆盖率
- 多查询召回带来的 `retrieval_boost`

目标不是做一个复杂学习排序器，而是先把“被 query 包装语稀释”的问题明显收敛。

## 非目标

- 不在这一步引入 cross-encoder reranker
- 不在这一步引入第二次 LLM query rewrite
- 不改现有 chunk schema
- 不改内置 MCP `wabity.rag.search` / `wabity.rag.query` 的输出结构

## 风险

- 启发式 stopword 规则天然不完美，可能对极短 query 或特殊命名产生误裁剪
- 多查询召回会增加一次检索中的候选规模；当前通过每 query 候选下限和最终统一裁剪控制成本
- 目前 query rewrite 仍主要覆盖中文/英文常见问句包装语，不是通用自然语言理解器

## 状态

- 2026-04-07：已落地 `QueryPlan`、多查询 embedding/FTS 召回、统一 rerank 加权，并补充单元测试与集成测试覆盖问句包装语剥离场景
