use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::query::QueryPayload;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRequest {
    pub action_id: String,
    pub query: QueryPayload,
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
