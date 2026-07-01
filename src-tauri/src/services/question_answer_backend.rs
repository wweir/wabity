use anyhow::Result;

use crate::{
    domain::{
        execution::{
            ExecutionConversationState, ExecutionConversationTurn, ExecutionProgressEvent,
            ExecutionResult,
        },
        settings::{LlmSettings, PromptsSettings, RagSettings},
    },
    services::rag_answer,
};

pub struct QuestionAnswerBackendRequest<'a> {
    pub data_dir: &'a std::path::Path,
    pub workspace_root: &'a std::path::Path,
    pub raw_text: &'a str,
    pub conversation: &'a [ExecutionConversationTurn],
    pub conversation_state: Option<&'a ExecutionConversationState>,
    pub prompts_settings: &'a PromptsSettings,
    pub rag_settings: &'a RagSettings,
    pub llm_settings: &'a LlmSettings,
    pub progress_event_tx: Option<std::sync::Arc<dyn Fn(ExecutionProgressEvent) + Send + Sync>>,
}

pub async fn answer_question(request: QuestionAnswerBackendRequest<'_>) -> Result<ExecutionResult> {
    rag_answer::answer_question(rag_answer::QuestionAnswerRequest {
        data_dir: request.data_dir,
        workspace_root: request.workspace_root,
        raw_text: request.raw_text,
        conversation: request.conversation,
        conversation_state: request.conversation_state,
        prompts_settings: request.prompts_settings,
        rag_settings: request.rag_settings,
        llm_settings: request.llm_settings,
        progress_event_tx: request.progress_event_tx,
    })
    .await
}
