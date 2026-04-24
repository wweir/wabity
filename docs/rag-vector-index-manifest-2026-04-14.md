# RAG Vector Index Manifest 设计

## 背景

2026-04-14 的完整性修复把启动校验从“USearch 能否 load”提升成了“逐条导出所有 active 向量并校验覆盖”。

正确性是补上了，但代价也很明显：

- 冷启动会对每个 active `vector_key` 做一次 `export`
- 索引越大，启动和 watcher 重启越慢
- 正确性问题被修成了性能问题

根因不是 `embedding_fingerprint`，而是缺少“active 向量集合的轻量完整性摘要”。

后续实现里如果只保留少量固定 probe 的抽查，这个假设仍然不成立：非 probe 向量被污染时会被静默放过。抽样只能提供信号，不能提供正确性证明。

## 决策

引入两层额外元数据，但不改变“SQLite 真相源 + USearch 派生文件”的总架构：

1. SQLite 新增单行元数据表 `rag_vector_index_meta`
2. USearch 旁边新增 sidecar manifest `rag-chunks.manifest.json`

两者都表达同一批 active 向量集合的轻量摘要：

- `active_vector_count`
- `vector_dimensions`
- `key_xor`
- `key_sum`
- `key_hash_xor`
- `key_hash_sum`

其中：

- SQLite 元数据是真相侧的增量摘要，跟随 active 集合增删切换一起事务更新
- `rag_chunks` 行本身会持久化每条向量的稳定 `vector_hash`，供异常路径做完整值校验
- manifest 是 USearch 成功落盘后的派生快照，额外记录：
  - `index_size_bytes`
  - `index_modified_at_ms`
  - `index_md5_hex`
  - 少量确定性 probe 向量的 `vector_key + vector_hash`

## 启动校验策略

稳定路径：

1. 读取 SQLite `rag_vector_index_meta`
2. 读取 sidecar manifest
3. 校验摘要字段一致
4. 校验索引文件可 load
5. 校验 `index.size()` 与 active 向量数一致
6. 逐条导出 manifest 里的少量 probe 向量，校验哈希一致
7. 计算当前 `.usearch` 整文件摘要，并与 manifest 里的 `index_md5_hex` 对账，兜底同大小/同 mtime 的非 probe 污染
8. 若缺 manifest、旧版 manifest 迁移或缺 digest 字段，则回退一次完整覆盖校验，用 SQLite 持久化的 `vector_hash` 对账所有 active 向量；通过后补写带 digest 的 manifest

这样正常启动路径不再逐条 `export` active 向量；完整值校验只在 manifest 缺失、旧版升级或缺 digest 字段时触发。稳定启动路径仍需要顺序扫描 `.usearch` 文件计算摘要，这是为了解决“非 probe 向量同大小/同 mtime 污染”的正确性缺口；写索引时则不再额外回读整份文件算 digest，把线性扫描成本收敛到启动校验侧。

迁移路径：

- 若旧索引没有 sidecar manifest，但 USearch 可 load，则只在这一次回退到完整覆盖校验
- 校验通过后补写 manifest
- 后续启动走轻量路径

## 预期收益

- 正常启动从 O(active_vectors \* dimensions) 降为“少量 probe 导出 + 一次顺序文件摘要”
- 缺 key、错维度、probe 向量内容漂移、manifest 过期，以及同大小/同 mtime 的非 probe 污染，都能被及时判脏
- 旧索引不会因为升级缺 manifest 而被直接误判损坏

## 影响范围

- `src-tauri/src/services/rag/storage.rs`
- `docs/rag-metadata-cache.md`
- `ARCHITECTURE.md`
- `src-tauri/src/services/README.md`

## 状态

- 2026-04-14：方案实施完成；同日补上行级 `vector_hash` 迁移与 manifest digest 兜底，避免旧索引因加列被整库重置，也避免同大小/同 mtime 的非 probe 污染被静默放过
