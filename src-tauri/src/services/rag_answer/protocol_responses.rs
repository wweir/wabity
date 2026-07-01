use anyhow::{Context, Result};
use serde_json::{json, Value};

use super::{parsing::parse_tool_arguments, LocalToolCall, ResponsesTurnRequest};
use crate::infrastructure::openai_compatible::{
    OpenAiCompatibleClient, OpenAiCompatibleResponseFormat,
};

pub(super) async fn request_responses_turn(
    request_args: ResponsesTurnRequest<'_>,
    on_text_delta: Option<&mut (dyn FnMut(&str) + Send)>,
) -> Result<Value> {
    let client = OpenAiCompatibleClient::new_async(
        request_args.client,
        request_args.base_url,
        request_args.api_key,
        "LLM provider base URL",
    )?;
    let mut body = build_responses_request_body(
        request_args.model,
        request_args.instructions,
        &request_args.input,
        &request_args.tool_catalog.request_tools,
        on_text_delta.is_some(),
    );
    if let Some(previous_response_id) = request_args.previous_response_id {
        body["previous_response_id"] = Value::String(previous_response_id.to_string());
    }

    if let Some(on_text_delta) = on_text_delta {
        client
            .post_json_with_text_stream(
                "/responses",
                &body,
                "question answering from responses API",
                OpenAiCompatibleResponseFormat::JsonOrSse,
                on_text_delta,
            )
            .await
    } else {
        client
            .post_json(
                "/responses",
                &body,
                "question answering from responses API",
                OpenAiCompatibleResponseFormat::JsonOrSse,
            )
            .await
    }
}

fn build_responses_request_body(
    model: &str,
    instructions: &str,
    input: &Value,
    request_tools: &[Value],
    stream: bool,
) -> Value {
    let mut body = json!({
        "model": model,
        "instructions": instructions,
        "input": input,
        "stream": stream,
    });
    if !request_tools.is_empty() {
        body["tools"] = Value::Array(request_tools.to_vec());
        body["parallel_tool_calls"] = Value::Bool(true);
    }
    body
}

pub(super) fn extract_local_tool_calls(payload: &Value) -> Result<Vec<LocalToolCall>> {
    let mut calls = Vec::new();
    let Some(items) = payload.get("output").and_then(Value::as_array) else {
        return Ok(calls);
    };

    for item in items {
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
        if item_type != "function_call" {
            continue;
        }

        let call_id = item
            .get("call_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .context("responses function_call 缺少 call_id")?;
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .context("responses function_call 缺少 name")?;
        let arguments = item
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

pub(super) fn should_retry_without_response_chain(
    round: usize,
    continue_previous_response: bool,
    error: &anyhow::Error,
) -> bool {
    round == 1 && continue_previous_response && is_budget_exceeded_error(error)
}

pub(super) fn should_retry_without_all_tools(
    disabled_all_tools_for_compat: bool,
    tool_catalog: &super::ToolCatalog,
    error: &anyhow::Error,
) -> bool {
    !disabled_all_tools_for_compat
        && !tool_catalog.request_tools.is_empty()
        && (is_provider_transport_or_server_error(error)
            || is_provider_tool_compatibility_error(error))
}

pub(super) fn is_budget_exceeded_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("budget has been exceeded")
        || (message.contains("budget") && message.contains("exceeded"))
        || message.contains("context_length_exceeded")
        || message.contains("maximum context length")
        || message.contains("prompt is too long")
        || message.contains("request too large")
}

fn is_provider_transport_or_server_error(error: &anyhow::Error) -> bool {
    if error
        .chain()
        .filter_map(|source| source.downcast_ref::<reqwest::Error>())
        .any(reqwest::Error::is_timeout)
    {
        return true;
    }

    let message = error.to_string();
    ["500", "502", "503", "504"]
        .iter()
        .any(|status| message.contains(&format!("({status} ")))
        || message.to_ascii_lowercase().contains("context canceled")
}

fn is_provider_tool_compatibility_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    if !message.contains("(400 ") {
        return false;
    }

    let mentions_tooling = [
        "parallel_tool_calls",
        "tool_choice",
        "function calling",
        "function call",
        "tool call",
        "\"tools\"",
        " tools ",
    ]
    .iter()
    .any(|needle| message.contains(needle));
    if !mentions_tooling {
        return false;
    }

    [
        "unsupported",
        "not supported",
        "does not support",
        "not support",
        "unknown field",
        "unexpected field",
        "invalid field",
        "extra inputs",
        "not allowed",
        "invalid input",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}
