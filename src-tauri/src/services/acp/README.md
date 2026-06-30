# services/acp

当前目录名仍保留为 `acp`，只是为了兼容既有 IPC、前端类型和配置迁移路径；运行时已经不再是 ACP client。

职责：

- 通过 `pi_agent_rust` 的 `pi::sdk` 创建内嵌 Pi Agent session
- 维护 session registry、active session、取消句柄和前端订阅 channel
- 将 Pi SDK `AgentEvent` 投影为现有 launcher timeline 可消费的 message block / action event
- 对旧 ACP saved session 只产生迁移提示，不再恢复、不再执行外部 agent 命令

边界：

- 不启动 `stdio` ACP agent 进程
- 不连接 `agent-client-protocol` crate
- 不透传全局 MCP server 到 Pi Agent session；RAG 问答仍按自己的 MCP 规则工作
- 不把 Pi Agent session 状态持久化进旧 `[acp].saved_sessions`
- `set_session_mode` 和 `set_session_config_option` 只保留 IPC 兼容错误，Pi Agent 不支持 ACP mode/config option

后续清理：

- 将目录、类型和 IPC 从 `acp` 逐步重命名为 `pi_agent`
- 增加显式 Pi Agent 配置模型后，再把 provider/model/tools 等设置接入 `SessionOptions`
