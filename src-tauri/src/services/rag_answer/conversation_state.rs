use std::path::Path;

use crate::domain::{
    acp::AcpActionEvent,
    execution::{
        ExecutionCitation, ExecutionConversationState, ExecutionConversationTurn, ExecutionToolCall,
    },
    settings::{LlmProviderConfig, LlmProviderProtocol},
};

pub(super) struct PreparedConversationState<'a> {
    pub(super) conversation: &'a [ExecutionConversationTurn],
    pub(super) state: ExecutionConversationState,
}

pub(super) fn normalize_previous_response_id(previous_response_id: Option<&str>) -> Option<String> {
    previous_response_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_continuation_scope(continuation_scope: Option<&str>) -> Option<String> {
    continuation_scope
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn provider_continuation_scope(
    provider: &LlmProviderConfig,
    workspace_root: &Path,
) -> String {
    let workspace = workspace_root.to_string_lossy();
    let scope_seed = format!(
        "rag-answer|{}|{}|{}|{}",
        match provider.protocol {
            LlmProviderProtocol::Responses => "responses",
            LlmProviderProtocol::ChatCompletions => "chat_completions",
        },
        provider.base_url.trim().trim_end_matches('/'),
        provider.model_name(),
        workspace
    );
    format!("{:x}", md5::compute(scope_seed))
}

pub(super) fn prepare_conversation_state<'a>(
    conversation: &'a [ExecutionConversationTurn],
    conversation_state: Option<&ExecutionConversationState>,
    provider: &LlmProviderConfig,
    workspace_root: &Path,
) -> PreparedConversationState<'a> {
    let expected_scope = provider_continuation_scope(provider, workspace_root);
    let scope_matches = conversation_state
        .and_then(|state| normalize_continuation_scope(state.continuation_scope.as_deref()))
        .is_some_and(|scope| scope == expected_scope);
    let state_has_data = conversation_state
        .map(ExecutionConversationState::has_state)
        .unwrap_or(false);
    let should_reset_context = state_has_data && !scope_matches;

    let mut next_state = if scope_matches {
        conversation_state.cloned().unwrap_or_default()
    } else {
        ExecutionConversationState::default()
    };
    next_state.previous_response_id =
        normalize_previous_response_id(next_state.previous_response_id.as_deref());
    next_state.continuation_scope = Some(expected_scope);

    PreparedConversationState {
        conversation: if should_reset_context {
            &[]
        } else {
            conversation
        },
        state: next_state,
    }
}

pub(super) fn build_answer_conversation_state(
    response_id: Option<String>,
    provider: &LlmProviderConfig,
    workspace_root: &Path,
    citations: &[ExecutionCitation],
    actions: &[AcpActionEvent],
    tool_calls: &[ExecutionToolCall],
) -> ExecutionConversationState {
    ExecutionConversationState {
        previous_response_id: response_id,
        continuation_scope: Some(provider_continuation_scope(provider, workspace_root)),
        citations: citations.to_vec(),
        actions: actions.to_vec(),
        tool_calls: tool_calls.to_vec(),
    }
}

pub(super) fn previous_response_id_for_follow_up(
    conversation_state: Option<&ExecutionConversationState>,
    supports_stateful: bool,
) -> Option<String> {
    if !supports_stateful {
        return None;
    }

    conversation_state
        .and_then(|state| normalize_previous_response_id(state.previous_response_id.as_deref()))
}
