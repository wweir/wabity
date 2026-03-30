use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use futures::future::join_all;
use reqwest::Client as HttpClient;
use serde_json::{json, Value};

mod conversation_state;
mod parsing;
mod protocol_chat;
mod protocol_responses;
mod result;
mod tool_catalog;
mod tool_execute;

use self::{
    conversation_state::{
        build_answer_conversation_state, prepare_conversation_state,
        previous_response_id_for_follow_up,
    },
    parsing::{
        json_string_to_pretty_text, json_value_to_pretty_text, question_explicitly_requests_open,
        question_payload,
    },
    protocol_chat::{
        build_chat_assistant_tool_call_message, build_initial_chat_messages,
        extract_chat_completion_message, extract_chat_local_tool_calls,
        request_chat_completions_turn,
    },
    protocol_responses::{
        extract_local_tool_calls, extract_mcp_tool_calls, has_mcp_approval_request,
        request_responses_turn, should_retry_without_all_tools, should_retry_without_mcp_tools,
        should_retry_without_response_chain,
    },
    result::{build_execution_result, deduplicate_and_number_citations},
    tool_catalog::{
        build_tool_catalog, load_cached_responses_tool_compatibility,
        responses_tool_compatibility_cache_key, store_cached_responses_tool_compatibility,
        tool_catalog_without_all_tools, tool_catalog_without_mcp_tools,
    },
    tool_execute::execute_local_tool_call,
};

use crate::{
    domain::{
        acp::{AcpActionEvent, AcpMcpServerConfig},
        execution::{
            ExecutionCitation, ExecutionConversationRole, ExecutionConversationState,
            ExecutionConversationTurn, ExecutionProgressEvent, ExecutionResult, ExecutionToolCall,
        },
        settings::{
            LlmProviderConfig, LlmProviderProtocol, LlmSettings, PromptsSettings, RagSettings,
        },
    },
    infrastructure::openai_compatible::{
        describe_chat_completions_response_issue, extract_chat_completions_message_parts,
        extract_responses_text, normalize_base_url,
    },
};

const RAG_ANSWER_COMMAND_ALIASES: [&str; 3] = ["/ask", "/qa", "/docs"];
const READ_FILE_TOOL_NAME: &str = "wabity.read_file_lines";
const READ_DOCUMENT_EXCERPT_TOOL_NAME: &str = "wabity.read_document_excerpt";
const RAG_QUERY_TOOL_NAME: &str = "wabity.rag.query";
const OPEN_TARGET_TOOL_NAME: &str = "wabity.system.open";
const MAX_TOOL_ROUNDS: usize = 8;
const MAX_READ_FILE_LINES: usize = 240;
const MAX_READ_FILE_BYTES: u64 = 512 * 1024;
const DEFAULT_RAG_TOOL_TOP_K: usize = 6;
const DEFAULT_RAG_TOOL_MIN_SCORE: f32 = 0.35;
const CITATION_SNIPPET_MAX_CHARS: usize = 240;
static RESPONSES_TOOL_COMPATIBILITY_CACHE: OnceLock<
    Mutex<HashMap<String, ResponsesToolCompatibilityMode>>,
> = OnceLock::new();

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

#[derive(Clone)]
struct QuestionToolRuntime<'a> {
    data_dir: &'a Path,
    workspace_root: &'a Path,
    rag_settings: &'a RagSettings,
    llm_settings: &'a LlmSettings,
    allow_open_target: bool,
    progress_event_tx: Option<Arc<dyn Fn(ExecutionProgressEvent) + Send + Sync>>,
}

#[derive(Debug, Clone)]
struct HostSystemContext {
    os_name: String,
    os_version: Option<String>,
    package_managers: Vec<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ResponsesToolCompatibilityMode {
    Full,
    NoMcp,
    NoTools,
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
    pub progress_event_tx: Option<Arc<dyn Fn(ExecutionProgressEvent) + Send + Sync>>,
}

pub async fn answer_question(request: QuestionAnswerRequest<'_>) -> Result<ExecutionResult> {
    let question = question_payload(request.raw_text, &RAG_ANSWER_COMMAND_ALIASES);
    if question.is_empty() {
        bail!("请输入要提问的内容");
    }

    emit_question_progress(
        request.progress_event_tx.as_ref(),
        "文档问答 · 正在检查模型与工具配置",
    );

    let provider = resolve_answer_provider(request.llm_settings)?;
    let client = HttpClient::builder()
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
        allow_open_target: question_explicitly_requests_open(question),
        progress_event_tx: request.progress_event_tx.clone(),
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
    let (answer, protocol, tool_catalog) = request_answer(initial_protocol, &execution).await?;
    let response_id = answer.response_id.clone();
    let citations = deduplicate_and_number_citations(answer.citations.clone());
    let conversation_state = build_answer_conversation_state(
        response_id,
        provider,
        request.workspace_root,
        &citations,
        &answer.actions,
        &answer.tool_calls,
    );

    build_execution_result(
        question,
        protocol_label(protocol, provider),
        answer,
        conversation_state,
        tool_catalog.available_names,
        tool_catalog.skipped_mcp_servers,
    )
}

struct QuestionAnswerExecutionContext<'a> {
    provider: &'a LlmProviderConfig,
    client: &'a HttpClient,
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
    emit_question_progress(
        context.runtime.progress_event_tx.as_ref(),
        match protocol {
            QuestionAnswerProtocol::Responses => {
                "文档问答 · 已建立 responses 链路，正在等待模型规划"
            }
            QuestionAnswerProtocol::ChatCompletions => {
                "文档问答 · 已建立 chat/completions 链路，正在等待模型规划"
            }
        },
    );
    let (answer, effective_tool_catalog) = match protocol {
        QuestionAnswerProtocol::Responses => {
            let (answer, effective_tool_catalog) = answer_with_responses(ResponsesAnswerRequest {
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
            .await?;
            (answer, effective_tool_catalog)
        }
        QuestionAnswerProtocol::ChatCompletions => (
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
            .await?,
            tool_catalog.clone(),
        ),
    };

    Ok((answer, protocol, effective_tool_catalog))
}

struct AnswerOutcome {
    final_answer: String,
    reasoning: Option<String>,
    response_id: Option<String>,
    citations: Vec<ExecutionCitation>,
    tool_calls: Vec<ExecutionToolCall>,
    actions: Vec<AcpActionEvent>,
}

struct ResponsesAnswerRequest<'a> {
    client: &'a HttpClient,
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
    client: &'a HttpClient,
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

async fn answer_with_responses(
    request: ResponsesAnswerRequest<'_>,
) -> Result<(AnswerOutcome, ToolCatalog)> {
    let initial_previous_response_id =
        previous_response_id_for_follow_up(request.conversation_state, request.supports_stateful);
    let continue_previous_response = initial_previous_response_id.is_some();
    let mut previous_response_id = initial_previous_response_id;
    let mcp_free_tool_catalog = tool_catalog_without_mcp_tools(request.tool_catalog);
    let tool_free_tool_catalog = tool_catalog_without_all_tools(request.tool_catalog);
    let compatibility_cache_key =
        responses_tool_compatibility_cache_key(request.base_url, request.model);
    let mut compatibility_mode = load_cached_responses_tool_compatibility(&compatibility_cache_key);
    let mut disabled_mcp_tools_for_compat = matches!(
        compatibility_mode,
        ResponsesToolCompatibilityMode::NoMcp | ResponsesToolCompatibilityMode::NoTools
    );
    let mut disabled_all_tools_for_compat =
        matches!(compatibility_mode, ResponsesToolCompatibilityMode::NoTools);
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
        emit_question_progress(
            request.runtime.progress_event_tx.as_ref(),
            if round == 1 {
                "文档问答 · 正在等待模型首轮响应"
            } else {
                "文档问答 · 正在等待模型继续分析"
            },
        );
        let active_tool_catalog = if disabled_all_tools_for_compat {
            &tool_free_tool_catalog
        } else if disabled_mcp_tools_for_compat {
            &mcp_free_tool_catalog
        } else {
            request.tool_catalog
        };
        let response = match request_responses_turn(ResponsesTurnRequest {
            client: request.client,
            base_url: request.base_url,
            api_key: request.api_key,
            model: request.model,
            instructions: request.system_prompt,
            tool_catalog: active_tool_catalog,
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
            Err(error)
                if should_retry_without_mcp_tools(
                    disabled_all_tools_for_compat,
                    disabled_mcp_tools_for_compat,
                    active_tool_catalog,
                    &error,
                ) =>
            {
                disabled_mcp_tools_for_compat = true;
                compatibility_mode = ResponsesToolCompatibilityMode::NoMcp;
                store_cached_responses_tool_compatibility(
                    &compatibility_cache_key,
                    compatibility_mode,
                );
                emit_question_progress(
                    request.runtime.progress_event_tx.as_ref(),
                    "文档问答 · 当前 provider 不兼容 MCP tools，已回退到内置工具重试",
                );
                round = round.saturating_sub(1);
                continue;
            }
            Err(error)
                if should_retry_without_all_tools(
                    disabled_all_tools_for_compat,
                    active_tool_catalog,
                    &error,
                ) =>
            {
                disabled_all_tools_for_compat = true;
                disabled_mcp_tools_for_compat = true;
                compatibility_mode = ResponsesToolCompatibilityMode::NoTools;
                store_cached_responses_tool_compatibility(
                    &compatibility_cache_key,
                    compatibility_mode,
                );
                emit_question_progress(
                    request.runtime.progress_event_tx.as_ref(),
                    "文档问答 · 当前 provider 不兼容 function tools，已回退到无工具请求",
                );
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

        if !mcp_calls.is_empty() {
            emit_question_progress(
                request.runtime.progress_event_tx.as_ref(),
                "文档问答 · 模型正在调用外部工具",
            );
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
            emit_question_progress(
                request.runtime.progress_event_tx.as_ref(),
                "文档问答 · 工具结果已收敛，正在整理最终回答",
            );
            break extract_responses_text(&response)
                .context("LLM provider responses 未返回可识别的回答")?;
        }

        if tool_calls.len() + local_calls.len() > MAX_TOOL_ROUNDS {
            bail!("问答工具调用轮数超过限制，模型没有稳定收敛");
        }

        emit_question_progress(
            request.runtime.progress_event_tx.as_ref(),
            summarize_local_tool_progress(&local_calls),
        );

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

    let effective_tool_catalog = if disabled_all_tools_for_compat {
        tool_free_tool_catalog
    } else if disabled_mcp_tools_for_compat {
        mcp_free_tool_catalog
    } else {
        request.tool_catalog.clone()
    };

    Ok((
        AnswerOutcome {
            final_answer,
            reasoning: None,
            response_id: previous_response_id,
            citations,
            tool_calls,
            actions,
        },
        effective_tool_catalog,
    ))
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

    let (final_answer, reasoning) = loop {
        round += 1;
        emit_question_progress(
            request.runtime.progress_event_tx.as_ref(),
            if round == 1 {
                "文档问答 · 正在等待模型首轮响应"
            } else {
                "文档问答 · 正在等待模型继续分析"
            },
        );
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
            emit_question_progress(
                request.runtime.progress_event_tx.as_ref(),
                "文档问答 · 工具结果已收敛，正在整理最终回答",
            );
            let message_parts =
                extract_chat_completions_message_parts(&response).with_context(|| {
                    format!(
                        "LLM provider chat/completions 未返回可识别的回答: {}",
                        describe_chat_completions_response_issue(&response)
                    )
                })?;
            let final_answer = message_parts
                .content
                .clone()
                .or_else(|| message_parts.reasoning.clone())
                .with_context(|| {
                    format!(
                        "LLM provider chat/completions 未返回可识别的回答: {}",
                        describe_chat_completions_response_issue(&response)
                    )
                })?;
            let reasoning = if message_parts.content.is_some() {
                message_parts
                    .reasoning
                    .filter(|candidate| candidate.trim() != final_answer.trim())
            } else {
                None
            };
            break (final_answer, reasoning);
        }

        if tool_calls.len() + local_calls.len() > MAX_TOOL_ROUNDS {
            bail!("问答工具调用轮数超过限制，模型没有稳定收敛");
        }

        emit_question_progress(
            request.runtime.progress_event_tx.as_ref(),
            summarize_local_tool_progress(&local_calls),
        );

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
        reasoning,
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

fn emit_question_progress(
    progress_event_tx: Option<&Arc<dyn Fn(ExecutionProgressEvent) + Send + Sync>>,
    status_text: &str,
) {
    let Some(progress_event_tx) = progress_event_tx else {
        return;
    };

    progress_event_tx(ExecutionProgressEvent {
        action_id: "rag_answer".to_string(),
        status_text: status_text.to_string(),
    });
}

fn summarize_local_tool_progress(local_calls: &[LocalToolCall]) -> &'static str {
    let rag_query_count = local_calls
        .iter()
        .filter(|call| call.name == RAG_QUERY_TOOL_NAME)
        .count();
    let read_file_count = local_calls
        .iter()
        .filter(|call| call.name == READ_FILE_TOOL_NAME)
        .count();
    let read_document_count = local_calls
        .iter()
        .filter(|call| call.name == READ_DOCUMENT_EXCERPT_TOOL_NAME)
        .count();
    let open_target_count = local_calls
        .iter()
        .filter(|call| call.name == OPEN_TARGET_TOOL_NAME)
        .count();

    match (
        rag_query_count > 0,
        read_file_count > 0,
        read_document_count > 0,
        open_target_count > 0,
    ) {
        (true, true, _, _) | (true, _, true, _) => "文档问答 · 正在检索索引并读取证据",
        (true, false, false, true) => "文档问答 · 正在检索并打开目标",
        (true, false, false, false) => "文档问答 · 正在检索本地文档索引",
        (false, true, _, true) | (false, _, true, true) => "文档问答 · 正在读取证据并打开目标",
        (false, true, _, false) | (false, _, true, false) => "文档问答 · 正在读取证据",
        (false, false, false, true) => "文档问答 · 正在打开目标",
        (false, false, false, false) => "文档问答 · 正在执行模型请求的工具",
    }
}

fn answer_protocol(provider: &LlmProviderConfig) -> QuestionAnswerProtocol {
    match provider.protocol {
        LlmProviderProtocol::Responses => QuestionAnswerProtocol::Responses,
        LlmProviderProtocol::ChatCompletions => QuestionAnswerProtocol::ChatCompletions,
    }
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

fn build_runtime_system_prompt(user_prompt: &str) -> String {
    format!(
        "{user_prompt}\n\nAdditional runtime rules:\n- Tools are available through the request tool list. Use them when the answer depends on repository files, indexed documents, MCP-connected systems, or when the user explicitly asks you to open a target.\n- Do not assume any RAG snippets are preloaded. Call `{RAG_QUERY_TOOL_NAME}` yourself when you need retrieval.\n- Prefer `{RAG_QUERY_TOOL_NAME}` to locate evidence, then use `{READ_FILE_TOOL_NAME}` for text files or `{READ_DOCUMENT_EXCERPT_TOOL_NAME}` for extracted documents such as PDFs.\n- `{OPEN_TARGET_TOOL_NAME}` has side effects. Only call it when the current user request explicitly asks you to open a link, file, or directory. Do not call it proactively.\n- When you rely on tool output, cite the relevant sources inline with [1], [2], ...\n- If the available tools do not provide enough evidence, say that directly.\n- Return Markdown only."
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
            "content": build_responses_text_content(question),
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
                "content": build_responses_text_content(content),
            }))
        })
        .collect::<Vec<_>>();
    input.push(json!({
        "role": "user",
        "content": build_responses_text_content(question),
    }));
    input
}

fn build_responses_text_content(text: &str) -> Vec<Value> {
    vec![json!({
        "type": "input_text",
        "text": text,
    })]
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

struct ResponsesTurnRequest<'a> {
    client: &'a HttpClient,
    base_url: &'a str,
    api_key: &'a str,
    model: &'a str,
    instructions: &'a str,
    tool_catalog: &'a ToolCatalog,
    previous_response_id: Option<&'a str>,
    input: Value,
}

struct ChatCompletionsTurnRequest<'a> {
    client: &'a HttpClient,
    base_url: &'a str,
    api_key: &'a str,
    model: &'a str,
    tool_catalog: &'a ToolCatalog,
    messages: &'a [Value],
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

#[cfg(test)]
fn extract_rag_answer_payload(raw_text: &str) -> Option<&str> {
    crate::services::command_prefix::extract_prefixed_payload(raw_text, &RAG_ANSWER_COMMAND_ALIASES)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Cursor, Write},
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        build_initial_responses_input, build_round_action, extract_rag_answer_payload,
        summarize_local_tool_progress, HostSystemContext, LocalToolCall, QuestionAnswerProtocol,
        QuestionToolRuntime, ResponsesToolCompatibilityMode, OPEN_TARGET_TOOL_NAME,
        RAG_ANSWER_COMMAND_ALIASES, RAG_QUERY_TOOL_NAME, READ_DOCUMENT_EXCERPT_TOOL_NAME,
        READ_FILE_TOOL_NAME,
    };
    use super::{
        conversation_state::{
            build_answer_conversation_state, normalize_previous_response_id,
            prepare_conversation_state, previous_response_id_for_follow_up,
            provider_continuation_scope,
        },
        parsing::{question_explicitly_requests_open, question_payload},
        protocol_responses::{
            extract_mcp_tool_calls, is_budget_exceeded_error, should_retry_without_all_tools,
            should_retry_without_mcp_tools, should_retry_without_response_chain,
        },
        result::deduplicate_and_number_citations,
        tool_catalog::{
            build_tool_catalog, load_cached_responses_tool_compatibility,
            open_target_tool_description, responses_tool_compatibility_cache_key,
            store_cached_responses_tool_compatibility, tool_catalog_without_all_tools,
            tool_catalog_without_mcp_tools,
        },
        tool_execute::{
            execute_open_target_tool, execute_read_file_tool, resolve_readable_file_path,
        },
    };
    use crate::domain::{
        acp::{AcpActionEvent, AcpMcpServerConfig, AcpMcpServerHttpConfig, AcpNameValuePair},
        execution::{
            ExecutionCitation, ExecutionConversationRole, ExecutionConversationState,
            ExecutionConversationTurn, ExecutionToolCall,
        },
        settings::{LlmModelType, LlmProviderConfig, LlmProviderProtocol},
        settings::{LlmSettings, RagSettings},
    };
    use crate::infrastructure::openai_compatible::{body_preview, parse_json_or_sse_payload};
    use zip::{write::SimpleFileOptions, ZipWriter};

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

    fn test_citation(path: &str) -> ExecutionCitation {
        ExecutionCitation {
            id: 1,
            absolute_path: path.to_string(),
            path: path.to_string(),
            document_kind: "markdown".to_string(),
            chunk_index: 1,
            line_start: Some(1),
            line_end: Some(3),
            paragraph_line_start: Some(1),
            page_start: None,
            page_end: None,
            heading_path: Vec::new(),
            anchor_label: None,
            score: 0.9,
            distance: 0.1,
            snippet: "alpha".to_string(),
        }
    }

    fn normalize_path(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn build_test_docx(document_xml: &str) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default();
        writer
            .start_file("word/document.xml", options)
            .expect("start document.xml");
        writer
            .write_all(document_xml.as_bytes())
            .expect("write document.xml");
        writer.finish().expect("finish docx writer").into_inner()
    }

    #[test]
    fn question_payload_strips_rag_alias() {
        assert_eq!(
            question_payload("/ask 解释架构", &RAG_ANSWER_COMMAND_ALIASES),
            "解释架构"
        );
        assert_eq!(extract_rag_answer_payload("/qa explain"), Some("explain"));
    }

    #[test]
    fn question_explicitly_requests_open_detects_open_intent() {
        assert!(question_explicitly_requests_open("打开 README.md"));
        assert!(question_explicitly_requests_open("open https://tauri.app"));
        assert!(!question_explicitly_requests_open(
            "解释 README.md 在做什么"
        ));
    }

    #[test]
    fn open_target_tool_description_embeds_host_context() {
        let description = open_target_tool_description(&HostSystemContext {
            os_name: "macOS".to_string(),
            os_version: Some("15.3".to_string()),
            package_managers: vec!["brew".to_string(), "cargo".to_string(), "pnpm".to_string()],
        });

        assert!(description.contains("OS=macOS 15.3"));
        assert!(description.contains("package managers on PATH=brew, cargo, pnpm"));
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
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "first question");
        assert_eq!(input[1]["content"][0]["type"], "input_text");
        assert_eq!(input[1]["content"][0]["text"], "first answer");
        assert_eq!(input[2]["content"][0]["type"], "input_text");
        assert_eq!(input[2]["content"][0]["text"], "follow up");
    }

    #[test]
    fn responses_input_uses_only_latest_question_when_response_chain_exists() {
        let input = build_initial_responses_input(&[], "follow up", true);

        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "follow up");
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
                document_kind: "markdown".to_string(),
                chunk_index: 1,
                line_start: Some(10),
                line_end: Some(12),
                paragraph_line_start: Some(10),
                page_start: None,
                page_end: None,
                heading_path: Vec::new(),
                anchor_label: None,
                score: 0.8,
                distance: 0.2,
                snippet: "a".to_string(),
            },
            ExecutionCitation {
                id: 0,
                absolute_path: "/tmp/a.md".to_string(),
                path: "/tmp/a.md".to_string(),
                document_kind: "markdown".to_string(),
                chunk_index: 1,
                line_start: Some(10),
                line_end: Some(12),
                paragraph_line_start: Some(10),
                page_start: None,
                page_end: None,
                heading_path: Vec::new(),
                anchor_label: None,
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
            citations: vec![test_citation("~/a.md")],
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
            citations: vec![test_citation("~/a.md")],
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
            &[test_citation("~/a.md")],
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
        assert_eq!(OPEN_TARGET_TOOL_NAME, "wabity.system.open");
    }

    #[test]
    fn summarize_local_tool_progress_reports_rag_query_only() {
        let progress = summarize_local_tool_progress(&[LocalToolCall {
            call_id: "call_1".to_string(),
            name: RAG_QUERY_TOOL_NAME.to_string(),
            arguments: serde_json::json!({}),
        }]);

        assert_eq!(progress, "文档问答 · 正在检索本地文档索引");
    }

    #[test]
    fn summarize_local_tool_progress_reports_combined_lookup() {
        let progress = summarize_local_tool_progress(&[
            LocalToolCall {
                call_id: "call_1".to_string(),
                name: RAG_QUERY_TOOL_NAME.to_string(),
                arguments: serde_json::json!({}),
            },
            LocalToolCall {
                call_id: "call_2".to_string(),
                name: READ_FILE_TOOL_NAME.to_string(),
                arguments: serde_json::json!({}),
            },
        ]);

        assert_eq!(progress, "文档问答 · 正在检索索引并读取证据");
    }

    #[test]
    fn summarize_local_tool_progress_reports_open_target_only() {
        let progress = summarize_local_tool_progress(&[LocalToolCall {
            call_id: "call_1".to_string(),
            name: OPEN_TARGET_TOOL_NAME.to_string(),
            arguments: serde_json::json!({}),
        }]);

        assert_eq!(progress, "文档问答 · 正在打开目标");
    }

    #[test]
    fn tool_catalog_without_mcp_tools_keeps_builtin_tools_only() {
        let tool_catalog = build_tool_catalog(
            &[AcpMcpServerConfig::Http(AcpMcpServerHttpConfig {
                name: "WebMCP".to_string(),
                url: "https://example.com/mcp".to_string(),
                headers: vec![AcpNameValuePair {
                    name: "Authorization".to_string(),
                    value: "Bearer token".to_string(),
                }],
            })],
            QuestionAnswerProtocol::Responses,
        );

        let filtered = tool_catalog_without_mcp_tools(&tool_catalog);

        assert_eq!(filtered.request_tools.len(), 4);
        assert_eq!(
            filtered.available_names,
            vec![
                READ_FILE_TOOL_NAME.to_string(),
                READ_DOCUMENT_EXCERPT_TOOL_NAME.to_string(),
                RAG_QUERY_TOOL_NAME.to_string(),
                OPEN_TARGET_TOOL_NAME.to_string(),
            ]
        );
        assert_eq!(filtered.skipped_mcp_servers, vec!["WebMCP".to_string()]);
    }

    #[test]
    fn tool_catalog_without_all_tools_strips_function_tools_too() {
        let tool_catalog = build_tool_catalog(
            &[AcpMcpServerConfig::Http(AcpMcpServerHttpConfig {
                name: "WebMCP".to_string(),
                url: "https://example.com/mcp".to_string(),
                headers: Vec::new(),
            })],
            QuestionAnswerProtocol::Responses,
        );

        let filtered = tool_catalog_without_all_tools(&tool_catalog);

        assert!(filtered.request_tools.is_empty());
        assert!(filtered.available_names.is_empty());
        assert_eq!(filtered.skipped_mcp_servers, vec!["WebMCP".to_string()]);
    }

    #[test]
    fn responses_tool_compatibility_cache_only_degrades() {
        let cache_key = responses_tool_compatibility_cache_key(
            "https://example.com/v1",
            &format!("gpt-test-{}", std::process::id()),
        );

        store_cached_responses_tool_compatibility(
            &cache_key,
            ResponsesToolCompatibilityMode::NoMcp,
        );
        store_cached_responses_tool_compatibility(&cache_key, ResponsesToolCompatibilityMode::Full);
        assert_eq!(
            load_cached_responses_tool_compatibility(&cache_key),
            ResponsesToolCompatibilityMode::NoMcp
        );

        store_cached_responses_tool_compatibility(
            &cache_key,
            ResponsesToolCompatibilityMode::NoTools,
        );
        assert_eq!(
            load_cached_responses_tool_compatibility(&cache_key),
            ResponsesToolCompatibilityMode::NoTools
        );
    }

    #[test]
    fn retry_without_mcp_tools_only_triggers_for_provider_5xx_with_mcp_tools() {
        let tool_catalog = build_tool_catalog(
            &[AcpMcpServerConfig::Http(AcpMcpServerHttpConfig {
                name: "WebMCP".to_string(),
                url: "https://example.com/mcp".to_string(),
                headers: Vec::new(),
            })],
            QuestionAnswerProtocol::Responses,
        );
        let provider_error = anyhow::anyhow!(
            "LLM provider responses 请求失败 (500 Internal Server Error): internal error"
        );
        let client_error =
            anyhow::anyhow!("LLM provider responses 请求失败 (400 Bad Request): invalid input");
        let cancelled_error = anyhow::anyhow!(
            "failed to request question answering from responses API: send request: context canceled"
        );

        assert!(should_retry_without_mcp_tools(
            false,
            false,
            &tool_catalog,
            &provider_error
        ));
        assert!(should_retry_without_mcp_tools(
            false,
            false,
            &tool_catalog,
            &cancelled_error
        ));
        assert!(!should_retry_without_mcp_tools(
            false,
            true,
            &tool_catalog,
            &provider_error
        ));
        assert!(!should_retry_without_mcp_tools(
            false,
            false,
            &tool_catalog,
            &client_error
        ));
    }

    #[test]
    fn retry_without_all_tools_triggers_for_provider_5xx_with_function_tools() {
        let tool_catalog = build_tool_catalog(&[], QuestionAnswerProtocol::Responses);
        let provider_error = anyhow::anyhow!(
            "LLM provider responses 请求失败 (500 Internal Server Error): internal error"
        );
        let client_error =
            anyhow::anyhow!("LLM provider responses 请求失败 (400 Bad Request): invalid input");

        assert!(should_retry_without_all_tools(
            false,
            &tool_catalog,
            &provider_error
        ));
        assert!(!should_retry_without_all_tools(
            true,
            &tool_catalog,
            &provider_error
        ));
        assert!(!should_retry_without_all_tools(
            false,
            &tool_catalog,
            &client_error
        ));
    }

    #[tokio::test]
    async fn open_target_tool_rejects_when_current_question_does_not_request_open() {
        let workspace_root = temp_test_root("open-tool-no-intent");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime = QuestionToolRuntime {
            data_dir: &workspace_root,
            workspace_root: &workspace_root,
            rag_settings: &RagSettings::default(),
            llm_settings: &LlmSettings::default(),
            allow_open_target: false,
            progress_event_tx: None,
        };

        let error = execute_open_target_tool(
            &runtime,
            &serde_json::json!({
                "target": "https://tauri.app"
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "当前问题没有明确要求打开目标，禁止调用 wabity.system.open"
        );
    }

    #[tokio::test]
    async fn open_target_tool_rejects_local_paths_outside_allowed_roots() {
        let root = temp_test_root("open-tool-outside");
        let workspace_root = root.join("workspace");
        let outside_root = root.join("outside");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::create_dir_all(&outside_root).expect("create outside root");

        let workspace_root = workspace_root
            .canonicalize()
            .expect("canonicalize workspace root");
        let outside_file = outside_root.join("secret.md");
        std::fs::write(&outside_file, "secret").expect("write outside file");
        let outside_file = outside_file
            .canonicalize()
            .expect("canonicalize outside file");
        let runtime = QuestionToolRuntime {
            data_dir: &workspace_root,
            workspace_root: &workspace_root,
            rag_settings: &RagSettings::default(),
            llm_settings: &LlmSettings::default(),
            allow_open_target: true,
            progress_event_tx: None,
        };

        let error = execute_open_target_tool(
            &runtime,
            &serde_json::json!({
                "target": outside_file.to_string_lossy()
            }),
        )
        .await
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("路径超出允许范围，只能打开当前 workspace 或显式配置的 RAG 目录"));
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

    #[tokio::test]
    async fn read_file_tool_reads_normalized_docx_text() {
        let root = temp_test_root("read-docx");
        let workspace_root = root.join("workspace");
        let docs_root = workspace_root.join("docs");
        let file_path = docs_root.join("manual.docx");
        let document_xml = r#"
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:body>
                <w:p>
                  <w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
                  <w:r><w:t>Guide</w:t></w:r>
                </w:p>
                <w:p>
                  <w:r><w:t>Alpha paragraph.</w:t></w:r>
                </w:p>
              </w:body>
            </w:document>
        "#;
        std::fs::create_dir_all(&docs_root).expect("create docs root");
        std::fs::write(&file_path, build_test_docx(document_xml)).expect("write docx");

        let workspace_root = workspace_root
            .canonicalize()
            .expect("canonicalize workspace root");
        let runtime = QuestionToolRuntime {
            data_dir: &workspace_root,
            workspace_root: &workspace_root,
            rag_settings: &RagSettings::default(),
            llm_settings: &LlmSettings::default(),
            allow_open_target: false,
            progress_event_tx: None,
        };
        let executed = execute_read_file_tool(
            &runtime,
            &serde_json::json!({
                "path": "docs/manual.docx",
                "line_start": 1,
                "line_count": 3
            }),
        )
        .await
        .expect("read file tool should support docx");

        assert!(executed.output.contains("# Guide"));
        assert!(executed.output.contains("Alpha paragraph."));

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
