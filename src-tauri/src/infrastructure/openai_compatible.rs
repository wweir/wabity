use std::{collections::BTreeMap, io::Read};

use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Map, Value};

use crate::domain::settings::LlmProviderModelEntry;

const BODY_PREVIEW_LIMIT: usize = 240;
const STREAM_READ_BUFFER_SIZE: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiCompatibleResponseFormat {
    Json,
    JsonOrSse,
}

pub struct OpenAiCompatibleClient<'a, HttpClient> {
    client: &'a HttpClient,
    base_url: String,
    api_key: &'a str,
}

pub fn normalize_base_url(base_url: &str, label: &str) -> Result<String> {
    let normalized = base_url.trim().trim_end_matches('/');
    if normalized.is_empty() {
        bail!("{label} 不能为空");
    }

    reqwest::Url::parse(normalized).with_context(|| format!("invalid {label}: {normalized}"))?;
    Ok(normalized.to_string())
}

impl<'a> OpenAiCompatibleClient<'a, reqwest::Client> {
    pub fn new_async(
        client: &'a reqwest::Client,
        base_url: &'a str,
        api_key: &'a str,
        base_url_label: &str,
    ) -> Result<Self> {
        Ok(Self {
            client,
            base_url: normalize_base_url(base_url, base_url_label)?,
            api_key: api_key.trim(),
        })
    }

    pub async fn post_json<T, B>(
        &self,
        endpoint: &str,
        body: &B,
        operation: &str,
        response_format: OpenAiCompatibleResponseFormat,
    ) -> Result<T>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let mut request =
            self.client
                .post(format!("{}{}", self.base_url, normalize_endpoint(endpoint)));
        if !self.api_key.is_empty() {
            request = request.bearer_auth(self.api_key);
        }

        let response = request
            .json(body)
            .send()
            .await
            .with_context(|| format!("failed to request {operation}"))?;
        let payload =
            read_async_response_payload(response, endpoint, operation, response_format).await?;
        serde_json::from_value(payload)
            .with_context(|| format!("failed to deserialize {operation} response"))
    }
}

impl<'a> OpenAiCompatibleClient<'a, reqwest::blocking::Client> {
    pub fn new_blocking(
        client: &'a reqwest::blocking::Client,
        base_url: &'a str,
        api_key: &'a str,
        base_url_label: &str,
    ) -> Result<Self> {
        Ok(Self {
            client,
            base_url: normalize_base_url(base_url, base_url_label)?,
            api_key: api_key.trim(),
        })
    }

    pub fn post_json<T, B>(
        &self,
        endpoint: &str,
        body: &B,
        operation: &str,
        response_format: OpenAiCompatibleResponseFormat,
    ) -> Result<T>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let mut request =
            self.client
                .post(format!("{}{}", self.base_url, normalize_endpoint(endpoint)));
        if !self.api_key.is_empty() {
            request = request.bearer_auth(self.api_key);
        }

        let response = request
            .json(body)
            .send()
            .with_context(|| format!("failed to request {operation}"))?;
        let payload =
            read_blocking_response_payload(response, endpoint, operation, response_format)?;
        serde_json::from_value(payload)
            .with_context(|| format!("failed to deserialize {operation} response"))
    }

    pub fn get_json<T>(&self, endpoint: &str, operation: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let mut request =
            self.client
                .get(format!("{}{}", self.base_url, normalize_endpoint(endpoint)));
        if !self.api_key.is_empty() {
            request = request.bearer_auth(self.api_key);
        }

        let response = request
            .send()
            .with_context(|| format!("failed to request {operation}"))?;
        let status = response.status();
        let body = response
            .text()
            .with_context(|| format!("failed to read {operation} response body"))?;
        let payload = parse_response_payload(
            status,
            body,
            endpoint,
            operation,
            OpenAiCompatibleResponseFormat::Json,
        )?;
        serde_json::from_value(payload)
            .with_context(|| format!("failed to deserialize {operation} response"))
    }
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

fn normalize_endpoint(endpoint: &str) -> String {
    if endpoint.starts_with('/') {
        endpoint.to_string()
    } else {
        format!("/{endpoint}")
    }
}

fn endpoint_display_name(endpoint: &str) -> &str {
    endpoint.trim_start_matches('/')
}

fn parse_response_payload(
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

async fn read_async_response_payload(
    mut response: reqwest::Response,
    endpoint: &str,
    operation: &str,
    response_format: OpenAiCompatibleResponseFormat,
) -> Result<Value> {
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .with_context(|| format!("failed to read {operation} response body"))?;
        return parse_response_payload(status, body, endpoint, operation, response_format);
    }

    match response_format {
        OpenAiCompatibleResponseFormat::Json => {
            let body = response
                .text()
                .await
                .with_context(|| format!("failed to read {operation} response body"))?;
            parse_response_payload(status, body, endpoint, operation, response_format)
        }
        OpenAiCompatibleResponseFormat::JsonOrSse => {
            let mut collector = StreamingPayloadCollector::default();
            while let Some(chunk) = response
                .chunk()
                .await
                .with_context(|| format!("failed to read {operation} response stream"))?
            {
                collector.push_bytes(&chunk)?;
            }

            collector.finish(&format!("{operation} response"))
        }
    }
}

fn read_blocking_response_payload(
    mut response: reqwest::blocking::Response,
    endpoint: &str,
    operation: &str,
    response_format: OpenAiCompatibleResponseFormat,
) -> Result<Value> {
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .with_context(|| format!("failed to read {operation} response body"))?;
        return parse_response_payload(status, body, endpoint, operation, response_format);
    }

    match response_format {
        OpenAiCompatibleResponseFormat::Json => {
            let body = response
                .text()
                .with_context(|| format!("failed to read {operation} response body"))?;
            parse_response_payload(status, body, endpoint, operation, response_format)
        }
        OpenAiCompatibleResponseFormat::JsonOrSse => {
            let mut collector = StreamingPayloadCollector::default();
            let mut buffer = [0_u8; STREAM_READ_BUFFER_SIZE];
            loop {
                let bytes_read = response
                    .read(&mut buffer)
                    .with_context(|| format!("failed to read {operation} response stream"))?;
                if bytes_read == 0 {
                    break;
                }

                collector.push_bytes(&buffer[..bytes_read])?;
            }

            collector.finish(&format!("{operation} response"))
        }
    }
}

#[derive(Debug, Default)]
struct StreamingPayloadCollector {
    kind: Option<StreamingBodyKind>,
    raw_body: Vec<u8>,
    sse: SseStreamState,
}

impl StreamingPayloadCollector {
    fn push_bytes(&mut self, chunk: &[u8]) -> Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }

        match self.kind {
            Some(StreamingBodyKind::Json) => self.raw_body.extend_from_slice(chunk),
            Some(StreamingBodyKind::Sse) => self.sse.push_bytes(chunk)?,
            None => {
                self.raw_body.extend_from_slice(chunk);
                if let Some(kind) = detect_streaming_body_kind(&self.raw_body) {
                    self.kind = Some(kind);
                    if kind == StreamingBodyKind::Sse {
                        let buffered = std::mem::take(&mut self.raw_body);
                        self.sse.push_bytes(&buffered)?;
                    }
                }
            }
        }

        Ok(())
    }

    fn finish(self, operation: &str) -> Result<Value> {
        match self.kind {
            Some(StreamingBodyKind::Sse) => self.sse.finish(),
            Some(StreamingBodyKind::Json) | None => {
                let body = String::from_utf8(self.raw_body)
                    .with_context(|| format!("{operation} is not valid UTF-8"))?;
                parse_json_or_sse_payload(&body, operation)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamingBodyKind {
    Json,
    Sse,
}

fn detect_streaming_body_kind(body: &[u8]) -> Option<StreamingBodyKind> {
    let trimmed = trim_ascii_leading_whitespace_and_bom(body);
    let first_byte = *trimmed.first()?;

    if first_byte == b'{'
        || first_byte == b'['
        || first_byte == b'"'
        || first_byte == b'-'
        || first_byte.is_ascii_digit()
    {
        return Some(StreamingBodyKind::Json);
    }

    for prefix in [
        b"data:".as_slice(),
        b"event:".as_slice(),
        b"id:".as_slice(),
        b"retry:".as_slice(),
    ] {
        if trimmed.starts_with(prefix) {
            return Some(StreamingBodyKind::Sse);
        }
    }

    (first_byte == b':').then_some(StreamingBodyKind::Sse)
}

fn trim_ascii_leading_whitespace_and_bom(bytes: &[u8]) -> &[u8] {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let first_non_whitespace = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    &bytes[first_non_whitespace..]
}

#[derive(Debug, Default)]
struct SseStreamState {
    pending_line: Vec<u8>,
    data_lines: Vec<String>,
    saw_data_frame: bool,
    last_payload: Option<Value>,
    responses: ResponsesSseState,
    chat: ChatCompletionsSseState,
}

impl SseStreamState {
    fn push_bytes(&mut self, chunk: &[u8]) -> Result<()> {
        for byte in chunk {
            self.pending_line.push(*byte);
            if *byte == b'\n' {
                self.flush_pending_line()?;
            }
        }

        Ok(())
    }

    fn finish(mut self) -> Result<Value> {
        if !self.pending_line.is_empty() {
            self.flush_pending_line()?;
        }
        self.flush_frame()?;

        if !self.saw_data_frame {
            bail!("response body is neither JSON nor SSE data frames");
        }

        if let Some(payload) = self.last_payload {
            return Ok(payload);
        }

        if let Some(payload) = self.responses.finish()? {
            return Ok(payload);
        }

        if let Some(payload) = self.chat.finish()? {
            return Ok(payload);
        }

        bail!("SSE response did not include a completed payload")
    }

    fn flush_pending_line(&mut self) -> Result<()> {
        let mut line = std::mem::take(&mut self.pending_line);
        if matches!(line.last(), Some(b'\n')) {
            line.pop();
        }
        if matches!(line.last(), Some(b'\r')) {
            line.pop();
        }

        let line = String::from_utf8(line).context("SSE line is not valid UTF-8")?;
        if line.is_empty() {
            self.flush_frame()?;
            return Ok(());
        }

        if let Some(payload) = line.strip_prefix("data:") {
            self.data_lines.push(payload.trim_start().to_string());
        }

        Ok(())
    }

    fn flush_frame(&mut self) -> Result<()> {
        if self.data_lines.is_empty() {
            return Ok(());
        }

        self.saw_data_frame = true;
        let frame = self.data_lines.join("\n");
        self.data_lines.clear();

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
        self.apply_payload(payload)
    }

    fn apply_payload(&mut self, payload: Value) -> Result<()> {
        if let Some(message) = extract_sse_error_message(&payload) {
            bail!("LLM provider SSE 请求失败: {message}");
        }

        if let Some(response) = payload.get("response").filter(|value| value.is_object()) {
            self.responses.merge_response(response);
            if matches!(
                payload.get("type").and_then(Value::as_str),
                Some("response.completed")
            ) {
                self.last_payload = Some(response.clone());
            }
            return Ok(());
        }

        if looks_like_responses_event(&payload) {
            self.responses.apply_event(&payload)?;
            return Ok(());
        }

        if looks_like_chat_chunk(&payload) {
            self.chat.apply_chunk(&payload)?;
            return Ok(());
        }

        if looks_like_completed_payload(&payload) {
            self.last_payload = Some(payload);
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
struct ResponsesSseState {
    response: Map<String, Value>,
    saw_event: bool,
}

impl ResponsesSseState {
    fn merge_response(&mut self, response: &Value) {
        if let Some(object) = response.as_object() {
            self.saw_event = true;
            merge_json_object(&mut self.response, object);
        }
    }

    fn apply_event(&mut self, payload: &Value) -> Result<()> {
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

    fn finish(self) -> Result<Option<Value>> {
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

#[derive(Debug, Default)]
struct ChatCompletionsSseState {
    root: Map<String, Value>,
    choices: BTreeMap<usize, ChatChoiceState>,
    saw_chunk: bool,
}

impl ChatCompletionsSseState {
    fn apply_chunk(&mut self, payload: &Value) -> Result<()> {
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
                        if let Some(parts) = extract_chat_content_parts(content) {
                            if let Some(text) = parts.content {
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
                }

                if let Some(finish_reason) = choice.get("finish_reason") {
                    state.finish_reason = Some(finish_reason.clone());
                }
            }
        }

        Ok(())
    }

    fn finish(self) -> Result<Option<Value>> {
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
                finish_reason,
            } = choice;
            choices.push(json!({
                "index": index,
                "message": {
                    "role": role.unwrap_or_else(|| "assistant".to_string()),
                    "content": ChatChoiceState::message_content(content, refusal),
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

fn looks_like_completed_payload(payload: &Value) -> bool {
    if looks_like_chat_chunk(payload) {
        return false;
    }

    payload.get("id").and_then(Value::as_str).is_some()
        && (payload.get("output").is_some()
            || payload.get("output_text").is_some()
            || payload.get("choices").is_some()
            || payload.get("data").is_some())
}

fn looks_like_responses_event(payload: &Value) -> bool {
    payload
        .get("type")
        .and_then(Value::as_str)
        .map(|event_type| event_type.starts_with("response."))
        .unwrap_or(false)
        || payload.get("response_id").and_then(Value::as_str).is_some()
}

fn looks_like_chat_chunk(payload: &Value) -> bool {
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

fn extract_sse_error_message(payload: &Value) -> Option<String> {
    if payload.get("error").is_some() {
        return extract_error_value(payload);
    }

    payload
        .get("type")
        .and_then(Value::as_str)
        .filter(|event_type| *event_type == "error")
        .and_then(|_| extract_error_value(payload))
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

fn append_chat_content(target: &mut String, payload: &Value) {
    if let Some(text) = payload.as_str() {
        target.push_str(text);
        return;
    }

    if let Some(text) = extract_text_content(payload) {
        target.push_str(&text);
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

fn split_tagged_reasoning_text(value: &str) -> Option<ChatCompletionsMessageParts> {
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
            if let Some(segment) = trim_to_owned(&value[cursor..]) {
                content.push(segment);
            }
            break;
        };

        let open_start = cursor + offset;
        if let Some(segment) = trim_to_owned(&value[cursor..open_start]) {
            content.push(segment);
        }

        let reasoning_start = open_start + open.len();
        let close_offset = lower[reasoning_start..].find(close)?;
        let reasoning_end = reasoning_start + close_offset;
        if let Some(segment) = trim_to_owned(&value[reasoning_start..reasoning_end]) {
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
    fn push(&mut self, kind: ChatContentKind, value: &str) {
        let Some(segment) = trim_to_owned(value) else {
            return;
        };

        match kind {
            ChatContentKind::Reasoning => self.reasoning.push(segment),
            ChatContentKind::Auto | ChatContentKind::Content => {
                if kind == ChatContentKind::Auto {
                    if let Some(parts) = split_tagged_reasoning_text(&segment) {
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

    fn extend_value(&mut self, payload: &Value, kind: ChatContentKind) {
        match payload {
            Value::Null => {}
            Value::String(text) => self.push(kind, text),
            Value::Array(items) => {
                for item in items {
                    self.extend_value(item, kind);
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
                        self.extend_value(value, ChatContentKind::Reasoning);
                    }
                }

                for key in ["value", "text", "output_text", "input_text"] {
                    if let Some(value) = object.get(key) {
                        self.extend_value(value, inferred_kind);
                    }
                }

                for key in ["content", "parts", "items"] {
                    if let Some(value) = object.get(key) {
                        self.extend_value(value, inferred_kind);
                    }
                }
            }
            _ => {}
        }
    }

    fn into_message_parts(self) -> Option<ChatCompletionsMessageParts> {
        let content = join_non_empty_segments(&self.content);
        let reasoning = join_non_empty_segments(&self.reasoning);
        (content.is_some() || reasoning.is_some())
            .then_some(ChatCompletionsMessageParts { content, reasoning })
    }
}

fn extract_chat_content_parts(payload: &Value) -> Option<ChatCompletionsMessageParts> {
    let mut accumulator = ChatContentAccumulator::default();
    accumulator.extend_value(payload, ChatContentKind::Auto);
    accumulator.into_message_parts()
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
    if let Some(text) = payload
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_string());
    }

    if let Some(text) = payload
        .get("value")
        .and_then(Value::as_str)
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
        .get("output_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_string());
    }

    if let Some(text) = payload
        .get("output_text")
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_string());
    }

    if let Some(text) = payload
        .get("input_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_string());
    }

    if let Some(text) = payload
        .get("input_text")
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

    for key in ["content", "parts", "items"] {
        if let Some(value) = payload.get(key).filter(|value| !value.is_null()) {
            if let Some(text) = extract_text_content(value) {
                return Some(text);
            }
        }
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
        body_preview, describe_chat_completions_response_issue,
        extract_chat_completions_message_parts, extract_chat_completions_text,
        extract_model_entries, extract_model_ids, extract_provider_error_message,
        extract_responses_text, normalize_base_url, parse_json_or_sse_payload,
        StreamingPayloadCollector,
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
    fn parse_json_or_sse_payload_reconstructs_responses_delta_stream() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "event: response.created\n",
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_123\",\"output\":[]}}\n\n",
                "event: response.output_item.added\n",
                "data: {\"type\":\"response.output_item.added\",\"response_id\":\"resp_123\",\"output_index\":0,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n",
                "event: response.output_text.delta\n",
                "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"content_index\":0,\"delta\":\"hel\"}\n\n",
                "event: response.output_text.delta\n",
                "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"content_index\":0,\"delta\":\"lo\"}\n\n",
                "data: [DONE]\n",
            ),
            "responses payload",
        )
        .expect("responses delta stream should reconstruct");

        assert_eq!(payload["id"], "resp_123");
        assert_eq!(extract_responses_text(&payload).as_deref(), Some("hello"));
    }

    #[test]
    fn parse_json_or_sse_payload_reconstructs_chat_completion_chunk_stream() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hel\"}}]}\n\n",
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n",
            ),
            "chat/completions payload",
        )
        .expect("chat completion chunk stream should reconstruct");

        assert_eq!(payload["id"], "chatcmpl_123");
        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn parse_json_or_sse_payload_reconstructs_chat_completion_reasoning_content_stream() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"hel\"}}]}\n\n",
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n",
            ),
            "chat/completions payload",
        )
        .expect("chat completion reasoning chunk stream should reconstruct");

        assert_eq!(payload["id"], "chatcmpl_123");
        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn parse_json_or_sse_payload_routes_reasoning_items_embedded_in_chat_content() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":[{\"type\":\"thinking\",\"text\":\"step 1\"}]}}]}\n\n",
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":[{\"type\":\"text\",\"text\":\"final answer\"}]},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n",
            ),
            "chat/completions payload",
        )
        .expect("chat completion mixed content chunk stream should reconstruct");

        let parts = extract_chat_completions_message_parts(&payload)
            .expect("chat completion mixed content chunk should yield message parts");
        assert_eq!(parts.content.as_deref(), Some("final answer"));
        assert_eq!(parts.reasoning.as_deref(), Some("step 1"));
    }

    #[test]
    fn streaming_payload_collector_handles_split_sse_frames() {
        let chunks = [
            b"data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.comple".as_slice(),
            b"tion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hel\"}}]}\n\n".as_slice(),
            b"data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n".as_slice(),
            b"data: [DONE]\n".as_slice(),
        ];

        let mut collector = StreamingPayloadCollector::default();
        for chunk in chunks {
            collector
                .push_bytes(chunk)
                .expect("split SSE chunk should stream");
        }
        let payload = collector.finish("chat/completions payload").unwrap();

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("hello")
        );
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
    fn extract_chat_completions_text_reads_output_text_parts() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": [
                            { "type": "output_text", "output_text": "hello" },
                            { "type": "output_text", "output_text": { "value": "world" } }
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
    fn extract_chat_completions_text_reads_message_level_refusal() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": [],
                        "refusal": "I can not comply."
                    }
                }
            ]
        });

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("I can not comply.")
        );
    }

    #[test]
    fn extract_chat_completions_text_reads_reasoning_content_when_content_is_empty() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "reasoning_content": "final answer from compatibility field"
                    }
                }
            ]
        });

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("final answer from compatibility field")
        );
    }

    #[test]
    fn extract_chat_completions_message_parts_separates_content_and_reasoning() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "final answer",
                        "reasoning_content": "step 1\nstep 2"
                    }
                }
            ]
        });

        let parts = extract_chat_completions_message_parts(&payload)
            .expect("chat/completions payload should produce message parts");

        assert_eq!(parts.content.as_deref(), Some("final answer"));
        assert_eq!(parts.reasoning.as_deref(), Some("step 1\nstep 2"));
    }

    #[test]
    fn extract_chat_completions_message_parts_reads_reasoning_from_content_items() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": [
                            {
                                "type": "thinking",
                                "text": "step 1"
                            },
                            {
                                "type": "text",
                                "text": "final answer"
                            }
                        ]
                    }
                }
            ]
        });

        let parts = extract_chat_completions_message_parts(&payload)
            .expect("chat/completions payload should produce message parts");

        assert_eq!(parts.content.as_deref(), Some("final answer"));
        assert_eq!(parts.reasoning.as_deref(), Some("step 1"));
    }

    #[test]
    fn extract_chat_completions_message_parts_splits_think_tagged_content() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": "<think>step 1</think>\n\nfinal answer"
                    }
                }
            ]
        });

        let parts = extract_chat_completions_message_parts(&payload)
            .expect("chat/completions payload should produce message parts");

        assert_eq!(parts.content.as_deref(), Some("final answer"));
        assert_eq!(parts.reasoning.as_deref(), Some("step 1"));
    }

    #[test]
    fn extract_chat_completions_message_parts_strips_self_closing_think_marker_from_content() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": "<think>step 1</think>\n\n<think />\n\nfinal answer"
                    }
                }
            ]
        });

        let parts = extract_chat_completions_message_parts(&payload)
            .expect("chat/completions payload should produce message parts");

        assert_eq!(parts.content.as_deref(), Some("final answer"));
        assert_eq!(parts.reasoning.as_deref(), Some("step 1"));
    }

    #[test]
    fn describe_chat_completions_response_issue_summarizes_empty_content() {
        let payload = json!({
            "choices": [
                {
                    "finish_reason": "stop",
                    "message": {
                        "role": "assistant",
                        "content": [],
                        "tool_calls": []
                    }
                }
            ]
        });

        let summary = describe_chat_completions_response_issue(&payload);

        assert!(summary.contains("finish_reason=stop"));
        assert!(summary.contains("tool_calls=0"));
        assert!(summary.contains("content=array(len=0)"));
        assert!(summary.contains("reasoning_content=missing"));
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
