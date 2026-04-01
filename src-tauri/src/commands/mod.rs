use tauri::{
    ipc::{CommandArg, CommandItem, Invoke, InvokeError},
    Wry,
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
        | "execute_action"
        | "launch_app"
        | "open_document_reference"
        | "hide_launcher_window"
        | "resize_launcher_window"
        | "begin_transient_window_interaction"
        | "end_transient_window_interaction"
        | "arm_launcher_blur_auto_hide_suppression"
        | "set_launcher_blur_auto_hide_enabled"
        | "get_shortcut"
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
        | "cancel_acp_session"
        | "close_acp_session"
        | "subscribe_acp_session_updates"
        | "subscribe_acp_session_removals" => acp::handle_invoke(invoke),
        _ => false,
    }
}

pub(crate) fn parse_arg<'de, T>(
    invoke: &'de Invoke<Wry>,
    command: &'static str,
    key: &'static str,
) -> Result<T, InvokeError>
where
    T: CommandArg<'de, Wry>,
{
    CommandArg::from_command(CommandItem {
        plugin: None,
        name: command,
        key,
        message: &invoke.message,
        acl: &invoke.acl,
    })
}

pub(crate) fn parse_or_invoke_error<'de, T>(
    invoke: &'de Invoke<Wry>,
    command: &'static str,
    key: &'static str,
) -> Option<T>
where
    T: CommandArg<'de, Wry>,
{
    match parse_arg(invoke, command, key) {
        Ok(value) => Some(value),
        Err(error) => {
            invoke.resolver.clone().invoke_error(error);
            None
        }
    }
}
