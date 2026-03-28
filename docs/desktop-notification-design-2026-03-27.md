# Desktop Notification Design

Date: 2026-03-27
Scope: 轻量问答完成通知、ACP Agent prompt 完成通知
Status: implemented (macOS v1)

## 目标

补一条桌面系统通知能力，在下面两类后台完成点上提醒用户：

- launcher 内轻量问答完成
- ACP Agent 单次 prompt 执行完成

当前阶段只落地 macOS，但接口和模块边界必须从第一版就为 Windows / Linux 预留扩展口。

## 非目标

- 不做前端 toast 或应用内红点替代系统通知
- 不把“session 创建成功”“session 恢复成功”“普通流式 chunk 到达”当成完成通知
- 不在 v1 引入通知模板系统、声音、分组、点击跳转深链
- 不先做 `osascript` 临时方案再重构

最后一条必须明确。`osascript display notification` 看起来快，实际上是在主动制造技术债：

- 它只覆盖 macOS
- 通知权限、能力和后续 Windows/Linux 实现无法共用一套抽象
- Shell 转义和错误处理脆弱
- 最终仍要回到 Tauri 官方通知能力，等于做两遍

如果目标从一开始就是“macOS 先落地，后续扩到其他桌面平台”，那第一版就不该选死路。

## 现状

当前仓库里已经有两条明确的“完成”边界：

### 1. 轻量问答

- `commands/launcher.rs::execute_action` 调 `AppState::execute_action_with_progress`
- `AppState::execute_action_with_progress` 在 `action_id == "rag_answer"` 时进入 `question_answer_backend::answer_question`
- 问答进度通过 `execution-progress` event 推给前端
- 最终完成点是 `answer_question(...)` 返回 `ExecutionResult` 或错误

这意味着问答通知应该挂在 Rust 侧最终返回边界，而不是前端的进度订阅。

### 2. ACP Agent

- `AcpService` 维护 session runtime event loop
- `PromptFinished` / `PromptFailed` 已经是明确的 prompt 终止事件
- `SessionExited` 表示 agent 进程退出，不等同于一次 prompt 正常完成

这意味着 ACP 通知应该挂在 `AcpService::apply_runtime_event(...)` 的终态分支，而不是前端 session panel 更新逻辑。

## 完成语义

这是这个需求里最容易说模糊、实现后变噪音的点。

### 1. 轻量问答完成

定义为：

- `rag_answer` 成功返回 `ExecutionResult`
- 或 `rag_answer` 以错误结束

不包括：

- `execution-progress` 中间态
- 普通 slash 动作
- 应用搜索、文件搜索、应用启动

### 2. ACP 执行完成

定义为单次 prompt 的完成，而不是整个 session 的生命周期变化：

- `PromptFinished`：成功完成
- `PromptFailed`：可恢复失败，也算“完成”
- `SessionExited`：只有当它打断了一个运行中的 prompt 时，才映射成失败完成通知

不包括：

- `create_session`
- `activate_session`
- `restore_session`
- 普通 `AssistantChunk` / `ThoughtChunk` / `ActionEvent`
- 空闲态下用户主动关闭 session

## 交互策略

### 1. 默认只在后台态通知

如果 launcher 正在前台且用户已经能直接看到结果，再弹系统通知就是噪音。

推荐 v1 策略：

- 只有在 launcher 不处于有效可见前台态时才发系统通知

这里不要偷懒只看一个前端状态位。macOS 下当前 launcher 是高层级 `NSPanel`，已有一整套原生可见性和焦点补偿逻辑。通知门槛应该复用 Rust 侧窗口状态判断，而不是让前端自己猜。

### 2. 默认不透出答案正文

系统通知本身就是外部输出。把问答答案、Agent 原文、工具输出直接塞进通知，是主动泄漏内容。

推荐 v1 的通知内容策略：

- title 只放结果类型
- body 只放低敏感摘要
- 不放完整答案
- 不放原始工具输出
- 不放未脱敏 provider 错误

建议文案：

- 轻量问答成功：`Wabity 问答已完成`
- 轻量问答失败：`Wabity 问答失败`
- ACP 成功：`Wabity Agent 执行已完成`
- ACP 失败：`Wabity Agent 执行失败`

body 示例：

- `返回 launcher 查看结果`
- `Codex 已完成当前任务`
- `执行失败，请回到 launcher 查看详情`

如果以后真要展示更多内容，必须做成显式 opt-in，而不是 v1 默认暴露。

## 推荐方案

### 1. 分层

#### domain

新增独立通知模型，保持前后端和平台实现之间的稳定语义边界。

建议新增：

- `src-tauri/src/domain/notification.rs`

建议模型：

```rust
pub enum NotificationTrigger {
    QuestionAnswer,
    AcpPrompt,
}

pub enum NotificationOutcome {
    Success,
    Error,
}

pub struct CompletionNotification {
    pub trigger: NotificationTrigger,
    pub outcome: NotificationOutcome,
    pub title: String,
    pub body: String,
}
```

这里故意不要先做成巨大的模板系统。当前只需要稳定表达“什么完成了、结果成败、发什么文案”。

#### infrastructure

新增平台通知适配层，专门负责“怎么把通知发给操作系统”。

建议新增：

- `src-tauri/src/infrastructure/notification/mod.rs`
- `src-tauri/src/infrastructure/notification/macos.rs`
- `src-tauri/src/infrastructure/notification/noop.rs`

建议 trait：

```rust
pub trait SystemNotificationBackend: Send + Sync {
    fn request_permission_if_needed(&self) -> anyhow::Result<NotificationPermissionState>;
    fn permission_state(&self) -> NotificationPermissionState;
    fn notify(&self, payload: &CompletionNotification) -> anyhow::Result<()>;
}
```

当前实现：

- macOS: 真正实现
- Windows/Linux: 先走 `NoopNotificationBackend`

注意，虽然当前阶段只实现 macOS，但 trait 必须从第一版就跨平台。否则你后面加 Windows/Linux 时，业务层一定会被平台细节反向污染。

#### services

新增一个通知编排服务，专门负责“该不该发、发哪种文案、什么时候跳过”。

建议新增：

- `src-tauri/src/services/notification.rs`

职责：

- 读取通知设置
- 判断当前是否处于后台态
- 把业务完成事件映射成 `CompletionNotification`
- 调用 `SystemNotificationBackend`

它不负责：

- 直接判断 ACP 协议事件
- 直接构造窗口状态
- 直接读写配置文件

### 2. 配置模型

不要把通知字段硬塞成几个松散布尔值挂在前端本地 state 里。通知是系统能力，不是临时 UI 偏好。

建议新增顶层配置：

```rust
pub struct NotificationSettings {
    pub enabled: bool,
    pub notify_question_answer_completion: bool,
    pub notify_acp_prompt_completion: bool,
    pub only_when_launcher_in_background: bool,
    pub content_preview: NotificationContentPreview,
}
```

挂载位置：

- Rust: `domain/settings.rs::AppSettings`
- Rust: `infrastructure/config.rs::AppConfig`
- TS: `src/lib/tauri/types.ts::AppSettings`

不建议把这些字段直接塞进 `GeneralSettings`。原因很简单：

- 通知本身已经是一个独立系统能力
- 未来大概率会扩出权限状态、平台支持状态、更多触发器
- 现在就把它和 `autoStart/showInDock/language` 混成一坨，会让后续扩展更脏

UI 上可以继续放在设置页“通用”分组里，但数据模型不该跟着 UI 分组走。

推荐默认值：

- `enabled = false`
- `notify_question_answer_completion = true`
- `notify_acp_prompt_completion = true`
- `only_when_launcher_in_background = true`
- `content_preview = brief`

`enabled` 默认关掉是更稳妥的选择。系统通知会打断用户，不该默认替用户做决定。

### 3. 权限模型

macOS v1 的实际实现要比最初设想更保守。当前 Tauri desktop notification 路径没有提供可信的“应用内权限请求”反馈，因此设置页不能继续伪装成自己能可靠拉起和判断系统授权状态。

当前策略：

- 设置页保留“通知”卡片
- 设置页只展示系统设置引导，不提供未验证的权限请求按钮
- 真正发通知时仍保留后端的非阻塞状态检查与错误日志
- 如果用户在系统层禁用了通知，由系统自身决定是否静默丢弃
- macOS 通知发送直接走原生通知库，并优先把发送者应用绑定到 `Wabity` 的 bundle identifier，让系统使用 `Wabity` 自己的应用图标；如果当前运行方式下系统拒绝这次绑定，则只降级图标归属，不允许把通知本身整条吞掉

换句话说，v1 先保证“功能真实”，而不是为了看起来完整去提供伪能力。

### 4. 触发点

#### 轻量问答

推荐挂在 `AppState::execute_action_with_progress(...)` 的 `rag_answer` 分支。

原因：

- 这里已经是问答用例的 Rust 汇合点
- 成功和失败都能拿到
- 不需要前端重复订阅、判断、去重

设计：

- `rag_answer` 成功返回后，调用 `NotificationService::notify_question_answer_success(...)`
- `rag_answer` 返回错误时，调用 `NotificationService::notify_question_answer_error(...)`

#### ACP Agent

推荐挂在 `AcpService::apply_runtime_event(...)`。

原因：

- 这里只有它最清楚 `PromptFinished`、`PromptFailed`、`SessionExited` 的真实语义
- 前端拿到的只是汇总后的 session detail，不适合再反推出“这是不是一次新的 prompt 完成”

设计：

- `PromptFinished` -> 成功通知
- `PromptFailed` -> 失败通知
- `SessionExited` -> 仅当退出前 session 处于 `Running`，或存在当前 pending assistant turn 时，映射为失败通知

### 5. 后台态判断

推荐新增统一 helper，由 Rust 侧窗口基础设施提供：

- `infrastructure::window::should_emit_background_notification(...)`

输入：

- `AppHandle`
- `ShortcutRuntimeState`
- 可选 session 是否 active

判定规则：

- 如果配置要求仅后台通知，则 launcher 处于有效前台态时跳过
- macOS 下复用现有窗口/面板状态判断，不重新发明一套“是否可见”猜测逻辑

不要把这个判断下放前端。当前窗口可见性问题本来就已经在 Rust 侧处理，前端再猜一次只会让条件分叉。

## 技术选型

推荐直接使用 Tauri 官方通知插件，而不是命令行或浏览器 Notification API。

原因：

- 它本身就是桌面通知官方能力
- macOS / Windows / Linux 有统一抽象
- 权限检查和请求有正式 API
- 后续不需要推倒重来

接入面：

- `src-tauri/Cargo.toml` 增加 `tauri-plugin-notification`
- `package.json` 增加 `@tauri-apps/plugin-notification`
- `src-tauri/src/app.rs` 注册插件
- `src-tauri/capabilities/default.json` 增加通知插件权限

关于 capability 里的具体权限项，建议以 `tauri add notification` 生成结果为准，不要手写猜测值后长期漂移。

## 为什么不放前端

看起来前端最容易做，但这是错误落点。

问题有四个：

1. 完成语义在 Rust 侧，不在 React state 里
2. 前端只拿到投影后的事件，容易把中间态和完成态混淆
3. macOS 下窗口显示/焦点判断已经被 Rust 接管，前端重复判断会分叉
4. 后续 Windows/Linux 接入会把平台能力继续堆到前端，边界更烂

结论很直接：

- 前端只负责设置项、内容粒度和系统设置引导 UI
- 通知判断和发送必须在 Rust 侧

## 推荐实施阶段

### Phase 1: 基础设施落地

- 接入 Tauri notification plugin
- 增加 `NotificationSettings`
- 增加 `NotificationService`
- 增加 macOS backend 和 `Noop` backend
- 设置页新增通知配置、摘要粒度选择和系统设置引导

### Phase 2: 问答通知

- 在 `rag_answer` 成功/失败边界发通知
- 加测试覆盖“前台跳过 / 后台发送 / 权限拒绝静默跳过”

### Phase 3: ACP 通知

- 在 `PromptFinished` / `PromptFailed` 发通知
- 在运行中异常 `SessionExited` 发失败通知
- 补充测试覆盖“空闲退出不通知”

### Phase 4: 其他平台

- Windows backend
- Linux backend
- 平台差异文档和设置页支持状态提示

## 测试建议

### Rust 单元测试

- `NotificationSettings` 默认值和配置解析
- `NotificationService` 对不同 trigger / outcome / 前后台态的决策
- 敏感信息不进入通知正文

### Rust 集成测试

- `rag_answer` 成功后触发通知
- `rag_answer` 失败后触发通知
- `PromptFinished` 触发 ACP 成功通知
- `PromptFailed` 触发 ACP 失败通知
- 空闲 `SessionExited` 不通知

### 手工验证

macOS：

- launcher 在前台，完成问答，不通知
- launcher 隐藏后完成问答，通知
- launcher 隐藏后 Agent 完成 prompt，通知
- 权限拒绝时无崩溃、无重复弹窗

## 结论

推荐方案很明确：

- 技术选型上直接用 Tauri 官方通知插件
- 架构上把“通知发送”放 `infrastructure`，“通知策略”放 `services`
- 触发点上把轻量问答挂在 `rag_answer` 返回边界，把 ACP 挂在 `PromptFinished/PromptFailed` 终态事件
- 默认只在后台态通知
- 默认只发低敏感摘要，不暴露答案正文

这条方案的重点不是“先把 macOS 弄响一声”，而是第一版就把边界立对。否则你后面加 Windows/Linux 时，必然要把通知逻辑从前端、命令层或 shell hack 里再拆出来重做一遍。
