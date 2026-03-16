use tauri::{
    ipc::{Invoke, InvokeError},
    State, Wry,
};

use crate::{
    domain::{
        rag::RagScanResult,
        settings::{LlmSettings, RagSettings},
    },
    state::AppState,
};

pub async fn scan_rag_sources(
    state: State<'_, AppState>,
    rag_settings: RagSettings,
    llm_settings: LlmSettings,
) -> Result<RagScanResult, String> {
    state
        .scan_rag_sources(rag_settings, llm_settings)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) fn handle_invoke(invoke: Invoke<Wry>) -> bool {
    match invoke.message.command() {
        "scan_rag_sources" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "scan_rag_sources", "state")?;
                let rag_settings = super::parse_arg(&invoke, "scan_rag_sources", "ragSettings")?;
                let llm_settings = super::parse_arg(&invoke, "scan_rag_sources", "llmSettings")?;

                scan_rag_sources(state, rag_settings, llm_settings)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        _ => false,
    }
}
