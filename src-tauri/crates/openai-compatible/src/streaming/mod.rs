mod chat;
mod responses;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::parsing::{body_preview, extract_error_value, parse_json_or_sse_payload};
use chat::{looks_like_chat_chunk, ChatCompletionsSseState};
use responses::ResponsesSseState;

#[derive(Debug, Default)]
pub(crate) struct StreamingPayloadCollector {
    kind: Option<StreamingBodyKind>,
    raw_body: Vec<u8>,
    sse: SseStreamState,
}

impl StreamingPayloadCollector {
    pub(crate) fn push_bytes(&mut self, chunk: &[u8]) -> Result<()> {
        self.push_bytes_internal(chunk, None::<&mut fn(&str)>)
    }

    pub(crate) fn push_bytes_with_text_stream<F>(
        &mut self,
        chunk: &[u8],
        on_text_delta: &mut F,
    ) -> Result<()>
    where
        F: FnMut(&str),
    {
        self.push_bytes_internal(chunk, Some(on_text_delta))
    }

    fn push_bytes_internal<F>(&mut self, chunk: &[u8], on_text_delta: Option<&mut F>) -> Result<()>
    where
        F: FnMut(&str),
    {
        if chunk.is_empty() {
            return Ok(());
        }

        match self.kind {
            Some(StreamingBodyKind::Json) => self.raw_body.extend_from_slice(chunk),
            Some(StreamingBodyKind::Sse) => self.sse.push_bytes_internal(chunk, on_text_delta)?,
            None => {
                self.raw_body.extend_from_slice(chunk);
                if let Some(kind) = detect_streaming_body_kind(&self.raw_body) {
                    self.kind = Some(kind);
                    if kind == StreamingBodyKind::Sse {
                        let buffered = std::mem::take(&mut self.raw_body);
                        self.sse.push_bytes_internal(&buffered, on_text_delta)?;
                    }
                }
            }
        }

        Ok(())
    }

    pub(crate) fn finish(self, operation: &str) -> Result<Value> {
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
    fn push_bytes_internal<F>(
        &mut self,
        chunk: &[u8],
        mut on_text_delta: Option<&mut F>,
    ) -> Result<()>
    where
        F: FnMut(&str),
    {
        for byte in chunk {
            self.pending_line.push(*byte);
            if *byte == b'\n' {
                self.flush_pending_line(on_text_delta.as_deref_mut())?;
            }
        }

        Ok(())
    }

    fn finish(mut self) -> Result<Value> {
        if !self.pending_line.is_empty() {
            self.flush_pending_line(None::<&mut fn(&str)>)?;
        }
        self.flush_frame(None::<&mut fn(&str)>)?;

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

    fn flush_pending_line<F>(&mut self, on_text_delta: Option<&mut F>) -> Result<()>
    where
        F: FnMut(&str),
    {
        let mut line = std::mem::take(&mut self.pending_line);
        if matches!(line.last(), Some(b'\n')) {
            line.pop();
        }
        if matches!(line.last(), Some(b'\r')) {
            line.pop();
        }

        let line = String::from_utf8(line).context("SSE line is not valid UTF-8")?;
        if line.is_empty() {
            self.flush_frame(on_text_delta)?;
            return Ok(());
        }

        if let Some(payload) = line.strip_prefix("data:") {
            self.data_lines.push(payload.trim_start().to_string());
        }

        Ok(())
    }

    fn flush_frame<F>(&mut self, on_text_delta: Option<&mut F>) -> Result<()>
    where
        F: FnMut(&str),
    {
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
        self.apply_payload(payload, on_text_delta)
    }

    fn apply_payload<F>(&mut self, payload: Value, on_text_delta: Option<&mut F>) -> Result<()>
    where
        F: FnMut(&str),
    {
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
            self.responses.apply_event(&payload, on_text_delta)?;
            return Ok(());
        }

        if looks_like_chat_chunk(&payload) {
            self.chat.apply_chunk(&payload, on_text_delta)?;
            return Ok(());
        }

        if looks_like_completed_payload(&payload) {
            self.last_payload = Some(payload);
        }

        Ok(())
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
