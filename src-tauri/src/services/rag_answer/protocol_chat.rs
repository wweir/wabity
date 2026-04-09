use anyhow::{Context, Result};
use serde_json::{json, Value};

use super::{parsing::parse_tool_arguments, ChatCompletionsTurnRequest, LocalToolCall};
use crate::{
    domain::execution::{ExecutionConversationRole, ExecutionConversationTurn},
    infrastructure::openai_compatible::{OpenAiCompatibleClient, OpenAiCompatibleResponseFormat},
};

pub(super) fn build_initial_chat_messages(
    system_prompt: &str,
    conversation: &[ExecutionConversationTurn],
    question: &str,
) -> Vec<Value> {
    let mut messages = vec![json!({
        "role": "system",
        "content": system_prompt,
    })];
    messages.extend(conversation.iter().filter_map(|turn| {
        let content = turn.content.trim();
        if content.is_empty() {
            return None;
        }

        Some(json!({
            "role": match turn.role {
                ExecutionConversationRole::User => "user",
                ExecutionConversationRole::Assistant => "assistant",
            },
            "content": content,
        }))
    }));
    messages.push(json!({
        "role": "user",
        "content": question,
    }));
    messages
}

pub(super) fn build_chat_assistant_tool_call_message(message: &Value) -> Value {
    json!({
        "role": "assistant",
        "content": message.get("content").cloned().unwrap_or(Value::Null),
        "tool_calls": message
            .get("tool_calls")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    })
}

pub(super) async fn request_chat_completions_turn(
    request_args: ChatCompletionsTurnRequest<'_>,
    on_text_delta: Option<&mut (dyn FnMut(&str) + Send)>,
) -> Result<Value> {
    let client = OpenAiCompatibleClient::new_async(
        request_args.client,
        request_args.base_url,
        request_args.api_key,
        "LLM provider base URL",
    )?;
    let body = build_chat_completions_request_body(
        request_args.model,
        request_args.messages,
        &request_args.tool_catalog.request_tools,
        on_text_delta.is_some(),
    );
    if let Some(on_text_delta) = on_text_delta {
        client
            .post_json_with_text_stream(
                "/chat/completions",
                &body,
                "question answering from chat/completions API",
                OpenAiCompatibleResponseFormat::JsonOrSse,
                on_text_delta,
            )
            .await
    } else {
        client
            .post_json(
                "/chat/completions",
                &body,
                "question answering from chat/completions API",
                OpenAiCompatibleResponseFormat::JsonOrSse,
            )
            .await
    }
}

fn build_chat_completions_request_body(
    model: &str,
    messages: &[Value],
    request_tools: &[Value],
    stream: bool,
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": stream,
    });
    if !request_tools.is_empty() {
        body["tools"] = Value::Array(request_tools.to_vec());
        body["tool_choice"] = Value::String("auto".to_string());
        body["parallel_tool_calls"] = Value::Bool(true);
    }
    body
}

pub(super) fn extract_chat_completion_message(payload: &Value) -> Result<Value> {
    payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .cloned()
        .context("chat/completions 未返回 assistant message")
}

pub(super) fn extract_chat_local_tool_calls(message: &Value) -> Result<Vec<LocalToolCall>> {
    let mut calls = Vec::new();
    let Some(items) = message.get("tool_calls").and_then(Value::as_array) else {
        return Ok(calls);
    };

    for item in items {
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
        if item_type != "function" {
            continue;
        }

        let function = item
            .get("function")
            .filter(|value| value.is_object())
            .context("chat/completions tool_call 缺少 function")?;
        let call_id = item
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .context("chat/completions tool_call 缺少 id")?;
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .context("chat/completions tool_call 缺少 function.name")?;
        let arguments = function
            .get("arguments")
            .map(parse_tool_arguments)
            .transpose()?
            .unwrap_or_else(|| json!({}));
        calls.push(LocalToolCall {
            call_id,
            name,
            arguments,
        });
    }

    Ok(calls)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::build_chat_completions_request_body;

    #[test]
    fn chat_completions_request_omits_tool_fields_when_catalog_is_empty() {
        let body = build_chat_completions_request_body(
            "test-model",
            &[json!({"role": "user", "content": "hello"})],
            &[],
            false,
        );

        assert_eq!(body["model"], json!("test-model"));
        assert_eq!(body["stream"], json!(false));
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn chat_completions_request_preserves_tool_fields_when_catalog_is_present() {
        let body = build_chat_completions_request_body(
            "test-model",
            &[json!({"role": "user", "content": "hello"})],
            &[json!({"type": "function", "function": {"name": "wabity.rag.query"}})],
            true,
        );

        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["tool_choice"], json!("auto"));
        assert_eq!(body["parallel_tool_calls"], json!(true));
        assert_eq!(body["tools"].as_array().map(Vec::len), Some(1));
    }
}
