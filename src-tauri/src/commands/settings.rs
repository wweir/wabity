use tauri::{
    ipc::{Invoke, InvokeError},
    AppHandle, State, Wry,
};
use tracing::error;

use crate::{
    domain::settings::{AppSettings, LlmProviderConfig, LlmProviderModelEntry},
    infrastructure::autostart,
    state::AppState,
};

pub async fn get_app_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state
        .app_settings()
        .await
        .map_err(|error| error.to_string())
}

pub async fn set_app_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let previous_settings = state
        .app_settings()
        .await
        .map_err(|error| error.to_string())?;
    let saved_settings = state
        .update_app_settings(settings)
        .await
        .map_err(|error| error.to_string())?;

    if previous_settings.general.auto_start != saved_settings.general.auto_start {
        if let Err(sync_error) = autostart::sync_autostart(&app, saved_settings.general.auto_start)
        {
            error!(
                ?sync_error,
                enabled = saved_settings.general.auto_start,
                "failed to sync autostart state after settings update"
            );

            let rollback_result = state.update_app_settings(previous_settings.clone()).await;
            return match rollback_result {
                Ok(_) => Err(format!("开机自启动同步失败：{sync_error}")),
                Err(rollback_error) => Err(format!(
                    "开机自启动同步失败：{sync_error}；回滚配置也失败：{rollback_error}"
                )),
            };
        }
    }

    Ok(saved_settings)
}

pub async fn list_llm_provider_models(
    state: State<'_, AppState>,
    provider: LlmProviderConfig,
) -> Result<Vec<LlmProviderModelEntry>, String> {
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
                let app = super::parse_arg(&invoke, "set_app_settings", "app")?;
                let state = super::parse_arg(&invoke, "set_app_settings", "state")?;
                let settings = super::parse_arg(&invoke, "set_app_settings", "settings")?;
                set_app_settings(app, state, settings)
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
