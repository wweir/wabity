use anyhow::Result;
use serde_json::Value;

use crate::domain::settings::LlmProviderModelEntry;

pub type OpenAiCompatibleClient<'a, HttpClient> =
    wabity_openai_compatible::OpenAiCompatibleClient<'a, HttpClient>;
pub type OpenAiCompatibleResponseFormat = wabity_openai_compatible::OpenAiCompatibleResponseFormat;
pub type ChatCompletionsMessageParts = wabity_openai_compatible::ChatCompletionsMessageParts;

pub fn normalize_base_url(base_url: &str, label: &str) -> Result<String> {
    wabity_openai_compatible::normalize_base_url(base_url, label)
}

pub fn extract_responses_text(payload: &Value) -> Option<String> {
    wabity_openai_compatible::extract_responses_text(payload)
}

pub fn extract_chat_completions_text(payload: &Value) -> Option<String> {
    wabity_openai_compatible::extract_chat_completions_text(payload)
}

pub fn extract_chat_completions_message_parts(
    payload: &Value,
) -> Option<ChatCompletionsMessageParts> {
    wabity_openai_compatible::extract_chat_completions_message_parts(payload)
}

pub fn describe_chat_completions_response_issue(payload: &Value) -> String {
    wabity_openai_compatible::describe_chat_completions_response_issue(payload)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn extract_provider_error_message(body: &str) -> String {
    wabity_openai_compatible::extract_provider_error_message(body)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn body_preview(body: &str) -> String {
    wabity_openai_compatible::body_preview(body)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn parse_json_or_sse_payload(body: &str, operation: &str) -> Result<Value> {
    wabity_openai_compatible::parse_json_or_sse_payload(body, operation)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn extract_model_ids(payload: &Value) -> Vec<String> {
    wabity_openai_compatible::extract_model_ids(payload)
}

pub fn extract_model_entries(payload: &Value) -> Vec<LlmProviderModelEntry> {
    wabity_openai_compatible::extract_model_entries(payload)
        .into_iter()
        .map(|entry| LlmProviderModelEntry {
            id: entry.id,
            identity_hint: entry.identity_hint,
        })
        .collect()
}
