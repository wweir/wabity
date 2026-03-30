use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::acp::AcpActionEvent;
use crate::domain::query::QueryPayload;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCitation {
    pub id: usize,
    pub absolute_path: String,
    pub path: String,
    pub document_kind: String,
    pub chunk_index: i32,
    pub line_start: Option<i32>,
    pub line_end: Option<i32>,
    pub paragraph_line_start: Option<i32>,
    pub page_start: Option<i32>,
    pub page_end: Option<i32>,
    pub heading_path: Vec<String>,
    pub anchor_label: Option<String>,
    pub score: f32,
    pub distance: f32,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionToolCall {
    pub name: String,
    pub source: String,
    pub status: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionConversationState {
    #[serde(default)]
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub continuation_scope: Option<String>,
    #[serde(default)]
    pub citations: Vec<ExecutionCitation>,
    #[serde(default)]
    pub actions: Vec<AcpActionEvent>,
    #[serde(default)]
    pub tool_calls: Vec<ExecutionToolCall>,
}

impl ExecutionConversationState {
    pub fn has_state(&self) -> bool {
        self.previous_response_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            || self
                .continuation_scope
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
            || !self.citations.is_empty()
            || !self.actions.is_empty()
            || !self.tool_calls.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionConversationRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionConversationTurn {
    pub role: ExecutionConversationRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRequest {
    pub action_id: String,
    pub query: QueryPayload,
    #[serde(default)]
    pub conversation: Vec<ExecutionConversationTurn>,
    #[serde(default)]
    pub conversation_state: Option<ExecutionConversationState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResult {
    pub status: ExecutionStatus,
    pub primary_text: Option<String>,
    pub secondary_text: Option<String>,
    pub structured_payload: Option<Value>,
    pub next_actions: Vec<String>,
    pub should_close_launcher: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionProgressEvent {
    pub action_id: String,
    pub status_text: String,
}

impl ExecutionResult {
    pub fn success(
        primary_text: Option<String>,
        secondary_text: Option<String>,
        structured_payload: Option<Value>,
        next_actions: Vec<&str>,
        should_close_launcher: bool,
    ) -> Self {
        Self {
            status: ExecutionStatus::Success,
            primary_text,
            secondary_text,
            structured_payload,
            next_actions: next_actions.into_iter().map(ToString::to_string).collect(),
            should_close_launcher,
        }
    }
}
