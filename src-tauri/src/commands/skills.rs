use tauri::{
    ipc::{Invoke, InvokeError},
    State, Wry,
};

use crate::{domain::skills::PublicSkillCatalog, state::AppState};

pub async fn get_public_skill_catalog(
    state: State<'_, AppState>,
) -> Result<PublicSkillCatalog, String> {
    state
        .public_skill_catalog()
        .await
        .map_err(|error| error.to_string())
}

pub(crate) fn handle_invoke(invoke: Invoke<Wry>) -> bool {
    match invoke.message.command() {
        "get_public_skill_catalog" => {
            let resolver = invoke.resolver.clone();
            resolver.respond_async(async move {
                let state = super::parse_arg(&invoke, "get_public_skill_catalog", "state")?;
                get_public_skill_catalog(state)
                    .await
                    .map_err(InvokeError::from)
            });
            true
        }
        _ => false,
    }
}
