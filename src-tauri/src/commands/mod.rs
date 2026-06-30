use std::future::Future;

use tauri::{
    ipc::{CommandArg, CommandItem, Invoke, InvokeError, InvokeResolver, IpcResponse},
    Runtime, Wry,
};

pub mod acp;
pub mod clipboard;
pub mod launcher;
pub mod rag;
pub mod settings;
pub mod workspace;

// Keep IPC dispatch on plain Rust so rust-analyzer diagnostics do not depend on Tauri proc-macro expansion.
pub(crate) fn handle_invoke(invoke: Invoke<Wry>) -> bool {
    let command = invoke.message.command().to_owned();

    match command.as_str() {
        "match_actions"
        | "search_files"
        | "search_apps"
        | "search_processes"
        | "execute_action"
        | "launch_app"
        | "open_document_reference"
        | "hide_launcher_window"
        | "dismiss_clipboard_history_panel"
        | "resize_launcher_window"
        | "insert_clipboard_history_text_into_launcher"
        | "begin_transient_window_interaction"
        | "end_transient_window_interaction"
        | "arm_launcher_blur_auto_hide_suppression"
        | "set_launcher_blur_auto_hide_enabled"
        | "get_launcher_pinned"
        | "set_launcher_pinned"
        | "get_shortcut"
        | "get_shortcut_runtime_status"
        | "set_shortcut"
        | "get_screenshot_review_preview"
        | "confirm_screenshot_review"
        | "cancel_screenshot_review"
        | "retry_screenshot_review"
        | "complete_screen_capture_region"
        | "cancel_screen_capture_region" => launcher::handle_invoke(invoke),
        "get_clipboard_history"
        | "toggle_clipboard_history_entry_pin"
        | "delete_clipboard_history_entry"
        | "paste_clipboard_history_entry" => clipboard::handle_invoke(invoke),
        "get_app_settings"
        | "set_app_settings"
        | "list_llm_provider_models"
        | "list_builtin_llm_provider_templates" => settings::handle_invoke(invoke),
        "scan_rag_sources" | "get_rag_runtime_status" => rag::handle_invoke(invoke),
        "get_workspace" | "set_workspace" => workspace::handle_invoke(invoke),
        "get_acp_agents"
        | "set_acp_agents"
        | "get_acp_mcp_servers"
        | "set_acp_mcp_servers"
        | "get_builtin_mcp_server_status"
        | "list_acp_sessions"
        | "take_acp_restore_notices"
        | "get_acp_session_detail"
        | "create_acp_session"
        | "activate_acp_session"
        | "send_acp_prompt"
        | "set_acp_session_mode"
        | "set_acp_session_config_option"
        | "cancel_acp_session"
        | "close_acp_session"
        | "subscribe_acp_session_updates"
        | "subscribe_acp_session_removals"
        | "unsubscribe_acp_session_updates"
        | "unsubscribe_acp_session_removals" => acp::handle_invoke(invoke),
        _ => false,
    }
}

pub(crate) fn parse_arg<'de, T, R>(
    invoke: &'de Invoke<R>,
    command: &'static str,
    key: &'static str,
) -> Result<T, InvokeError>
where
    T: CommandArg<'de, R>,
    R: Runtime,
{
    CommandArg::from_command(CommandItem {
        plugin: None,
        name: command,
        key,
        message: &invoke.message,
        acl: &invoke.acl,
    })
}

pub(crate) fn parse_or_invoke_error<'de, T, R>(
    invoke: &'de Invoke<R>,
    command: &'static str,
    key: &'static str,
) -> Option<T>
where
    T: CommandArg<'de, R>,
    R: Runtime,
{
    match parse_arg(invoke, command, key) {
        Ok(value) => Some(value),
        Err(error) => {
            invoke.resolver.clone().invoke_error(error);
            None
        }
    }
}

pub(crate) fn respond_sync<T>(resolver: InvokeResolver<Wry>, result: Result<T, InvokeError>) -> bool
where
    T: IpcResponse,
{
    resolver.respond(result);
    true
}

pub(crate) fn respond_async<T, F>(resolver: InvokeResolver<Wry>, task: F) -> bool
where
    T: IpcResponse,
    F: Future<Output = Result<T, InvokeError>> + Send + 'static,
{
    resolver.respond_async(task);
    true
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    #[tauri::command(rename_all = "camelCase")]
    fn handle_optional_window_label_test(window_label: Option<String>) -> Value {
        json!({ "windowLabel": window_label })
    }

    #[test]
    fn optional_ipc_arg_missing_yields_none() {
        assert_eq!(
            handle_optional_window_label_test(None),
            json!({ "windowLabel": null })
        );
    }

    #[test]
    fn optional_ipc_arg_invalid_type_returns_error() {
        let error = serde_json::from_value::<Option<String>>(json!(123))
            .expect_err("invalid optional arg type should be rejected");
        let message = error.to_string();

        assert!(message.contains("invalid type"));
    }
}
