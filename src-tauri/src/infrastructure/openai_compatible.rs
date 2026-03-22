use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::domain::settings::LlmProviderModelEntry;

const BODY_PREVIEW_LIMIT: usize = 240;

pub fn normalize_base_url(base_url: &str, label: &str) -> Result<String> {
    let normalized = base_url.trim().trim_end_matches('/');
    if normalized.is_empty() {
        bail!("{label} 不能为空");
    }

    reqwest::Url::parse(normalized).with_context(|| format!("invalid {label}: {normalized}"))?;
    Ok(normalized.to_string())
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

fn extract_error_value(payload: &Value) -> Option<String> {
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
    let mut data_lines = Vec::new();
    let mut last_payload = None;
    let mut saw_data_frame = false;

    for raw_line in body.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            parse_sse_event_frame(&mut data_lines, &mut saw_data_frame, &mut last_payload)?;
            continue;
        }

        if let Some(payload) = line.strip_prefix("data:") {
            data_lines.push(payload.trim_start().to_string());
        }
    }

    parse_sse_event_frame(&mut data_lines, &mut saw_data_frame, &mut last_payload)?;

    if !saw_data_frame {
        bail!("response body is neither JSON nor SSE data frames");
    }

    last_payload.context("SSE response did not include a completed payload")
}

fn parse_sse_event_frame(
    data_lines: &mut Vec<String>,
    saw_data_frame: &mut bool,
    last_payload: &mut Option<Value>,
) -> Result<()> {
    if data_lines.is_empty() {
        return Ok(());
    }

    *saw_data_frame = true;
    let frame = data_lines.join("\n");
    data_lines.clear();

    let trimmed = frame.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Ok(());
    }

    let payload: Value = serde_json::from_str(trimmed).with_context(|| {
        format!(
            "SSE data frame is not valid JSON: {}",
            body_preview(trimmed)
        )
    })?;
    if let Some(response) = payload.get("response").filter(|value| value.is_object()) {
        *last_payload = Some(response.clone());
        return Ok(());
    }

    let looks_like_completed_payload = payload.get("id").and_then(Value::as_str).is_some()
        && (payload.get("output").is_some()
            || payload.get("output_text").is_some()
            || payload.get("choices").is_some()
            || payload.get("data").is_some());
    if looks_like_completed_payload {
        *last_payload = Some(payload);
    }

    Ok(())
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

pub fn extract_responses_text(payload: &Value) -> Option<String> {
    if let Some(output_text) = payload
        .get("output_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(output_text.to_string());
    }

    let mut segments = Vec::new();
    for output in payload
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if output.get("content").and_then(Value::as_array).is_none() {
            continue;
        }

        for content in output
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(text) = extract_text_content(content) {
                segments.push(text);
            }
        }
    }

    join_non_empty_segments(&segments)
}

pub fn extract_chat_completions_text(payload: &Value) -> Option<String> {
    let message = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))?;
    extract_text_content(message.get("content")?)
}

pub fn extract_text_content(payload: &Value) -> Option<String> {
    if let Some(text) = payload
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_string());
    }

    if let Some(text) = payload
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_string());
    }

    if let Some(text) = payload
        .get("text")
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_string());
    }

    if let Some(text) = payload
        .get("refusal")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_string());
    }

    if let Some(items) = payload.as_array() {
        let segments = items
            .iter()
            .filter_map(extract_text_content)
            .collect::<Vec<_>>();
        return join_non_empty_segments(&segments);
    }

    None
}

#[cfg(test)]
pub fn extract_model_ids(payload: &Value) -> Vec<String> {
    extract_model_entries(payload)
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

pub fn extract_model_entries(payload: &Value) -> Vec<LlmProviderModelEntry> {
    let mut models = BTreeMap::new();

    if let Some(items) = payload.get("data").and_then(Value::as_array) {
        for item in items {
            if let Some(model_id) = item
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let identity_hint = extract_model_identity_hint(item);
                models
                    .entry(model_id.to_string())
                    .and_modify(|current_hint: &mut Option<String>| {
                        if current_hint.is_none() && identity_hint.is_some() {
                            *current_hint = identity_hint.clone();
                        }
                    })
                    .or_insert(identity_hint);
            }
        }
    }

    models
        .into_iter()
        .map(|(id, identity_hint)| LlmProviderModelEntry { id, identity_hint })
        .collect()
}

fn extract_model_identity_hint(item: &Value) -> Option<String> {
    const DIRECT_KEYS: &[&str] = &[
        "digest",
        "sha256",
        "model_digest",
        "modelDigest",
        "model_sha256",
        "modelSha256",
        "checksum",
        "fingerprint",
        "model_fingerprint",
        "modelFingerprint",
    ];
    const NESTED_KEYS: &[&str] = &["details", "metadata", "model_info", "modelInfo"];

    for key in DIRECT_KEYS {
        if let Some(hint) = item
            .get(key)
            .and_then(Value::as_str)
            .and_then(normalize_model_identity_hint)
        {
            return Some(hint);
        }
    }

    for key in NESTED_KEYS {
        let Some(value) = item.get(key) else {
            continue;
        };
        for nested_key in DIRECT_KEYS {
            if let Some(hint) = value
                .get(nested_key)
                .and_then(Value::as_str)
                .and_then(normalize_model_identity_hint)
            {
                return Some(hint);
            }
        }
    }

    None
}

fn normalize_model_identity_hint(raw: &str) -> Option<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return None;
    }

    if let Some((algorithm, digest)) = trimmed.split_once(':') {
        if !algorithm.is_empty()
            && algorithm
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
            && digest.len() >= 16
            && digest.chars().all(|ch| ch.is_ascii_hexdigit())
        {
            return Some(format!("digest:{algorithm}:{digest}"));
        }
    }

    if trimmed.len() >= 16 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Some(format!("digest:hex:{trimmed}"));
    }

    None
}

fn join_non_empty_segments(segments: &[String]) -> Option<String> {
    join_non_empty_segments_with_delimiter(segments, "\n")
}

fn join_non_empty_segments_with_delimiter(segments: &[String], delimiter: &str) -> Option<String> {
    let joined = segments
        .iter()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(delimiter);

    (!joined.is_empty()).then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::{
        body_preview, extract_chat_completions_text, extract_model_entries, extract_model_ids,
        extract_provider_error_message, extract_responses_text, normalize_base_url,
        parse_json_or_sse_payload,
    };
    use crate::domain::settings::LlmProviderModelEntry;
    use serde_json::json;

    #[test]
    fn normalize_base_url_trims_trailing_slash() {
        let normalized = normalize_base_url(
            " https://api.openai.example.com/v1/ ",
            "LLM provider base URL",
        )
        .expect("base url should normalize");

        assert_eq!(normalized, "https://api.openai.example.com/v1");
    }

    #[test]
    fn parse_json_or_sse_payload_accepts_plain_json() {
        let payload = parse_json_or_sse_payload(
            r#"{"id":"resp_123","output":[{"type":"message","content":[{"type":"output_text","text":"ok"}]}]}"#,
            "responses payload",
        )
        .expect("plain JSON response should parse");

        assert_eq!(payload["id"], "resp_123");
    }

    #[test]
    fn parse_json_or_sse_payload_accepts_sse_completed_event() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "event: response.created\n",
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_ignore\"}}\n\n",
                "event: response.completed\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"ok\"}]}]}}\n\n",
                "data: [DONE]\n",
            ),
            "responses payload",
        )
        .expect("SSE completed response should parse");

        assert_eq!(payload["id"], "resp_123");
    }

    #[test]
    fn body_preview_collapses_whitespace_and_truncates() {
        let preview = body_preview(&format!("  a\n b\t{}  ", "x".repeat(300)));

        assert!(preview.starts_with("a b"));
        assert!(preview.ends_with("..."));
        assert!(preview.len() <= 243);
    }

    #[test]
    fn extract_provider_error_message_prefers_nested_fields() {
        let message = extract_provider_error_message(
            r#"{"error":{"details":[{"message":"quota exceeded"}]}}"#,
        );

        assert_eq!(message, "quota exceeded");
    }

    #[test]
    fn extract_responses_text_reads_output_text_and_refusal_parts() {
        let payload = json!({
            "output": [
                {
                    "type": "message",
                    "content": [
                        { "type": "output_text", "text": "first" },
                        { "type": "refusal", "refusal": "second" }
                    ]
                }
            ]
        });

        assert_eq!(
            extract_responses_text(&payload).as_deref(),
            Some("first\nsecond")
        );
    }

    #[test]
    fn extract_chat_completions_text_reads_array_content() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": [
                            { "type": "text", "text": "hello" },
                            { "type": "text", "text": { "value": "world" } }
                        ]
                    }
                }
            ]
        });

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("hello\nworld")
        );
    }

    #[test]
    fn extract_model_ids_deduplicates_and_sorts_ids() {
        let payload = json!({
            "data": [
                { "id": "gpt-4.1" },
                { "id": "gpt-4.1-mini" },
                { "id": "gpt-4.1" }
            ]
        });

        assert_eq!(
            extract_model_ids(&payload),
            vec!["gpt-4.1".to_string(), "gpt-4.1-mini".to_string()]
        );
    }

    #[test]
    fn extract_model_entries_preserves_digest_identity_hint() {
        let payload = json!({
            "data": [
                {
                    "id": "embedding-a",
                    "digest": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                },
                {
                    "id": "embedding-b",
                    "details": {
                        "model_digest": "0123456789abcdef0123456789abcdef"
                    }
                }
            ]
        });

        assert_eq!(
            extract_model_entries(&payload),
            vec![
                LlmProviderModelEntry {
                    id: "embedding-a".to_string(),
                    identity_hint: Some(
                        "digest:sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
                    ),
                },
                LlmProviderModelEntry {
                    id: "embedding-b".to_string(),
                    identity_hint: Some(
                        "digest:hex:0123456789abcdef0123456789abcdef".to_string()
                    ),
                }
            ]
        );
    }
}
