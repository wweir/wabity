use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::extract::{
    extract_chat_content_parts_preserving_whitespace, extract_text_content_preserving_whitespace,
};

#[derive(Debug, Default)]
pub(super) struct ChatCompletionsSseState {
    root: Map<String, Value>,
    choices: BTreeMap<usize, ChatChoiceState>,
    saw_chunk: bool,
}

impl ChatCompletionsSseState {
    pub(super) fn apply_chunk<F>(
        &mut self,
        payload: &Value,
        mut on_text_delta: Option<&mut F>,
    ) -> Result<()>
    where
        F: FnMut(&str),
    {
        self.saw_chunk = true;
        if let Some(object) = payload.as_object() {
            for key in [
                "id",
                "model",
                "created",
                "system_fingerprint",
                "service_tier",
            ] {
                if let Some(value) = object.get(key) {
                    self.root.insert(key.to_string(), value.clone());
                }
            }
        }

        if let Some(choices) = payload.get("choices").and_then(Value::as_array) {
            for choice in choices {
                let Some(index) = choice
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                else {
                    continue;
                };
                let state = self.choices.entry(index).or_default();
                if let Some(delta) = choice.get("delta") {
                    if let Some(role) = delta.get("role").and_then(Value::as_str) {
                        state.role = Some(role.to_string());
                    }

                    if let Some(content) = delta.get("content") {
                        if let Some(parts) =
                            extract_chat_content_parts_preserving_whitespace(content)
                        {
                            if let Some(text) = parts.content {
                                if let Some(callback) = on_text_delta.as_deref_mut() {
                                    callback(&text);
                                }
                                append_chat_content(&mut state.content, &Value::String(text));
                            }
                            if let Some(reasoning) = parts.reasoning {
                                append_chat_content(
                                    &mut state.reasoning_content,
                                    &Value::String(reasoning),
                                );
                            }
                        }
                    }

                    if let Some(reasoning_content) = delta.get("reasoning_content") {
                        append_chat_content(&mut state.reasoning_content, reasoning_content);
                    }

                    if let Some(reasoning) = delta.get("reasoning") {
                        append_chat_content(&mut state.reasoning_content, reasoning);
                    }

                    if let Some(refusal) = delta.get("refusal") {
                        append_chat_content(&mut state.refusal, refusal);
                    }

                    if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                        merge_chat_tool_call_deltas(&mut state.tool_calls, tool_calls);
                    }
                }

                if let Some(finish_reason) = choice.get("finish_reason") {
                    state.finish_reason = Some(finish_reason.clone());
                }
            }
        }

        Ok(())
    }

    pub(super) fn finish(self) -> Result<Option<Value>> {
        if !self.saw_chunk {
            return Ok(None);
        }

        let mut choices = Vec::new();
        for (index, choice) in self.choices {
            let ChatChoiceState {
                role,
                content,
                reasoning_content,
                refusal,
                tool_calls,
                finish_reason,
            } = choice;
            choices.push(json!({
                "index": index,
                "message": {
                    "role": role.unwrap_or_else(|| "assistant".to_string()),
                    "content": ChatChoiceState::message_content(content, refusal),
                    "tool_calls": tool_calls,
                    "reasoning_content": if reasoning_content.is_empty() {
                        Value::Null
                    } else {
                        Value::String(reasoning_content)
                    },
                },
                "finish_reason": finish_reason.unwrap_or(Value::Null),
            }));
        }

        let mut payload = self.root;
        payload
            .entry("object".to_string())
            .or_insert_with(|| Value::String("chat.completion".to_string()));
        payload.insert("choices".to_string(), Value::Array(choices));
        Ok(Some(Value::Object(payload)))
    }
}

#[derive(Debug, Default)]
struct ChatChoiceState {
    role: Option<String>,
    content: String,
    reasoning_content: String,
    refusal: String,
    tool_calls: Vec<Value>,
    finish_reason: Option<Value>,
}

impl ChatChoiceState {
    fn message_content(content: String, refusal: String) -> Value {
        match (content.is_empty(), refusal.is_empty()) {
            (false, true) => Value::String(content),
            (true, false) => Value::Array(vec![json!({
                "type": "refusal",
                "refusal": refusal,
            })]),
            (false, false) => Value::Array(vec![
                json!({
                    "type": "text",
                    "text": content,
                }),
                json!({
                    "type": "refusal",
                    "refusal": refusal,
                }),
            ]),
            (true, true) => Value::String(String::new()),
        }
    }
}

pub(super) fn looks_like_chat_chunk(payload: &Value) -> bool {
    payload
        .get("object")
        .and_then(Value::as_str)
        .map(|value| value == "chat.completion.chunk")
        .unwrap_or(false)
        || payload
            .get("choices")
            .and_then(Value::as_array)
            .map(|choices| choices.iter().any(|choice| choice.get("delta").is_some()))
            .unwrap_or(false)
}

fn append_chat_content(target: &mut String, payload: &Value) {
    if let Some(text) = payload.as_str() {
        target.push_str(text);
        return;
    }

    if let Some(text) = extract_text_content_preserving_whitespace(payload) {
        target.push_str(&text);
    }
}

fn merge_chat_tool_call_deltas(target: &mut Vec<Value>, source: &[Value]) {
    for (fallback_index, item) in source.iter().enumerate() {
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(fallback_index);
        let slot = ensure_index(target, index);
        merge_chat_tool_call_delta(slot, item);
    }
}

fn merge_chat_tool_call_delta(target: &mut Value, source: &Value) {
    match (target, source) {
        (Value::Object(target_object), Value::Object(source_object)) => {
            for (key, value) in source_object {
                if key == "arguments" {
                    if let Some(delta) = value.as_str() {
                        append_string_field(target_object, key, delta);
                        continue;
                    }
                }

                match target_object.get_mut(key) {
                    Some(existing) => merge_chat_tool_call_delta(existing, value),
                    None => {
                        target_object.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (target, source) => *target = source.clone(),
    }
}

fn ensure_index(items: &mut Vec<Value>, index: usize) -> &mut Value {
    while items.len() <= index {
        items.push(Value::Null);
    }
    &mut items[index]
}

fn append_string_field(object: &mut Map<String, Value>, key: &str, delta: &str) {
    if delta.is_empty() {
        return;
    }

    let entry = object
        .entry(key.to_string())
        .or_insert_with(|| Value::String(String::new()));
    let current = entry.as_str().unwrap_or_default().to_string() + delta;
    *entry = Value::String(current);
}
