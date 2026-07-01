# Pi Agent 单运行时迁移执行方案（2026-06-29）

## 1. 决策

Wabity 不再作为通用 ACP client，也不再维护外部 `stdio` ACP agent 启动、握手、session mode、runtime config option 和 ACP session/load 恢复链路。后续 launcher 内的长期 agent 会话统一由 `pi_agent_rust` 提供的 `pi::sdk` 驱动。

这不是新增第二套运行时，而是删除 ACP 运行时并收敛到 Pi-only：

- 移除 `agent-client-protocol` 依赖。
- 移除 `codex-acp` / `claude-agent-acp` / `opencode acp` 这类外部 agent 预设和命令启动配置。
- 保留 launcher 轻量 RAG 问答，但它仍是短问答链路，不升级为 Pi session。
- 后续设计已更新：Wabity 不再暴露内置 MCP server；内置工具模块通过 Pi SDK ToolFactory 注入 Agent session。

## 2. 目标边界

### 2.1 必须完成

1. 文档边界从 ACP session 改为 Pi Agent session。
2. Rust 后端不再依赖 ACP crate，也不再启动外部 ACP agent 进程。
3. 新运行时通过 `pi_agent_rust` 的 `pi::sdk` 创建和驱动本地 agent session。
4. launcher 保持“普通执行 / RAG 问答 / Agent session”三条链路分离。
5. 设置页从 `ACP Agent` 改为 `Pi Agent`，删除外部 agent catalog、命令、启动模式和预设安装说明。
6. 旧 `[acp]` 配置只作为兼容读取和迁移提示来源；新运行时配置不得继续依赖外部 agent 命令。
7. 代码质量基线通过格式化、lint、测试；rust-analyzer diagnostics 若受 Tauri/proc macro 误报影响，必须明确记录差异。

### 2.2 第一阶段明确不做

1. 不保留 ACP 与 Pi SDK 双运行时 adapter。
2. 不实现远程 Pi RPC 子进程模式，除非 in-process SDK 被实证证明不可用。
3. 全局 MCP server 不再作为 Wabity 对外能力；内置工具模块转为 Pi SDK ToolFactory 注入。
4. 不把 RAG 问答历史和 Pi Agent session 历史混用。
5. 不做未授权 UI 重设计，只做 ACP -> Pi Agent 所需的必要删除和重命名。

## 3. Pi SDK 接入依据

`pi_agent_rust 0.1.18` 的 crate package 名是 `pi_agent_rust`，library 名是 `pi`。SDK 稳定入口为 `pi::sdk`。

关键 API：

- `SessionOptions`：配置 provider、model、api_key、thinking、enabled_tools、working_directory、session_dir、max_tool_iterations、事件回调。
- `create_agent_session(SessionOptions)`：创建 in-process agent session。
- `AgentSessionHandle::prompt` / `prompt_with_abort`：发送 prompt 和取消。
- `AgentEvent`：agent 生命周期、turn、message、tool execution、compaction/retry/extension 错误事件。
- `SessionTransport`：统一 in-process / RPC adapter。第一阶段优先使用 in-process。

推荐依赖：

```toml
pi_agent_rust = { version = "0.1.18", default-features = false }
```

理由：Wabity 是 Tauri 桌面应用，不需要 Pi 自带 TUI。`pi_agent_rust 0.1.18` 会通过 `sqlmodel-sqlite` 拉入 `libsqlite3-sys 0.37`；本仓库同步将 `rusqlite` 升到 `0.39`，确保同一依赖图里只有一个 `links = "sqlite3"`。第一阶段不启用 Pi SDK 自身 session 持久化语义，新 Pi Agent session 不写入旧 `[acp].saved_sessions`。

## 4. 目标架构

```text
launcher UI
  -> Tauri IPC
    -> AppState
      -> services/pi_agent
        -> pi::sdk::{create_agent_session, AgentSessionHandle, AgentEvent}
```

运行时职责：

- `services/pi_agent` 维护 session registry、active session、prompt 状态、取消句柄、消息投影和订阅通知。
- `domain/agent` 提供前后端共享的 Pi Agent session summary/detail/message/action 类型。
- `commands/pi_agent` 只做 IPC 参数边界和错误转换。
- `state` 负责 workspace、配置、持久化和 service 调度，不沉淀 Pi SDK 事件细节。

## 5. 事件映射

| Pi SDK 事件                    | Wabity 投影                              |
| ------------------------------ | ---------------------------------------- |
| `AgentStart`                   | session 进入 running                     |
| `TurnStart`                    | 新 assistant turn 开始                   |
| `MessageUpdate::TextDelta`     | 追加 `content` block                     |
| `MessageUpdate::ThinkingDelta` | 追加 `thought` block                     |
| `MessageUpdate::ToolCallEnd`   | 追加 tool call action                    |
| `ToolExecutionStart`           | action start                             |
| `ToolExecutionUpdate`          | action progress                          |
| `ToolExecutionEnd`             | action result                            |
| `TurnEnd`                      | assistant message 完成                   |
| `AgentEnd { error: None }`     | prompt finished，session idle            |
| `AgentEnd { error: Some(..) }` | prompt failed，session recoverable error |

保留原则：transcript 仍按真实事件顺序渲染，前端不得重排成“正文 / 工具 / thought”固定摘要。

## 6. 配置迁移

### 6.1 当前配置策略

第一阶段不新增 Wabity 自有 Pi Agent 配置块。`provider/model/api_key/thinking/tools/max_tool_iterations` 暂时交给 Pi SDK 自身配置解析；Wabity 只传入当前 workspace 作为 `working_directory`，避免在未验证 Pi SDK 配置语义前再造一套并行配置。

后续若需要桌面设置页显式控制 Pi Agent，必须新增独立 `agent` 配置块，表达 Pi Agent 运行时默认值，例如：

```toml
[agent]
provider = ""
model = ""
api_key = ""
thinking = "off"
enabled_tools = ["read", "grep", "find", "ls", "bash", "edit"]
max_tool_iterations = 16
```

`provider/model/api_key` 允许为空，空值时交给 Pi SDK 读取自身配置；Wabity 不伪造默认云模型。

### 6.2 旧 ACP 配置

旧 `[acp]` 中的 agent catalog、saved_sessions、active_session_id 不自动迁移，因为它们描述的是外部 ACP agent 命令，和 Pi SDK session 不是同一语义。

兼容策略：

1. 短期保留反序列化能力，避免用户升级后配置文件直接报错。
2. 不再执行旧 agent 命令，不再恢复旧 ACP saved session。
3. 设置页展示“旧 ACP 配置已停用”的迁移提示。
4. 后续版本可删除旧字段读取。

## 7. 实施阶段

### 阶段 A：文档和边界

- 更新 `ARCHITECTURE.md`：产品边界、外部系统、核心数据流、关键设计决策。
- 更新 `src-tauri/src/services/README.md`：`acp` 改为 `pi_agent`。
- 更新 `src-tauri/src/domain/README.md`：共享结构从 ACP session 改为 Pi Agent session。
- 更新 `src/features/launcher/README.md` 和 `src/features/settings/README.md`：UI 语义从 ACP Agent 改为 Pi Agent。

验收：全文不再把长期 agent 主链路描述为 ACP client。

### 阶段 B：后端运行时替换

- Cargo 移除 `agent-client-protocol`，加入 `pi_agent_rust`。
- 新建/改造 Pi Agent service 内部实现；第一阶段可保留旧 IPC/type 名称作为兼容层，但实现不得依赖 ACP。
- 删除外部 agent 命令构建、ACP JSON-RPC stream、permission handler、session mode/config option 映射。
- `AppState` 使用 Pi Agent service 创建、发送、取消、关闭 session。
- 旧 ACP session 恢复改为不恢复，并暴露迁移提示。

验收：Rust 不引用 `agent_client_protocol`，不启动 `codex-acp` 等外部 agent 命令。

### 阶段 C：前端设置和 launcher 调整

- 设置页删除 ACP agent catalog 表单、启动模式和预设。
- 第一阶段展示 Pi SDK 单运行时说明和旧 ACP 配置停用提示；暂不新增 Wabity 自有 Pi Agent 配置表单。
- launcher 文案、按钮、错误信息从 ACP 改为 Pi Agent。
- timeline 组件继续复用原布局，但类型名和文案改为 Agent/Pi Agent。
- 移除 session mode/config option 控件。

验收：用户不再看到 ACP Agent、codex-acp、claude-agent-acp、opencode acp 等入口。

### 阶段 D：清理和验证

- 清理废弃文件：`services/acp/command_builder.rs`、`services/acp/mapping.rs`、ACP 预设和相关测试。
- 全局搜索 `ACP` / `acp`，只允许迁移说明、旧配置兼容层或尚未重命名的兼容 IPC/type 路径中出现。
- 运行验证命令。

验收命令：

```bash
bun run lint
bun run test
cd src-tauri && cargo fmt --check
cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
cd src-tauri && cargo test
rust-analyzer diagnostics src-tauri --severity error
```

当前 `rust-analyzer diagnostics src-tauri --severity error` 会在 `tauri_panel!` proc macro 处误报 `E0282`；禁用 proc macro/build script 又会制造大量 Tauri 宏 false positives。因此本阶段必须记录诊断差异，不能把轻量诊断当作 cargo 门禁替代品。

## 8. 风险

1. `pi_agent_rust` 默认功能可能拉入与 Tauri 无关的 TUI 依赖：必须关闭 default features。
2. in-process SDK 可能与 Wabity Tokio runtime、全局配置路径或 tracing 初始化有冲突：若实证失败，再评估 `SessionTransport::rpc_subprocess`，但这属于第二选择，不是双运行时。
3. Pi SDK 的 provider 配置读取规则与 Wabity LLM provider 不同：第一阶段允许使用 Pi 自身配置，后续再决定是否把 Wabity 模型接入映射到 Pi provider。
4. 后续调整已移除 Wabity loopback MCP server；Agent 工具必须通过 Pi SDK ToolFactory 显式注入，不能偷偷回退成 HTTP MCP bridge。
5. 前端类型和文案迁移面大：必须优先保证编译和 IPC 契约一致，再做彻底命名清理。

## 9. 完成定义

- 代码不再依赖 ACP 协议 crate。
- 用户不能再配置或启动外部 ACP agent。
- 新 Pi Agent session 不写入旧 `[acp].saved_sessions`。
- launcher 只提供 Pi Agent session。
- 文档明确 Pi-only 单运行时边界。
- 验证命令通过，或对外部环境导致的失败给出可复现原因和后续处理项。
