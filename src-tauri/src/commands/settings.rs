use tauri::{
    ipc::{Invoke, InvokeError},
    State, Wry,
};

use crate::{
    domain::settings::{AppSettings, LlmProviderConfig},
    state::AppState,
};

pub async fn get_app_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state
        .app_settings()
        .await
        .map_err(|error| error.to_string())
}

pub async fn set_app_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    state
        .update_app_settings(settings)
        .await
        .map_err(|error| error.to_string())
}

pub async fn list_llm_provider_models(
    state: State<'_, AppState>,
    provider: LlmProviderConfig,
) -> Result<Vec<String>, String> {
    state
        .list_llm_provider_models(provider)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) fn handle_invoke(invoke: Invoke<Wry>) -> bool {
    match invoke.message.command() {
        "get_app_settings" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "get_app_settings", "state")?;
                get_app_settings(state).await.map_err(InvokeError::from)
            });
            true
        }
        "set_app_settings" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "set_app_settings", "state")?;
                let settings = super::parse_arg(&invoke, "set_app_settings", "settings")?;
                set_app_settings(state, settings)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        "list_llm_provider_models" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "list_llm_provider_models", "state")?;
                let provider = super::parse_arg(&invoke, "list_llm_provider_models", "provider")?;
                list_llm_provider_models(state, provider)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        _ => false,
    }
}
