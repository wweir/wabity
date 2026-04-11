use std::time::Duration;

use tauri::{
    ipc::{Invoke, InvokeError},
    AppHandle, State, Wry,
};
use tokio::time::sleep;

use crate::{domain::clipboard::ClipboardHistorySnapshot, infrastructure::window, state::AppState};

const EXTERNAL_PASTE_AFTER_HIDE_DELAY: Duration = Duration::from_millis(140);
const EXTERNAL_PASTE_TARGET_FRONTMOST_TIMEOUT: Duration = Duration::from_millis(500);

pub async fn get_clipboard_history(
    state: State<'_, AppState>,
) -> Result<ClipboardHistorySnapshot, String> {
    Ok(state.clipboard().snapshot().await)
}

pub async fn toggle_clipboard_history_entry_pin(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<ClipboardHistorySnapshot, String> {
    state
        .clipboard()
        .toggle_pin(&entry_id)
        .await
        .map_err(|error| error.to_string())
}

pub async fn delete_clipboard_history_entry(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<ClipboardHistorySnapshot, String> {
    state
        .clipboard()
        .delete_entry(&entry_id)
        .await
        .map_err(|error| error.to_string())
}

pub async fn paste_clipboard_history_entry(
    app: AppHandle,
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<ClipboardHistorySnapshot, String> {
    let snapshot = state
        .clipboard()
        .prepare_entry_for_external_paste(&entry_id)
        .await
        .map_err(|error| error.to_string())?;
    window::hide_launcher_window(&app, Some("clipboard")).map_err(|error| error.to_string())?;
    match window::reactivate_clipboard_external_paste_target(&app) {
        Ok(Some(target_pid)) => {
            let target_became_frontmost = window::wait_for_frontmost_application_pid(
                target_pid,
                EXTERNAL_PASTE_TARGET_FRONTMOST_TIMEOUT,
            )
            .await;
            if !target_became_frontmost {
                tracing::warn!(
                    target_pid,
                    timeout_ms = EXTERNAL_PASTE_TARGET_FRONTMOST_TIMEOUT.as_millis(),
                    "clipboard external paste target did not become frontmost before paste"
                );
            }
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(
                error = format_args!("{:#}", error),
                "failed to reactivate remembered application before clipboard paste"
            );
        }
    }
    sleep(EXTERNAL_PASTE_AFTER_HIDE_DELAY).await;
    state
        .clipboard()
        .send_paste_shortcut()
        .await
        .map_err(|error| error.to_string())?;
    Ok(snapshot)
}

pub(crate) fn handle_invoke(invoke: Invoke<Wry>) -> bool {
    match invoke.message.command() {
        "get_clipboard_history" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "get_clipboard_history", "state")?;
                get_clipboard_history(state)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "toggle_clipboard_history_entry_pin" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state =
                    super::parse_arg(&invoke, "toggle_clipboard_history_entry_pin", "state")?;
                let entry_id =
                    super::parse_arg(&invoke, "toggle_clipboard_history_entry_pin", "entryId")?;
                toggle_clipboard_history_entry_pin(state, entry_id)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "delete_clipboard_history_entry" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "delete_clipboard_history_entry", "state")?;
                let entry_id =
                    super::parse_arg(&invoke, "delete_clipboard_history_entry", "entryId")?;
                delete_clipboard_history_entry(state, entry_id)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "paste_clipboard_history_entry" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let app = super::parse_arg(&invoke, "paste_clipboard_history_entry", "app")?;
                let state = super::parse_arg(&invoke, "paste_clipboard_history_entry", "state")?;
                let entry_id =
                    super::parse_arg(&invoke, "paste_clipboard_history_entry", "entryId")?;
                paste_clipboard_history_entry(app, state, entry_id)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        _ => false,
    }
}
