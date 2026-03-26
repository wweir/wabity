mod app;
mod commands;
mod domain;
mod infrastructure;
mod services;
mod state;

pub mod question_answer_backend {
    pub use crate::domain::{
        acp::AcpMcpServerConfig,
        execution::{
            ExecutionConversationRole, ExecutionConversationState, ExecutionConversationTurn,
            ExecutionProgressEvent, ExecutionResult, ExecutionStatus, ExecutionToolCall,
        },
        settings::{
            LlmModelType, LlmProviderConfig, LlmProviderProtocol, LlmSettings, PromptsSettings,
            RagSettings,
        },
    };
    pub use crate::services::question_answer_backend::{
        answer_question, QuestionAnswerBackendRequest,
    };
}

pub mod rag_backend {
    pub use crate::domain::{
        rag::RagScanResult,
        settings::{
            LlmModelType, LlmProviderConfig, LlmProviderProtocol, LlmSettings, RagSettings,
        },
    };
    pub use crate::services::rag::RagIndexService;
    pub use crate::services::rag_query::{search_chunks, RagSearchHit, RagSearchResult};
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(error) = app::run() {
        panic!("failed to start wabity: {error:#}");
    }
}
