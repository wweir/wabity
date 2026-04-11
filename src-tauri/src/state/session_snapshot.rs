use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use anyhow::Result;
use tokio::sync::RwLock as AsyncRwLock;

use crate::{
    domain::acp::{AcpAgentConfig, AcpMcpServerConfig, AcpSessionSummary},
    infrastructure::config::{ConfigStore, SavedAcpSession},
    services::acp::SessionPersistHook,
};

pub(super) fn build_acp_session_persist_hook(
    config_store: Arc<AsyncRwLock<ConfigStore>>,
) -> SessionPersistHook {
    Arc::new(move |summary, agent, mcp_servers| {
        let config_store = config_store.clone();
        Box::pin(async move {
            if let Err(error) = store_session_snapshot(
                &config_store,
                &summary,
                &agent,
                &mcp_servers,
                SnapshotWriteMode::UpdateOnly,
            )
            .await
            {
                tracing::warn!(
                    session_id = %summary.session_id,
                    error = format_args!("{:#}", error),
                    "failed to persist ACP session snapshot"
                );
            }
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapshotWriteMode {
    InsertOrUpdate,
    UpdateOnly,
}

pub(super) async fn store_session_snapshot(
    config_store: &Arc<AsyncRwLock<ConfigStore>>,
    summary: &AcpSessionSummary,
    agent: &AcpAgentConfig,
    mcp_servers: &[AcpMcpServerConfig],
    write_mode: SnapshotWriteMode,
) -> Result<()> {
    let store = config_store.write().await;
    let mut config = store.load().await?;
    let snapshot = SavedAcpSession {
        session_id: summary.session_id.clone(),
        workspace_root: summary.workspace_root.clone(),
        title: summary.title.clone(),
        agent_id: normalized_agent_id(Some(agent.id.as_str())),
        agent_name: agent.name.clone(),
        agent_program: agent.program.clone(),
        agent_args: agent.args.clone(),
        agent_shell_command: agent.shell_command.clone(),
        agent_launch_mode: agent.launch_mode,
        mcp_servers: mcp_servers.to_vec(),
        last_updated_at_ms: summary.last_updated_at_ms,
    };
    if !apply_session_snapshot(&mut config.acp.saved_sessions, snapshot, write_mode) {
        return Ok(());
    }
    sort_and_truncate_snapshots(&mut config.acp.saved_sessions);
    store.save(&config).await?;
    Ok(())
}

pub(super) fn merge_restored_session_snapshots(
    current_snapshots: Vec<SavedAcpSession>,
    initial_snapshot_ids: &HashSet<String>,
    retained_snapshots: Vec<SavedAcpSession>,
) -> Vec<SavedAcpSession> {
    let retained_by_id = retained_snapshots
        .into_iter()
        .map(|snapshot| (snapshot.session_id.clone(), snapshot))
        .collect::<HashMap<_, _>>();
    let mut merged = current_snapshots
        .into_iter()
        .filter(|snapshot| {
            !initial_snapshot_ids.contains(&snapshot.session_id)
                || retained_by_id.contains_key(&snapshot.session_id)
        })
        .collect::<Vec<_>>();

    for snapshot in retained_by_id.into_values() {
        match merged
            .iter_mut()
            .find(|item| item.session_id == snapshot.session_id)
        {
            Some(existing) if existing.last_updated_at_ms >= snapshot.last_updated_at_ms => {}
            Some(existing) => *existing = snapshot,
            None => merged.push(snapshot),
        }
    }

    sort_and_truncate_snapshots(&mut merged);
    merged
}

pub(super) fn apply_session_snapshot(
    saved_sessions: &mut Vec<SavedAcpSession>,
    snapshot: SavedAcpSession,
    write_mode: SnapshotWriteMode,
) -> bool {
    let existing_index = saved_sessions
        .iter()
        .position(|item| item.session_id == snapshot.session_id);

    match (existing_index, write_mode) {
        (Some(index), _) => {
            if !should_replace_existing_snapshot(&saved_sessions[index], &snapshot) {
                return false;
            }
            saved_sessions[index] = snapshot;
            true
        }
        (None, SnapshotWriteMode::InsertOrUpdate) => {
            saved_sessions.push(snapshot);
            true
        }
        (None, SnapshotWriteMode::UpdateOnly) => false,
    }
}

fn should_replace_existing_snapshot(
    existing: &SavedAcpSession,
    incoming: &SavedAcpSession,
) -> bool {
    existing.last_updated_at_ms < incoming.last_updated_at_ms
}

fn sort_and_truncate_snapshots(snapshots: &mut Vec<SavedAcpSession>) {
    snapshots.sort_by(|left, right| {
        right
            .last_updated_at_ms
            .cmp(&left.last_updated_at_ms)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    snapshots.truncate(16);
}

fn normalized_agent_id(agent_id: Option<&str>) -> Option<String> {
    agent_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}
