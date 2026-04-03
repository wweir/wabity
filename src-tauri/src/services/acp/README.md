# acp

职责：

- 启动本地 `stdio` ACP agent 进程
- 建立 launcher 作为 ACP client 的连接
- 管理 session 生命周期、消息流和状态同步
- 将后台 session 更新投影成前端可消费的结构化事件
- 将全局 MCP server 列表透传到 `session/new` / `session/load`
- 对 `session/update` 和 `session/prompt` response 统一从 ACP SDK 的原始有序 stream 消费，再映射成内部 runtime event，避免 SDK 回调层并发派发造成消息块乱序或错误收口
- `SessionUpdate::ToolCall` / `ToolCallUpdate` 会把 ACP `tool_call_id` 投影为前端 action event 的 `correlation_id`，供 UI 可靠融合 tool call / tool result，而不是退化成按标题猜
- 由于 ACP 稳定协议目前只能稳定表达 turn 边界和 `tool_call_id`，不能稳定标注更高层的语义段落，后端当前只会把“紧邻且同类型”的 assistant chunk 合并成连续 block；一旦中间穿插 tool call / tool update / thought / content，就必须保留原始 block 顺序，前端据此还原真实时间线，而不是再压成固定的 `thought/actions/content` 三段摘要

当前约束：

- 只支持本地 `stdio` transport
- 当前不声明 `fs.readTextFile`、`fs.writeTextFile`、`terminal` 等 client capability
- 当前不实现 MCP client/bridge；MCP server 仍由 ACP agent 自己连接
- MCP server 配置当前支持 `stdio/http/sse`；其中 `http/sse` 会在 ACP initialize 后按 agent 返回的 `mcp_capabilities` 做能力校验
- 如果 agent 仍主动请求权限或工具调用，服务会记录系统消息并拒绝
- 不直接以 `prompt().await` 返回作为 assistant turn 结束信号；真正的 `PromptFinished` / `PromptFailed` 必须和同一条有序 stream 上的 `session/update` 一起判断
- 每个 session 在创建时绑定一个 workspace；切换全局 workspace 不会漂移旧 session
- 可以同时维护多个已配置 agent，但每个 session 仍然只绑定一个 agent 配置
- session 摘要会显式区分 `PromptFailed` 这种可恢复错误和 `SessionExited` 这种不可恢复错误，供前端通知光点稳定映射
- agent 启动支持三种显式模式：`direct` 直接执行 `program + args`，`login_shell` 通过用户默认 shell 的 login 模式执行单行命令，`interactive_shell` 再额外打开 interactive shell；后者兼容 `.zshrc` / `.bashrc` 的概率更高，但任何 stdout 输出都可能污染 ACP `stdio` 握手
- session 快照持久化时会保存创建它的 agent 信息；后续即使默认 agent 改了，也不会把旧 session 恢复成别的 agent
