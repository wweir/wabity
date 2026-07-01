# rag_answer 拆解记录（2026-03-30）

## 背景

`src-tauri/src/services/rag_answer.rs` 已经同时承担：

- 问答用例入口与结果投影
- `responses` / `chat/completions` 两套协议请求与响应解析
- 工具目录构建、兼容性回退与宿主机探测
- 内置工具执行、路径白名单与 citation 生成
- 对话状态续链与 continuation scope 管理

这不是“文件太长”问题，而是职责边界失效。

## 迁移目标

把 `rag_answer` 调整为“薄入口 + 子模块”的结构，让变化原因分离：

1. 入口编排
2. 对话状态
3. 结果投影
4. 解析辅助
5. 工具目录
6. 工具执行
7. `responses` 协议适配
8. `chat/completions` 协议适配

## 按文件创建顺序的迁移清单

1. `src-tauri/src/services/rag_answer/conversation_state.rs`
   承接 continuation scope、previous response id、conversation state reset/build。
2. `src-tauri/src/services/rag_answer/result.rs`
   承接最终 `ExecutionResult` 组装、payload 结构、citation 去重编号。
3. `src-tauri/src/services/rag_answer/parsing.rs`
   承接命令载荷提取、open intent 判断、JSON 参数解析与展示辅助。
4. `src-tauri/src/services/rag_answer/tool_catalog.rs`
   承接内置 function tool schema、host context 探测与兼容目录裁剪。
5. `src-tauri/src/services/rag_answer/tool_execute.rs`
   承接内置工具执行、路径白名单、citation 构造。
6. `src-tauri/src/services/rag_answer/protocol_responses.rs`
   承接 `responses` 请求构造、响应解析、function tool 兼容性缓存。
7. `src-tauri/src/services/rag_answer/protocol_chat.rs`
   承接 `chat/completions` 请求构造、消息构建与 tool call 解析。
8. 收缩 `src-tauri/src/services/rag_answer.rs`
   只保留共享类型、统一入口、两条协议的回合编排。

## 实施进度

### 阶段 1：骨架建立

- 状态：已完成
- 动作：
  - 新建迁移文档
  - 按顺序创建子模块文件
  - 根文件改为模块装配入口

### 阶段 2：模块迁移

- 状态：已完成
- 结果：
  - `conversation_state` 承接续链状态与 scope 计算
  - `result` 承接 payload 与 citation 编号
  - `parsing` 承接命令载荷、open intent 与展示辅助
  - `tool_catalog` 承接 tool schema 与 host context
  - `tool_execute` 承接内置工具执行与路径白名单
  - `protocol_responses` / `protocol_chat` 承接各自协议请求与解析

### 阶段 3：校验与文档同步

- 状态：已完成
- 校验：
  - `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
  - `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`
  - `cargo test --manifest-path src-tauri/Cargo.toml`
  - `rust-analyzer diagnostics src-tauri --severity error`
