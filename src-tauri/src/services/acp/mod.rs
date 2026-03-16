use std::{
    collections::HashMap,
    process::Stdio,
    rc::Rc,
    sync::{Arc, Mutex as StdMutex},
    time::{SystemTime, UNIX_EPOCH},
};

use agent_client_protocol::{
    self as acp, Agent as _, ClientCapabilities, FileSystemCapabilities, RequestId,
    RequestPermissionOutcome, SelectedPermissionOutcome, SessionUpdate, StreamMessage,
    StreamMessageContent, StreamMessageDirection,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use tauri::ipc::Channel;
use tokio::{
    process::Command,
    sync::{mpsc, oneshot, RwLock},
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::domain::acp::{
    AcpActionEvent, AcpAgentConfig, AcpMcpServerConfig, AcpMessageBlock, AcpMessageRole,
    AcpRestoreNotice, AcpSessionDetail, AcpSessionErrorLevel, AcpSessionMessage, AcpSessionStatus,
    AcpSessionSummary,
};
use crate::infrastructure::config::SavedAcpSession;

#[derive(Clone)]
pub struct AcpService {
    sessions: Arc<RwLock<HashMap<String, SessionRecord>>>,
    active_session_id: Arc<RwLock<Option<String>>>,
    restore_notices: Arc<RwLock<Vec<AcpRestoreNotice>>>,
    runtime_event_tx: mpsc::UnboundedSender<RuntimeEvent>,
    runtime_event_rx: Arc<StdMutex<Option<mpsc::UnboundedReceiver<RuntimeEvent>>>>,
    session_update_channels: Arc<RwLock<Vec<Channel<AcpSessionDetail>>>>,
    session_removal_channels: Arc<RwLock<Vec<Channel<String>>>>,
}

struct SessionRecord {
    agent: AcpAgentConfig,
    mcp_servers: Vec<AcpMcpServerConfig>,
    summary: AcpSessionSummary,
    messages: Vec<AcpSessionMessage>,
    next_message_id: u64,
    command_tx: mpsc::UnboundedSender<SessionCommand>,
}

#[derive(Clone)]
pub struct SessionRuntimeConfig {
    pub agent: AcpAgentConfig,
    pub mcp_servers: Vec<AcpMcpServerConfig>,
}

#[derive(Debug)]
enum SessionCommand {
    Prompt {
        prompt: String,
        respond_to: oneshot::Sender<Result<()>>,
    },
    Cancel {
        respond_to: oneshot::Sender<Result<()>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<()>,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum RuntimeEvent {
    AssistantChunk {
        session_id: String,
        content: String,
    },
    ThoughtChunk {
        session_id: String,
        content: String,
    },
    ActionEvent {
        session_id: String,
        kind: String,
        title: String,
        correlation_id: Option<String>,
        detail: Option<String>,
    },
    SystemMessage {
        session_id: String,
        content: String,
    },
    PromptFinished {
        session_id: String,
    },
    PromptFailed {
        session_id: String,
        error: String,
    },
    SessionExited {
        session_id: String,
        error: Option<String>,
    },
}

struct StartedSession {
    session_id: String,
    agent_title: String,
}

enum SessionBootstrap {
    New,
    Load { session_id: String },
}

struct PromptCompletion {
    error: Option<String>,
}

pub struct RestoreAttemptResult {
    pub restored: Option<AcpSessionDetail>,
    pub keep_snapshot: bool,
}

struct AcpRuntimeClient;

#[async_trait(?Send)]
impl acp::Client for AcpRuntimeClient {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        let outcome = args
            .options
            .into_iter()
            .find(|option| {
                matches!(
                    option.kind,
                    acp::PermissionOptionKind::RejectOnce | acp::PermissionOptionKind::RejectAlways
                )
            })
            .map(|option| {
                RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option.option_id))
            })
            .unwrap_or(RequestPermissionOutcome::Cancelled);

        Ok(acp::RequestPermissionResponse::new(outcome))
    }

    async fn session_notification(
        &self,
        _args: acp::SessionNotification,
    ) -> acp::Result<(), acp::Error> {
        Ok(())
    }
}

impl AcpService {
    pub fn new() -> Self {
        let (runtime_event_tx, runtime_event_rx) = mpsc::unbounded_channel();

        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            active_session_id: Arc::new(RwLock::new(None)),
            restore_notices: Arc::new(RwLock::new(Vec::new())),
            runtime_event_tx,
            runtime_event_rx: Arc::new(StdMutex::new(Some(runtime_event_rx))),
            session_update_channels: Arc::new(RwLock::new(Vec::new())),
            session_removal_channels: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn subscribe_session_updates(&self, channel: Channel<AcpSessionDetail>) {
        self.session_update_channels.write().await.push(channel);
    }

    pub async fn subscribe_session_removals(&self, channel: Channel<String>) {
        self.session_removal_channels.write().await.push(channel);
    }

    async fn broadcast_session_update(&self, detail: AcpSessionDetail) {
        tracing::info!(
            session_id = %detail.session.session_id,
            status = ?detail.session.status,
            message_count = detail.messages.len(),
            "broadcasting session update"
        );
        let channels = self.session_update_channels.read().await;
        for channel in channels.iter() {
            let _ = channel.send(detail.clone());
        }
    }

    async fn broadcast_session_removal(&self, session_id: String) {
        let channels = self.session_removal_channels.read().await;
        for channel in channels.iter() {
            let _ = channel.send(session_id.clone());
        }
    }

    pub fn start_event_loop(&self) {
        let Some(mut receiver) = self.runtime_event_rx.lock().unwrap().take() else {
            return;
        };
        let service = self.clone();

        tauri::async_runtime::spawn(async move {
            while let Some(event) = receiver.recv().await {
                if let Err(error) = service.apply_runtime_event(event).await {
                    tracing::warn!(?error, "failed to apply acp runtime event");
                }
            }
        });
    }

    pub async fn list_sessions(&self) -> Vec<AcpSessionSummary> {
        let sessions = self.sessions.read().await;
        let mut summaries = sessions
            .values()
            .map(|record| record.summary.clone())
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            right
                .last_updated_at_ms
                .cmp(&left.last_updated_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        summaries
    }

    pub async fn session_detail(&self, session_id: &str) -> Option<AcpSessionDetail> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(SessionRecord::detail)
    }

    pub async fn session_runtime_config(&self, session_id: &str) -> Option<SessionRuntimeConfig> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).map(|record| SessionRuntimeConfig {
            agent: record.agent.clone(),
            mcp_servers: record.mcp_servers.clone(),
        })
    }

    pub async fn take_restore_notices(&self) -> Vec<AcpRestoreNotice> {
        let mut notices = self.restore_notices.write().await;
        std::mem::take(&mut *notices)
    }

    pub async fn create_session(
        &self,
        workspace_root: std::path::PathBuf,
        agent: AcpAgentConfig,
        mcp_servers: Vec<AcpMcpServerConfig>,
    ) -> Result<AcpSessionDetail> {
        let has_shell_command = agent
            .shell_command
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        if agent.program.trim().is_empty() && !has_shell_command {
            anyhow::bail!("ACP agent 程序未配置");
        }

        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (started_tx, started_rx) = oneshot::channel();
        let runtime_event_tx = self.runtime_event_tx.clone();

        spawn_session_runtime(
            workspace_root.clone(),
            agent.clone(),
            mcp_servers.clone(),
            SessionBootstrap::New,
            command_rx,
            started_tx,
            runtime_event_tx,
        )?;

        let started = started_rx
            .await
            .context("ACP session startup channel closed")??;

        let title = workspace_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("workspace")
            .to_string();
        let now = now_ms();

        let mut record = SessionRecord {
            agent: agent.clone(),
            mcp_servers,
            summary: AcpSessionSummary {
                session_id: started.session_id.clone(),
                workspace_root: workspace_root.to_string_lossy().into_owned(),
                title,
                agent_id: Some(agent.id.clone()),
                agent_name: agent.name.clone(),
                status: AcpSessionStatus::Idle,
                error_level: None,
                attention: false,
                is_active: false,
                last_error: None,
                last_updated_at_ms: now,
            },
            messages: Vec::new(),
            next_message_id: 0,
            command_tx,
        };
        record.push_system_message(format!(
            "ACP session 已连接到 {}，工作目录为 {}",
            started.agent_title,
            workspace_root.display()
        ));

        let detail = record.detail();
        self.sessions
            .write()
            .await
            .insert(started.session_id.clone(), record);
        Ok(detail)
    }

    pub async fn restore_session(&self, snapshot: SavedAcpSession) -> RestoreAttemptResult {
        let workspace_root =
            match crate::infrastructure::config::normalize_workspace_root(&snapshot.workspace_root)
            {
                Ok(path) => path,
                Err(error) => {
                    self.push_restore_notice(AcpRestoreNotice {
                        session_id: snapshot.session_id,
                        workspace_root: snapshot.workspace_root,
                        message: format!("无法恢复 session：workspace 无效：{error}"),
                    })
                    .await;
                    return RestoreAttemptResult {
                        restored: None,
                        keep_snapshot: false,
                    };
                }
            };

        let agent_name = if snapshot.agent_name.trim().is_empty() {
            started_agent_name(
                snapshot.agent_shell_command.as_deref(),
                &snapshot.agent_program,
                &snapshot.session_id,
            )
        } else {
            snapshot.agent_name.clone()
        };

        let snapshot_agent_id = snapshot.agent_id.clone();
        let agent = AcpAgentConfig {
            id: snapshot_agent_id
                .clone()
                .unwrap_or_else(|| format!("restored-{}", snapshot.session_id)),
            name: agent_name,
            program: snapshot.agent_program.clone(),
            args: snapshot.agent_args.clone(),
            shell_command: snapshot.agent_shell_command.clone(),
            mcp_servers: Vec::new(),
        };
        let mcp_servers = snapshot.mcp_servers.clone();

        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (started_tx, started_rx) = oneshot::channel();
        let runtime_event_tx = self.runtime_event_tx.clone();

        if let Err(error) = spawn_session_runtime(
            workspace_root.clone(),
            agent.clone(),
            mcp_servers.clone(),
            SessionBootstrap::Load {
                session_id: snapshot.session_id.clone(),
            },
            command_rx,
            started_tx,
            runtime_event_tx,
        ) {
            self.push_restore_notice(AcpRestoreNotice {
                session_id: snapshot.session_id,
                workspace_root: snapshot.workspace_root,
                message: format!("无法恢复 session：启动 agent 失败：{error}"),
            })
            .await;
            return RestoreAttemptResult {
                restored: None,
                keep_snapshot: true,
            };
        }

        let started = match started_rx.await {
            Ok(Ok(started)) => started,
            Ok(Err(error)) => {
                let keep_snapshot = !is_load_not_supported(&error);
                self.push_restore_notice(AcpRestoreNotice {
                    session_id: snapshot.session_id,
                    workspace_root: snapshot.workspace_root,
                    message: format!("无法恢复 session：{error}"),
                })
                .await;
                return RestoreAttemptResult {
                    restored: None,
                    keep_snapshot,
                };
            }
            Err(error) => {
                self.push_restore_notice(AcpRestoreNotice {
                    session_id: snapshot.session_id,
                    workspace_root: snapshot.workspace_root,
                    message: format!("无法恢复 session：恢复通道关闭：{error}"),
                })
                .await;
                return RestoreAttemptResult {
                    restored: None,
                    keep_snapshot: true,
                };
            }
        };

        let mut record = SessionRecord {
            agent: agent.clone(),
            mcp_servers,
            summary: AcpSessionSummary {
                session_id: started.session_id.clone(),
                workspace_root: workspace_root.to_string_lossy().into_owned(),
                title: snapshot.title,
                agent_id: snapshot_agent_id,
                agent_name: agent.name.clone(),
                status: AcpSessionStatus::Idle,
                error_level: None,
                attention: false,
                is_active: false,
                last_error: None,
                last_updated_at_ms: snapshot.last_updated_at_ms.max(now_ms()),
            },
            messages: Vec::new(),
            next_message_id: 0,
            command_tx,
        };
        record.push_system_message(format!(
            "已恢复 ACP session，连接到 {}，工作目录为 {}",
            started.agent_title,
            workspace_root.display()
        ));

        let detail = record.detail();
        self.sessions
            .write()
            .await
            .insert(started.session_id, record);

        RestoreAttemptResult {
            restored: Some(detail),
            keep_snapshot: true,
        }
    }

    pub async fn activate_session(&self, session_id: Option<String>) -> Vec<AcpSessionSummary> {
        {
            let mut active = self.active_session_id.write().await;
            *active = session_id.clone();
        }

        let mut sessions = self.sessions.write().await;
        for record in sessions.values_mut() {
            let is_active = session_id
                .as_ref()
                .map(|value| value == &record.summary.session_id)
                .unwrap_or(false);
            record.summary.is_active = is_active;
            if is_active {
                record.summary.attention = false;
            }
        }

        let mut summaries = sessions
            .values()
            .map(|record| record.summary.clone())
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            right
                .last_updated_at_ms
                .cmp(&left.last_updated_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        summaries
    }

    pub async fn send_prompt(&self, session_id: &str, prompt: String) -> Result<AcpSessionDetail> {
        let (respond_to, response) = oneshot::channel();
        let detail = {
            let mut sessions = self.sessions.write().await;
            let record = sessions
                .get_mut(session_id)
                .with_context(|| format!("unknown ACP session: {session_id}"))?;
            record.summary.status = AcpSessionStatus::Running;
            record.summary.error_level = None;
            record.summary.last_error = None;
            record.summary.last_updated_at_ms = now_ms();
            record.summary.attention = false;
            record.push_message(AcpMessageRole::User, prompt.clone(), false);
            record.start_assistant_message();
            record
                .command_tx
                .send(SessionCommand::Prompt { prompt, respond_to })
                .map_err(|_| anyhow::anyhow!("ACP session runtime is unavailable"))?;
            record.detail()
        };

        response
            .await
            .context("ACP prompt response channel closed")??;
        Ok(detail)
    }

    pub async fn cancel_session(&self, session_id: &str) -> Result<()> {
        let (respond_to, response) = oneshot::channel();
        let sessions = self.sessions.read().await;
        let record = sessions
            .get(session_id)
            .with_context(|| format!("unknown ACP session: {session_id}"))?;
        record
            .command_tx
            .send(SessionCommand::Cancel { respond_to })
            .map_err(|_| anyhow::anyhow!("ACP session runtime is unavailable"))?;
        drop(sessions);

        response
            .await
            .context("ACP cancel response channel closed")??;
        Ok(())
    }

    pub async fn close_session(&self, session_id: &str) -> Result<()> {
        let record = {
            let mut sessions = self.sessions.write().await;
            sessions.remove(session_id)
        };

        let Some(record) = record else {
            return Ok(());
        };

        let (respond_to, response) = oneshot::channel();
        let _ = record
            .command_tx
            .send(SessionCommand::Shutdown { respond_to });
        let _ = response.await;

        {
            let mut active = self.active_session_id.write().await;
            if active.as_deref() == Some(session_id) {
                *active = None;
            }
        }

        self.broadcast_session_removal(session_id.to_string()).await;
        Ok(())
    }

    async fn apply_runtime_event(&self, event: RuntimeEvent) -> Result<()> {
        let active_session_id = self.active_session_id.read().await.clone();
        let session_id = match &event {
            RuntimeEvent::AssistantChunk { session_id, .. }
            | RuntimeEvent::ThoughtChunk { session_id, .. }
            | RuntimeEvent::ActionEvent { session_id, .. }
            | RuntimeEvent::SystemMessage { session_id, .. }
            | RuntimeEvent::PromptFinished { session_id }
            | RuntimeEvent::PromptFailed { session_id, .. }
            | RuntimeEvent::SessionExited { session_id, .. } => session_id.clone(),
        };

        tracing::info!(?event, %session_id, "applying runtime event");

        let detail = {
            let mut sessions = self.sessions.write().await;
            let Some(record) = sessions.get_mut(&session_id) else {
                return Ok(());
            };

            match event {
                RuntimeEvent::AssistantChunk { content, .. } => {
                    record.summary.status = AcpSessionStatus::Running;
                    record.summary.error_level = None;
                    record.summary.last_updated_at_ms = now_ms();
                    if active_session_id.as_deref() != Some(&record.summary.session_id) {
                        record.summary.attention = true;
                    }
                    record.append_assistant_chunk(content);
                }
                RuntimeEvent::ThoughtChunk { content, .. } => {
                    record.summary.status = AcpSessionStatus::Running;
                    record.summary.last_updated_at_ms = now_ms();
                    record.append_thought_chunk(content);
                }
                RuntimeEvent::ActionEvent {
                    kind,
                    title,
                    correlation_id,
                    detail,
                    ..
                } => {
                    record.summary.last_updated_at_ms = now_ms();
                    if active_session_id.as_deref() != Some(&record.summary.session_id) {
                        record.summary.attention = true;
                    }
                    record.append_action_event(kind, title, correlation_id, detail);
                }
                RuntimeEvent::SystemMessage { content, .. } => {
                    record.summary.last_updated_at_ms = now_ms();
                    if active_session_id.as_deref() != Some(&record.summary.session_id) {
                        record.summary.attention = true;
                    }
                    record.push_system_message(content);
                }
                RuntimeEvent::PromptFinished { .. } => {
                    record.summary.status = AcpSessionStatus::Idle;
                    record.summary.error_level = None;
                    record.summary.last_updated_at_ms = now_ms();
                    record.finish_assistant_message();
                }
                RuntimeEvent::PromptFailed { error, .. } => {
                    record.summary.status = AcpSessionStatus::Error;
                    record.summary.error_level = Some(AcpSessionErrorLevel::Recoverable);
                    record.summary.last_error = Some(error.clone());
                    record.summary.last_updated_at_ms = now_ms();
                    if active_session_id.as_deref() != Some(&record.summary.session_id) {
                        record.summary.attention = true;
                    }
                    record.finish_assistant_message();
                    record.push_system_message(format!("prompt 失败：{error}"));
                }
                RuntimeEvent::SessionExited { error, .. } => {
                    record.summary.status = if error.is_some() {
                        AcpSessionStatus::Error
                    } else {
                        AcpSessionStatus::Exited
                    };
                    record.summary.error_level =
                        error.as_ref().map(|_| AcpSessionErrorLevel::Fatal);
                    record.summary.last_error = error.clone();
                    record.summary.last_updated_at_ms = now_ms();
                    if active_session_id.as_deref() != Some(&record.summary.session_id) {
                        record.summary.attention = true;
                    }
                    if let Some(error) = error {
                        record.push_system_message(format!("agent 已退出：{error}"));
                    } else {
                        record.push_system_message("agent 已退出".to_string());
                    }
                    record.finish_assistant_message();
                }
            }

            if active_session_id.as_deref() == Some(&record.summary.session_id) {
                record.summary.is_active = true;
                record.summary.attention = false;
            } else {
                record.summary.is_active = false;
            }

            record.detail()
        };

        self.broadcast_session_update(detail).await;
        Ok(())
    }

    async fn push_restore_notice(&self, notice: AcpRestoreNotice) {
        self.restore_notices.write().await.push(notice);
    }
}

impl SessionRecord {
    fn detail(&self) -> AcpSessionDetail {
        AcpSessionDetail {
            session: self.summary.clone(),
            messages: self.messages.clone(),
        }
    }

    fn push_message(&mut self, role: AcpMessageRole, content: String, pending: bool) {
        let message = AcpSessionMessage {
            id: self.next_message_id().to_string(),
            role,
            blocks: vec![AcpMessageBlock::Content { text: content }],
            pending,
        };
        self.messages.push(message);
    }

    fn push_system_message(&mut self, content: String) {
        self.push_message(AcpMessageRole::System, content, false);
    }

    fn start_assistant_message(&mut self) {
        if matches!(
            self.messages.last(),
            Some(AcpSessionMessage {
                role: AcpMessageRole::Assistant,
                pending: true,
                ..
            })
        ) {
            return;
        }

        let id = self.next_message_id().to_string();
        self.messages.push(AcpSessionMessage {
            id,
            role: AcpMessageRole::Assistant,
            blocks: Vec::new(),
            pending: true,
        });
    }

    // Only the tail pending assistant belongs to the current turn.
    // If a user message has already been appended after an older pending assistant,
    // new runtime events must not backfill into that older message.
    fn pending_assistant_message_mut(&mut self) -> Option<&mut AcpSessionMessage> {
        match self.messages.last_mut() {
            Some(AcpSessionMessage {
                role: AcpMessageRole::Assistant,
                pending: true,
                ..
            }) => self.messages.last_mut(),
            _ => None,
        }
    }

    fn append_assistant_chunk(&mut self, content: String) {
        if let Some(message) = self.pending_assistant_message_mut() {
            // Append to the last Content block if it exists, otherwise create a new one
            if let Some(AcpMessageBlock::Content { text }) = message.blocks.last_mut() {
                text.push_str(&content);
                return;
            }
            message
                .blocks
                .push(AcpMessageBlock::Content { text: content });
            return;
        }

        self.start_assistant_message();
        if let Some(message) = self.pending_assistant_message_mut() {
            message
                .blocks
                .push(AcpMessageBlock::Content { text: content });
        }
    }

    fn append_thought_chunk(&mut self, content: String) {
        if let Some(message) = self.pending_assistant_message_mut() {
            // Append to the last Thought block if it exists, otherwise create a new one
            if let Some(AcpMessageBlock::Thought { content: thought }) = message.blocks.last_mut() {
                thought.push_str(&content);
                return;
            }
            message.blocks.push(AcpMessageBlock::Thought { content });
            return;
        }

        self.start_assistant_message();
        if let Some(message) = self.pending_assistant_message_mut() {
            message.blocks.push(AcpMessageBlock::Thought { content });
        }
    }

    fn append_action_event(
        &mut self,
        kind: String,
        title: String,
        correlation_id: Option<String>,
        detail: Option<String>,
    ) {
        if let Some(message) = self.pending_assistant_message_mut() {
            // Append to the last Actions block if it exists, otherwise create a new one
            if let Some(AcpMessageBlock::Actions { items }) = message.blocks.last_mut() {
                items.push(AcpActionEvent {
                    kind,
                    title,
                    correlation_id,
                    detail,
                });
                return;
            }
            message.blocks.push(AcpMessageBlock::Actions {
                items: vec![AcpActionEvent {
                    kind,
                    title,
                    correlation_id,
                    detail,
                }],
            });
            return;
        }

        self.start_assistant_message();
        if let Some(message) = self.pending_assistant_message_mut() {
            message.blocks.push(AcpMessageBlock::Actions {
                items: vec![AcpActionEvent {
                    kind,
                    title,
                    correlation_id,
                    detail,
                }],
            });
        }
    }

    fn finish_assistant_message(&mut self) {
        if let Some(index) = self
            .messages
            .iter()
            .rposition(|message| message.pending && message.role == AcpMessageRole::Assistant)
        {
            let msg = &mut self.messages[index];
            // Remove empty Content blocks, but keep Thought and Actions blocks
            msg.blocks.retain(|block| match block {
                AcpMessageBlock::Content { text } => !text.is_empty(),
                _ => true,
            });
            // Only remove the message if all blocks are empty
            if msg.blocks.is_empty() {
                self.messages.remove(index);
            } else {
                msg.pending = false;
            }
        }
    }

    fn next_message_id(&mut self) -> u64 {
        let next = self.next_message_id;
        self.next_message_id = self.next_message_id.saturating_add(1);
        next
    }
}

fn spawn_session_runtime(
    workspace_root: std::path::PathBuf,
    agent: AcpAgentConfig,
    mcp_servers: Vec<AcpMcpServerConfig>,
    bootstrap: SessionBootstrap,
    mut command_rx: mpsc::UnboundedReceiver<SessionCommand>,
    started_tx: oneshot::Sender<Result<StartedSession>>,
    runtime_event_tx: mpsc::UnboundedSender<RuntimeEvent>,
) -> Result<()> {
    std::thread::Builder::new()
        .name("wabity-acp-session".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = started_tx.send(Err(anyhow::Error::new(error)));
                    return;
                }
            };
            let local_set = tokio::task::LocalSet::new();

            local_set.block_on(&runtime, async move {
                let mut started_tx = Some(started_tx);
                if let Err(error) = run_session_runtime(
                    workspace_root,
                    agent,
                    mcp_servers,
                    bootstrap,
                    &mut command_rx,
                    &mut started_tx,
                    runtime_event_tx.clone(),
                )
                .await
                {
                    if let Some(started_tx) = started_tx.take() {
                        let _ = started_tx.send(Err(anyhow::anyhow!(error.to_string())));
                    }
                    tracing::warn!(?error, "acp session runtime stopped with error");
                }
            });
        })
        .context("failed to spawn ACP session thread")?;

    Ok(())
}

async fn run_session_runtime(
    workspace_root: std::path::PathBuf,
    agent: AcpAgentConfig,
    mcp_servers: Vec<AcpMcpServerConfig>,
    bootstrap: SessionBootstrap,
    command_rx: &mut mpsc::UnboundedReceiver<SessionCommand>,
    started_tx: &mut Option<oneshot::Sender<Result<StartedSession>>>,
    runtime_event_tx: mpsc::UnboundedSender<RuntimeEvent>,
) -> Result<()> {
    let mut command = build_agent_command(&agent, &workspace_root)?;
    let spawn_target = if agent
        .shell_command
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        agent.shell_command.clone().unwrap_or_default()
    } else {
        agent.program.clone()
    };
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn ACP agent: {spawn_target}"))?;

    let stdout = child
        .stdout
        .take()
        .context("ACP agent stdout is unavailable")?;
    let stdin = child
        .stdin
        .take()
        .context("ACP agent stdin is unavailable")?;

    let runtime_client = AcpRuntimeClient;
    let (connection, handle_io) = acp::ClientSideConnection::new(
        runtime_client,
        stdin.compat_write(),
        stdout.compat(),
        |fut| {
            tokio::task::spawn_local(fut);
        },
    );
    let connection = Rc::new(connection);
    tokio::task::spawn_local(async move {
        if let Err(error) = handle_io.await {
            tracing::warn!(?error, "acp io loop stopped");
        }
    });

    let initialize_response = connection
        .initialize(
            acp::InitializeRequest::new(acp::ProtocolVersion::V1)
                .client_capabilities(
                    ClientCapabilities::new()
                        .fs(FileSystemCapabilities::new()
                            .read_text_file(false)
                            .write_text_file(false))
                        .terminal(false),
                )
                .client_info(acp::Implementation::new("wabity", "0.1.0").title("Wabity")),
        )
        .await
        .context("failed to initialize ACP client")?;
    validate_mcp_server_capabilities(
        &mcp_servers,
        &initialize_response.agent_capabilities.mcp_capabilities,
    )?;
    let mcp_servers = build_mcp_servers(&mcp_servers);

    let session_id = match bootstrap {
        SessionBootstrap::New => connection
            .new_session(acp::NewSessionRequest::new(&workspace_root).mcp_servers(mcp_servers))
            .await
            .context("failed to create ACP session")?
            .session_id
            .to_string(),
        SessionBootstrap::Load { session_id } => {
            if !initialize_response.agent_capabilities.load_session {
                anyhow::bail!("agent 不支持 session/load，无法恢复之前的 session");
            }

            connection
                .load_session(
                    acp::LoadSessionRequest::new(session_id.clone(), &workspace_root)
                        .mcp_servers(mcp_servers),
                )
                .await
                .context("agent 拒绝恢复 session/load")?;
            session_id
        }
    };
    let agent_title = initialize_response
        .agent_info
        .and_then(|info| info.title.or(Some(info.name)))
        .unwrap_or_else(|| agent.program.clone());
    if let Some(started_tx) = started_tx.take() {
        let _ = started_tx.send(Ok(StartedSession {
            session_id: session_id.clone(),
            agent_title,
        }));
    }

    let mut stream_rx = connection.subscribe();
    let ordered_runtime_event_tx = runtime_event_tx.clone();
    let ordered_session_id = session_id.clone();
    tokio::task::spawn_local(async move {
        let mut pending_prompt_request_id: Option<RequestId> = None;
        while let Ok(message) = stream_rx.recv().await {
            for event in
                ordered_stream_events(&ordered_session_id, message, &mut pending_prompt_request_id)
            {
                let _ = ordered_runtime_event_tx.send(event);
            }
        }
    });

    let (prompt_completion_tx, mut prompt_completion_rx) = mpsc::unbounded_channel();
    let mut prompt_running = false;

    loop {
        tokio::select! {
            Some(command) = command_rx.recv() => {
                match command {
                    SessionCommand::Prompt { prompt, respond_to } => {
                        if prompt_running {
                            let _ = respond_to.send(Err(anyhow::anyhow!("ACP session is already processing a prompt")));
                            continue;
                        }

                        prompt_running = true;
                        let prompt_connection = Rc::clone(&connection);
                        let prompt_completion_tx = prompt_completion_tx.clone();
                        let prompt_session_id = session_id.clone();
                        tracing::info!(?prompt, session_id = ?prompt_session_id, "sending prompt to agent");
                        tokio::task::spawn_local(async move {
                            let completion = match prompt_connection
                                .prompt(acp::PromptRequest::new(prompt_session_id.clone(), vec![prompt.into()]))
                                .await
                            {
                                Ok(response) => {
                                    tracing::info!(?response, session_id = ?prompt_session_id, "prompt response received");
                                    PromptCompletion { error: None }
                                }
                                Err(error) => {
                                    tracing::error!(?error, session_id = ?prompt_session_id, "prompt failed");
                                    PromptCompletion {
                                    error: Some(anyhow::Error::new(error).context("ACP prompt failed").to_string()),
                                }
                                },
                            };
                            let _ = prompt_completion_tx.send(completion);
                        });

                        let _ = respond_to.send(Ok(()));
                    }
                    SessionCommand::Cancel { respond_to } => {
                        let result = connection
                            .cancel(acp::CancelNotification::new(session_id.clone()))
                            .await
                            .map_err(|error| anyhow::Error::new(error).context("failed to cancel ACP session"));
                        let _ = respond_to.send(result);
                    }
                    SessionCommand::Shutdown { respond_to } => {
                        let _ = respond_to.send(());
                        break;
                    }
                }
            }
            Some(completion) = prompt_completion_rx.recv() => {
                prompt_running = false;
                if let Some(error) = completion.error {
                    tracing::warn!(session_id = %session_id, %error, "prompt rpc finished with error");
                }
            }
            status = child.wait() => {
                match status {
                    Ok(status) => {
                        let error = if status.success() {
                            None
                        } else {
                            Some(format!("exit status {status}"))
                        };
                        let _ = runtime_event_tx.send(RuntimeEvent::SessionExited {
                            session_id: session_id.clone(),
                            error,
                        });
                    }
                    Err(error) => {
                        let _ = runtime_event_tx.send(RuntimeEvent::SessionExited {
                            session_id: session_id.clone(),
                            error: Some(error.to_string()),
                        });
                    }
                }
                break;
            }
        }
    }

    Ok(())
}

fn ordered_stream_events(
    session_id: &str,
    message: StreamMessage,
    pending_prompt_request_id: &mut Option<RequestId>,
) -> Vec<RuntimeEvent> {
    match (message.direction, message.message) {
        (
            StreamMessageDirection::Outgoing,
            StreamMessageContent::Request { id, method, params },
        ) if method.as_ref() == acp::AGENT_METHOD_NAMES.session_prompt => {
            let Some(params) = params else {
                return Vec::new();
            };
            let Ok(prompt_request) = serde_json::from_value::<acp::PromptRequest>(params) else {
                return Vec::new();
            };
            if prompt_request.session_id.0.as_ref() == session_id {
                *pending_prompt_request_id = Some(id);
            }
            Vec::new()
        }
        (
            StreamMessageDirection::Incoming,
            StreamMessageContent::Notification { method, params },
        ) if method.as_ref() == acp::CLIENT_METHOD_NAMES.session_update => {
            let Some(params) = params else {
                return Vec::new();
            };
            let Ok(notification) = serde_json::from_value::<acp::SessionNotification>(params)
            else {
                return Vec::new();
            };
            if notification.session_id.0.as_ref() != session_id {
                return Vec::new();
            }
            session_update_events(notification)
        }
        (
            StreamMessageDirection::Incoming,
            StreamMessageContent::Request { method, params, .. },
        ) if method.as_ref() == acp::CLIENT_METHOD_NAMES.session_request_permission => {
            let Some(params) = params else {
                return Vec::new();
            };
            let Ok(permission_request) =
                serde_json::from_value::<acp::RequestPermissionRequest>(params)
            else {
                return Vec::new();
            };
            if permission_request.session_id.0.as_ref() != session_id {
                return Vec::new();
            }
            vec![RuntimeEvent::SystemMessage {
                session_id: session_id.to_string(),
                content: format!(
                    "agent 请求权限，但当前客户端未提供工具能力：{}",
                    summarize_tool_call(&permission_request.tool_call)
                ),
            }]
        }
        (StreamMessageDirection::Incoming, StreamMessageContent::Response { id, result })
            if pending_prompt_request_id.as_ref() == Some(&id) =>
        {
            *pending_prompt_request_id = None;
            match result {
                Ok(_) => vec![RuntimeEvent::PromptFinished {
                    session_id: session_id.to_string(),
                }],
                Err(error) => vec![RuntimeEvent::PromptFailed {
                    session_id: session_id.to_string(),
                    error: format!("ACP prompt failed: {error}"),
                }],
            }
        }
        _ => Vec::new(),
    }
}

fn session_update_events(notification: acp::SessionNotification) -> Vec<RuntimeEvent> {
    match notification.update {
        SessionUpdate::AgentMessageChunk(chunk) => extract_chunk_text(&chunk.content)
            .map(|content| RuntimeEvent::AssistantChunk {
                session_id: notification.session_id.to_string(),
                content,
            })
            .into_iter()
            .collect(),
        SessionUpdate::AgentThoughtChunk(chunk) => extract_chunk_text(&chunk.content)
            .map(|content| RuntimeEvent::ThoughtChunk {
                session_id: notification.session_id.to_string(),
                content,
            })
            .into_iter()
            .collect(),
        SessionUpdate::ToolCall(tool_call) => {
            let detail = tool_call.raw_input.as_ref().map(|input| {
                serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
            });
            vec![RuntimeEvent::ActionEvent {
                session_id: notification.session_id.to_string(),
                kind: "tool-call".to_string(),
                title: tool_call.title.clone(),
                correlation_id: Some(tool_call.tool_call_id.to_string()),
                detail,
            }]
        }
        SessionUpdate::ToolCallUpdate(update) => {
            let title = update
                .fields
                .title
                .clone()
                .unwrap_or_else(|| "工具结果".to_string());
            let detail = update.fields.raw_output.as_ref().map(|output| {
                serde_json::to_string_pretty(output).unwrap_or_else(|_| output.to_string())
            });
            vec![RuntimeEvent::ActionEvent {
                session_id: notification.session_id.to_string(),
                kind: "tool-update".to_string(),
                title,
                correlation_id: Some(update.tool_call_id.to_string()),
                detail,
            }]
        }
        SessionUpdate::Plan(_) => vec![RuntimeEvent::ActionEvent {
            session_id: notification.session_id.to_string(),
            kind: "plan".to_string(),
            title: "更新执行计划".to_string(),
            correlation_id: None,
            detail: None,
        }],
        SessionUpdate::AvailableCommandsUpdate(_) => vec![RuntimeEvent::ActionEvent {
            session_id: notification.session_id.to_string(),
            kind: "commands".to_string(),
            title: "更新可用命令".to_string(),
            correlation_id: None,
            detail: None,
        }],
        SessionUpdate::CurrentModeUpdate(update) => vec![RuntimeEvent::ActionEvent {
            session_id: notification.session_id.to_string(),
            kind: "mode".to_string(),
            title: format!("切换到 {}", update.current_mode_id),
            correlation_id: None,
            detail: None,
        }],
        SessionUpdate::ConfigOptionUpdate(_) => vec![RuntimeEvent::ActionEvent {
            session_id: notification.session_id.to_string(),
            kind: "config".to_string(),
            title: "更新配置".to_string(),
            correlation_id: None,
            detail: None,
        }],
        SessionUpdate::SessionInfoUpdate(_) => vec![RuntimeEvent::ActionEvent {
            session_id: notification.session_id.to_string(),
            kind: "info".to_string(),
            title: "更新会话信息".to_string(),
            correlation_id: None,
            detail: None,
        }],
        SessionUpdate::UserMessageChunk(_) => Vec::new(),
        _ => Vec::new(),
    }
}

fn build_agent_command(
    agent: &AcpAgentConfig,
    workspace_root: &std::path::Path,
) -> Result<Command> {
    let mut command = if let Some(shell_command) = agent
        .shell_command
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        shell_command_command(shell_command)
    } else {
        if agent.program.trim().is_empty() {
            anyhow::bail!("ACP agent program is empty");
        }

        let mut command = Command::new(&agent.program);
        command.args(&agent.args);
        command
    };

    command
        .current_dir(workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    Ok(command)
}

fn validate_mcp_server_capabilities(
    servers: &[AcpMcpServerConfig],
    capabilities: &acp::McpCapabilities,
) -> Result<()> {
    for server in servers {
        match server {
            AcpMcpServerConfig::Stdio(_) => {}
            AcpMcpServerConfig::Http(server) => {
                if !capabilities.http {
                    anyhow::bail!(
                        "agent 不支持 MCP HTTP transport，但 session 配置了 HTTP server：{}",
                        server.name
                    );
                }
            }
            AcpMcpServerConfig::Sse(server) => {
                if !capabilities.sse {
                    anyhow::bail!(
                        "agent 不支持 MCP SSE transport，但 session 配置了 SSE server：{}",
                        server.name
                    );
                }
            }
        }
    }

    Ok(())
}

fn build_mcp_servers(servers: &[AcpMcpServerConfig]) -> Vec<acp::McpServer> {
    servers
        .iter()
        .map(|server| match server {
            AcpMcpServerConfig::Stdio(server) => acp::McpServer::Stdio(
                acp::McpServerStdio::new(&server.name, &server.command)
                    .args(server.args.clone())
                    .env(
                        server
                            .env
                            .iter()
                            .map(|pair| acp::EnvVariable::new(&pair.name, &pair.value))
                            .collect(),
                    ),
            ),
            AcpMcpServerConfig::Http(server) => acp::McpServer::Http(
                acp::McpServerHttp::new(&server.name, &server.url).headers(
                    server
                        .headers
                        .iter()
                        .map(|pair| acp::HttpHeader::new(&pair.name, &pair.value))
                        .collect(),
                ),
            ),
            AcpMcpServerConfig::Sse(server) => acp::McpServer::Sse(
                acp::McpServerSse::new(&server.name, &server.url).headers(
                    server
                        .headers
                        .iter()
                        .map(|pair| acp::HttpHeader::new(&pair.name, &pair.value))
                        .collect(),
                ),
            ),
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn shell_command_command(shell_command: &str) -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", shell_command]);
    command
}

#[cfg(not(target_os = "windows"))]
fn shell_command_command(shell_command: &str) -> Command {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut command = Command::new(shell);
    command.args(["-lc", shell_command]);
    command
}

fn summarize_tool_call(update: &acp::ToolCallUpdate) -> String {
    update
        .fields
        .title
        .clone()
        .unwrap_or_else(|| "tool update".to_string())
}

fn extract_chunk_text(content: &acp::ContentBlock) -> Option<String> {
    match content {
        acp::ContentBlock::Text(text) => Some(text.text.clone()),
        acp::ContentBlock::ResourceLink(resource) => Some(resource.uri.clone()),
        acp::ContentBlock::Image(_) => Some("<image>".to_string()),
        acp::ContentBlock::Audio(_) => Some("<audio>".to_string()),
        acp::ContentBlock::Resource(_) => Some("<resource>".to_string()),
        _ => None,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn started_agent_name(shell_command: Option<&str>, program: &str, session_id: &str) -> String {
    shell_command
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.split_whitespace().next())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if program.trim().is_empty() {
                session_id
            } else {
                program
            }
        })
        .to_string()
}

fn is_load_not_supported(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("session/load") && message.contains("不支持")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_stream_events_finish_after_prior_updates() {
        let session_id = "session-1";
        let mut pending_prompt_request_id = None;

        let prompt_request = StreamMessage {
            direction: StreamMessageDirection::Outgoing,
            message: StreamMessageContent::Request {
                id: RequestId::Number(7),
                method: acp::AGENT_METHOD_NAMES.session_prompt.into(),
                params: Some(
                    serde_json::to_value(acp::PromptRequest::new(session_id, vec!["hello".into()]))
                        .expect("prompt request should serialize"),
                ),
            },
        };
        assert!(
            ordered_stream_events(session_id, prompt_request, &mut pending_prompt_request_id)
                .is_empty()
        );
        assert_eq!(pending_prompt_request_id, Some(RequestId::Number(7)));

        let message_chunk = StreamMessage {
            direction: StreamMessageDirection::Incoming,
            message: StreamMessageContent::Notification {
                method: acp::CLIENT_METHOD_NAMES.session_update.into(),
                params: Some(
                    serde_json::to_value(acp::SessionNotification::new(
                        session_id,
                        SessionUpdate::AgentMessageChunk(acp::ContentChunk::new("part-1".into())),
                    ))
                    .expect("session notification should serialize"),
                ),
            },
        };
        assert_eq!(
            ordered_stream_events(session_id, message_chunk, &mut pending_prompt_request_id),
            vec![RuntimeEvent::AssistantChunk {
                session_id: session_id.to_string(),
                content: "part-1".to_string(),
            }]
        );

        let prompt_response = StreamMessage {
            direction: StreamMessageDirection::Incoming,
            message: StreamMessageContent::Response {
                id: RequestId::Number(7),
                result: Ok(Some(
                    serde_json::to_value(acp::PromptResponse::new(acp::StopReason::EndTurn))
                        .expect("prompt response should serialize"),
                )),
            },
        };
        assert_eq!(
            ordered_stream_events(session_id, prompt_response, &mut pending_prompt_request_id),
            vec![RuntimeEvent::PromptFinished {
                session_id: session_id.to_string(),
            }]
        );
        assert_eq!(pending_prompt_request_id, None);
    }

    #[test]
    fn ordered_stream_events_report_permission_requests_as_system_messages() {
        let session_id = "session-1";
        let mut pending_prompt_request_id = None;

        let permission_request = StreamMessage {
            direction: StreamMessageDirection::Incoming,
            message: StreamMessageContent::Request {
                id: RequestId::Number(9),
                method: acp::CLIENT_METHOD_NAMES.session_request_permission.into(),
                params: Some(
                    serde_json::to_value(acp::RequestPermissionRequest::new(
                        session_id,
                        acp::ToolCallUpdate::new(
                            "tool-1",
                            acp::ToolCallUpdateFields::new().title("Read file"),
                        ),
                        vec![],
                    ))
                    .expect("permission request should serialize"),
                ),
            },
        };

        let events = ordered_stream_events(
            session_id,
            permission_request,
            &mut pending_prompt_request_id,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            RuntimeEvent::SystemMessage { session_id: event_session_id, content }
                if event_session_id == session_id && content.contains("agent 请求权限")
        ));
    }

    #[test]
    fn new_turn_events_do_not_backfill_into_older_pending_assistant() {
        let mut record = SessionRecord {
            agent: AcpAgentConfig {
                id: "agent".to_string(),
                name: "Agent".to_string(),
                program: "agent".to_string(),
                args: Vec::new(),
                shell_command: None,
                mcp_servers: Vec::new(),
            },
            mcp_servers: Vec::new(),
            summary: AcpSessionSummary {
                session_id: "session-1".to_string(),
                workspace_root: "/tmp".to_string(),
                title: "tmp".to_string(),
                agent_id: Some("agent".to_string()),
                agent_name: "Agent".to_string(),
                status: AcpSessionStatus::Idle,
                error_level: None,
                attention: false,
                is_active: false,
                last_error: None,
                last_updated_at_ms: 0,
            },
            messages: vec![AcpSessionMessage {
                id: "0".to_string(),
                role: AcpMessageRole::Assistant,
                blocks: vec![AcpMessageBlock::Content {
                    text: "older".to_string(),
                }],
                pending: true,
            }],
            next_message_id: 1,
            command_tx: mpsc::unbounded_channel().0,
        };

        record.push_message(AcpMessageRole::User, "next".to_string(), false);
        record.start_assistant_message();
        record.append_action_event("tool-call".to_string(), "Read file".to_string(), None, None);

        assert_eq!(record.messages.len(), 3);
        assert!(matches!(
            &record.messages[0].blocks[..],
            [AcpMessageBlock::Content { text }] if text == "older"
        ));
        assert!(matches!(record.messages[1].role, AcpMessageRole::User));
        assert!(matches!(
            &record.messages[2].blocks[..],
            [AcpMessageBlock::Actions { items }]
                if items.len() == 1
                    && items[0].title == "Read file"
                    && items[0].correlation_id.is_none()
        ));
    }
}
