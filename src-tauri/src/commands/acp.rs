use tauri::{
    ipc::{Channel, Invoke, InvokeError},
    State, Wry,
};

use crate::{
    domain::acp::{
        AcpAgentCatalog, AcpMcpServerCatalog, AcpMcpServerConfig, AcpRestoreNotice,
        AcpSessionDetail, AcpSessionSummary, BuiltinAgentToolStatus,
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
    builtin: crate::domain::acp::BuiltinMcpConfig,
) -> Result<AcpMcpServerCatalog, String> {
    state
        .update_acp_mcp_servers(AcpMcpServerCatalog { servers, builtin })
        .await
        .map_err(|error| error.to_string())
}

pub async fn get_builtin_agent_tool_status(
    state: State<'_, AppState>,
) -> Result<BuiltinAgentToolStatus, String> {
    Ok(state.builtin_agent_tool_status().await)
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
    state
        .acp_session_detail(&session_id)
        .await
        .map_err(|error| error.to_string())
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

pub async fn set_acp_session_mode(
    state: State<'_, AppState>,
    session_id: String,
    mode_id: String,
) -> Result<AcpSessionDetail, String> {
    state
        .set_acp_session_mode(&session_id, mode_id)
        .await
        .map_err(|error| error.to_string())
}

pub async fn set_acp_session_config_option(
    state: State<'_, AppState>,
    session_id: String,
    config_id: String,
    value_id: String,
) -> Result<AcpSessionDetail, String> {
    state
        .set_acp_session_config_option(&session_id, config_id, value_id)
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

pub async fn unsubscribe_acp_session_updates(
    state: State<'_, AppState>,
    channel_id: u32,
) -> Result<(), String> {
    state.acp().unsubscribe_session_updates(channel_id).await;
    Ok(())
}

pub async fn unsubscribe_acp_session_removals(
    state: State<'_, AppState>,
    channel_id: u32,
) -> Result<(), String> {
    state.acp().unsubscribe_session_removals(channel_id).await;
    Ok(())
}

pub(crate) fn handle_invoke(invoke: Invoke<Wry>) -> bool {
    match invoke.message.command() {
        "get_acp_agents" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "get_acp_agents", "state")?;
            get_acp_agents(state).await.map_err(InvokeError::from)
        }),
        "set_acp_agents" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "set_acp_agents", "state")?;
            let agents = super::parse_arg(&invoke, "set_acp_agents", "agents")?;
            let default_agent_id = super::parse_arg(&invoke, "set_acp_agents", "defaultAgentId")?;
            set_acp_agents(state, agents, default_agent_id)
                .await
                .map_err(InvokeError::from)
        }),
        "get_acp_mcp_servers" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "get_acp_mcp_servers", "state")?;
            get_acp_mcp_servers(state).await.map_err(InvokeError::from)
        }),
        "set_acp_mcp_servers" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "set_acp_mcp_servers", "state")?;
            let servers = super::parse_arg(&invoke, "set_acp_mcp_servers", "servers")?;
            let builtin = super::parse_arg(&invoke, "set_acp_mcp_servers", "builtin")?;
            set_acp_mcp_servers(state, servers, builtin)
                .await
                .map_err(InvokeError::from)
        }),
        "get_builtin_agent_tool_status" => {
            super::respond_async(invoke.resolver.clone(), async move {
                let state = super::parse_arg(&invoke, "get_builtin_agent_tool_status", "state")?;
                get_builtin_agent_tool_status(state)
                    .await
                    .map_err(InvokeError::from)
            })
        }
        "list_acp_sessions" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "list_acp_sessions", "state")?;
            list_acp_sessions(state).await.map_err(InvokeError::from)
        }),
        "take_acp_restore_notices" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "take_acp_restore_notices", "state")?;
            take_acp_restore_notices(state)
                .await
                .map_err(InvokeError::from)
        }),
        "get_acp_session_detail" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "get_acp_session_detail", "state")?;
            let session_id = super::parse_arg(&invoke, "get_acp_session_detail", "sessionId")?;
            get_acp_session_detail(state, session_id)
                .await
                .map_err(InvokeError::from)
        }),
        "create_acp_session" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "create_acp_session", "state")?;
            let agent_id = super::parse_arg(&invoke, "create_acp_session", "agentId")?;
            create_acp_session(state, agent_id)
                .await
                .map_err(InvokeError::from)
        }),
        "activate_acp_session" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "activate_acp_session", "state")?;
            let session_id = super::parse_arg(&invoke, "activate_acp_session", "sessionId")?;
            activate_acp_session(state, session_id)
                .await
                .map_err(InvokeError::from)
        }),
        "send_acp_prompt" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "send_acp_prompt", "state")?;
            let session_id = super::parse_arg(&invoke, "send_acp_prompt", "sessionId")?;
            let prompt = super::parse_arg(&invoke, "send_acp_prompt", "prompt")?;
            send_acp_prompt(state, session_id, prompt)
                .await
                .map_err(InvokeError::from)
        }),
        "set_acp_session_mode" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "set_acp_session_mode", "state")?;
            let session_id = super::parse_arg(&invoke, "set_acp_session_mode", "sessionId")?;
            let mode_id = super::parse_arg(&invoke, "set_acp_session_mode", "modeId")?;
            set_acp_session_mode(state, session_id, mode_id)
                .await
                .map_err(InvokeError::from)
        }),
        "set_acp_session_config_option" => {
            super::respond_async(invoke.resolver.clone(), async move {
                let state = super::parse_arg(&invoke, "set_acp_session_config_option", "state")?;
                let session_id =
                    super::parse_arg(&invoke, "set_acp_session_config_option", "sessionId")?;
                let config_id =
                    super::parse_arg(&invoke, "set_acp_session_config_option", "configId")?;
                let value_id =
                    super::parse_arg(&invoke, "set_acp_session_config_option", "valueId")?;
                set_acp_session_config_option(state, session_id, config_id, value_id)
                    .await
                    .map_err(InvokeError::from)
            })
        }
        "cancel_acp_session" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "cancel_acp_session", "state")?;
            let session_id = super::parse_arg(&invoke, "cancel_acp_session", "sessionId")?;
            cancel_acp_session(state, session_id)
                .await
                .map_err(InvokeError::from)
        }),
        "close_acp_session" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "close_acp_session", "state")?;
            let session_id = super::parse_arg(&invoke, "close_acp_session", "sessionId")?;
            close_acp_session(state, session_id)
                .await
                .map_err(InvokeError::from)
        }),
        "subscribe_acp_session_updates" => {
            super::respond_async(invoke.resolver.clone(), async move {
                let state = super::parse_arg(&invoke, "subscribe_acp_session_updates", "state")?;
                let on_event =
                    super::parse_arg(&invoke, "subscribe_acp_session_updates", "onEvent")?;
                subscribe_acp_session_updates(state, on_event)
                    .await
                    .map_err(InvokeError::from)
            })
        }
        "subscribe_acp_session_removals" => {
            super::respond_async(invoke.resolver.clone(), async move {
                let state = super::parse_arg(&invoke, "subscribe_acp_session_removals", "state")?;
                let on_event =
                    super::parse_arg(&invoke, "subscribe_acp_session_removals", "onEvent")?;
                subscribe_acp_session_removals(state, on_event)
                    .await
                    .map_err(InvokeError::from)
            })
        }
        "unsubscribe_acp_session_updates" => {
            super::respond_async(invoke.resolver.clone(), async move {
                let state = super::parse_arg(&invoke, "unsubscribe_acp_session_updates", "state")?;
                let channel_id =
                    super::parse_arg(&invoke, "unsubscribe_acp_session_updates", "channelId")?;
                unsubscribe_acp_session_updates(state, channel_id)
                    .await
                    .map_err(InvokeError::from)
            })
        }
        "unsubscribe_acp_session_removals" => {
            super::respond_async(invoke.resolver.clone(), async move {
                let state = super::parse_arg(&invoke, "unsubscribe_acp_session_removals", "state")?;
                let channel_id =
                    super::parse_arg(&invoke, "unsubscribe_acp_session_removals", "channelId")?;
                unsubscribe_acp_session_removals(state, channel_id)
                    .await
                    .map_err(InvokeError::from)
            })
        }
        _ => false,
    }
}
