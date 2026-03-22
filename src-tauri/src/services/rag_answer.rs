use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use futures::future::join_all;
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::{
    domain::{
        acp::{AcpActionEvent, AcpMcpServerConfig, AcpNameValuePair},
        execution::{
            ExecutionCitation, ExecutionConversationRole, ExecutionConversationState,
            ExecutionConversationTurn, ExecutionResult, ExecutionToolCall,
        },
        settings::{
            LlmProviderConfig, LlmProviderProtocol, LlmSettings, PromptsSettings, RagSettings,
        },
    },
    infrastructure::openai_compatible::{
        extract_provider_error_message, extract_responses_text, extract_text_content,
        normalize_base_url, parse_json_or_sse_payload, parse_json_payload,
    },
    services::{
        rag,
        rag_query::{self, RagSearchHit},
    },
};

const RAG_ANSWER_COMMAND_ALIASES: [&str; 3] = ["/ask", "/qa", "/docs"];
const READ_FILE_TOOL_NAME: &str = "wabity.read_file_lines";
const RAG_QUERY_TOOL_NAME: &str = "wabity.rag.query";
const MAX_TOOL_ROUNDS: usize = 8;
const MAX_READ_FILE_LINES: usize = 240;
const MAX_READ_FILE_BYTES: u64 = 512 * 1024;
const DEFAULT_RAG_TOOL_TOP_K: usize = 6;
const DEFAULT_RAG_TOOL_MIN_SCORE: f32 = 0.2;
const CITATION_SNIPPET_MAX_CHARS: usize = 240;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RagAnswerPayload {
    kind: &'static str,
    render: &'static str,
    response_id: Option<String>,
    conversation_state: ExecutionConversationState,
    citations: Vec<ExecutionCitation>,
    retrieval: RagRetrievalPayload,
    actions: Vec<AcpActionEvent>,
    tools: RagToolUsagePayload,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RagRetrievalPayload {
    query: String,
    match_count: usize,
    file_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RagToolUsagePayload {
    available: Vec<String>,
    skipped: Vec<String>,
    calls: Vec<ExecutionToolCall>,
}

#[derive(Debug, Clone)]
struct LocalToolCall {
    call_id: String,
    name: String,
    arguments: Value,
}

#[derive(Debug, Clone)]
struct ExecutedToolCall {
    call_id: String,
    name: String,
    output: String,
    citations: Vec<ExecutionCitation>,
    trace: ExecutionToolCall,
}

#[derive(Debug, Clone)]
struct McpToolCallTrace {
    call_id: Option<String>,
    trace: ExecutionToolCall,
    input_detail: Option<String>,
    output_detail: Option<String>,
}

#[derive(Debug, Clone)]
struct QuestionToolRuntime<'a> {
    data_dir: &'a Path,
    workspace_root: &'a Path,
    rag_settings: &'a RagSettings,
    llm_settings: &'a LlmSettings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadFileToolLine {
    number: usize,
    text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadFileToolResult {
    absolute_path: String,
    path: String,
    line_start: usize,
    line_end: usize,
    line_count: usize,
    lines: Vec<ReadFileToolLine>,
}

#[derive(Debug, Clone)]
struct ToolCatalog {
    request_tools: Vec<Value>,
    available_names: Vec<String>,
    skipped_mcp_servers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuestionAnswerProtocol {
    Responses,
    ChatCompletions,
}

pub struct QuestionAnswerRequest<'a> {
    pub data_dir: &'a Path,
    pub workspace_root: &'a Path,
    pub raw_text: &'a str,
    pub conversation: &'a [ExecutionConversationTurn],
    pub conversation_state: Option<&'a ExecutionConversationState>,
    pub prompts_settings: &'a PromptsSettings,
    pub rag_settings: &'a RagSettings,
    pub llm_settings: &'a LlmSettings,
    pub mcp_servers: &'a [AcpMcpServerConfig],
}

struct PreparedConversationState<'a> {
    conversation: &'a [ExecutionConversationTurn],
    state: ExecutionConversationState,
}

pub async fn answer_question(request: QuestionAnswerRequest<'_>) -> Result<ExecutionResult> {
    let question = question_payload(request.raw_text);
    if question.is_empty() {
        bail!("请输入要提问的内容");
    }

    let provider = resolve_answer_provider(request.llm_settings)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .context("failed to build HTTP client for question answering")?;
    let base_url = normalize_base_url(&provider.base_url, "LLM provider base URL")?;
    let api_key = provider.api_key.trim();
    let model = provider
        .llm_model_name()
        .context("问答使用的 LLM 模型不能为空")?;
    let system_prompt =
        build_runtime_system_prompt(&request.prompts_settings.rag_answer_system_prompt);
    let prepared_conversation_state = prepare_conversation_state(
        request.conversation,
        request.conversation_state,
        provider,
        request.workspace_root,
    );
    let runtime = QuestionToolRuntime {
        data_dir: request.data_dir,
        workspace_root: request.workspace_root,
        rag_settings: request.rag_settings,
        llm_settings: request.llm_settings,
    };
    let execution = QuestionAnswerExecutionContext {
        provider,
        client: &client,
        base_url: &base_url,
        api_key,
        model,
        system_prompt: &system_prompt,
        runtime: &runtime,
        question,
        conversation: prepared_conversation_state.conversation,
        conversation_state: &prepared_conversation_state.state,
        mcp_servers: request.mcp_servers,
    };
    let initial_protocol = answer_protocol(provider);
    let (answer, protocol, tool_catalog) = match request_answer(initial_protocol, &execution).await
    {
        Ok(answer) => answer,
        Err(primary_error) => {
            let fallback_protocol = alternate_answer_protocol(initial_protocol);
            if !should_retry_answer_with_protocol(initial_protocol, &primary_error) {
                return Err(primary_error);
            }

            request_answer(fallback_protocol, &execution).await?
        }
    };

    let citations = deduplicate_and_number_citations(answer.citations);
    let retrieval = RagRetrievalPayload {
        query: question.to_string(),
        match_count: citations.len(),
        file_count: citations
            .iter()
            .map(|citation| citation.absolute_path.clone())
            .collect::<BTreeSet<_>>()
            .len(),
    };
    let secondary_text = if answer.tool_calls.is_empty() {
        format!(
            "{} 未调用工具，直接生成了回答",
            protocol_label(protocol, provider)
        )
    } else {
        format!(
            "{} 已执行 {} 次工具调用，引用 {} 个文件",
            protocol_label(protocol, provider),
            answer.tool_calls.len(),
            retrieval.file_count
        )
    };
    let secondary_text = if tool_catalog.skipped_mcp_servers.is_empty() {
        secondary_text
    } else {
        format!(
            "{secondary_text}；跳过 {} 个当前协议不支持的 MCP server",
            tool_catalog.skipped_mcp_servers.len()
        )
    };
    let response_id = answer.response_id.clone();

    Ok(ExecutionResult::success(
        Some(answer.final_answer),
        Some(secondary_text),
        Some(serde_json::to_value(RagAnswerPayload {
            kind: "rag_answer",
            render: "markdown",
            response_id: response_id.clone(),
            conversation_state: build_answer_conversation_state(
                response_id,
                provider,
                request.workspace_root,
                &citations,
                &answer.actions,
                &answer.tool_calls,
            ),
            citations,
            retrieval,
            tools: RagToolUsagePayload {
                available: tool_catalog.available_names.clone(),
                skipped: tool_catalog.skipped_mcp_servers.clone(),
                calls: answer.tool_calls,
            },
            actions: answer.actions,
        })?),
        vec!["copy_text"],
        false,
    ))
}

struct QuestionAnswerExecutionContext<'a> {
    provider: &'a LlmProviderConfig,
    client: &'a Client,
    base_url: &'a str,
    api_key: &'a str,
    model: &'a str,
    system_prompt: &'a str,
    runtime: &'a QuestionToolRuntime<'a>,
    question: &'a str,
    conversation: &'a [ExecutionConversationTurn],
    conversation_state: &'a ExecutionConversationState,
    mcp_servers: &'a [AcpMcpServerConfig],
}

async fn request_answer(
    protocol: QuestionAnswerProtocol,
    context: &QuestionAnswerExecutionContext<'_>,
) -> Result<(AnswerOutcome, QuestionAnswerProtocol, ToolCatalog)> {
    let tool_catalog = build_tool_catalog(context.mcp_servers, protocol);
    let answer = match protocol {
        QuestionAnswerProtocol::Responses => {
            answer_with_responses(ResponsesAnswerRequest {
                client: context.client,
                base_url: context.base_url,
                api_key: context.api_key,
                model: context.model,
                supports_stateful: context.provider.supports_stateful(),
                system_prompt: context.system_prompt,
                tool_catalog: &tool_catalog,
                conversation: context.conversation,
                conversation_state: Some(context.conversation_state),
                question: context.question,
                runtime: context.runtime,
            })
            .await?
        }
        QuestionAnswerProtocol::ChatCompletions => {
            answer_with_chat_completions(ChatCompletionsAnswerRequest {
                client: context.client,
                base_url: context.base_url,
                api_key: context.api_key,
                model: context.model,
                system_prompt: context.system_prompt,
                tool_catalog: &tool_catalog,
                conversation: context.conversation,
                conversation_state: Some(context.conversation_state),
                question: context.question,
                runtime: context.runtime,
            })
            .await?
        }
    };

    Ok((answer, protocol, tool_catalog))
}

struct AnswerOutcome {
    final_answer: String,
    response_id: Option<String>,
    citations: Vec<ExecutionCitation>,
    tool_calls: Vec<ExecutionToolCall>,
    actions: Vec<AcpActionEvent>,
}

struct ResponsesAnswerRequest<'a> {
    client: &'a Client,
    base_url: &'a str,
    api_key: &'a str,
    model: &'a str,
    supports_stateful: bool,
    system_prompt: &'a str,
    tool_catalog: &'a ToolCatalog,
    conversation: &'a [ExecutionConversationTurn],
    conversation_state: Option<&'a ExecutionConversationState>,
    question: &'a str,
    runtime: &'a QuestionToolRuntime<'a>,
}

struct ChatCompletionsAnswerRequest<'a> {
    client: &'a Client,
    base_url: &'a str,
    api_key: &'a str,
    model: &'a str,
    system_prompt: &'a str,
    tool_catalog: &'a ToolCatalog,
    conversation: &'a [ExecutionConversationTurn],
    conversation_state: Option<&'a ExecutionConversationState>,
    question: &'a str,
    runtime: &'a QuestionToolRuntime<'a>,
}

async fn answer_with_responses(request: ResponsesAnswerRequest<'_>) -> Result<AnswerOutcome> {
    let initial_previous_response_id =
        previous_response_id_for_follow_up(request.conversation_state, request.supports_stateful);
    let continue_previous_response = initial_previous_response_id.is_some();
    let mut previous_response_id = initial_previous_response_id;
    let mut pending_input = json!(build_initial_responses_input(
        request.conversation,
        request.question,
        continue_previous_response,
    ));
    let mut citations = request
        .conversation_state
        .map(|state| state.citations.clone())
        .unwrap_or_default();
    let mut tool_calls = request
        .conversation_state
        .map(|state| state.tool_calls.clone())
        .unwrap_or_default();
    let mut actions = request
        .conversation_state
        .map(|state| state.actions.clone())
        .unwrap_or_default();
    let mut round = 0usize;

    let final_answer = loop {
        round += 1;
        let response = match request_responses_turn(ResponsesTurnRequest {
            client: request.client,
            base_url: request.base_url,
            api_key: request.api_key,
            model: request.model,
            instructions: request.system_prompt,
            tool_catalog: request.tool_catalog,
            previous_response_id: previous_response_id.as_deref(),
            input: pending_input.clone(),
        })
        .await
        {
            Ok(response) => response,
            Err(error)
                if should_retry_without_response_chain(
                    round,
                    continue_previous_response,
                    &error,
                ) =>
            {
                previous_response_id = None;
                pending_input = json!(build_initial_responses_input(
                    request.conversation,
                    request.question,
                    false,
                ));
                round = round.saturating_sub(1);
                continue;
            }
            Err(error) => return Err(error),
        };
        previous_response_id = Some(
            response
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .context("responses API 未返回 response id")?,
        );

        if has_mcp_approval_request(&response) {
            bail!("问答请求触发了 MCP approval，但当前实现要求所有注入工具都可直接执行");
        }

        let mcp_calls = extract_mcp_tool_calls(&response);
        let local_calls = extract_local_tool_calls(&response)?;
        if !mcp_calls.is_empty() || !local_calls.is_empty() {
            actions.push(build_round_action(
                round,
                mcp_calls.len() + local_calls.len(),
            ));
        } else if round > 1 {
            actions.push(AcpActionEvent {
                kind: "info".to_string(),
                title: format!("第 {round} 步"),
                correlation_id: None,
                detail: Some("工具结果已收敛，开始生成最终回答".to_string()),
            });
        }

        for (index, call) in mcp_calls.iter().enumerate() {
            tool_calls.push(call.trace.clone());
            let correlation_id = call
                .call_id
                .clone()
                .unwrap_or_else(|| format!("mcp-round-{round}-{index}"));
            actions.push(AcpActionEvent {
                kind: "tool-call".to_string(),
                title: call.trace.name.clone(),
                correlation_id: Some(correlation_id.clone()),
                detail: call.input_detail.clone(),
            });
            actions.push(AcpActionEvent {
                kind: "tool-update".to_string(),
                title: call.trace.name.clone(),
                correlation_id: Some(correlation_id),
                detail: call
                    .output_detail
                    .clone()
                    .or_else(|| Some(call.trace.summary.clone())),
            });
        }

        if local_calls.is_empty() {
            break extract_responses_text(&response)
                .context("LLM provider responses 未返回可识别的回答")?;
        }

        if tool_calls.len() + local_calls.len() > MAX_TOOL_ROUNDS {
            bail!("问答工具调用轮数超过限制，模型没有稳定收敛");
        }

        let executed = join_all(
            local_calls
                .iter()
                .cloned()
                .map(|call| execute_local_tool_call(request.runtime, call)),
        )
        .await;
        let mut tool_outputs = Vec::new();
        for (call, result) in local_calls.iter().zip(executed) {
            actions.push(AcpActionEvent {
                kind: "tool-call".to_string(),
                title: call.name.clone(),
                correlation_id: Some(call.call_id.clone()),
                detail: Some(json_value_to_pretty_text(&call.arguments)),
            });

            let executed = result?;
            citations.extend(executed.citations.clone());
            tool_calls.push(executed.trace.clone());
            actions.push(AcpActionEvent {
                kind: "tool-update".to_string(),
                title: executed.name.clone(),
                correlation_id: Some(executed.call_id.clone()),
                detail: Some(json_string_to_pretty_text(&executed.output)),
            });
            tool_outputs.push(executed);
        }
        pending_input = json!(build_tool_output_items(&tool_outputs));
    };

    Ok(AnswerOutcome {
        final_answer,
        response_id: previous_response_id,
        citations,
        tool_calls,
        actions,
    })
}

async fn answer_with_chat_completions(
    request: ChatCompletionsAnswerRequest<'_>,
) -> Result<AnswerOutcome> {
    let mut messages = build_initial_chat_messages(
        request.system_prompt,
        request.conversation,
        request.question,
    );
    let mut citations = request
        .conversation_state
        .map(|state| state.citations.clone())
        .unwrap_or_default();
    let mut tool_calls = request
        .conversation_state
        .map(|state| state.tool_calls.clone())
        .unwrap_or_default();
    let mut actions = request
        .conversation_state
        .map(|state| state.actions.clone())
        .unwrap_or_default();
    let mut round = 0usize;

    let final_answer = loop {
        round += 1;
        let response = request_chat_completions_turn(ChatCompletionsTurnRequest {
            client: request.client,
            base_url: request.base_url,
            api_key: request.api_key,
            model: request.model,
            tool_catalog: request.tool_catalog,
            messages: &messages,
        })
        .await?;
        let assistant_message = extract_chat_completion_message(&response)?;
        let local_calls = extract_chat_local_tool_calls(&assistant_message)?;

        if !local_calls.is_empty() {
            actions.push(build_round_action(round, local_calls.len()));
        } else if round > 1 {
            actions.push(AcpActionEvent {
                kind: "info".to_string(),
                title: format!("第 {round} 步"),
                correlation_id: None,
                detail: Some("工具结果已收敛，开始生成最终回答".to_string()),
            });
        }

        if local_calls.is_empty() {
            let content = assistant_message
                .get("content")
                .context("chat/completions message 缺少 content")?;
            break extract_text_content(content)
                .context("LLM provider chat/completions 未返回可识别的回答")?;
        }

        if tool_calls.len() + local_calls.len() > MAX_TOOL_ROUNDS {
            bail!("问答工具调用轮数超过限制，模型没有稳定收敛");
        }

        let assistant_tool_call_message =
            build_chat_assistant_tool_call_message(&assistant_message);
        messages.push(assistant_tool_call_message);

        let executed = join_all(
            local_calls
                .iter()
                .cloned()
                .map(|call| execute_local_tool_call(request.runtime, call)),
        )
        .await;
        for (call, result) in local_calls.iter().zip(executed) {
            actions.push(AcpActionEvent {
                kind: "tool-call".to_string(),
                title: call.name.clone(),
                correlation_id: Some(call.call_id.clone()),
                detail: Some(json_value_to_pretty_text(&call.arguments)),
            });

            let executed = result?;
            citations.extend(executed.citations.clone());
            tool_calls.push(executed.trace.clone());
            actions.push(AcpActionEvent {
                kind: "tool-update".to_string(),
                title: executed.name.clone(),
                correlation_id: Some(executed.call_id.clone()),
                detail: Some(json_string_to_pretty_text(&executed.output)),
            });
            messages.push(json!({
                "role": "tool",
                "tool_call_id": executed.call_id,
                "content": executed.output,
            }));
        }
    };

    Ok(AnswerOutcome {
        final_answer,
        response_id: None,
        citations,
        tool_calls,
        actions,
    })
}

fn build_round_action(round: usize, tool_count: usize) -> AcpActionEvent {
    AcpActionEvent {
        kind: "info".to_string(),
        title: format!("第 {round} 步"),
        correlation_id: None,
        detail: Some(format!("模型发起了 {tool_count} 个工具调用")),
    }
}

fn answer_protocol(provider: &LlmProviderConfig) -> QuestionAnswerProtocol {
    match provider.protocol {
        LlmProviderProtocol::Responses => QuestionAnswerProtocol::Responses,
        LlmProviderProtocol::ChatCompletions => QuestionAnswerProtocol::ChatCompletions,
    }
}

fn alternate_answer_protocol(protocol: QuestionAnswerProtocol) -> QuestionAnswerProtocol {
    match protocol {
        QuestionAnswerProtocol::Responses => QuestionAnswerProtocol::ChatCompletions,
        QuestionAnswerProtocol::ChatCompletions => QuestionAnswerProtocol::Responses,
    }
}

fn should_retry_answer_with_protocol(
    protocol: QuestionAnswerProtocol,
    error: &anyhow::Error,
) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    match protocol {
        QuestionAnswerProtocol::Responses => {
            message.contains("route protocol chat")
                || message.contains("stateless responses requests")
                || retryable_endpoint_mismatch(&message, "responses")
        }
        QuestionAnswerProtocol::ChatCompletions => {
            message.contains("route protocol responses")
                || (message.contains("chat/completions") && message.contains("does not support"))
                || retryable_endpoint_mismatch(&message, "chat/completions")
        }
    }
}

fn retryable_endpoint_mismatch(message: &str, endpoint: &str) -> bool {
    let endpoint_not_available = message.contains(endpoint)
        && (message.contains("404")
            || message.contains("405")
            || message.contains("501")
            || message.contains("not found")
            || message.contains("method not allowed")
            || message.contains("unsupported"));
    let endpoint_route_missing =
        message.contains(&format!("/{endpoint}")) && message.contains("not found");

    endpoint_not_available || endpoint_route_missing
}

fn should_retry_without_response_chain(
    round: usize,
    continue_previous_response: bool,
    error: &anyhow::Error,
) -> bool {
    round == 1 && continue_previous_response && is_budget_exceeded_error(error)
}

fn is_budget_exceeded_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("budget has been exceeded")
        || (message.contains("budget") && message.contains("exceeded"))
        || message.contains("context_length_exceeded")
        || message.contains("maximum context length")
        || message.contains("prompt is too long")
        || message.contains("request too large")
}

fn protocol_label(protocol: QuestionAnswerProtocol, provider: &LlmProviderConfig) -> String {
    match protocol {
        QuestionAnswerProtocol::Responses => {
            if provider.supports_stateful() {
                "responses(stateful)".to_string()
            } else {
                "responses(stateless)".to_string()
            }
        }
        QuestionAnswerProtocol::ChatCompletions => "chat/completions".to_string(),
    }
}

fn build_tool_catalog(
    mcp_servers: &[AcpMcpServerConfig],
    protocol: QuestionAnswerProtocol,
) -> ToolCatalog {
    let mut request_tools = vec![
        build_read_file_tool(protocol),
        build_rag_query_tool(protocol),
    ];
    let mut available_names = vec![
        READ_FILE_TOOL_NAME.to_string(),
        RAG_QUERY_TOOL_NAME.to_string(),
    ];
    let mut skipped_mcp_servers = Vec::new();

    for server in mcp_servers {
        if protocol == QuestionAnswerProtocol::ChatCompletions {
            let name = match server {
                AcpMcpServerConfig::Http(server) => &server.name,
                AcpMcpServerConfig::Sse(server) => &server.name,
                AcpMcpServerConfig::Stdio(server) => &server.name,
            };
            skipped_mcp_servers.push(name.clone());
            continue;
        }

        match server {
            AcpMcpServerConfig::Http(server) => {
                available_names.push(format!("mcp:{}", server.name));
                request_tools.push(json!({
                    "type": "mcp",
                    "server_label": server.name,
                    "server_url": server.url,
                    "headers": name_value_pairs_to_json_object(&server.headers),
                    "require_approval": "never",
                }));
            }
            AcpMcpServerConfig::Sse(server) => {
                available_names.push(format!("mcp:{}", server.name));
                request_tools.push(json!({
                    "type": "mcp",
                    "server_label": server.name,
                    "server_url": server.url,
                    "headers": name_value_pairs_to_json_object(&server.headers),
                    "require_approval": "never",
                }));
            }
            AcpMcpServerConfig::Stdio(server) => {
                skipped_mcp_servers.push(server.name.clone());
            }
        }
    }

    ToolCatalog {
        request_tools,
        available_names,
        skipped_mcp_servers,
    }
}

fn build_read_file_tool(protocol: QuestionAnswerProtocol) -> Value {
    build_function_tool(
            protocol,
            READ_FILE_TOOL_NAME,
            "Read exact lines from a local text file after you already know which file matters. Access is restricted to the current workspace root and explicitly configured RAG source roots. Prefer calling wabity.rag.query first to locate evidence, then use this tool to verify the precise file path and line range you want to cite. Do not use this as a blind file discovery tool.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path, ~/ path, or workspace-relative path of the file to read, but it must resolve inside the current workspace root or an explicit RAG source root. Use a concrete path you already identified from retrieval results."
                    },
                "line_start": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-based starting line number. Choose the smallest range that still captures the exact evidence you need."
                },
                "line_count": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_READ_FILE_LINES,
                    "description": "Number of lines to read. Keep the window tight; expand only if the first slice is insufficient."
                }
            },
            "required": ["path", "line_start", "line_count"]
        }),
    )
}

fn build_rag_query_tool(protocol: QuestionAnswerProtocol) -> Value {
    build_function_tool(
        protocol,
        RAG_QUERY_TOOL_NAME,
        "Search the local RAG index to find likely evidence before answering repository or documentation questions. Use this first when you do not yet know which file or section is relevant. Then follow up with wabity.read_file_lines on the best hits to verify exact wording and line ranges before making specific claims.",
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language retrieval query. Write it in terms of the concept, behavior, API, error, or file/topic you need evidence for."
                },
                "top_k": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 20,
                    "description": "How many hits to return. Start small for focused queries; increase only when the first pass is too narrow."
                },
                "min_score": {
                    "type": "number",
                    "minimum": 0.0,
                    "maximum": 1.0,
                    "description": "Minimum similarity score. Lower it only if an initial focused search returns too few relevant hits."
                }
            },
            "required": ["query"]
        }),
    )
}

fn build_function_tool(
    protocol: QuestionAnswerProtocol,
    name: &str,
    description: &str,
    parameters: Value,
) -> Value {
    match protocol {
        QuestionAnswerProtocol::Responses => json!({
            "type": "function",
            "name": name,
            "description": description,
            "strict": true,
            "parameters": parameters,
        }),
        QuestionAnswerProtocol::ChatCompletions => json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "strict": true,
                "parameters": parameters,
            },
        }),
    }
}

fn build_runtime_system_prompt(user_prompt: &str) -> String {
    format!(
        "{user_prompt}\n\nAdditional runtime rules:\n- Tools are available through the request tool list. Use them when the answer depends on repository files, indexed documents, or MCP-connected systems.\n- Do not assume any RAG snippets are preloaded. Call `{RAG_QUERY_TOOL_NAME}` yourself when you need retrieval.\n- Prefer `{RAG_QUERY_TOOL_NAME}` to locate evidence, then `{READ_FILE_TOOL_NAME}` to inspect exact lines.\n- When you rely on tool output, cite the relevant sources inline with [1], [2], ...\n- If the available tools do not provide enough evidence, say that directly.\n- Return Markdown only."
    )
}

fn build_initial_responses_input(
    conversation: &[ExecutionConversationTurn],
    question: &str,
    continue_previous_response: bool,
) -> Vec<Value> {
    if continue_previous_response {
        return vec![json!({
            "role": "user",
            "content": question,
        })];
    }

    let mut input = conversation
        .iter()
        .filter_map(|turn| {
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
        })
        .collect::<Vec<_>>();
    input.push(json!({
        "role": "user",
        "content": question,
    }));
    input
}

fn build_initial_chat_messages(
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

fn build_chat_assistant_tool_call_message(message: &Value) -> Value {
    json!({
        "role": "assistant",
        "content": message.get("content").cloned().unwrap_or(Value::Null),
        "tool_calls": message
            .get("tool_calls")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    })
}

fn normalize_previous_response_id(previous_response_id: Option<&str>) -> Option<String> {
    previous_response_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_continuation_scope(continuation_scope: Option<&str>) -> Option<String> {
    continuation_scope
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn provider_continuation_scope(provider: &LlmProviderConfig, workspace_root: &Path) -> String {
    let workspace = workspace_root.to_string_lossy();
    let scope_seed = format!(
        "rag-answer|{}|{}|{}|{}",
        match provider.protocol {
            LlmProviderProtocol::Responses => "responses",
            LlmProviderProtocol::ChatCompletions => "chat_completions",
        },
        provider.base_url.trim().trim_end_matches('/'),
        provider.model_name(),
        workspace
    );
    format!("{:x}", md5::compute(scope_seed))
}

fn prepare_conversation_state<'a>(
    conversation: &'a [ExecutionConversationTurn],
    conversation_state: Option<&ExecutionConversationState>,
    provider: &LlmProviderConfig,
    workspace_root: &Path,
) -> PreparedConversationState<'a> {
    let expected_scope = provider_continuation_scope(provider, workspace_root);
    let scope_matches = conversation_state
        .and_then(|state| normalize_continuation_scope(state.continuation_scope.as_deref()))
        .is_some_and(|scope| scope == expected_scope);
    let state_has_data = conversation_state
        .map(ExecutionConversationState::has_state)
        .unwrap_or(false);
    let should_reset_context = state_has_data && !scope_matches;

    let mut next_state = if scope_matches {
        conversation_state.cloned().unwrap_or_default()
    } else {
        ExecutionConversationState::default()
    };
    next_state.previous_response_id =
        normalize_previous_response_id(next_state.previous_response_id.as_deref());
    next_state.continuation_scope = Some(expected_scope);

    PreparedConversationState {
        conversation: if should_reset_context {
            &[]
        } else {
            conversation
        },
        state: next_state,
    }
}

fn build_answer_conversation_state(
    response_id: Option<String>,
    provider: &LlmProviderConfig,
    workspace_root: &Path,
    citations: &[ExecutionCitation],
    actions: &[AcpActionEvent],
    tool_calls: &[ExecutionToolCall],
) -> ExecutionConversationState {
    ExecutionConversationState {
        previous_response_id: response_id,
        continuation_scope: Some(provider_continuation_scope(provider, workspace_root)),
        citations: citations.to_vec(),
        actions: actions.to_vec(),
        tool_calls: tool_calls.to_vec(),
    }
}

fn previous_response_id_for_follow_up(
    conversation_state: Option<&ExecutionConversationState>,
    supports_stateful: bool,
) -> Option<String> {
    if !supports_stateful {
        return None;
    }

    conversation_state
        .and_then(|state| normalize_previous_response_id(state.previous_response_id.as_deref()))
}

fn build_tool_output_items(executed_calls: &[ExecutedToolCall]) -> Vec<Value> {
    executed_calls
        .iter()
        .map(|call| {
            json!({
                "type": "function_call_output",
                "call_id": call.call_id,
                "output": call.output,
            })
        })
        .collect()
}

async fn execute_local_tool_call(
    runtime: &QuestionToolRuntime<'_>,
    call: LocalToolCall,
) -> Result<ExecutedToolCall> {
    let executed = match call.name.as_str() {
        READ_FILE_TOOL_NAME => execute_read_file_tool(runtime, &call.arguments).await,
        RAG_QUERY_TOOL_NAME => execute_rag_query_tool(runtime, &call.arguments).await,
        _ => Err(anyhow::anyhow!("未知内置工具: {}", call.name)),
    };

    match executed {
        Ok(mut executed) => {
            executed.call_id = call.call_id;
            Ok(executed)
        }
        Err(error) => Ok(ExecutedToolCall {
            call_id: call.call_id,
            name: call.name.clone(),
            output: serde_json::to_string(&json!({
                "ok": false,
                "error": error.to_string(),
            }))?,
            citations: Vec::new(),
            trace: ExecutionToolCall {
                name: call.name,
                source: "builtin".to_string(),
                status: "error".to_string(),
                summary: error.to_string(),
            },
        }),
    }
}

async fn execute_read_file_tool(
    runtime: &QuestionToolRuntime<'_>,
    arguments: &Value,
) -> Result<ExecutedToolCall> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("read_file_lines.path 不能为空")?;
    let line_start = value_as_usize(arguments, "line_start")?.max(1);
    let line_count = value_as_usize(arguments, "line_count")?.clamp(1, MAX_READ_FILE_LINES);
    let allowed_roots =
        rag::collect_document_access_roots(runtime.workspace_root, runtime.rag_settings);
    let resolved_path = resolve_readable_file_path(path, &allowed_roots)?;
    let metadata = fs::metadata(&resolved_path)
        .await
        .with_context(|| format!("无法读取文件 metadata: {}", resolved_path.display()))?;
    if metadata.len() > MAX_READ_FILE_BYTES {
        bail!(
            "文件过大，read_file_lines 只允许读取不超过 {} KB 的文本文件",
            MAX_READ_FILE_BYTES / 1024
        );
    }

    let text = fs::read_to_string(&resolved_path)
        .await
        .with_context(|| format!("无法读取 UTF-8 文本文件: {}", resolved_path.display()))?;
    let lines = text.lines().collect::<Vec<_>>();
    let start_index = line_start.saturating_sub(1);
    let selected = lines
        .iter()
        .enumerate()
        .skip(start_index)
        .take(line_count)
        .map(|(index, line)| ReadFileToolLine {
            number: index + 1,
            text: (*line).to_string(),
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        bail!(
            "请求的文件范围为空: {}:{}+{}",
            resolved_path.display(),
            line_start,
            line_count
        );
    }

    let line_end = selected
        .last()
        .map(|line| line.number)
        .unwrap_or(line_start);
    let result = ReadFileToolResult {
        absolute_path: resolved_path.to_string_lossy().into_owned(),
        path: display_path(&resolved_path),
        line_start,
        line_end,
        line_count: selected.len(),
        lines: selected.clone(),
    };
    let snippet = compact_snippet(
        &selected
            .iter()
            .map(|line| format!("{}: {}", line.number, line.text))
            .collect::<Vec<_>>()
            .join("\n"),
        CITATION_SNIPPET_MAX_CHARS,
    );

    Ok(ExecutedToolCall {
        call_id: String::new(),
        name: READ_FILE_TOOL_NAME.to_string(),
        output: serde_json::to_string(&result)?,
        citations: vec![ExecutionCitation {
            id: 0,
            absolute_path: result.absolute_path.clone(),
            path: result.path.clone(),
            chunk_index: -1,
            line_start: i32::try_from(result.line_start).unwrap_or(i32::MAX),
            line_end: i32::try_from(result.line_end).unwrap_or(i32::MAX),
            paragraph_line_start: i32::try_from(result.line_start).unwrap_or(i32::MAX),
            heading_path: Vec::new(),
            score: 1.0,
            distance: 0.0,
            snippet,
        }],
        trace: ExecutionToolCall {
            name: READ_FILE_TOOL_NAME.to_string(),
            source: "builtin".to_string(),
            status: "ok".to_string(),
            summary: format!("{}:{}-{}", result.path, result.line_start, result.line_end),
        },
    })
}

async fn execute_rag_query_tool(
    runtime: &QuestionToolRuntime<'_>,
    arguments: &Value,
) -> Result<ExecutedToolCall> {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("rag.query.query 不能为空")?;
    let top_k = arguments
        .get("top_k")
        .map(|_| value_as_usize(arguments, "top_k"))
        .transpose()?
        .unwrap_or(DEFAULT_RAG_TOOL_TOP_K);
    let min_score = arguments
        .get("min_score")
        .and_then(Value::as_f64)
        .map(|value| value.clamp(0.0, 1.0) as f32)
        .unwrap_or(DEFAULT_RAG_TOOL_MIN_SCORE);
    let result = rag_query::search_chunks(
        runtime.data_dir,
        query,
        runtime.rag_settings,
        runtime.llm_settings,
        top_k,
        min_score,
    )
    .await?;
    let citations = result
        .hits
        .iter()
        .map(citation_from_rag_hit)
        .collect::<Vec<_>>();

    Ok(ExecutedToolCall {
        call_id: String::new(),
        name: RAG_QUERY_TOOL_NAME.to_string(),
        output: serde_json::to_string(&result)?,
        citations,
        trace: ExecutionToolCall {
            name: RAG_QUERY_TOOL_NAME.to_string(),
            source: "builtin".to_string(),
            status: if result.pending_indexing {
                "error".to_string()
            } else {
                "ok".to_string()
            },
            summary: if result.pending_indexing {
                format!(
                    "query=`{}` hits={} pending_indexing=true",
                    result.query, result.hit_count
                )
            } else {
                format!("query=`{}` hits={}", result.query, result.hit_count)
            },
        },
    })
}

fn citation_from_rag_hit(hit: &RagSearchHit) -> ExecutionCitation {
    ExecutionCitation {
        id: 0,
        absolute_path: hit.absolute_path.clone(),
        path: hit.path.clone(),
        chunk_index: hit.chunk_index,
        line_start: hit.line_start,
        line_end: hit.line_end,
        paragraph_line_start: hit.paragraph_line_start,
        heading_path: hit.heading_path.clone(),
        score: hit.score,
        distance: hit.distance,
        snippet: compact_snippet(&hit.text, CITATION_SNIPPET_MAX_CHARS),
    }
}

struct ResponsesTurnRequest<'a> {
    client: &'a Client,
    base_url: &'a str,
    api_key: &'a str,
    model: &'a str,
    instructions: &'a str,
    tool_catalog: &'a ToolCatalog,
    previous_response_id: Option<&'a str>,
    input: Value,
}

struct ChatCompletionsTurnRequest<'a> {
    client: &'a Client,
    base_url: &'a str,
    api_key: &'a str,
    model: &'a str,
    tool_catalog: &'a ToolCatalog,
    messages: &'a [Value],
}

async fn request_responses_turn(request_args: ResponsesTurnRequest<'_>) -> Result<Value> {
    let mut request = request_args
        .client
        .post(format!("{}/responses", request_args.base_url));
    if !request_args.api_key.is_empty() {
        request = request.bearer_auth(request_args.api_key);
    }

    let mut body = json!({
        "model": request_args.model,
        "instructions": request_args.instructions,
        "input": request_args.input,
        "tools": request_args.tool_catalog.request_tools,
        "parallel_tool_calls": true,
        "stream": false,
    });
    if let Some(previous_response_id) = request_args.previous_response_id {
        body["previous_response_id"] = Value::String(previous_response_id.to_string());
    }

    let response = request
        .json(&body)
        .send()
        .await
        .context("failed to request question answering from responses API")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read responses API body")?;
    if !status.is_success() {
        let message = extract_provider_error_message(&body);
        bail!("LLM provider responses 请求失败 ({status}): {message}");
    }

    parse_json_or_sse_payload(&body, "question answer JSON from responses")
}

async fn request_chat_completions_turn(
    request_args: ChatCompletionsTurnRequest<'_>,
) -> Result<Value> {
    let mut request = request_args
        .client
        .post(format!("{}/chat/completions", request_args.base_url));
    if !request_args.api_key.is_empty() {
        request = request.bearer_auth(request_args.api_key);
    }

    let response = request
        .json(&json!({
            "model": request_args.model,
            "messages": request_args.messages,
            "tools": request_args.tool_catalog.request_tools,
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "stream": false,
        }))
        .send()
        .await
        .context("failed to request question answering from chat/completions API")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read chat/completions API body")?;
    if !status.is_success() {
        let message = extract_provider_error_message(&body);
        bail!("LLM provider chat/completions 请求失败 ({status}): {message}");
    }

    parse_json_payload(&body, "question answer JSON from chat/completions")
}

fn has_mcp_approval_request(payload: &Value) -> bool {
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

fn extract_local_tool_calls(payload: &Value) -> Result<Vec<LocalToolCall>> {
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

fn extract_chat_completion_message(payload: &Value) -> Result<Value> {
    payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .cloned()
        .context("chat/completions 未返回 assistant message")
}

fn extract_chat_local_tool_calls(message: &Value) -> Result<Vec<LocalToolCall>> {
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

fn parse_tool_arguments(value: &Value) -> Result<Value> {
    if value.is_object() {
        return Ok(value.clone());
    }
    let raw = value
        .as_str()
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .context("tool call arguments 不能为空")?;
    serde_json::from_str(raw).with_context(|| format!("tool call arguments 不是有效 JSON: {raw}"))
}

fn extract_mcp_tool_calls(payload: &Value) -> Vec<McpToolCallTrace> {
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

fn resolve_answer_provider(llm_settings: &LlmSettings) -> Result<&LlmProviderConfig> {
    let provider_id = llm_settings
        .question_answer_provider_id
        .as_deref()
        .context("没有配置问答 LLM，请先在 AI 功能页选择一个条目")?;
    let provider = llm_settings
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .with_context(|| format!("问答 LLM provider 不存在: {provider_id}"))?;
    validate_answer_provider(provider)?;
    Ok(provider)
}

fn validate_answer_provider(provider: &LlmProviderConfig) -> Result<()> {
    if provider.base_url.trim().is_empty() {
        bail!("问答使用的 LLM provider base URL 不能为空");
    }
    if provider.llm_model_name().is_none() {
        bail!("问答使用的 LLM 模型不能为空");
    }

    Ok(())
}

fn question_payload(raw_text: &str) -> &str {
    let trimmed = raw_text.trim_start();
    if trimmed.starts_with('/') {
        return extract_rag_answer_payload(raw_text).unwrap_or("").trim();
    }

    raw_text.trim()
}

fn extract_rag_answer_payload(raw_text: &str) -> Option<&str> {
    extract_prefixed_payload(raw_text, &RAG_ANSWER_COMMAND_ALIASES)
}

fn extract_prefixed_payload<'a>(raw_text: &'a str, aliases: &[&str]) -> Option<&'a str> {
    let trimmed = raw_text.trim_start();

    for alias in aliases {
        let Some(remainder) = trimmed.strip_prefix(alias) else {
            continue;
        };

        if remainder.is_empty() {
            return None;
        }

        let next_character = remainder.chars().next();
        if !matches!(next_character, Some(character) if character.is_whitespace()) {
            continue;
        }

        let payload = remainder.trim();
        return (!payload.is_empty()).then_some(payload);
    }

    for alias in aliases {
        let max_prefix_length = alias.len().min(trimmed.len().saturating_sub(1));
        for prefix_length in (2..=max_prefix_length).rev() {
            let Some(alias_prefix) = alias.get(..prefix_length) else {
                continue;
            };
            let Some(candidate_prefix) = trimmed.get(..prefix_length) else {
                continue;
            };
            if !candidate_prefix.eq_ignore_ascii_case(alias_prefix) {
                continue;
            }

            let Some(remainder) = trimmed.get(prefix_length..) else {
                continue;
            };
            let payload = remainder.trim();
            if payload.is_empty() {
                continue;
            }

            return Some(payload);
        }
    }

    None
}

fn resolve_readable_file_path(path: &str, allowed_roots: &[PathBuf]) -> Result<PathBuf> {
    let candidate = if let Some(remainder) = path.strip_prefix("~/") {
        dirs::home_dir()
            .map(|home| home.join(remainder))
            .context("无法展开 ~/ 路径，因为 HOME 不可用")?
    } else {
        let candidate = PathBuf::from(path);
        if candidate.is_absolute() {
            candidate
        } else {
            allowed_roots
                .first()
                .cloned()
                .context("当前没有可访问的文档根目录")?
                .join(candidate)
        }
    };
    let resolved = candidate
        .canonicalize()
        .with_context(|| format!("无法解析文件路径: {}", candidate.display()))?;
    if !resolved.is_file() {
        bail!("不是可读取的文件: {}", resolved.display());
    }
    if !rag::path_is_within_roots(&resolved, allowed_roots) {
        bail!(
            "文件路径超出允许范围，只能读取当前 workspace 或显式配置的 RAG 目录: {}",
            resolved.display()
        );
    }
    Ok(resolved)
}

fn value_as_usize(arguments: &Value, key: &str) -> Result<usize> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .with_context(|| format!("{key} 必须是正整数"))
}

fn display_path(path: &Path) -> String {
    let Some(home) = dirs::home_dir() else {
        return path.to_string_lossy().into_owned();
    };
    if let Ok(relative) = path.strip_prefix(&home) {
        return format!("~/{}", relative.to_string_lossy());
    }
    path.to_string_lossy().into_owned()
}

fn compact_snippet(text: &str, max_chars: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }

    let mut snippet = compact.chars().take(max_chars).collect::<String>();
    snippet.push_str("...");
    snippet
}

fn json_string_to_pretty_text(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .map(|value| json_value_to_pretty_text(&value))
        .unwrap_or_else(|_| raw.to_string())
}

fn json_value_to_pretty_text(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn deduplicate_and_number_citations(citations: Vec<ExecutionCitation>) -> Vec<ExecutionCitation> {
    let mut seen = HashSet::new();
    citations
        .into_iter()
        .filter(|citation| {
            seen.insert(format!(
                "{}:{}:{}:{}",
                citation.absolute_path,
                citation.chunk_index,
                citation.line_start,
                citation.line_end
            ))
        })
        .enumerate()
        .map(|(index, mut citation)| {
            citation.id = index + 1;
            citation
        })
        .collect()
}

fn name_value_pairs_to_json_object(pairs: &[AcpNameValuePair]) -> Value {
    let mut object = serde_json::Map::new();
    for pair in pairs {
        object.insert(pair.name.clone(), Value::String(pair.value.clone()));
    }
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        alternate_answer_protocol, build_answer_conversation_state, build_initial_responses_input,
        build_round_action, deduplicate_and_number_citations, extract_mcp_tool_calls,
        extract_rag_answer_payload, is_budget_exceeded_error, normalize_previous_response_id,
        prepare_conversation_state, previous_response_id_for_follow_up,
        provider_continuation_scope, question_payload, resolve_readable_file_path,
        should_retry_answer_with_protocol, should_retry_without_response_chain,
        QuestionAnswerProtocol, RAG_QUERY_TOOL_NAME,
    };
    use crate::domain::{
        acp::AcpActionEvent,
        execution::{
            ExecutionCitation, ExecutionConversationRole, ExecutionConversationState,
            ExecutionConversationTurn, ExecutionToolCall,
        },
        settings::{LlmModelType, LlmProviderConfig, LlmProviderProtocol},
    };
    use crate::infrastructure::openai_compatible::{body_preview, parse_json_or_sse_payload};

    fn test_provider() -> LlmProviderConfig {
        LlmProviderConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            model_type: LlmModelType::Llm,
            protocol: LlmProviderProtocol::Responses,
            model: "gpt-test".to_string(),
            ..LlmProviderConfig::default()
        }
    }

    fn temp_test_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("wabity-rag-answer-{label}-{unique}"))
    }

    fn normalize_path(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    #[test]
    fn question_payload_strips_rag_alias() {
        assert_eq!(question_payload("/ask 解释架构"), "解释架构");
        assert_eq!(extract_rag_answer_payload("/qa explain"), Some("explain"));
    }

    #[test]
    fn responses_input_preserves_multi_turn_history_without_response_chain() {
        let input = build_initial_responses_input(
            &[
                ExecutionConversationTurn {
                    role: ExecutionConversationRole::User,
                    content: "first question".to_string(),
                },
                ExecutionConversationTurn {
                    role: ExecutionConversationRole::Assistant,
                    content: "first answer".to_string(),
                },
            ],
            "follow up",
            false,
        );

        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[2]["content"], "follow up");
    }

    #[test]
    fn responses_input_uses_only_latest_question_when_response_chain_exists() {
        let input = build_initial_responses_input(&[], "follow up", true);

        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"], "follow up");
    }

    #[test]
    fn previous_response_id_is_trimmed_and_empty_filtered() {
        assert_eq!(
            normalize_previous_response_id(Some("  resp_123  ")).as_deref(),
            Some("resp_123")
        );
        assert_eq!(normalize_previous_response_id(Some("   ")), None);
        assert_eq!(normalize_previous_response_id(None), None);
    }

    #[test]
    fn follow_up_ignores_previous_response_id_when_stateful_disabled() {
        let state = ExecutionConversationState {
            previous_response_id: Some("resp_123".to_string()),
            ..ExecutionConversationState::default()
        };

        assert_eq!(
            previous_response_id_for_follow_up(Some(&state), false),
            None
        );
        assert_eq!(
            previous_response_id_for_follow_up(Some(&state), true).as_deref(),
            Some("resp_123")
        );
    }

    #[test]
    fn citations_are_deduplicated_and_renumbered() {
        let citations = deduplicate_and_number_citations(vec![
            ExecutionCitation {
                id: 99,
                absolute_path: "/tmp/a.md".to_string(),
                path: "/tmp/a.md".to_string(),
                chunk_index: 1,
                line_start: 10,
                line_end: 12,
                paragraph_line_start: 10,
                heading_path: Vec::new(),
                score: 0.8,
                distance: 0.2,
                snippet: "a".to_string(),
            },
            ExecutionCitation {
                id: 0,
                absolute_path: "/tmp/a.md".to_string(),
                path: "/tmp/a.md".to_string(),
                chunk_index: 1,
                line_start: 10,
                line_end: 12,
                paragraph_line_start: 10,
                heading_path: Vec::new(),
                score: 0.8,
                distance: 0.2,
                snippet: "a".to_string(),
            },
        ]);

        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0].id, 1);
    }

    #[test]
    fn prepare_conversation_state_keeps_matching_scope_and_carried_evidence() {
        let provider = test_provider();
        let workspace_root = Path::new("/workspace");
        let carried_state = ExecutionConversationState {
            previous_response_id: Some("resp_123".to_string()),
            continuation_scope: Some(provider_continuation_scope(&provider, workspace_root)),
            citations: vec![ExecutionCitation {
                id: 1,
                absolute_path: "/workspace/a.md".to_string(),
                path: "~/a.md".to_string(),
                chunk_index: 1,
                line_start: 1,
                line_end: 3,
                paragraph_line_start: 1,
                heading_path: Vec::new(),
                score: 0.9,
                distance: 0.1,
                snippet: "alpha".to_string(),
            }],
            actions: vec![AcpActionEvent {
                kind: "info".to_string(),
                title: "第 1 步".to_string(),
                correlation_id: None,
                detail: Some("ok".to_string()),
            }],
            tool_calls: vec![ExecutionToolCall {
                name: "wabity.rag.query".to_string(),
                source: "builtin".to_string(),
                status: "ok".to_string(),
                summary: "hits=1".to_string(),
            }],
        };
        let conversation = [ExecutionConversationTurn {
            role: ExecutionConversationRole::User,
            content: "hello".to_string(),
        }];

        let prepared = prepare_conversation_state(
            &conversation,
            Some(&carried_state),
            &provider,
            workspace_root,
        );

        assert_eq!(prepared.conversation.len(), 1);
        assert_eq!(
            prepared.state.previous_response_id.as_deref(),
            Some("resp_123")
        );
        assert_eq!(prepared.state.citations.len(), 1);
        assert_eq!(prepared.state.actions.len(), 1);
        assert_eq!(prepared.state.tool_calls.len(), 1);
    }

    #[test]
    fn prepare_conversation_state_resets_mismatched_scope() {
        let provider = test_provider();
        let workspace_root = Path::new("/workspace");
        let carried_state = ExecutionConversationState {
            previous_response_id: Some("resp_123".to_string()),
            continuation_scope: Some("stale-scope".to_string()),
            citations: vec![ExecutionCitation {
                id: 1,
                absolute_path: "/workspace/a.md".to_string(),
                path: "~/a.md".to_string(),
                chunk_index: 1,
                line_start: 1,
                line_end: 3,
                paragraph_line_start: 1,
                heading_path: Vec::new(),
                score: 0.9,
                distance: 0.1,
                snippet: "alpha".to_string(),
            }],
            actions: Vec::new(),
            tool_calls: Vec::new(),
        };
        let conversation = [ExecutionConversationTurn {
            role: ExecutionConversationRole::User,
            content: "stale".to_string(),
        }];

        let prepared = prepare_conversation_state(
            &conversation,
            Some(&carried_state),
            &provider,
            workspace_root,
        );
        let expected_scope = provider_continuation_scope(&provider, workspace_root);

        assert!(prepared.conversation.is_empty());
        assert_eq!(prepared.state.previous_response_id, None);
        assert!(prepared.state.citations.is_empty());
        assert_eq!(
            prepared.state.continuation_scope.as_deref(),
            Some(expected_scope.as_str())
        );
    }

    #[test]
    fn answer_conversation_state_captures_scope_and_evidence() {
        let provider = test_provider();
        let workspace_root = Path::new("/workspace");
        let state = build_answer_conversation_state(
            Some("resp_456".to_string()),
            &provider,
            workspace_root,
            &[ExecutionCitation {
                id: 1,
                absolute_path: "/workspace/a.md".to_string(),
                path: "~/a.md".to_string(),
                chunk_index: 1,
                line_start: 1,
                line_end: 3,
                paragraph_line_start: 1,
                heading_path: Vec::new(),
                score: 0.9,
                distance: 0.1,
                snippet: "alpha".to_string(),
            }],
            &[AcpActionEvent {
                kind: "info".to_string(),
                title: "第 1 步".to_string(),
                correlation_id: None,
                detail: None,
            }],
            &[ExecutionToolCall {
                name: "wabity.rag.query".to_string(),
                source: "builtin".to_string(),
                status: "ok".to_string(),
                summary: "hits=1".to_string(),
            }],
        );
        let expected_scope = provider_continuation_scope(&provider, workspace_root);

        assert_eq!(state.previous_response_id.as_deref(), Some("resp_456"));
        assert_eq!(state.citations.len(), 1);
        assert_eq!(state.actions.len(), 1);
        assert_eq!(state.tool_calls.len(), 1);
        assert_eq!(
            state.continuation_scope.as_deref(),
            Some(expected_scope.as_str())
        );
    }

    #[test]
    fn tool_names_stay_stable() {
        assert_eq!(RAG_QUERY_TOOL_NAME, "wabity.rag.query");
    }

    #[test]
    fn read_file_tool_rejects_paths_outside_allowed_roots() {
        let root = temp_test_root("allowed-roots");
        let workspace_root = root.join("workspace");
        let outside_root = root.join("outside");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::create_dir_all(&outside_root).expect("create outside root");

        let workspace_root = workspace_root
            .canonicalize()
            .expect("canonicalize workspace root");
        let outside_file = outside_root.join("secret.txt");
        std::fs::write(&outside_file, "top secret").expect("write outside file");
        let outside_file = outside_file
            .canonicalize()
            .expect("canonicalize outside file");

        let error = resolve_readable_file_path(
            outside_file.to_string_lossy().as_ref(),
            std::slice::from_ref(&workspace_root),
        )
        .expect_err("outside file should be rejected");

        assert!(error.to_string().contains("超出允许范围"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_file_tool_accepts_relative_paths_inside_first_allowed_root() {
        let root = temp_test_root("relative-path");
        let workspace_root = root.join("workspace");
        std::fs::create_dir_all(workspace_root.join("docs")).expect("create workspace docs");
        let file_path = workspace_root.join("docs").join("note.md");
        std::fs::write(&file_path, "hello").expect("write workspace file");
        let workspace_root = workspace_root
            .canonicalize()
            .expect("canonicalize workspace root");
        let file_path = file_path
            .canonicalize()
            .expect("canonicalize workspace file");

        let resolved =
            resolve_readable_file_path("docs/note.md", std::slice::from_ref(&workspace_root))
                .expect("relative workspace file should resolve");

        assert_eq!(normalize_path(&resolved), normalize_path(&file_path));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_responses_payload_accepts_plain_json() {
        let payload = parse_json_or_sse_payload(
            r#"{"id":"resp_123","output":[{"type":"message","content":[{"type":"output_text","text":"ok"}]}]}"#,
            "question answer JSON from responses",
        )
        .expect("plain JSON response should parse");

        assert_eq!(payload["id"], "resp_123");
    }

    #[test]
    fn parse_responses_payload_accepts_sse_completed_event() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "event: response.created\n",
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_ignore\"}}\n\n",
                "event: response.completed\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"ok\"}]}]}}\n\n",
                "data: [DONE]\n",
            ),
            "question answer JSON from responses",
        )
        .expect("SSE completed response should parse");

        assert_eq!(payload["id"], "resp_123");
        assert_eq!(payload["output"][0]["type"], "message");
    }

    #[test]
    fn body_preview_collapses_whitespace_and_truncates() {
        let preview = body_preview(&format!("  a\n b\t{}  ", "x".repeat(300)));

        assert!(preview.starts_with("a b"));
        assert!(preview.ends_with("..."));
        assert!(preview.len() <= 243);
    }

    #[test]
    fn build_round_action_marks_multi_step_progress() {
        let action = build_round_action(2, 3);

        assert_eq!(action.kind, "info");
        assert_eq!(action.title, "第 2 步");
        assert_eq!(action.detail.as_deref(), Some("模型发起了 3 个工具调用"));
    }

    #[test]
    fn extract_mcp_tool_calls_preserves_input_and_output_details() {
        let payload = serde_json::json!({
            "output": [
                {
                    "type": "mcp_call",
                    "id": "mcp_1",
                    "server_label": "docs",
                    "name": "search",
                    "arguments": {
                        "query": "rag"
                    },
                    "output": {
                        "hits": 2
                    }
                }
            ]
        });

        let calls = extract_mcp_tool_calls(&payload);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_id.as_deref(), Some("mcp_1"));
        assert_eq!(calls[0].trace.name, "docs::search");
        assert_eq!(
            calls[0].input_detail.as_deref(),
            Some("{\n  \"query\": \"rag\"\n}")
        );
        assert_eq!(
            calls[0].output_detail.as_deref(),
            Some("{\n  \"hits\": 2\n}")
        );
    }

    #[test]
    fn protocol_retry_switches_to_other_endpoint() {
        assert_eq!(
            alternate_answer_protocol(QuestionAnswerProtocol::Responses),
            QuestionAnswerProtocol::ChatCompletions
        );
        assert_eq!(
            alternate_answer_protocol(QuestionAnswerProtocol::ChatCompletions),
            QuestionAnswerProtocol::Responses
        );
    }

    #[test]
    fn protocol_retry_detects_chat_only_route_mismatch() {
        let error = anyhow::anyhow!(
            "LLM provider responses 请求失败 (400 Bad Request): route protocol chat does not support stateless responses requests"
        );

        assert!(should_retry_answer_with_protocol(
            QuestionAnswerProtocol::Responses,
            &error
        ));
        assert!(!should_retry_answer_with_protocol(
            QuestionAnswerProtocol::ChatCompletions,
            &error
        ));
    }

    #[test]
    fn protocol_retry_detects_responses_only_route_mismatch() {
        let error = anyhow::anyhow!(
            "LLM provider chat/completions 请求失败 (400 Bad Request): route protocol responses does not support chat/completions requests"
        );

        assert!(should_retry_answer_with_protocol(
            QuestionAnswerProtocol::ChatCompletions,
            &error
        ));
        assert!(!should_retry_answer_with_protocol(
            QuestionAnswerProtocol::Responses,
            &error
        ));
    }

    #[test]
    fn protocol_retry_detects_missing_responses_endpoint_by_status() {
        let error = anyhow::anyhow!("LLM provider responses 请求失败 (404 Not Found): not found");

        assert!(should_retry_answer_with_protocol(
            QuestionAnswerProtocol::Responses,
            &error
        ));
    }

    #[test]
    fn budget_retry_detects_provider_budget_error() {
        let error = anyhow::anyhow!(
            "LLM provider responses 请求失败 (400 Bad Request): Budget has been exceeded! Current cost: 162.85522300000002, Max budget: 150.0"
        );

        assert!(is_budget_exceeded_error(&error));
    }

    #[test]
    fn budget_retry_only_resets_stateful_chain_on_first_turn() {
        let error = anyhow::anyhow!(
            "LLM provider responses 请求失败 (400 Bad Request): Budget has been exceeded! Current cost: 162.85522300000002, Max budget: 150.0"
        );

        assert!(should_retry_without_response_chain(1, true, &error));
        assert!(!should_retry_without_response_chain(2, true, &error));
        assert!(!should_retry_without_response_chain(1, false, &error));
    }
}
