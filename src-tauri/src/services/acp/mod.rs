use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use pi::sdk::{create_agent_session, AbortHandle, AgentEvent, AgentSessionHandle, SessionOptions};
use tauri::ipc::Channel;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};

use crate::domain::acp::{
    AcpActionEvent, AcpAgentConfig, AcpMcpServerConfig, AcpMessageBlock, AcpMessageRole,
    AcpRestoreNotice, AcpSessionDetail, AcpSessionErrorLevel, AcpSessionMessage,
    AcpSessionRuntimeState, AcpSessionStatus, AcpSessionSummary,
};
use crate::infrastructure::config::SavedAcpSession;
use crate::services::notification::NotificationService;

pub type SessionPersistHook = Arc<
    dyn Fn(
            AcpSessionSummary,
            AcpAgentConfig,
            Vec<AcpMcpServerConfig>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct AcpService {
    sessions: Arc<RwLock<HashMap<String, SessionRecord>>>,
    active_session_id: Arc<RwLock<Option<String>>>,
    restore_notices: Arc<RwLock<Vec<AcpRestoreNotice>>>,
    runtime_event_tx: mpsc::UnboundedSender<RuntimeEvent>,
    runtime_event_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<RuntimeEvent>>>>,
    session_update_channels: Arc<RwLock<Vec<Channel<AcpSessionDetail>>>>,
    session_removal_channels: Arc<RwLock<Vec<Channel<String>>>>,
    session_persist_hook: Option<SessionPersistHook>,
    notification: NotificationService,
}

struct SessionRecord {
    agent: AcpAgentConfig,
    mcp_servers: Vec<AcpMcpServerConfig>,
    summary: AcpSessionSummary,
    runtime: AcpSessionRuntimeState,
    messages: Vec<AcpSessionMessage>,
    next_message_id: u64,
    handle: Arc<Mutex<AgentSessionHandle>>,
    current_abort: Option<AbortHandle>,
}

#[derive(Clone)]
pub struct SessionRuntimeConfig {
    pub agent: AcpAgentConfig,
    pub mcp_servers: Vec<AcpMcpServerConfig>,
}

pub struct RestoreAttemptResult {
    pub restored: Option<AcpSessionDetail>,
    pub keep_snapshot: bool,
}

#[derive(Debug)]
enum RuntimeEvent {
    TextDelta {
        session_id: String,
        delta: String,
    },
    ThinkingDelta {
        session_id: String,
        delta: String,
    },
    ToolEvent {
        session_id: String,
        kind: String,
        title: String,
        correlation_id: Option<String>,
        detail: Option<String>,
    },
    PromptFinished {
        session_id: String,
    },
    PromptFailed {
        session_id: String,
        error: String,
    },
}

impl AcpService {
    pub fn new(
        notification: NotificationService,
        session_persist_hook: Option<SessionPersistHook>,
    ) -> Self {
        let (runtime_event_tx, runtime_event_rx) = mpsc::unbounded_channel();
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            active_session_id: Arc::new(RwLock::new(None)),
            restore_notices: Arc::new(RwLock::new(Vec::new())),
            runtime_event_tx,
            runtime_event_rx: Arc::new(Mutex::new(Some(runtime_event_rx))),
            session_update_channels: Arc::new(RwLock::new(Vec::new())),
            session_removal_channels: Arc::new(RwLock::new(Vec::new())),
            session_persist_hook,
            notification,
        }
    }

    pub fn start_event_loop(&self) {
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut receiver = {
                let mut guard = service.runtime_event_rx.lock().await;
                guard.take()
            };
            let Some(mut receiver) = receiver.take() else {
                return;
            };
            while let Some(event) = receiver.recv().await {
                if let Err(error) = service.apply_runtime_event(event).await {
                    tracing::warn!(
                        error = format_args!("{:#}", error),
                        "failed to apply Pi Agent event"
                    );
                }
            }
        });
    }

    pub async fn subscribe_session_updates(&self, channel: Channel<AcpSessionDetail>) {
        self.session_update_channels.write().await.push(channel);
    }

    pub async fn unsubscribe_session_updates(&self, channel_id: u32) {
        self.session_update_channels
            .write()
            .await
            .retain(|channel| channel.id() != channel_id);
    }

    pub async fn subscribe_session_removals(&self, channel: Channel<String>) {
        self.session_removal_channels.write().await.push(channel);
    }

    pub async fn unsubscribe_session_removals(&self, channel_id: u32) {
        self.session_removal_channels
            .write()
            .await
            .retain(|channel| channel.id() != channel_id);
    }

    pub async fn list_sessions(&self) -> Vec<AcpSessionSummary> {
        let sessions = self.sessions.read().await;
        let mut summaries = sessions
            .values()
            .map(|record| record.summary.clone())
            .collect::<Vec<_>>();
        summaries.sort_by_key(|summary| std::cmp::Reverse(summary.last_updated_at_ms));
        summaries
    }

    pub async fn session_detail(&self, session_id: &str) -> Option<AcpSessionDetail> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(record_detail)
    }

    pub async fn session_runtime_config(&self, session_id: &str) -> Option<SessionRuntimeConfig> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(|record| SessionRuntimeConfig {
            agent: record.agent.clone(),
            mcp_servers: record.mcp_servers.clone(),
        })
    }

    pub async fn create_session(
        &self,
        workspace_root: PathBuf,
        agent: AcpAgentConfig,
        mcp_servers: Vec<AcpMcpServerConfig>,
    ) -> Result<AcpSessionDetail> {
        let session_id = format!("pi-agent-{}", now_ms());
        let title = workspace_root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("Pi Agent")
            .to_string();
        let handle = create_agent_session(SessionOptions {
            working_directory: Some(workspace_root.clone()),
            no_session: false,
            ..SessionOptions::default()
        })
        .await
        .context("failed to create Pi Agent session")?;

        let summary = AcpSessionSummary {
            session_id: session_id.clone(),
            workspace_root: workspace_root.to_string_lossy().into_owned(),
            title,
            agent_id: Some(agent.id.clone()).filter(|value| !value.is_empty()),
            agent_name: display_agent_name(&agent),
            status: AcpSessionStatus::Idle,
            error_level: None,
            attention: false,
            is_active: false,
            last_error: None,
            last_updated_at_ms: now_ms(),
        };
        let record = SessionRecord {
            agent,
            mcp_servers,
            summary,
            runtime: AcpSessionRuntimeState::default(),
            messages: Vec::new(),
            next_message_id: 1,
            handle: Arc::new(Mutex::new(handle)),
            current_abort: None,
        };

        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session_id.clone(), record);
        }
        self.activate_session(Some(session_id.clone())).await;
        let detail = self
            .session_detail(&session_id)
            .await
            .with_context(|| format!("unknown Pi Agent session after creation: {session_id}"))?;
        self.emit_session_update(&detail).await;
        Ok(detail)
    }

    pub async fn restore_session(&self, snapshot: SavedAcpSession) -> RestoreAttemptResult {
        self.restore_notices.write().await.push(AcpRestoreNotice {
            session_id: snapshot.session_id,
            workspace_root: snapshot.workspace_root,
            message: "旧 ACP session 快照不会迁移到 Pi Agent；请创建新的 Pi Agent session。"
                .to_string(),
        });
        RestoreAttemptResult {
            restored: None,
            keep_snapshot: false,
        }
    }

    pub async fn activate_session(&self, session_id: Option<String>) -> Vec<AcpSessionSummary> {
        let mut active_session_id = self.active_session_id.write().await;
        *active_session_id = session_id.filter(|id| !id.trim().is_empty());
        let active = active_session_id.clone();
        drop(active_session_id);

        let mut sessions = self.sessions.write().await;
        for record in sessions.values_mut() {
            record.summary.is_active =
                active.as_deref() == Some(record.summary.session_id.as_str());
        }
        let mut summaries = sessions
            .values()
            .map(|record| record.summary.clone())
            .collect::<Vec<_>>();
        summaries.sort_by_key(|summary| std::cmp::Reverse(summary.last_updated_at_ms));
        summaries
    }

    pub async fn send_prompt(&self, session_id: &str, prompt: String) -> Result<AcpSessionDetail> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            anyhow::bail!("Pi Agent prompt 不能为空");
        }

        let (handle, abort_signal) = {
            let mut sessions = self.sessions.write().await;
            let record = sessions
                .get_mut(session_id)
                .with_context(|| format!("unknown Pi Agent session: {session_id}"))?;
            ensure_prompt_allowed(&record.summary.status)?;
            record.summary.status = AcpSessionStatus::Running;
            record.summary.error_level = None;
            record.summary.attention = false;
            record.summary.last_error = None;
            record.summary.last_updated_at_ms = now_ms();
            let user_id = next_message_id(record);
            record.messages.push(AcpSessionMessage {
                id: user_id,
                role: AcpMessageRole::User,
                blocks: vec![AcpMessageBlock::Content {
                    text: prompt.clone(),
                }],
                pending: false,
            });
            let assistant_id = next_message_id(record);
            record.messages.push(AcpSessionMessage {
                id: assistant_id,
                role: AcpMessageRole::Assistant,
                blocks: Vec::new(),
                pending: true,
            });
            let (abort_handle, abort_signal) = AgentSessionHandle::new_abort_handle();
            record.current_abort = Some(abort_handle);
            (record.handle.clone(), abort_signal)
        };

        if let Some(detail) = self.session_detail(session_id).await {
            self.emit_session_update(&detail).await;
        }

        let event_tx = self.runtime_event_tx.clone();
        let callback_session_id = session_id.to_string();
        let result = handle
            .lock()
            .await
            .prompt_with_abort(prompt, abort_signal, move |event| {
                forward_agent_event(&event_tx, &callback_session_id, event);
            })
            .await;

        match result {
            Ok(_) => {
                let _ = self.runtime_event_tx.send(RuntimeEvent::PromptFinished {
                    session_id: session_id.to_string(),
                });
            }
            Err(error) => {
                let _ = self.runtime_event_tx.send(RuntimeEvent::PromptFailed {
                    session_id: session_id.to_string(),
                    error: error.to_string(),
                });
            }
        }

        let (respond_to, respond_rx) = oneshot::channel();
        let session_id_for_wait = session_id.to_string();
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            let detail = service.session_detail(&session_id_for_wait).await;
            let _ = respond_to.send(detail);
        });
        respond_rx
            .await
            .context("Pi Agent prompt response channel closed")?
            .with_context(|| format!("unknown Pi Agent session: {session_id}"))
    }

    pub async fn set_session_mode(
        &self,
        _session_id: &str,
        _mode_id: String,
    ) -> Result<AcpSessionDetail> {
        anyhow::bail!("Pi Agent 不支持 ACP session mode")
    }

    pub async fn set_session_config_option(
        &self,
        _session_id: &str,
        _config_id: String,
        _value_id: String,
    ) -> Result<AcpSessionDetail> {
        anyhow::bail!("Pi Agent 不支持 ACP config option")
    }

    pub async fn cancel_session(&self, session_id: &str) -> Result<()> {
        let abort = {
            let sessions = self.sessions.read().await;
            sessions
                .get(session_id)
                .with_context(|| format!("unknown Pi Agent session: {session_id}"))?
                .current_abort
                .clone()
        };
        if let Some(abort) = abort {
            abort.abort();
        }
        Ok(())
    }

    pub async fn close_session(&self, session_id: &str) -> Result<()> {
        let removed = self.sessions.write().await.remove(session_id);
        if removed.is_none() {
            anyhow::bail!("unknown Pi Agent session: {session_id}");
        }
        {
            let mut active = self.active_session_id.write().await;
            if active.as_deref() == Some(session_id) {
                *active = None;
            }
        }
        self.emit_session_removal(session_id.to_string()).await;
        Ok(())
    }

    pub async fn take_restore_notices(&self) -> Vec<AcpRestoreNotice> {
        std::mem::take(&mut *self.restore_notices.write().await)
    }

    async fn apply_runtime_event(&self, event: RuntimeEvent) -> Result<()> {
        let mut notify_success = None;
        let mut notify_failure = None;
        let session_id = match &event {
            RuntimeEvent::TextDelta { session_id, .. }
            | RuntimeEvent::ThinkingDelta { session_id, .. }
            | RuntimeEvent::ToolEvent { session_id, .. }
            | RuntimeEvent::PromptFinished { session_id }
            | RuntimeEvent::PromptFailed { session_id, .. } => session_id.clone(),
        };

        let detail = {
            let mut sessions = self.sessions.write().await;
            let Some(record) = sessions.get_mut(&session_id) else {
                return Ok(());
            };
            match event {
                RuntimeEvent::TextDelta { delta, .. } => {
                    append_to_assistant(record, BlockAppend::Content(delta))
                }
                RuntimeEvent::ThinkingDelta { delta, .. } => {
                    append_to_assistant(record, BlockAppend::Thought(delta))
                }
                RuntimeEvent::ToolEvent {
                    kind,
                    title,
                    correlation_id,
                    detail,
                    ..
                } => append_to_assistant(
                    record,
                    BlockAppend::Action(AcpActionEvent {
                        kind,
                        title,
                        correlation_id,
                        detail,
                    }),
                ),
                RuntimeEvent::PromptFinished { .. } => {
                    record.summary.status = AcpSessionStatus::Idle;
                    record.summary.error_level = None;
                    record.summary.attention = false;
                    record.summary.last_error = None;
                    record.current_abort = None;
                    finish_pending_assistant(record);
                    notify_success = Some((
                        record.summary.agent_name.clone(),
                        last_assistant_text(record),
                    ));
                }
                RuntimeEvent::PromptFailed { error, .. } => {
                    record.summary.status = AcpSessionStatus::Error;
                    record.summary.error_level = Some(AcpSessionErrorLevel::Recoverable);
                    record.summary.attention = true;
                    record.summary.last_error = Some(error.clone());
                    record.current_abort = None;
                    finish_pending_assistant(record);
                    append_system_message(record, format!("Pi Agent prompt failed: {error}"));
                    notify_failure = Some((record.summary.agent_name.clone(), error));
                }
            }
            record.summary.last_updated_at_ms = now_ms();
            let detail = record_detail(record);
            if let Some(hook) = &self.session_persist_hook {
                let summary = detail.session.clone();
                let agent = record.agent.clone();
                let mcp_servers = record.mcp_servers.clone();
                let hook = hook.clone();
                tauri::async_runtime::spawn(async move {
                    hook(summary, agent, mcp_servers).await;
                });
            }
            detail
        };

        self.emit_session_update(&detail).await;
        if let Some((agent_name, preview)) = notify_success {
            self.notification
                .notify_acp_prompt_success(&agent_name, preview)
                .await;
        }
        if let Some((agent_name, error)) = notify_failure {
            self.notification
                .notify_acp_prompt_failure(&agent_name, Some(error))
                .await;
        }
        Ok(())
    }

    async fn emit_session_update(&self, detail: &AcpSessionDetail) {
        let mut channels = self.session_update_channels.write().await;
        channels.retain(|channel| channel.send(detail.clone()).is_ok());
    }

    async fn emit_session_removal(&self, session_id: String) {
        let mut channels = self.session_removal_channels.write().await;
        channels.retain(|channel| channel.send(session_id.clone()).is_ok());
    }
}

fn forward_agent_event(
    tx: &mpsc::UnboundedSender<RuntimeEvent>,
    session_id: &str,
    event: AgentEvent,
) {
    match event {
        AgentEvent::MessageUpdate {
            assistant_message_event,
            ..
        } => forward_message_event(tx, session_id, assistant_message_event),
        AgentEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args,
        } => {
            let _ = tx.send(RuntimeEvent::ToolEvent {
                session_id: session_id.to_string(),
                kind: "tool-call".to_string(),
                title: format!("调用工具 {tool_name}"),
                correlation_id: Some(tool_call_id),
                detail: Some(args.to_string()),
            });
        }
        AgentEvent::ToolExecutionUpdate {
            tool_call_id,
            tool_name,
            partial_result,
            ..
        } => {
            let _ = tx.send(RuntimeEvent::ToolEvent {
                session_id: session_id.to_string(),
                kind: "tool-update".to_string(),
                title: format!("工具运行中 {tool_name}"),
                correlation_id: Some(tool_call_id),
                detail: Some(format!("{partial_result:?}")),
            });
        }
        AgentEvent::ToolExecutionEnd {
            tool_call_id,
            tool_name,
            result,
            is_error,
        } => {
            let _ = tx.send(RuntimeEvent::ToolEvent {
                session_id: session_id.to_string(),
                kind: if is_error {
                    "tool-error"
                } else {
                    "tool-result"
                }
                .to_string(),
                title: if is_error {
                    format!("工具失败 {tool_name}")
                } else {
                    format!("工具完成 {tool_name}")
                },
                correlation_id: Some(tool_call_id),
                detail: Some(format!("{result:?}")),
            });
        }
        AgentEvent::ExtensionError { event, error, .. } => {
            let _ = tx.send(RuntimeEvent::ToolEvent {
                session_id: session_id.to_string(),
                kind: "extension-error".to_string(),
                title: event,
                correlation_id: None,
                detail: Some(error),
            });
        }
        _ => {}
    }
}

fn forward_message_event(
    tx: &mpsc::UnboundedSender<RuntimeEvent>,
    session_id: &str,
    event: pi::model::AssistantMessageEvent,
) {
    match event {
        pi::model::AssistantMessageEvent::TextDelta { delta, .. } => {
            let _ = tx.send(RuntimeEvent::TextDelta {
                session_id: session_id.to_string(),
                delta,
            });
        }
        pi::model::AssistantMessageEvent::TextEnd { content, .. } if !content.is_empty() => {
            let _ = tx.send(RuntimeEvent::TextDelta {
                session_id: session_id.to_string(),
                delta: String::new(),
            });
        }
        pi::model::AssistantMessageEvent::ThinkingDelta { delta, .. } => {
            let _ = tx.send(RuntimeEvent::ThinkingDelta {
                session_id: session_id.to_string(),
                delta,
            });
        }
        pi::model::AssistantMessageEvent::ToolCallEnd { tool_call, .. } => {
            let _ = tx.send(RuntimeEvent::ToolEvent {
                session_id: session_id.to_string(),
                kind: "tool-call".to_string(),
                title: tool_call.name.clone(),
                correlation_id: Some(tool_call.id.clone()),
                detail: Some(tool_call.arguments.to_string()),
            });
        }
        pi::model::AssistantMessageEvent::Error { error, .. } => {
            let _ = tx.send(RuntimeEvent::PromptFailed {
                session_id: session_id.to_string(),
                error: format!("{error:?}"),
            });
        }
        _ => {}
    }
}

enum BlockAppend {
    Content(String),
    Thought(String),
    Action(AcpActionEvent),
}

fn append_to_assistant(record: &mut SessionRecord, append: BlockAppend) {
    if !matches!(append, BlockAppend::Content(ref text) if text.is_empty()) {
        ensure_pending_assistant(record);
    }
    let Some(message) = record
        .messages
        .iter_mut()
        .rev()
        .find(|message| message.role == AcpMessageRole::Assistant && message.pending)
    else {
        return;
    };

    match append {
        BlockAppend::Content(delta) => append_text_block(&mut message.blocks, delta),
        BlockAppend::Thought(delta) => append_thought_block(&mut message.blocks, delta),
        BlockAppend::Action(action) => message.blocks.push(AcpMessageBlock::Actions {
            items: vec![action],
        }),
    }
}

fn append_text_block(blocks: &mut Vec<AcpMessageBlock>, delta: String) {
    if delta.is_empty() {
        return;
    }
    if let Some(AcpMessageBlock::Content { text }) = blocks.last_mut() {
        text.push_str(&delta);
    } else {
        blocks.push(AcpMessageBlock::Content { text: delta });
    }
}

fn append_thought_block(blocks: &mut Vec<AcpMessageBlock>, delta: String) {
    if delta.is_empty() {
        return;
    }
    if let Some(AcpMessageBlock::Thought { content }) = blocks.last_mut() {
        content.push_str(&delta);
    } else {
        blocks.push(AcpMessageBlock::Thought { content: delta });
    }
}

fn append_system_message(record: &mut SessionRecord, content: String) {
    let id = next_message_id(record);
    record.messages.push(AcpSessionMessage {
        id,
        role: AcpMessageRole::System,
        blocks: vec![AcpMessageBlock::Content { text: content }],
        pending: false,
    });
}

fn ensure_pending_assistant(record: &mut SessionRecord) {
    let has_pending = record
        .messages
        .iter()
        .any(|message| message.role == AcpMessageRole::Assistant && message.pending);
    if !has_pending {
        let id = next_message_id(record);
        record.messages.push(AcpSessionMessage {
            id,
            role: AcpMessageRole::Assistant,
            blocks: Vec::new(),
            pending: true,
        });
    }
}

fn finish_pending_assistant(record: &mut SessionRecord) {
    for message in record.messages.iter_mut().rev() {
        if message.role == AcpMessageRole::Assistant && message.pending {
            message.pending = false;
            break;
        }
    }
}

fn last_assistant_text(record: &SessionRecord) -> Option<String> {
    record
        .messages
        .iter()
        .rev()
        .find(|message| message.role == AcpMessageRole::Assistant)
        .and_then(|message| {
            let text = message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    AcpMessageBlock::Content { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            (!text.trim().is_empty()).then_some(text)
        })
}

fn record_detail(record: &SessionRecord) -> AcpSessionDetail {
    AcpSessionDetail {
        session: record.summary.clone(),
        messages: record.messages.clone(),
        runtime: record.runtime.clone(),
    }
}

fn next_message_id(record: &mut SessionRecord) -> String {
    let id = record.next_message_id;
    record.next_message_id += 1;
    format!("msg-{id}")
}

fn display_agent_name(agent: &AcpAgentConfig) -> String {
    let name = agent.name.trim();
    if name.is_empty() {
        "Pi Agent".to_string()
    } else {
        name.to_string()
    }
}

fn ensure_prompt_allowed(status: &AcpSessionStatus) -> Result<()> {
    match status {
        AcpSessionStatus::Running | AcpSessionStatus::Starting => {
            anyhow::bail!("Pi Agent session 正在处理上一条消息")
        }
        AcpSessionStatus::Exited => anyhow::bail!("Pi Agent session 已退出"),
        AcpSessionStatus::Idle | AcpSessionStatus::Error => Ok(()),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
