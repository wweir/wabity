use anyhow::{Context, Result};
use serde_json::{json, Value};

use super::{
    parsing::{json_value_to_pretty_text, parse_tool_arguments},
    LocalToolCall, McpToolCallTrace, ResponsesTurnRequest,
};
use crate::{
    domain::execution::ExecutionToolCall,
    infrastructure::openai_compatible::{OpenAiCompatibleClient, OpenAiCompatibleResponseFormat},
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
    let mut body = json!({
        "model": request_args.model,
        "instructions": request_args.instructions,
        "input": request_args.input,
        "stream": on_text_delta.is_some(),
    });
    if !request_args.tool_catalog.request_tools.is_empty() {
        body["tools"] = Value::Array(request_args.tool_catalog.request_tools.clone());
        body["parallel_tool_calls"] = Value::Bool(true);
    }
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

pub(super) fn has_mcp_approval_request(payload: &Value) -> bool {
    payload
        .get("output")
        .and_then(Value::as_array)
        .map(|items| {
            items.iter().any(|item| {
                item.get("type")
                    .and_then(Value::as_str)
                    .map(|value| value == "mcp_approval_request")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
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

pub(super) fn extract_mcp_tool_calls(payload: &Value) -> Vec<McpToolCallTrace> {
    payload
        .get("output")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
                    if item_type != "mcp_call" {
                        return None;
                    }

                    let call_id = item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .or_else(|| item.get("id").and_then(Value::as_str))
                        .map(ToOwned::to_owned);
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown_mcp_tool");
                    let server = item
                        .get("server_label")
                        .and_then(Value::as_str)
                        .unwrap_or("mcp");
                    let error = item.get("error").and_then(Value::as_str).map(str::trim);
                    let formatted_name = format!("{server}::{name}");
                    Some(McpToolCallTrace {
                        call_id,
                        input_detail: extract_mcp_call_input_detail(item),
                        output_detail: extract_mcp_call_output_detail(item),
                        trace: ExecutionToolCall {
                            name: formatted_name.clone(),
                            source: "mcp".to_string(),
                            status: if error.is_some() {
                                "error".to_string()
                            } else {
                                "ok".to_string()
                            },
                            summary: error
                                .filter(|value| !value.is_empty())
                                .map(ToOwned::to_owned)
                                .unwrap_or_else(|| formatted_name),
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn should_retry_without_response_chain(
    round: usize,
    continue_previous_response: bool,
    error: &anyhow::Error,
) -> bool {
    round == 1 && continue_previous_response && is_budget_exceeded_error(error)
}

pub(super) fn should_retry_without_mcp_tools(
    disabled_all_tools_for_compat: bool,
    disabled_mcp_tools_for_compat: bool,
    tool_catalog: &super::ToolCatalog,
    error: &anyhow::Error,
) -> bool {
    !disabled_all_tools_for_compat
        && !disabled_mcp_tools_for_compat
        && tool_catalog
            .request_tools
            .iter()
            .any(super::tool_catalog::is_mcp_tool_definition)
        && is_provider_transport_or_server_error(error)
}

pub(super) fn should_retry_without_all_tools(
    disabled_all_tools_for_compat: bool,
    tool_catalog: &super::ToolCatalog,
    error: &anyhow::Error,
) -> bool {
    !disabled_all_tools_for_compat
        && !tool_catalog.request_tools.is_empty()
        && is_provider_transport_or_server_error(error)
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

fn extract_mcp_call_input_detail(item: &Value) -> Option<String> {
    item.get("arguments")
        .or_else(|| item.get("input"))
        .map(json_value_to_pretty_text)
}

fn extract_mcp_call_output_detail(item: &Value) -> Option<String> {
    if let Some(error) = item
        .get("error")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(error.to_string());
    }

    item.get("output")
        .or_else(|| item.get("result"))
        .or_else(|| item.get("content"))
        .map(json_value_to_pretty_text)
}
