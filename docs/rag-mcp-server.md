# Built-in MCP Server

## 背景

现有内置 MCP 入口最初只暴露 `RAG Query`，并把“是否启用内置能力”硬编码成一条普通 HTTP MCP server 草稿。这种建模有两个问题：

1. 同一个本地 endpoint 被伪装成普通外部 server，配置边界不清晰。
2. 后续如果继续增加内置工具，只能靠复制多条同 URL 的假 server 记录，无法表达模块级开关。

## 目标

1. 保持一个统一的本机 loopback MCP server，不拆成多个 server URL。
2. 内置能力按模块注册，当前至少拆成 `rag` 和 `document` 两个只读模块。
3. 设置页允许单独开关 server 和模块，但最终对外仍是同一个 built-in server。
4. 不新建第二套索引或文档缓存，只复用现有 `RagSettings`、`LlmSettings`、workspace root 和现有 RAG / 文档读取链路。

## 设计

### Server 形态

- 当前仓库没有现成 HTTP listener，因此实现改成“统一内置 loopback HTTP 端口 + 路径路由”。
- Wabity 启动时在 `127.0.0.1:43189/internal/mcp` 暴露内置 MCP server，并兼容保留旧的 `/internal/mcp/rag` 路径。
- 当前只实现 JSON response mode：`POST /internal/mcp` 处理 JSON-RPC；`GET /internal/mcp` 明确返回 `405`，不伪装 SSE 已可用。
- 只接受 loopback `Host` / `Origin`，避免把桌面内置 endpoint 暴露成任意来源都可打的本地开放接口。
- tool 调用路径直接消费运行态 `workspace root + RagSettings + LlmSettings + 内置模块配置` 快照；设置保存或 workspace 切换时由状态层同步更新，避免每次 `tools/call` 再回读 `ConfigStore`

### Tool 设计

- `rag` 模块：
  - 暴露 `wabity.rag.search`
  - 输入 `query`、可选 `topK`、`minScore`
  - 输出检索命中、路径、chunk 元数据和分数
  - 只要本地 metadata 里仍存在 pending 文件，就返回显式 error payload，并把 partial result 放进结构化字段
- `document` 模块：
  - 暴露 `wabity.read_file_lines`
  - 暴露 `wabity.read_document_excerpt`
  - 访问范围严格限制在当前 workspace root 和显式配置的 RAG source roots 内
  - 不提供任意路径浏览或目录枚举能力

### 复用边界

- `rag_query` 继续只做 query embedding + LanceDB top-k 检索。
- `builtin_mcp` 只负责 MCP 协议适配、模块注册和 tool 包装。
- `rag_answer` 继续保留自己的问答裁剪与生成链路，不被这次需求强行重写。
- `document_extract`、`rag::load_document_excerpt_for_chunk` 和路径白名单逻辑继续复用现有实现，不为 MCP 再复制一套读取栈。

## 进度

### 阶段 1：统一 server 与模块注册

- [x] 启动内置 loopback MCP HTTP server
- [x] 暴露 `initialize`、`ping`、`tools/list`、`tools/call`
- [x] 将内置 MCP 抽成统一 server + 模块注册表
- [x] 实现 `rag` 模块和 `document` 模块

### 阶段 2：配置与设置页接入

- [x] MCP 页面显示内置 server 的运行状态
- [x] MCP 页面在目录区提供内置 server 卡片，并用 server 开关 + 模块开关控制草稿
- [x] 配置层单独持久化内置 MCP 配置，而不是把同一 URL 写进普通 server 清单

### 阶段 3：收尾

- [x] 更新架构与模块文档
- [x] 跑格式化、lint、build、test、clippy 和 `rust-analyzer diagnostics`
