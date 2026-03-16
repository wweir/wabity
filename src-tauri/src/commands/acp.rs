use tauri::{
    ipc::{Channel, Invoke, InvokeError},
    State, Wry,
};

use crate::{
    domain::acp::{
        AcpAgentCatalog, AcpMcpServerCatalog, AcpMcpServerConfig, AcpRestoreNotice,
        AcpSessionDetail, AcpSessionSummary,
    },
    state::AppState,
};

pub async fn get_acp_agents(state: State<'_, AppState>) -> Result<AcpAgentCatalog, String> {
    state.acp_agents().await.map_err(|error| error.to_string())
}

pub async fn set_acp_agents(
    state: State<'_, AppState>,
    agents: Vec<crate::domain::acp::AcpAgentConfig>,
    default_agent_id: Option<String>,
) -> Result<AcpAgentCatalog, String> {
    state
        .update_acp_agents(AcpAgentCatalog {
            agents,
            default_agent_id,
        })
        .await
        .map_err(|error| error.to_string())
}

pub async fn get_acp_mcp_servers(
    state: State<'_, AppState>,
) -> Result<AcpMcpServerCatalog, String> {
    state
        .acp_mcp_servers()
        .await
        .map_err(|error| error.to_string())
}

pub async fn set_acp_mcp_servers(
    state: State<'_, AppState>,
    servers: Vec<AcpMcpServerConfig>,
) -> Result<AcpMcpServerCatalog, String> {
    state
        .update_acp_mcp_servers(AcpMcpServerCatalog { servers })
        .await
        .map_err(|error| error.to_string())
}

pub async fn list_acp_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<AcpSessionSummary>, String> {
    Ok(state.acp().list_sessions().await)
}

pub async fn take_acp_restore_notices(
    state: State<'_, AppState>,
) -> Result<Vec<AcpRestoreNotice>, String> {
    Ok(state.take_acp_restore_notices().await)
}

pub async fn get_acp_session_detail(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<AcpSessionDetail>, String> {
    Ok(state.acp().session_detail(&session_id).await)
}

pub async fn create_acp_session(
    state: State<'_, AppState>,
    agent_id: Option<String>,
) -> Result<AcpSessionDetail, String> {
    state
        .create_acp_session(agent_id)
        .await
        .map_err(|error| error.to_string())
}

pub async fn activate_acp_session(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<Vec<AcpSessionSummary>, String> {
    state
        .activate_acp_session(session_id)
        .await
        .map_err(|error| error.to_string())
}

pub async fn send_acp_prompt(
    state: State<'_, AppState>,
    session_id: String,
    prompt: String,
) -> Result<AcpSessionDetail, String> {
    state
        .send_acp_prompt(&session_id, prompt)
        .await
        .map_err(|error| error.to_string())
}

pub async fn cancel_acp_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    state
        .acp()
        .cancel_session(&session_id)
        .await
        .map_err(|error| error.to_string())
}

pub async fn close_acp_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    state
        .close_acp_session(&session_id)
        .await
        .map_err(|error| error.to_string())
}

pub async fn subscribe_acp_session_updates(
    state: State<'_, AppState>,
    on_event: Channel<AcpSessionDetail>,
) -> Result<(), String> {
    state.acp().subscribe_session_updates(on_event).await;
    Ok(())
}

pub async fn subscribe_acp_session_removals(
    state: State<'_, AppState>,
    on_event: Channel<String>,
) -> Result<(), String> {
    state.acp().subscribe_session_removals(on_event).await;
    Ok(())
}

pub(crate) fn handle_invoke(invoke: Invoke<Wry>) -> bool {
    match invoke.message.command() {
        "get_acp_agents" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "get_acp_agents", "state")?;
                get_acp_agents(state).await.map_err(InvokeError::from)
            });
            true
        }
        "set_acp_agents" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "set_acp_agents", "state")?;
                let agents = super::parse_arg(&invoke, "set_acp_agents", "agents")?;
                let default_agent_id =
                    super::parse_arg(&invoke, "set_acp_agents", "defaultAgentId")?;
                set_acp_agents(state, agents, default_agent_id)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "get_acp_mcp_servers" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "get_acp_mcp_servers", "state")?;
                get_acp_mcp_servers(state).await.map_err(InvokeError::from)
            });
            true
        }
        "set_acp_mcp_servers" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "set_acp_mcp_servers", "state")?;
                let servers = super::parse_arg(&invoke, "set_acp_mcp_servers", "servers")?;
                set_acp_mcp_servers(state, servers)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "list_acp_sessions" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "list_acp_sessions", "state")?;
                list_acp_sessions(state).await.map_err(InvokeError::from)
            });
            true
        }
        "take_acp_restore_notices" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "take_acp_restore_notices", "state")?;
                take_acp_restore_notices(state)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "get_acp_session_detail" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "get_acp_session_detail", "state")?;
                let session_id = super::parse_arg(&invoke, "get_acp_session_detail", "sessionId")?;
                get_acp_session_detail(state, session_id)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "create_acp_session" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "create_acp_session", "state")?;
                let agent_id = super::parse_arg(&invoke, "create_acp_session", "agentId")?;
                create_acp_session(state, agent_id)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "activate_acp_session" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "activate_acp_session", "state")?;
                let session_id = super::parse_arg(&invoke, "activate_acp_session", "sessionId")?;
                activate_acp_session(state, session_id)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "send_acp_prompt" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "send_acp_prompt", "state")?;
                let session_id = super::parse_arg(&invoke, "send_acp_prompt", "sessionId")?;
                let prompt = super::parse_arg(&invoke, "send_acp_prompt", "prompt")?;
                send_acp_prompt(state, session_id, prompt)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "cancel_acp_session" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "cancel_acp_session", "state")?;
                let session_id = super::parse_arg(&invoke, "cancel_acp_session", "sessionId")?;
                cancel_acp_session(state, session_id)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "close_acp_session" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "close_acp_session", "state")?;
                let session_id = super::parse_arg(&invoke, "close_acp_session", "sessionId")?;
                close_acp_session(state, session_id)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "subscribe_acp_session_updates" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "subscribe_acp_session_updates", "state")?;
                let on_event =
                    super::parse_arg(&invoke, "subscribe_acp_session_updates", "onEvent")?;
                subscribe_acp_session_updates(state, on_event)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "subscribe_acp_session_removals" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "subscribe_acp_session_removals", "state")?;
                let on_event =
                    super::parse_arg(&invoke, "subscribe_acp_session_removals", "onEvent")?;
                subscribe_acp_session_removals(state, on_event)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        _ => false,
    }
}
