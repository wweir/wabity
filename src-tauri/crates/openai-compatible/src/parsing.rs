use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::{
    client::OpenAiCompatibleResponseFormat, extract::join_non_empty_segments_with_delimiter,
    streaming::StreamingPayloadCollector,
};

const BODY_PREVIEW_LIMIT: usize = 240;

fn endpoint_display_name(endpoint: &str) -> &str {
    endpoint.trim_start_matches('/')
}

pub(crate) fn parse_response_payload(
    status: reqwest::StatusCode,
    body: String,
    endpoint: &str,
    operation: &str,
    response_format: OpenAiCompatibleResponseFormat,
) -> Result<Value> {
    if !status.is_success() {
        let message = extract_provider_error_message(&body);
        bail!(
            "LLM provider {} 请求失败 ({status}): {message}",
            endpoint_display_name(endpoint)
        );
    }

    match response_format {
        OpenAiCompatibleResponseFormat::Json => {
            parse_json_payload(&body, &format!("{operation} response"))
        }
        OpenAiCompatibleResponseFormat::JsonOrSse => {
            parse_json_or_sse_payload(&body, &format!("{operation} response"))
        }
    }
}

pub fn parse_json_payload(body: &str, operation: &str) -> Result<Value> {
    serde_json::from_str(body)
        .with_context(|| format!("failed to parse {operation}: {}", body_preview(body)))
}

pub fn parse_json_or_sse_payload(body: &str, operation: &str) -> Result<Value> {
    match serde_json::from_str(body) {
        Ok(payload) => Ok(payload),
        Err(json_error) => parse_sse_payload(body).with_context(|| {
            format!(
                "failed to parse {operation}: {json_error}; body preview: {}",
                body_preview(body)
            )
        }),
    }
}

fn parse_sse_payload(body: &str) -> Result<Value> {
    let mut collector = StreamingPayloadCollector::default();
    collector.push_bytes(body.as_bytes())?;
    collector.finish("SSE response")
}

pub fn extract_provider_error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|payload| extract_error_value(&payload))
        .unwrap_or_else(|| {
            body.lines()
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("unknown error")
                .to_string()
        })
}

pub(crate) fn extract_error_value(payload: &Value) -> Option<String> {
    match payload {
        Value::String(value) => {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_string())
        }
        Value::Array(items) => {
            let segments = items
                .iter()
                .filter_map(extract_error_value)
                .collect::<Vec<_>>();
            join_non_empty_segments_with_delimiter(&segments, "; ")
        }
        Value::Object(map) => {
            for key in ["error", "message", "detail", "details", "title"] {
                if let Some(value) = map.get(key).and_then(extract_error_value) {
                    return Some(value);
                }
            }

            None
        }
        _ => None,
    }
}

pub fn body_preview(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.len() <= BODY_PREVIEW_LIMIT {
        return trimmed.to_string();
    }

    let mut preview = trimmed
        .chars()
        .take(BODY_PREVIEW_LIMIT)
        .collect::<String>()
        .trim()
        .to_string();
    preview.push_str("...");
    preview
}
