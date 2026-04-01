use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tauri::{
    ipc::{Invoke, InvokeError},
    AppHandle, Emitter, State, Wry,
};

use crate::{
    domain::{
        actions::ActionMatch, application::InstalledAppMatch, execution::ExecutionProgressEvent,
        execution::ExecutionRequest, execution::ExecutionResult, file_search::FileSearchMatch,
        query::QueryPayload,
    },
    infrastructure::{
        config::{normalize_workspace_root, ShortcutKey},
        window,
    },
    services::rag,
    state::{AppState, ShortcutRuntimeState},
};

const EXECUTION_PROGRESS_EVENT: &str = "execution-progress";
const MAX_LAUNCHER_BLUR_AUTO_HIDE_SUPPRESSION_MS: u64 = 5_000;

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

pub async fn execute_action(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ExecutionRequest,
) -> Result<ExecutionResult, String> {
    state
        .execute_action_with_progress(
            request,
            Some(Arc::new(move |progress: ExecutionProgressEvent| {
                let _ = app.emit(EXECUTION_PROGRESS_EVENT, progress);
            })),
        )
        .await
        .map_err(|error| error.to_string())
}

pub fn launch_app(state: State<'_, AppState>, path: String) -> Result<ExecutionResult, String> {
    state
        .application()
        .launch(&path)
        .map_err(|error| error.to_string())
}

pub async fn open_document_reference(
    state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let normalized = path.trim();
    if normalized.is_empty() {
        return Err("文档引用路径不能为空".to_string());
    }

    let workspace = state.workspace().await.map_err(|error| error.to_string())?;
    let workspace_root =
        normalize_workspace_root(&workspace.root_path).map_err(|error| error.to_string())?;
    let settings = state
        .app_settings()
        .await
        .map_err(|error| error.to_string())?;
    let allowed_roots = rag::collect_document_access_roots(&workspace_root, &settings.rag);
    let canonical_path = PathBuf::from(normalized)
        .canonicalize()
        .map_err(|error| format!("无法解析文档引用路径: {error}"))?;
    if !canonical_path.is_file() {
        return Err(format!("文档引用不是文件: {}", canonical_path.display()));
    }
    if !rag::path_is_within_roots(&canonical_path, &allowed_roots) {
        return Err(format!(
            "文档引用超出允许范围，只能打开当前 workspace 或显式配置的 RAG 目录: {}",
            canonical_path.display()
        ));
    }

    state
        .open_document_path(&canonical_path)
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

pub fn arm_launcher_blur_auto_hide_suppression(
    shortcut_state: State<'_, ShortcutRuntimeState>,
    duration_ms: u64,
) -> Result<(), String> {
    let applied_duration =
        Duration::from_millis(duration_ms.clamp(1, MAX_LAUNCHER_BLUR_AUTO_HIDE_SUPPRESSION_MS));
    let sequence = shortcut_state.cancel_launcher_blur_auto_hide_confirmation();
    shortcut_state.arm_launcher_blur_auto_hide_suppression(applied_duration);
    tracing::info!(
        suppression_ms = applied_duration.as_millis(),
        blur_auto_hide_sequence = sequence,
        "armed launcher blur auto-hide suppression via IPC"
    );
    Ok(())
}

pub fn set_launcher_blur_auto_hide_enabled(
    shortcut_state: State<'_, ShortcutRuntimeState>,
    enabled: bool,
) -> Result<(), String> {
    shortcut_state.cancel_launcher_blur_auto_hide_confirmation();
    shortcut_state.set_launcher_blur_auto_hide_enabled(enabled);
    tracing::info!(enabled, "set launcher blur auto-hide enabled via IPC");
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
    let key = ShortcutKey::parse(&key).map_err(|error| error.to_string())?;
    let next_shortcut = crate::infrastructure::hotkey::parse_shortcut(&shortcut)
        .ok_or_else(|| format!("invalid shortcut: {shortcut}"))?;

    let previous_shortcut = shortcut_state.current_shortcut(key);

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

    if let Err(error) = state.update_shortcut(key, &shortcut).await {
        let _ = crate::infrastructure::hotkey::unregister_shortcut(&app, next_shortcut);
        if let Some(previous_shortcut) = previous_shortcut {
            let _ = crate::infrastructure::hotkey::register_shortcut(&app, previous_shortcut);
        }
        return Err(error.to_string());
    }

    shortcut_state.set_shortcut(key, Some(next_shortcut));

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
            resolver.respond_async(async move {
                let app = super::parse_arg(&invoke, "execute_action", "app")?;
                let state = super::parse_arg(&invoke, "execute_action", "state")?;
                let request = super::parse_arg(&invoke, "execute_action", "request")?;

                execute_action(app, state, request)
                    .await
                    .map_err(InvokeError::from)
            });
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
        "open_document_reference" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "open_document_reference", "state")?;
                let path = super::parse_arg(&invoke, "open_document_reference", "path")?;

                open_document_reference(state, path)
                    .await
                    .map_err(InvokeError::from)
            });
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
        "arm_launcher_blur_auto_hide_suppression" => {
            let resolver = invoke.resolver.clone();
            let Some(shortcut_state) = super::parse_or_invoke_error(
                &invoke,
                "arm_launcher_blur_auto_hide_suppression",
                "shortcutState",
            ) else {
                return true;
            };
            let Some(duration_ms) = super::parse_or_invoke_error(
                &invoke,
                "arm_launcher_blur_auto_hide_suppression",
                "durationMs",
            ) else {
                return true;
            };

            resolver.respond(
                arm_launcher_blur_auto_hide_suppression(shortcut_state, duration_ms)
                    .map_err(InvokeError::from),
            );
            true
        }
        "set_launcher_blur_auto_hide_enabled" => {
            let resolver = invoke.resolver.clone();
            let Some(shortcut_state) = super::parse_or_invoke_error(
                &invoke,
                "set_launcher_blur_auto_hide_enabled",
                "shortcutState",
            ) else {
                return true;
            };
            let Some(enabled) = super::parse_or_invoke_error(
                &invoke,
                "set_launcher_blur_auto_hide_enabled",
                "enabled",
            ) else {
                return true;
            };

            resolver.respond(
                set_launcher_blur_auto_hide_enabled(shortcut_state, enabled)
                    .map_err(InvokeError::from),
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
