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
pub mod skills;
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
        | "get_shortcut"
        | "get_shortcut_runtime_status"
        | "set_shortcut" => launcher::handle_invoke(invoke),
        "get_clipboard_history"
        | "toggle_clipboard_history_entry_pin"
        | "delete_clipboard_history_entry"
        | "paste_clipboard_history_entry" => clipboard::handle_invoke(invoke),
        "get_app_settings"
        | "set_app_settings"
        | "list_llm_provider_models"
        | "list_builtin_llm_provider_templates" => settings::handle_invoke(invoke),
        "scan_rag_sources" | "get_rag_runtime_status" => rag::handle_invoke(invoke),
        "get_public_skill_catalog" => skills::handle_invoke(invoke),
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
    use tauri::{
        ipc::{Invoke, InvokeBody, InvokeError},
        test::{
            get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY,
        },
        webview::InvokeRequest,
        Runtime,
    };

    fn handle_optional_window_label_test<R: Runtime>(invoke: Invoke<R>) -> bool {
        if invoke.message.command() != "handle_optional_window_label_test" {
            return false;
        }

        let resolver = invoke.resolver.clone();
        let Some(window_label) = super::parse_or_invoke_error::<Option<String>, R>(
            &invoke,
            "handle_optional_window_label_test",
            "windowLabel",
        ) else {
            return true;
        };

        resolver.respond(Ok::<Value, InvokeError>(json!({
            "windowLabel": window_label,
        })));
        true
    }

    fn build_test_webview() -> (tauri::App<MockRuntime>, tauri::WebviewWindow<MockRuntime>) {
        let app = mock_builder()
            .invoke_handler(handle_optional_window_label_test::<MockRuntime>)
            .build(mock_context(noop_assets()))
            .expect("test app should build");
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("test webview should build");

        (app, webview)
    }

    fn invoke_request(body: Value) -> InvokeRequest {
        InvokeRequest {
            cmd: "handle_optional_window_label_test".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().expect("url should parse"),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        }
    }

    #[test]
    fn optional_ipc_arg_missing_yields_none() {
        let (_app, webview) = build_test_webview();

        let response = get_ipc_response(&webview, invoke_request(json!({})))
            .expect("missing optional arg should succeed")
            .deserialize::<Value>()
            .expect("response should deserialize");

        assert_eq!(response, json!({ "windowLabel": null }));
    }

    #[test]
    fn optional_ipc_arg_invalid_type_returns_error() {
        let (_app, webview) = build_test_webview();

        let error = get_ipc_response(&webview, invoke_request(json!({ "windowLabel": 123 })))
            .expect_err("invalid optional arg type should be rejected");
        let message = error
            .as_str()
            .expect("invoke error should serialize as a string");

        assert!(message.contains("windowLabel"));
        assert!(message.contains("invalid type"));
    }
}
