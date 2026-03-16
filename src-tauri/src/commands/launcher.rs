use tauri::{
    ipc::{Invoke, InvokeError},
    AppHandle, Emitter, State, Wry,
};

use crate::{
    domain::{
        actions::ActionMatch, application::InstalledAppMatch, execution::ExecutionRequest,
        execution::ExecutionResult, file_search::FileSearchMatch, query::QueryPayload,
    },
    infrastructure::{config::normalize_workspace_root, window},
    state::{AppState, ShortcutRuntimeState},
};

pub fn match_actions(
    state: State<'_, AppState>,
    query: QueryPayload,
) -> Result<Vec<ActionMatch>, String> {
    state
        .matcher()
        .match_actions(&query)
        .map_err(|error| error.to_string())
}

pub async fn search_files(
    state: State<'_, AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<FileSearchMatch>, String> {
    let workspace = state.workspace().await.map_err(|error| error.to_string())?;
    let workspace_root =
        normalize_workspace_root(&workspace.root_path).map_err(|error| error.to_string())?;

    state
        .file_search()
        .search(&workspace_root, &query, limit)
        .map_err(|error| error.to_string())
}

pub fn search_apps(
    state: State<'_, AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<InstalledAppMatch>, String> {
    state
        .application()
        .search(&query, limit)
        .map_err(|error| error.to_string())
}

pub fn execute_action(
    state: State<'_, AppState>,
    request: ExecutionRequest,
) -> Result<ExecutionResult, String> {
    state
        .executor()
        .execute(&request)
        .map_err(|error| error.to_string())
}

pub fn launch_app(state: State<'_, AppState>, path: String) -> Result<ExecutionResult, String> {
    state
        .application()
        .launch(&path)
        .map_err(|error| error.to_string())
}

pub fn hide_launcher_window(app: AppHandle) -> Result<(), String> {
    window::hide_main_window(&app).map_err(|error| error.to_string())
}

pub fn resize_launcher_window(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    window::resize_main_window(&app, width, height).map_err(|error| error.to_string())
}

pub fn begin_transient_window_interaction(
    shortcut_state: State<'_, ShortcutRuntimeState>,
) -> Result<(), String> {
    shortcut_state.begin_transient_window_interaction();
    Ok(())
}

pub fn end_transient_window_interaction(
    shortcut_state: State<'_, ShortcutRuntimeState>,
) -> Result<(), String> {
    shortcut_state.end_transient_window_interaction();
    Ok(())
}

pub async fn get_shortcut(
    state: State<'_, AppState>,
) -> Result<crate::infrastructure::config::ShortcutConfig, String> {
    state.config().await.map_err(|error| error.to_string())
}

pub async fn set_shortcut(
    app: AppHandle,
    state: State<'_, AppState>,
    shortcut_state: State<'_, ShortcutRuntimeState>,
    key: String,
    shortcut: String,
) -> Result<(), String> {
    let next_shortcut = crate::infrastructure::hotkey::parse_shortcut(&shortcut)
        .ok_or_else(|| format!("invalid shortcut: {shortcut}"))?;

    let previous_shortcut = match key.as_str() {
        "toggle_launcher" => shortcut_state.current_launcher_shortcut(),
        "ocr_capture" => shortcut_state.current_ocr_shortcut(),
        _ => return Err(format!("unknown shortcut key: {key}")),
    };

    if let Some(previous_shortcut) = previous_shortcut {
        crate::infrastructure::hotkey::unregister_shortcut(&app, previous_shortcut)
            .map_err(|error| error.to_string())?;
    }

    if let Err(error) = crate::infrastructure::hotkey::register_shortcut(&app, next_shortcut) {
        if let Some(previous_shortcut) = previous_shortcut {
            let _ = crate::infrastructure::hotkey::register_shortcut(&app, previous_shortcut);
        }
        return Err(error.to_string());
    }

    if let Err(error) = state.update_shortcut(&key, &shortcut).await {
        let _ = crate::infrastructure::hotkey::unregister_shortcut(&app, next_shortcut);
        if let Some(previous_shortcut) = previous_shortcut {
            let _ = crate::infrastructure::hotkey::register_shortcut(&app, previous_shortcut);
        }
        return Err(error.to_string());
    }

    match key.as_str() {
        "toggle_launcher" => shortcut_state.set_launcher_shortcut(Some(next_shortcut)),
        "ocr_capture" => shortcut_state.set_ocr_shortcut(Some(next_shortcut)),
        _ => return Err(format!("unknown shortcut key: {key}")),
    }

    let config = state.config().await.map_err(|error| error.to_string())?;
    app.emit("shortcut-updated", config)
        .map_err(|error| error.to_string())?;

    Ok(())
}

pub(crate) fn handle_invoke(invoke: Invoke<Wry>) -> bool {
    match invoke.message.command() {
        "match_actions" => {
            let resolver = invoke.resolver.clone();
            let Some(state) = super::parse_or_invoke_error(&invoke, "match_actions", "state")
            else {
                return true;
            };
            let Some(query) = super::parse_or_invoke_error(&invoke, "match_actions", "query")
            else {
                return true;
            };

            resolver.respond(match_actions(state, query).map_err(InvokeError::from));
            true
        }
        "search_files" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "search_files", "state")?;
                let query = super::parse_arg(&invoke, "search_files", "query")?;
                let limit = super::parse_arg(&invoke, "search_files", "limit")?;

                search_files(state, query, limit)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "search_apps" => {
            let resolver = invoke.resolver.clone();
            let Some(state) = super::parse_or_invoke_error(&invoke, "search_apps", "state") else {
                return true;
            };
            let Some(query) = super::parse_or_invoke_error(&invoke, "search_apps", "query") else {
                return true;
            };
            let Some(limit) = super::parse_or_invoke_error(&invoke, "search_apps", "limit") else {
                return true;
            };

            resolver.respond(search_apps(state, query, limit).map_err(InvokeError::from));
            true
        }
        "execute_action" => {
            let resolver = invoke.resolver.clone();
            let Some(state) = super::parse_or_invoke_error(&invoke, "execute_action", "state")
            else {
                return true;
            };
            let Some(request) = super::parse_or_invoke_error(&invoke, "execute_action", "request")
            else {
                return true;
            };

            resolver.respond(execute_action(state, request).map_err(InvokeError::from));
            true
        }
        "launch_app" => {
            let resolver = invoke.resolver.clone();
            let Some(state) = super::parse_or_invoke_error(&invoke, "launch_app", "state") else {
                return true;
            };
            let Some(path) = super::parse_or_invoke_error(&invoke, "launch_app", "path") else {
                return true;
            };

            resolver.respond(launch_app(state, path).map_err(InvokeError::from));
            true
        }
        "hide_launcher_window" => {
            let Some(app) = super::parse_or_invoke_error(&invoke, "hide_launcher_window", "app")
            else {
                return true;
            };

            invoke
                .resolver
                .respond(hide_launcher_window(app).map_err(InvokeError::from));
            true
        }
        "resize_launcher_window" => {
            let Some(app) = super::parse_or_invoke_error(&invoke, "resize_launcher_window", "app")
            else {
                return true;
            };
            let Some(width) =
                super::parse_or_invoke_error(&invoke, "resize_launcher_window", "width")
            else {
                return true;
            };
            let Some(height) =
                super::parse_or_invoke_error(&invoke, "resize_launcher_window", "height")
            else {
                return true;
            };

            invoke
                .resolver
                .respond(resize_launcher_window(app, width, height).map_err(InvokeError::from));
            true
        }
        "begin_transient_window_interaction" => {
            let resolver = invoke.resolver.clone();
            let Some(shortcut_state) = super::parse_or_invoke_error(
                &invoke,
                "begin_transient_window_interaction",
                "shortcutState",
            ) else {
                return true;
            };

            resolver.respond(
                begin_transient_window_interaction(shortcut_state).map_err(InvokeError::from),
            );
            true
        }
        "end_transient_window_interaction" => {
            let resolver = invoke.resolver.clone();
            let Some(shortcut_state) = super::parse_or_invoke_error(
                &invoke,
                "end_transient_window_interaction",
                "shortcutState",
            ) else {
                return true;
            };

            resolver.respond(
                end_transient_window_interaction(shortcut_state).map_err(InvokeError::from),
            );
            true
        }
        "get_shortcut" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "get_shortcut", "state")?;
                get_shortcut(state).await.map_err(InvokeError::from)
            });
            true
        }
        "set_shortcut" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let app = super::parse_arg(&invoke, "set_shortcut", "app")?;
                let state = super::parse_arg(&invoke, "set_shortcut", "state")?;
                let shortcut_state = super::parse_arg(&invoke, "set_shortcut", "shortcutState")?;
                let key = super::parse_arg(&invoke, "set_shortcut", "key")?;
                let shortcut = super::parse_arg(&invoke, "set_shortcut", "shortcut")?;

                set_shortcut(app, state, shortcut_state, key, shortcut)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        _ => false,
    }
}
