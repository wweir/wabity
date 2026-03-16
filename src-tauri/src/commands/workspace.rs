use tauri::{
    ipc::{Invoke, InvokeError},
    AppHandle, Emitter, State, Wry,
};

use crate::{domain::workspace::WorkspaceState, state::AppState};

pub async fn get_workspace(state: State<'_, AppState>) -> Result<WorkspaceState, String> {
    state.workspace().await.map_err(|error| error.to_string())
}

pub async fn set_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    root_path: String,
) -> Result<WorkspaceState, String> {
    let workspace = state
        .update_workspace_root(&root_path)
        .await
        .map_err(|error| error.to_string())?;

    app.emit("workspace-updated", workspace.clone())
        .map_err(|error| error.to_string())?;

    Ok(workspace)
}

pub(crate) fn handle_invoke(invoke: Invoke<Wry>) -> bool {
    match invoke.message.command() {
        "get_workspace" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "get_workspace", "state")?;
                get_workspace(state).await.map_err(InvokeError::from)
            });
            true
        }
        "set_workspace" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let app = super::parse_arg(&invoke, "set_workspace", "app")?;
                let state = super::parse_arg(&invoke, "set_workspace", "state")?;
                let root_path = super::parse_arg(&invoke, "set_workspace", "rootPath")?;
                set_workspace(app, state, root_path)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        _ => false,
    }
}
