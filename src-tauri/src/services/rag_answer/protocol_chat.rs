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
) -> Result<Value> {
    let client = OpenAiCompatibleClient::new_async(
        request_args.client,
        request_args.base_url,
        request_args.api_key,
        "LLM provider base URL",
    )?;
    client
        .post_json(
            "/chat/completions",
            &json!({
                "model": request_args.model,
                "messages": request_args.messages,
                "tools": request_args.tool_catalog.request_tools,
                "tool_choice": "auto",
                "parallel_tool_calls": true,
                "stream": false,
            }),
            "question answering from chat/completions API",
            OpenAiCompatibleResponseFormat::Json,
        )
        .await
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
