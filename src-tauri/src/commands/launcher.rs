use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{
    ipc::{Invoke, InvokeError},
    AppHandle, Emitter, State, Wry,
};

use crate::{
    domain::{
        actions::ActionMatch, application::InstalledAppMatch, execution::ExecutionProgressEvent,
        execution::ExecutionRequest, execution::ExecutionResult, file_search::FileSearchMatch,
        process::RunningProcessMatch, query::QueryPayload,
    },
    infrastructure::{
        config::{normalize_workspace_root, ShortcutKey},
        window,
    },
    services::rag,
    state::{AppState, ShortcutRuntimeState, ShortcutRuntimeStatusSnapshot},
};

const EXECUTION_PROGRESS_EVENT: &str = "execution-progress";
const SHORTCUT_RUNTIME_STATUS_CHANGED_EVENT: &str = "shortcut-runtime-status-changed";
const MAX_LAUNCHER_BLUR_AUTO_HIDE_SUPPRESSION_MS: u64 = 5_000;

fn log_launcher_search_completion<T>(
    operation: &'static str,
    query: &str,
    limit: usize,
    started_at: Instant,
    result: &Result<Vec<T>, String>,
) {
    tracing::info!(
        operation,
        query_len = query.trim().chars().count(),
        limit,
        elapsed_ms = started_at.elapsed().as_millis(),
        result_count = result.as_ref().map(|items| items.len()).unwrap_or_default(),
        "launcher search completed"
    );
}

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
    let started_at = Instant::now();
    let workspace = state.workspace().await.map_err(|error| error.to_string())?;
    let workspace_root =
        normalize_workspace_root(&workspace.root_path).map_err(|error| error.to_string())?;

    let result = state
        .file_search()
        .search(&workspace_root, &query, limit)
        .map_err(|error| error.to_string());
    log_launcher_search_completion("search_files", &query, limit, started_at, &result);
    result
}

pub fn search_apps(
    state: State<'_, AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<InstalledAppMatch>, String> {
    let started_at = Instant::now();
    let result = state
        .application()
        .search(&query, limit)
        .map_err(|error| error.to_string());
    log_launcher_search_completion("search_apps", &query, limit, started_at, &result);
    result
}

pub fn search_processes(
    state: State<'_, AppState>,
    query: String,
    limit: usize,
) -> Result<Vec<RunningProcessMatch>, String> {
    let started_at = Instant::now();
    let result = state
        .process()
        .search_running(&query, limit)
        .map_err(|error| error.to_string());
    log_launcher_search_completion("search_processes", &query, limit, started_at, &result);
    result
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

pub fn hide_launcher_window(app: AppHandle, window_label: Option<String>) -> Result<(), String> {
    window::hide_launcher_window(&app, window_label.as_deref()).map_err(|error| error.to_string())
}

pub fn dismiss_clipboard_history_panel(app: AppHandle) -> Result<(), String> {
    window::dismiss_clipboard_history_panel(&app).map_err(|error| error.to_string())
}

pub fn resize_launcher_window(
    app: AppHandle,
    width: f64,
    height: f64,
    window_label: Option<String>,
) -> Result<(), String> {
    window::resize_launcher_window(&app, width, height, window_label.as_deref())
        .map_err(|error| error.to_string())
}

pub fn insert_clipboard_history_text_into_launcher(
    app: AppHandle,
    text: String,
) -> Result<(), String> {
    window::insert_clipboard_history_text_into_launcher(&app, text)
        .map_err(|error| error.to_string())
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

pub fn get_shortcut_runtime_status(
    shortcut_state: State<'_, ShortcutRuntimeState>,
) -> ShortcutRuntimeStatusSnapshot {
    shortcut_state.shortcut_runtime_status()
}

pub fn emit_shortcut_runtime_status(
    app: &AppHandle,
    shortcut_state: &ShortcutRuntimeState,
) -> Result<(), String> {
    app.emit(
        SHORTCUT_RUNTIME_STATUS_CHANGED_EVENT,
        shortcut_state.shortcut_runtime_status(),
    )
    .map_err(|error| error.to_string())
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
        shortcut_state.set_shortcut_registration_status(
            key,
            shortcut.clone(),
            false,
            Some(error.to_string()),
        );
        let _ = emit_shortcut_runtime_status(&app, shortcut_state.inner());
        return Err(error.to_string());
    }

    if let Err(error) = state.update_shortcut(key, &shortcut).await {
        let _ = crate::infrastructure::hotkey::unregister_shortcut(&app, next_shortcut);
        if let Some(previous_shortcut) = previous_shortcut {
            let _ = crate::infrastructure::hotkey::register_shortcut(&app, previous_shortcut);
        }
        shortcut_state.set_shortcut_registration_status(
            key,
            shortcut.clone(),
            previous_shortcut.is_some(),
            Some(error.to_string()),
        );
        let _ = emit_shortcut_runtime_status(&app, shortcut_state.inner());
        return Err(error.to_string());
    }

    shortcut_state.set_shortcut(key, Some(next_shortcut));
    shortcut_state.set_shortcut_registration_status(key, shortcut.clone(), true, None);

    let config = state.config().await.map_err(|error| error.to_string())?;
    app.emit("shortcut-updated", config)
        .map_err(|error| error.to_string())?;
    emit_shortcut_runtime_status(&app, shortcut_state.inner())?;

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

            super::respond_sync(
                resolver,
                match_actions(state, query).map_err(InvokeError::from),
            )
        }
        "search_files" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "search_files", "state")?;
            let query = super::parse_arg(&invoke, "search_files", "query")?;
            let limit = super::parse_arg(&invoke, "search_files", "limit")?;

            search_files(state, query, limit)
                .await
                .map_err(InvokeError::from)
        }),
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

            super::respond_sync(
                resolver,
                search_apps(state, query, limit).map_err(InvokeError::from),
            )
        }
        "search_processes" => {
            let resolver = invoke.resolver.clone();
            let Some(state) = super::parse_or_invoke_error(&invoke, "search_processes", "state")
            else {
                return true;
            };
            let Some(query) = super::parse_or_invoke_error(&invoke, "search_processes", "query")
            else {
                return true;
            };
            let Some(limit) = super::parse_or_invoke_error(&invoke, "search_processes", "limit")
            else {
                return true;
            };

            super::respond_sync(
                resolver,
                search_processes(state, query, limit).map_err(InvokeError::from),
            )
        }
        "execute_action" => super::respond_async(invoke.resolver.clone(), async move {
            let app = super::parse_arg(&invoke, "execute_action", "app")?;
            let state = super::parse_arg(&invoke, "execute_action", "state")?;
            let request = super::parse_arg(&invoke, "execute_action", "request")?;

            execute_action(app, state, request)
                .await
                .map_err(InvokeError::from)
        }),
        "launch_app" => {
            let resolver = invoke.resolver.clone();
            let Some(state) = super::parse_or_invoke_error(&invoke, "launch_app", "state") else {
                return true;
            };
            let Some(path) = super::parse_or_invoke_error(&invoke, "launch_app", "path") else {
                return true;
            };

            super::respond_sync(resolver, launch_app(state, path).map_err(InvokeError::from))
        }
        "open_document_reference" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "open_document_reference", "state")?;
            let path = super::parse_arg(&invoke, "open_document_reference", "path")?;

            open_document_reference(state, path)
                .await
                .map_err(InvokeError::from)
        }),
        "hide_launcher_window" => {
            let resolver = invoke.resolver.clone();
            let Some(app) = super::parse_or_invoke_error(&invoke, "hide_launcher_window", "app")
            else {
                return true;
            };
            let Some(window_label) =
                super::parse_or_invoke_error(&invoke, "hide_launcher_window", "windowLabel")
            else {
                return true;
            };

            super::respond_sync(
                resolver,
                hide_launcher_window(app, window_label).map_err(InvokeError::from),
            )
        }
        "dismiss_clipboard_history_panel" => {
            let resolver = invoke.resolver.clone();
            let Some(app) =
                super::parse_or_invoke_error(&invoke, "dismiss_clipboard_history_panel", "app")
            else {
                return true;
            };

            super::respond_sync(
                resolver,
                dismiss_clipboard_history_panel(app).map_err(InvokeError::from),
            )
        }
        "resize_launcher_window" => {
            let resolver = invoke.resolver.clone();
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
            let Some(window_label) =
                super::parse_or_invoke_error(&invoke, "resize_launcher_window", "windowLabel")
            else {
                return true;
            };

            super::respond_sync(
                resolver,
                resize_launcher_window(app, width, height, window_label).map_err(InvokeError::from),
            )
        }
        "insert_clipboard_history_text_into_launcher" => {
            let resolver = invoke.resolver.clone();
            let Some(app) = super::parse_or_invoke_error(
                &invoke,
                "insert_clipboard_history_text_into_launcher",
                "app",
            ) else {
                return true;
            };
            let Some(text) = super::parse_or_invoke_error(
                &invoke,
                "insert_clipboard_history_text_into_launcher",
                "text",
            ) else {
                return true;
            };

            super::respond_sync(
                resolver,
                insert_clipboard_history_text_into_launcher(app, text).map_err(InvokeError::from),
            )
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

            super::respond_sync(
                resolver,
                begin_transient_window_interaction(shortcut_state).map_err(InvokeError::from),
            )
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

            super::respond_sync(
                resolver,
                end_transient_window_interaction(shortcut_state).map_err(InvokeError::from),
            )
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

            super::respond_sync(
                resolver,
                arm_launcher_blur_auto_hide_suppression(shortcut_state, duration_ms)
                    .map_err(InvokeError::from),
            )
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

            super::respond_sync(
                resolver,
                set_launcher_blur_auto_hide_enabled(shortcut_state, enabled)
                    .map_err(InvokeError::from),
            )
        }
        "get_shortcut" => super::respond_async(invoke.resolver.clone(), async move {
            let state = super::parse_arg(&invoke, "get_shortcut", "state")?;
            get_shortcut(state).await.map_err(InvokeError::from)
        }),
        "get_shortcut_runtime_status" => {
            let resolver = invoke.resolver.clone();
            let Some(shortcut_state) = super::parse_or_invoke_error(
                &invoke,
                "get_shortcut_runtime_status",
                "shortcutState",
            ) else {
                return true;
            };

            super::respond_sync(
                resolver,
                Ok::<ShortcutRuntimeStatusSnapshot, InvokeError>(get_shortcut_runtime_status(
                    shortcut_state,
                )),
            )
        }
        "set_shortcut" => super::respond_async(invoke.resolver.clone(), async move {
            let app = super::parse_arg(&invoke, "set_shortcut", "app")?;
            let state = super::parse_arg(&invoke, "set_shortcut", "state")?;
            let shortcut_state = super::parse_arg(&invoke, "set_shortcut", "shortcutState")?;
            let key = super::parse_arg(&invoke, "set_shortcut", "key")?;
            let shortcut = super::parse_arg(&invoke, "set_shortcut", "shortcut")?;

            set_shortcut(app, state, shortcut_state, key, shortcut)
                .await
                .map_err(InvokeError::from)
        }),
        _ => false,
    }
}
