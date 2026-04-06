# 大文件拆分实施记录

## 背景

本次工作针对仓库内多个超过 1000 行、且同时承担多种变化轴的源码文件做职责拆分。

目标不是机械降行数，而是把以下边界收紧：

- 前端页面入口只保留页面壳层、少量装配和顶层流程分发
- 前端跨 feature 的 IPC client 按领域拆开，默认值与运行时封装分离
- Rust 运行时入口与并发/协议/配置归一化逻辑分模块收口
- README / `ARCHITECTURE.md` 同步反映新的稳定边界

## 范围

优先处理以下文件：

- `src/lib/tauri/client.ts`
- `src/features/settings/SettingsPage.tsx`
- `src/features/launcher/LauncherPage.tsx`
- `src-tauri/src/infrastructure/config.rs`
- `src-tauri/src/state/mod.rs`
- `src-tauri/src/services/acp/mod.rs`
- `src-tauri/src/services/rag_answer.rs`

## 分阶段计划

### 阶段 1

- 拆分 `src/lib/tauri/client.ts`
- 沉淀运行时封装、浏览器默认值、内置模板与 feature client

状态：已完成

### 阶段 2

- 拆分 `SettingsPage.tsx`
- 抽出导航、数据加载、持久化与分组草稿控制器

状态：已完成

### 阶段 3

- 拆分 `LauncherPage.tsx`
- 抽出 suggestions、sessions、clipboard、execution 控制器

状态：已完成

### 阶段 4

- 拆分 Rust `config/state/acp/rag_answer`
- 收紧 `mod.rs` / 根文件入口职责

状态：已完成

### 阶段 5

- 全量格式化、lint、测试与诊断
- 修正文档，记录阶段完成

状态：已完成

## 当前已完成拆分

- `src/features/settings/SettingsPage.tsx`：导航和持久化逻辑分别下沉到 `useSettingsSectionNavigation.ts`、`useSettingsPersistence.ts`
- `src/features/launcher/LauncherPage.tsx`：suggestions 状态机和异步补全请求下沉到 `useLauncherSuggestions.ts`
- `src-tauri/src/state/mod.rs`：session snapshot、设置校验/OCR provider、ACP/MCP 目录归一化分别拆到 `session_snapshot.rs`、`settings.rs`、`acp_catalog.rs`
- `src-tauri/src/infrastructure/config.rs`：ACP agent 命令解析、命名和 ID 生成下沉到 `config/acp_agent.rs`
- `src-tauri/src/services/acp/mod.rs`：命令构建和运行态映射分别下沉到 `command_builder.rs`、`mapping.rs`
- `src-tauri/src/services/rag_answer.rs`：进度事件投影下沉到 `progress.rs`

## 当前验证进度

- 前端：`npm run lint -- --quiet`、`npx tsc --noEmit` 已通过
- Rust：`cargo fmt --manifest-path src-tauri/Cargo.toml`、`cargo check --manifest-path src-tauri/Cargo.toml` 已通过
- 待执行：`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`、`cargo test --manifest-path src-tauri/Cargo.toml`、`rust-analyzer diagnostics ...`
