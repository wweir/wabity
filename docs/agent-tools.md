# Agent tools and MCP configuration

Wabity no longer exposes a local MCP server endpoint. The former built-in MCP modules are now treated as Agent-scoped tool modules.

## Scope

- The settings section is named **Agent 配置**.
- Built-in Wabity tools are configured as modules and injected into newly created Pi Agent sessions through the Pi Rust SDK `ToolFactory` hook.
- Wabity does not bind `127.0.0.1:43189/internal/mcp` during normal startup.
- Launcher document Q&A does not read the global MCP server catalog. Its local RAG/document/open tools remain part of the question-answer backend directly.
- Legacy saved Agent session snapshots are reconciled by removing the old Wabity loopback MCP server entry instead of re-adding it.

## Built-in tool modules

The built-in module toggle controls only future Agent sessions:

- `rag`: registers `wabity.rag.search` for local vector index search.
- `document`: registers `wabity.read_file_lines` and `wabity.read_document_excerpt` for workspace/RAG-source-bounded document reads.

Tool execution keeps the same local safety boundaries as the former MCP implementation:

- File access is limited to the active workspace root and explicitly configured RAG source roots.
- RAG search uses the existing USearch + SQLite index and does not create another indexing path.
- Tool results are converted to Pi SDK `ToolOutput` blocks; structured payloads are kept in `details`.

## Custom MCP service catalog

The custom `stdio` / `http` / `sse` MCP service catalog remains persisted as Agent configuration data. It is not consumed by the launcher RAG answer path and is not exposed by Wabity as a proxy server.

If Wabity later bridges third-party MCP services into Pi sessions, that bridge must be implemented explicitly at the Agent tool layer. Do not make the launcher question-answer backend consume this catalog implicitly.

## Deprecated behavior

The following behavior is obsolete:

- Starting a Wabity loopback MCP server at `127.0.0.1:43189`.
- Showing a local MCP endpoint in settings.
- Adding Wabity's built-in loopback MCP server to effective global MCP server lists.
- Injecting global HTTP/SSE MCP servers into launcher document Q&A requests.
