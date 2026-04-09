use anyhow::{bail, Result};
use serde_json::{Map, Value};

use crate::parsing::extract_error_value;

#[derive(Debug, Default)]
pub(super) struct ResponsesSseState {
    response: Map<String, Value>,
    saw_event: bool,
}

impl ResponsesSseState {
    pub(super) fn merge_response(&mut self, response: &Value) {
        if let Some(object) = response.as_object() {
            self.saw_event = true;
            merge_json_object(&mut self.response, object);
        }
    }

    pub(super) fn apply_event<F>(
        &mut self,
        payload: &Value,
        on_text_delta: Option<&mut F>,
    ) -> Result<()>
    where
        F: FnMut(&str),
    {
        self.saw_event = true;
        if let Some(response) = payload.get("response") {
            self.merge_response(response);
        }

        if let Some(response_id) = payload.get("response_id").and_then(Value::as_str) {
            self.response
                .insert("id".to_string(), Value::String(response_id.to_string()));
        }

        match payload.get("type").and_then(Value::as_str) {
            Some("response.output_item.added") | Some("response.output_item.done") => {
                let Some(output_index) = payload
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                else {
                    return Ok(());
                };
                let Some(item) = payload.get("item") else {
                    return Ok(());
                };
                let output = ensure_array_field(&mut self.response, "output");
                let target = ensure_index(output, output_index);
                merge_json_value(target, item);
            }
            Some("response.content_part.added") | Some("response.content_part.done") => {
                let Some(output_index) = payload
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                else {
                    return Ok(());
                };
                let Some(content_index) = payload
                    .get("content_index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                else {
                    return Ok(());
                };
                let Some(part) = payload.get("part") else {
                    return Ok(());
                };
                let content = ensure_output_content(&mut self.response, output_index);
                let target = ensure_index(content, content_index);
                merge_json_value(target, part);
            }
            Some("response.output_text.delta") | Some("response.output_text.done") => {
                let Some(output_index) = payload
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                else {
                    return Ok(());
                };
                let Some(content_index) = payload
                    .get("content_index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                else {
                    return Ok(());
                };

                let part = ensure_output_text_part(&mut self.response, output_index, content_index);
                if let Some(delta) = payload.get("delta").and_then(Value::as_str) {
                    if let Some(callback) = on_text_delta {
                        callback(delta);
                    }
                    append_string_field(part, "text", delta);
                }
                if let Some(text) = payload.get("text").and_then(Value::as_str) {
                    part.insert("text".to_string(), Value::String(text.to_string()));
                }
            }
            Some("response.refusal.delta") | Some("response.refusal.done") => {
                let Some(output_index) = payload
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                else {
                    return Ok(());
                };
                let Some(content_index) = payload
                    .get("content_index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                else {
                    return Ok(());
                };

                let part =
                    ensure_output_refusal_part(&mut self.response, output_index, content_index);
                if let Some(delta) = payload.get("delta").and_then(Value::as_str) {
                    append_string_field(part, "refusal", delta);
                }
                if let Some(text) = payload.get("refusal").and_then(Value::as_str) {
                    part.insert("refusal".to_string(), Value::String(text.to_string()));
                }
            }
            Some("response.failed") => {
                if let Some(message) = payload
                    .get("response")
                    .and_then(extract_error_value)
                    .or_else(|| extract_error_value(payload))
                {
                    bail!("LLM provider SSE 请求失败: {message}");
                }
            }
            _ => {}
        }

        Ok(())
    }

    pub(super) fn finish(self) -> Result<Option<Value>> {
        if !self.saw_event {
            return Ok(None);
        }

        if let Some(message) = extract_error_value(&Value::Object(self.response.clone())) {
            let has_output = self
                .response
                .get("output")
                .and_then(Value::as_array)
                .map(|value| !value.is_empty())
                .unwrap_or(false);
            if !has_output {
                bail!("LLM provider SSE 请求失败: {message}");
            }
        }

        Ok(Some(Value::Object(self.response)))
    }
}

fn merge_json_object(target: &mut Map<String, Value>, source: &Map<String, Value>) {
    for (key, value) in source {
        match target.get_mut(key) {
            Some(existing) => merge_json_value(existing, value),
            None => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

fn merge_json_value(target: &mut Value, source: &Value) {
    match (target, source) {
        (Value::Object(target_object), Value::Object(source_object)) => {
            merge_json_object(target_object, source_object);
        }
        (target, source) => *target = source.clone(),
    }
}

fn ensure_output_content(
    response: &mut Map<String, Value>,
    output_index: usize,
) -> &mut Vec<Value> {
    let output = ensure_array_field(response, "output");
    let item = ensure_index(output, output_index);
    let item = ensure_object_value(item);
    ensure_array_field(item, "content")
}

fn ensure_output_text_part(
    response: &mut Map<String, Value>,
    output_index: usize,
    content_index: usize,
) -> &mut Map<String, Value> {
    let content = ensure_output_content(response, output_index);
    let part = ensure_index(content, content_index);
    let part = ensure_object_value(part);
    part.entry("type".to_string())
        .or_insert_with(|| Value::String("output_text".to_string()));
    part
}

fn ensure_output_refusal_part(
    response: &mut Map<String, Value>,
    output_index: usize,
    content_index: usize,
) -> &mut Map<String, Value> {
    let content = ensure_output_content(response, output_index);
    let part = ensure_index(content, content_index);
    let part = ensure_object_value(part);
    part.entry("type".to_string())
        .or_insert_with(|| Value::String("refusal".to_string()));
    part
}

fn ensure_array_field<'a>(object: &'a mut Map<String, Value>, key: &str) -> &'a mut Vec<Value> {
    let entry = object
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    ensure_array_value(entry)
}

fn ensure_array_value(value: &mut Value) -> &mut Vec<Value> {
    if !value.is_array() {
        *value = Value::Array(Vec::new());
    }
    value.as_array_mut().expect("array value should exist")
}

fn ensure_object_value(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("object value should exist")
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
