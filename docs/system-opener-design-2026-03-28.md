# System Opener Design 2026-03-28

## Goal

为 launcher 增加统一的系统打开能力，让 `/open` 不再只处理 URL，而是同时支持：

- 显式 URL 和裸域名
- 当前 workspace 相对路径
- 绝对路径
- `~` 路径

同时避免把平台差异直接散落在前端或 `ApplicationService` 里。

## Design

- 在 `src-tauri/src/infrastructure/opener.rs` 收敛系统 opener 调用，统一复用 `tauri-plugin-opener`
- 新增 `src-tauri/src/services/open_target.rs`，负责 `/open` 的目标解析、路径归一化和错误语义
- `AppState::execute_action_with_progress` 对 `open_target` 做运行时分发，不把有副作用的打开能力塞进纯逻辑 `executor`
- `open_document_reference` 和 `ApplicationService::launch` 也复用同一条 opener 链路
- 问答链路把 opener 额外封装成内置工具 `wabity.system.open` 注入给模型，但运行时不会把它当成普通读工具放开；只有当前问题明确要求“打开”时才允许执行
- `wabity.system.open` 的本地路径能力继续复用路径白名单：只允许当前 workspace 和显式配置的 RAG source roots，不允许模型借问答链路探测任意本地路径
- `wabity.system.open` 的 tool description 在构建问答请求时动态拼装当前宿主机的操作系统、版本，以及 PATH 上检测到的包管理器列表，减少模型对平台能力的错误假设

## Status

- 2026-03-28：设计完成并已落地代码
- 2026-03-28：问答内置工具 `wabity.system.open` 已接入 `rag_answer`，并增加“显式打开意图 + 路径白名单”运行时约束
- 2026-03-28：`wabity.system.open` 的问答工具说明已改为动态宿主机上下文文案
- 2026-03-28：已同步更新 `ARCHITECTURE.md`、`src-tauri/src/services/README.md`、`src/features/launcher/README.md`
