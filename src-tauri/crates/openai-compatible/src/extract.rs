use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatCompletionsMessageParts {
    pub content: Option<String>,
    pub reasoning: Option<String>,
}

#[derive(Debug, Default)]
struct ChatContentAccumulator {
    content: Vec<String>,
    reasoning: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatContentKind {
    Auto,
    Content,
    Reasoning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextExtractionMode {
    PreserveWhitespace,
    TrimBoundary,
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

fn is_reasoning_content_type(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "analysis"
            | "reasoning"
            | "reasoning_content"
            | "reasoning_text"
            | "thinking"
            | "thinking_text"
            | "thought"
    )
}

fn is_text_content_type(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "content" | "input_text" | "output_text" | "text"
    )
}

fn trim_to_owned(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn preserve_non_empty_to_owned(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_string())
}

fn strip_reasoning_tag_markers(value: &str) -> Option<String> {
    let markers = [
        "<think>",
        "</think>",
        "<think/>",
        "<think />",
        "<thinking>",
        "</thinking>",
        "<thinking/>",
        "<thinking />",
        "<reasoning>",
        "</reasoning>",
        "<reasoning/>",
        "<reasoning />",
    ];
    let mut normalized = value.to_string();
    for marker in markers {
        normalized = normalized.replace(marker, " ");
        normalized = normalized.replace(&marker.to_ascii_uppercase(), " ");
    }
    trim_to_owned(&normalized)
}

fn split_tagged_reasoning_text(
    value: &str,
    mode: TextExtractionMode,
) -> Option<ChatCompletionsMessageParts> {
    let lower = value.to_ascii_lowercase();
    let tag_pairs = [
        ("<think>", "</think>"),
        ("<thinking>", "</thinking>"),
        ("<reasoning>", "</reasoning>"),
    ];
    let mut cursor = 0usize;
    let mut content = Vec::new();
    let mut reasoning = Vec::new();
    let mut saw_reasoning_tag = false;

    loop {
        let next_tag = tag_pairs
            .iter()
            .filter_map(|(open, close)| {
                lower[cursor..]
                    .find(open)
                    .map(|offset| (offset, *open, *close))
            })
            .min_by_key(|(offset, _, _)| *offset);
        let Some((offset, open, close)) = next_tag else {
            if let Some(segment) = string_value_to_owned(&value[cursor..], mode) {
                content.push(segment);
            }
            break;
        };

        let open_start = cursor + offset;
        if let Some(segment) = string_value_to_owned(&value[cursor..open_start], mode) {
            content.push(segment);
        }

        let reasoning_start = open_start + open.len();
        let close_offset = lower[reasoning_start..].find(close)?;
        let reasoning_end = reasoning_start + close_offset;
        if let Some(segment) = string_value_to_owned(&value[reasoning_start..reasoning_end], mode) {
            reasoning.push(segment);
            saw_reasoning_tag = true;
        }
        cursor = reasoning_end + close.len();
    }

    saw_reasoning_tag.then(|| ChatCompletionsMessageParts {
        content: join_non_empty_segments(
            &content
                .into_iter()
                .filter_map(|segment| strip_reasoning_tag_markers(&segment))
                .collect::<Vec<_>>(),
        ),
        reasoning: join_non_empty_segments(&reasoning),
    })
}

impl ChatContentAccumulator {
    fn push(&mut self, kind: ChatContentKind, value: &str, mode: TextExtractionMode) {
        let Some(segment) = string_value_to_owned(value, mode) else {
            return;
        };

        match kind {
            ChatContentKind::Reasoning => self.reasoning.push(segment),
            ChatContentKind::Auto | ChatContentKind::Content => {
                if kind == ChatContentKind::Auto {
                    if let Some(parts) = split_tagged_reasoning_text(&segment, mode) {
                        if let Some(content) = parts.content {
                            self.content.push(content);
                        }
                        if let Some(reasoning) = parts.reasoning {
                            self.reasoning.push(reasoning);
                        }
                        return;
                    }
                }
                self.content.push(segment);
            }
        }
    }

    fn extend_value(&mut self, payload: &Value, kind: ChatContentKind, mode: TextExtractionMode) {
        match payload {
            Value::Null => {}
            Value::String(text) => self.push(kind, text, mode),
            Value::Array(items) => {
                for item in items {
                    self.extend_value(item, kind, mode);
                }
            }
            Value::Object(object) => {
                let inferred_kind = object
                    .get("type")
                    .and_then(Value::as_str)
                    .map(|value| {
                        if is_reasoning_content_type(value) {
                            ChatContentKind::Reasoning
                        } else if is_text_content_type(value) {
                            ChatContentKind::Content
                        } else {
                            kind
                        }
                    })
                    .unwrap_or(kind);

                for key in [
                    "reasoning_content",
                    "reasoning",
                    "thinking",
                    "analysis",
                    "thought",
                ] {
                    if let Some(value) = object.get(key) {
                        self.extend_value(value, ChatContentKind::Reasoning, mode);
                    }
                }

                for key in ["value", "text", "output_text", "input_text"] {
                    if let Some(value) = object.get(key) {
                        self.extend_value(value, inferred_kind, mode);
                    }
                }

                for key in ["content", "parts", "items"] {
                    if let Some(value) = object.get(key) {
                        self.extend_value(value, inferred_kind, mode);
                    }
                }
            }
            _ => {}
        }
    }

    fn into_message_parts(self, mode: TextExtractionMode) -> Option<ChatCompletionsMessageParts> {
        let content = join_non_empty_segments_for_mode(&self.content, mode);
        let reasoning = join_non_empty_segments_for_mode(&self.reasoning, mode);
        (content.is_some() || reasoning.is_some())
            .then_some(ChatCompletionsMessageParts { content, reasoning })
    }
}

pub(crate) fn extract_chat_content_parts(payload: &Value) -> Option<ChatCompletionsMessageParts> {
    extract_chat_content_parts_with_mode(payload, TextExtractionMode::TrimBoundary)
}

pub(crate) fn extract_chat_content_parts_preserving_whitespace(
    payload: &Value,
) -> Option<ChatCompletionsMessageParts> {
    extract_chat_content_parts_with_mode(payload, TextExtractionMode::PreserveWhitespace)
}

fn extract_chat_content_parts_with_mode(
    payload: &Value,
    mode: TextExtractionMode,
) -> Option<ChatCompletionsMessageParts> {
    let mut accumulator = ChatContentAccumulator::default();
    accumulator.extend_value(payload, ChatContentKind::Auto, mode);
    accumulator.into_message_parts(mode)
}

pub fn extract_chat_completions_message_parts(
    payload: &Value,
) -> Option<ChatCompletionsMessageParts> {
    let message = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))?;

    let content_parts = message.get("content").and_then(extract_chat_content_parts);
    let content = content_parts
        .as_ref()
        .and_then(|parts| parts.content.clone())
        .or_else(|| extract_message_refusal(message));
    let reasoning = message
        .get("reasoning_content")
        .and_then(extract_text_content)
        .or_else(|| message.get("reasoning").and_then(extract_text_content))
        .or_else(|| content_parts.and_then(|parts| parts.reasoning));

    if content.is_none() && reasoning.is_none() {
        return extract_text_content(message).map(|fallback| ChatCompletionsMessageParts {
            content: Some(fallback),
            reasoning: None,
        });
    }

    Some(ChatCompletionsMessageParts { content, reasoning })
}

pub fn extract_chat_completions_text(payload: &Value) -> Option<String> {
    let parts = extract_chat_completions_message_parts(payload)?;
    parts.content.or(parts.reasoning)
}

pub fn describe_chat_completions_response_issue(payload: &Value) -> String {
    let Some(choice) = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
    else {
        return "choices 缺失或为空".to_string();
    };

    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("missing");
    let Some(message) = choice.get("message") else {
        return format!("finish_reason={finish_reason}; message 缺失");
    };

    let tool_call_count = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    let refusal_present = extract_message_refusal(message).is_some();
    let content_shape = describe_json_shape(message.get("content"));
    let reasoning_shape = describe_json_shape(message.get("reasoning_content"));
    let message_keys = message
        .as_object()
        .map(describe_object_keys)
        .unwrap_or_else(|| describe_json_shape(Some(message)));

    format!(
        "finish_reason={finish_reason}; tool_calls={tool_call_count}; refusal={refusal_present}; content={content_shape}; reasoning_content={reasoning_shape}; message_keys={message_keys}"
    )
}

pub fn extract_text_content(payload: &Value) -> Option<String> {
    extract_text_content_with_mode(payload, TextExtractionMode::TrimBoundary)
}

pub(crate) fn extract_text_content_preserving_whitespace(payload: &Value) -> Option<String> {
    extract_text_content_with_mode(payload, TextExtractionMode::PreserveWhitespace)
}

fn extract_text_content_with_mode(payload: &Value, mode: TextExtractionMode) -> Option<String> {
    if let Some(text) = payload
        .as_str()
        .and_then(|value| string_value_to_owned(value, mode))
    {
        return Some(text);
    }

    for key in ["value", "text", "output_text", "input_text", "refusal"] {
        if let Some(text) = payload
            .get(key)
            .and_then(Value::as_str)
            .and_then(|value| string_value_to_owned(value, mode))
        {
            return Some(text);
        }

        if let Some(text) = payload
            .get(key)
            .and_then(|value| value.get("value"))
            .and_then(Value::as_str)
            .and_then(|value| string_value_to_owned(value, mode))
        {
            return Some(text);
        }
    }

    for key in ["content", "parts", "items"] {
        if let Some(value) = payload.get(key).filter(|value| !value.is_null()) {
            if let Some(text) = extract_text_content_with_mode(value, mode) {
                return Some(text);
            }
        }
    }

    if let Some(items) = payload.as_array() {
        let segments = items
            .iter()
            .filter_map(|item| extract_text_content_with_mode(item, mode))
            .collect::<Vec<_>>();
        return join_non_empty_segments_for_mode(&segments, mode);
    }

    None
}

fn string_value_to_owned(value: &str, mode: TextExtractionMode) -> Option<String> {
    match mode {
        TextExtractionMode::PreserveWhitespace => preserve_non_empty_to_owned(value),
        TextExtractionMode::TrimBoundary => trim_to_owned(value),
    }
}

fn extract_message_refusal(message: &Value) -> Option<String> {
    message
        .get("refusal")
        .and_then(extract_text_content)
        .or_else(|| {
            message
                .get("content")
                .and_then(extract_refusal_from_content)
        })
}

fn extract_refusal_from_content(content: &Value) -> Option<String> {
    match content {
        Value::Array(items) => {
            let segments = items
                .iter()
                .filter_map(extract_refusal_from_content)
                .collect::<Vec<_>>();
            join_non_empty_segments(&segments)
        }
        Value::Object(_) => content
            .get("refusal")
            .and_then(extract_text_content)
            .or_else(|| {
                let is_refusal = content
                    .get("type")
                    .and_then(Value::as_str)
                    .map(|value| value.eq_ignore_ascii_case("refusal"))
                    .unwrap_or(false);
                is_refusal.then(|| extract_text_content(content)).flatten()
            })
            .or_else(|| {
                content
                    .get("content")
                    .and_then(extract_refusal_from_content)
            }),
        _ => None,
    }
}

fn describe_json_shape(value: Option<&Value>) -> String {
    match value {
        None => "missing".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            format!(
                "string({})",
                if trimmed.is_empty() {
                    "empty"
                } else {
                    "non-empty"
                }
            )
        }
        Some(Value::Array(items)) => {
            let part_types = items
                .iter()
                .filter_map(|item| {
                    item.get("type")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                        .or_else(|| item.as_object().map(describe_object_keys))
                })
                .take(4)
                .collect::<Vec<_>>();
            if part_types.is_empty() {
                format!("array(len={})", items.len())
            } else {
                format!("array(len={}, parts={})", items.len(), part_types.join("|"))
            }
        }
        Some(Value::Object(map)) => format!("object(keys={})", describe_object_keys(map)),
        Some(Value::Bool(_)) => "bool".to_string(),
        Some(Value::Number(_)) => "number".to_string(),
    }
}

fn describe_object_keys(map: &Map<String, Value>) -> String {
    let mut keys = map.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    if keys.is_empty() {
        return "[]".to_string();
    }

    let preview = keys.into_iter().take(6).collect::<Vec<_>>();
    format!("[{}]", preview.join(","))
}

pub(crate) fn join_non_empty_segments(segments: &[String]) -> Option<String> {
    join_non_empty_segments_with_delimiter(segments, "\n")
}

fn join_non_empty_segments_for_mode(
    segments: &[String],
    mode: TextExtractionMode,
) -> Option<String> {
    match mode {
        TextExtractionMode::PreserveWhitespace => {
            join_non_empty_segments_preserving_whitespace(segments)
        }
        TextExtractionMode::TrimBoundary => join_non_empty_segments(segments),
    }
}

fn join_non_empty_segments_preserving_whitespace(segments: &[String]) -> Option<String> {
    let mut joined = String::new();
    for segment in segments {
        if !segment.trim().is_empty() {
            joined.push_str(segment);
        }
    }

    (!joined.trim().is_empty()).then_some(joined)
}

pub(crate) fn join_non_empty_segments_with_delimiter(
    segments: &[String],
    delimiter: &str,
) -> Option<String> {
    let joined = segments
        .iter()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(delimiter);

    (!joined.is_empty()).then_some(joined)
}
