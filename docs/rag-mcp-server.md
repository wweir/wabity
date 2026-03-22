# RAG MCP Server

## 背景

现有 RAG 能力只服务于 launcher 内部的 `rag_answer` 动作。ACP agent 虽然已经能接收全局 `mcp_servers`，但没有一个可直接复用 Wabity 自身向量索引的 MCP server，因此 agent 无法把现有 LanceDB 当作外部知识源调用。

## 目标

1. 把现有 LanceDB 向量检索能力包装成一个本机 loopback MCP server。
2. transport 使用 `streamable HTTP`，这样它能直接作为 MCP 页面里的 `http` server 被 ACP agent 使用。
3. 不新建第二套索引或配置模型，只复用现有 `RagSettings`、embedding provider 和 LanceDB 数据目录。

## 设计

### Server 形态

- 当前仓库没有现成 HTTP listener，因此实现改成“统一内置 loopback HTTP 端口 + 路径路由”。
- Wabity 启动时在 `127.0.0.1:43189/internal/mcp/rag` 暴露内置 MCP server。
- 当前只实现 JSON response mode：`POST /internal/mcp/rag` 处理 JSON-RPC；`GET /internal/mcp/rag` 明确返回 `405`，不伪装 SSE 已可用。
- 只接受 loopback `Host` / `Origin`，避免把桌面内置 endpoint 暴露成任意来源都可打的本地开放接口。

### Tool 设计

- 暴露一个只读 tool：`wabity.rag.search`
- 输入：
  - `query`: 自然语言检索语句
  - `topK`: 返回的 chunk 数量上限，当前限制 `1..=20`
  - `minScore`: 最低相似度阈值，范围 `0..=1`
- 输出：
  - `query`
  - `hitCount`
  - `pendingIndexing`
  - `hits[]`，包含 `sourceRoot`、`absolutePath`、`path`、`chunkIndex`、`lineStart`、`lineEnd`、`paragraphLineStart`、`headingPath`、`text`、`distance`、`score`
- 只要本地 metadata 里仍存在 pending 文件，tool 就返回显式 error payload，并把当前 partial result 挂在结构化字段里；不能把部分命中伪装成稳定真相

### 复用边界

- 新增 `rag_query` 服务，只做 query embedding + LanceDB top-k 检索。
- `rag_mcp` 只负责 MCP 协议适配和 tool 包装。
- `rag_answer` 继续保留自己的问答裁剪与生成链路，不被这次需求强行重写。

## 进度

### 阶段 1：协议与运行时接入

- [x] 启动内置 loopback MCP HTTP server
- [x] 暴露 `initialize`、`ping`、`tools/list`、`tools/call`
- [x] 实现 `wabity.rag.search` tool

### 阶段 2：设置页接入

- [x] MCP 页面显示内置 server 的运行状态
- [x] MCP 页面在目录区提供内置 server 卡片，并用开关控制是否写入草稿

### 阶段 3：收尾

- [x] 更新架构与模块文档
- [x] 补最小单元测试
