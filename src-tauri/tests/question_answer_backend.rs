use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::{fs, net::TcpListener, task::JoinHandle};
use wabity_lib::question_answer_backend::{
    answer_question, ExecutionConversationRole, ExecutionConversationState,
    ExecutionConversationTurn, ExecutionProgressEvent, ExecutionStatus, LlmModelType,
    LlmProviderConfig, LlmProviderProtocol, LlmSettings, QuestionAnswerBackendRequest, RagSettings,
};

#[derive(Clone)]
enum MockScenario {
    ReadWorkspaceFile,
    DirectAnswerOnly,
    DirectAnswerWithReasoning,
    DirectAnswerWithEmbeddedThinkingContent,
    HistoryAwareDirectAnswer,
    RejectOutsideWorkspaceRead,
    ChatRejectsStreaming,
    ResponsesStatelessReadWorkspaceFile,
    ResponsesStatelessFollowUp,
    ResponsesStatefulFollowUp,
    ResponsesRejectStreaming,
}

#[derive(Clone)]
struct MockLlmState {
    round: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Value>>>,
    scenario: MockScenario,
}

fn temp_test_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("wabity-question-backend-{label}-{unique}"))
}

fn build_llm_settings(
    base_url: String,
    protocol: LlmProviderProtocol,
    supports_stateful: bool,
) -> LlmSettings {
    let protocol_name = match protocol {
        LlmProviderProtocol::Responses => "responses",
        LlmProviderProtocol::ChatCompletions => "chat_completions",
    };
    let provider: LlmProviderConfig = serde_json::from_value(json!({
        "id": "qa-provider",
        "name": "QA Provider",
        "baseUrl": base_url,
        "apiKey": "test-key",
        "protocol": protocol_name,
        "models": [{
            "id": "qa-provider",
            "modelType": "llm",
            "model": "mock-model",
            "supportsStateful": supports_stateful
        }]
    }))
    .expect("failed to deserialize test provider");
    assert_eq!(provider.models[0].model_type, LlmModelType::Llm);
    assert_eq!(provider.protocol, protocol);

    serde_json::from_value(json!({
        "providers": [provider],
        "questionAnswerModelId": "qa-provider"
    }))
    .expect("failed to deserialize llm settings")
}

async fn spawn_chat_completion_server(
    scenario: MockScenario,
) -> (String, Arc<Mutex<Vec<Value>>>, JoinHandle<()>) {
    let state = MockLlmState {
        round: Arc::new(AtomicUsize::new(0)),
        requests: Arc::new(Mutex::new(Vec::new())),
        scenario,
    };
    let shared_requests = state.requests.clone();
    let app = Router::new()
        .route("/v1/chat/completions", post(mock_chat_completions))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind mock llm server");
    let address = listener
        .local_addr()
        .expect("failed to read mock llm server address");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock llm server exited unexpectedly");
    });

    (format!("http://{address}/v1"), shared_requests, handle)
}

async fn spawn_responses_server(
    scenario: MockScenario,
) -> (String, Arc<Mutex<Vec<Value>>>, JoinHandle<()>) {
    let state = MockLlmState {
        round: Arc::new(AtomicUsize::new(0)),
        requests: Arc::new(Mutex::new(Vec::new())),
        scenario,
    };
    let shared_requests = state.requests.clone();
    let app = Router::new()
        .route("/v1/responses", post(mock_responses))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind mock responses server");
    let address = listener
        .local_addr()
        .expect("failed to read mock responses server address");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock responses server exited unexpectedly");
    });

    (format!("http://{address}/v1"), shared_requests, handle)
}

async fn mock_chat_completions(
    State(state): State<MockLlmState>,
    Json(payload): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state
        .requests
        .lock()
        .expect("failed to lock request log")
        .push(payload.clone());
    let round = state.round.fetch_add(1, Ordering::SeqCst);

    match (&state.scenario, round) {
        (MockScenario::ReadWorkspaceFile, 0) => (
            StatusCode::OK,
            Json(json!({
                "id": "chatcmpl-round-1",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_read_notes",
                            "type": "function",
                            "function": {
                                "name": "wabity.read_file_lines",
                                "arguments": "{\"path\":\"notes.md\",\"line_start\":1,\"line_count\":2}"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            })),
        ),
        (MockScenario::ReadWorkspaceFile, _) => (
            StatusCode::OK,
            Json(json!({
                "id": "chatcmpl-round-2",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "结论：第一行和第二行已经被读取。"
                    },
                    "finish_reason": "stop"
                }]
            })),
        ),
        (MockScenario::DirectAnswerOnly, _) => (
            StatusCode::OK,
            Json(json!({
                "id": "chatcmpl-direct",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "这是直接回答，不需要读取任何文件。"
                    },
                    "finish_reason": "stop"
                }]
            })),
        ),
        (MockScenario::DirectAnswerWithReasoning, _) => (
            StatusCode::OK,
            Json(json!({
                "id": "chatcmpl-reasoning",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "最终答案在这里。",
                        "reasoning_content": "先检查问题范围，再整理回答结构。"
                    },
                    "finish_reason": "stop"
                }]
            })),
        ),
        (MockScenario::DirectAnswerWithEmbeddedThinkingContent, _) => (
            StatusCode::OK,
            Json(json!({
                "id": "chatcmpl-content-thinking",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": [
                            {
                                "type": "thinking",
                                "text": "先枚举问题范围，再决定回答结构。"
                            },
                            {
                                "type": "text",
                                "text": "最终答案在这里。"
                            }
                        ]
                    },
                    "finish_reason": "stop"
                }]
            })),
        ),
        (MockScenario::HistoryAwareDirectAnswer, _) => {
            let messages = payload
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let saw_history = messages.iter().any(|message| {
                message.get("role").and_then(Value::as_str) == Some("assistant")
                    && message.get("content").and_then(Value::as_str) == Some("之前的答案")
            });
            let answer = if saw_history {
                "我看到了历史上下文：之前的答案"
            } else {
                "没有收到历史上下文"
            };
            (
                StatusCode::OK,
                Json(json!({
                    "id": "chatcmpl-history",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": answer
                        },
                        "finish_reason": "stop"
                    }]
                })),
            )
        }
        (MockScenario::RejectOutsideWorkspaceRead, 0) => (
            StatusCode::OK,
            Json(json!({
                "id": "chatcmpl-reject-1",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": "call_read_secret",
                            "type": "function",
                            "function": {
                                "name": "wabity.read_file_lines",
                                "arguments": "{\"path\":\"../secret.txt\",\"line_start\":1,\"line_count\":2}"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            })),
        ),
        (MockScenario::RejectOutsideWorkspaceRead, _) => (
            StatusCode::OK,
            Json(json!({
                "id": "chatcmpl-reject-2",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "读取被拒绝：目标文件不在允许目录内。"
                    },
                    "finish_reason": "stop"
                }]
            })),
        ),
        (MockScenario::ChatRejectsStreaming, 0)
            if payload.get("stream") == Some(&Value::Bool(true)) =>
        {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "message": "streaming is not supported for this provider"
                    }
                })),
            )
        }
        (MockScenario::ChatRejectsStreaming, _) => (
            StatusCode::OK,
            Json(json!({
                "id": "chatcmpl-stream-fallback",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "已回退到非流式 chat/completions。"
                    },
                    "finish_reason": "stop"
                }]
            })),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": {
                    "message": "unexpected chat scenario round"
                }
            })),
        ),
    }
}

async fn mock_responses(
    State(state): State<MockLlmState>,
    Json(payload): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state
        .requests
        .lock()
        .expect("failed to lock request log")
        .push(payload.clone());
    let round = state.round.fetch_add(1, Ordering::SeqCst);

    match (&state.scenario, round) {
        (MockScenario::ResponsesStatelessReadWorkspaceFile, 0) => (
            StatusCode::OK,
            Json(json!({
                "id": "resp-stateless-1",
                "output": [{
                    "type": "function_call",
                    "call_id": "call_read_notes",
                    "name": "wabity.read_file_lines",
                    "arguments": "{\"path\":\"notes.md\",\"line_start\":1,\"line_count\":2}"
                }]
            })),
        ),
        (MockScenario::ResponsesStatelessReadWorkspaceFile, _) => (
            StatusCode::OK,
            Json(json!({
                "id": "resp-stateless-2",
                "output": [{
                    "type": "message",
                    "content": [{
                        "type": "output_text",
                        "text": "结论：responses 已读取第一行和第二行。"
                    }]
                }]
            })),
        ),
        (MockScenario::ResponsesStatelessFollowUp, 0) => (
            StatusCode::OK,
            Json(json!({
                "id": "resp-stateless-follow-up-1",
                "output": [{
                    "type": "message",
                    "content": [{
                        "type": "output_text",
                        "text": "第一次回答，不使用 stateful 续链。"
                    }]
                }]
            })),
        ),
        (MockScenario::ResponsesStatelessFollowUp, _) => {
            let has_previous_response_id = payload.get("previous_response_id").is_some();
            let input_items = payload
                .get("input")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let saw_history = input_items.iter().any(|item| {
                item.get("role").and_then(Value::as_str) == Some("assistant")
                    && item
                        .get("content")
                        .and_then(Value::as_array)
                        .and_then(|items| items.first())
                        .and_then(|item| item.get("text"))
                        .and_then(Value::as_str)
                        == Some("第一次回答，不使用 stateful 续链。")
            });
            let answer = if !has_previous_response_id && saw_history {
                "stateless 追问保留显式历史，且没有发送 previous_response_id。"
            } else {
                "stateless 追问请求形态不符合预期。"
            };
            (
                StatusCode::OK,
                Json(json!({
                    "id": "resp-stateless-follow-up-2",
                    "output": [{
                        "type": "message",
                        "content": [{
                            "type": "output_text",
                            "text": answer
                        }]
                    }]
                })),
            )
        }
        (MockScenario::ResponsesStatefulFollowUp, 0) => (
            StatusCode::OK,
            Json(json!({
                "id": "resp-stateful-1",
                "output": [{
                    "type": "message",
                    "content": [{
                        "type": "output_text",
                        "text": "第一次回答，后续应通过 response chain 继续。"
                    }]
                }]
            })),
        ),
        (MockScenario::ResponsesStatefulFollowUp, _) => {
            let previous_response_id = payload
                .get("previous_response_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let input_items = payload
                .get("input")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let only_latest_question = input_items.len() == 1
                && input_items[0].get("role").and_then(Value::as_str) == Some("user")
                && input_items[0]
                    .get("content")
                    .and_then(Value::as_array)
                    .and_then(|items| items.first())
                    .and_then(|item| item.get("text"))
                    .and_then(Value::as_str)
                    == Some("继续追问 stateful 行为");
            let answer = if previous_response_id == "resp-stateful-1" && only_latest_question {
                "stateful 追问已发送 previous_response_id，且只携带最新问题。"
            } else {
                "stateful 追问请求形态不符合预期。"
            };
            (
                StatusCode::OK,
                Json(json!({
                    "id": "resp-stateful-2",
                    "output": [{
                        "type": "message",
                        "content": [{
                            "type": "output_text",
                            "text": answer
                        }]
                    }]
                })),
            )
        }
        (MockScenario::ResponsesRejectStreaming, 0)
            if payload.get("stream") == Some(&Value::Bool(true)) =>
        {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "message": "streaming is not supported for this provider"
                    }
                })),
            )
        }
        (MockScenario::ResponsesRejectStreaming, _) => (
            StatusCode::OK,
            Json(json!({
                "id": "resp-stream-fallback",
                "output": [{
                    "type": "message",
                    "content": [{
                        "type": "output_text",
                        "text": "已回退到非流式 responses。"
                    }]
                }]
            })),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": {
                    "message": "unexpected responses scenario round"
                }
            })),
        ),
    }
}

#[tokio::test]
async fn question_answer_backend_supports_standalone_integration_test() {
    let workspace_root = temp_test_root("workspace");
    fs::create_dir_all(&workspace_root)
        .await
        .expect("failed to create temp workspace");
    fs::write(
        workspace_root.join("notes.md"),
        "alpha line\nbeta line\ngamma line\n",
    )
    .await
    .expect("failed to write notes fixture");

    let (base_url, requests, server_handle) =
        spawn_chat_completion_server(MockScenario::ReadWorkspaceFile).await;
    let prompts_settings = Default::default();
    let rag_settings = RagSettings::default();
    let conversation = [ExecutionConversationTurn {
        role: ExecutionConversationRole::User,
        content: "上一个问题".to_string(),
    }];
    let llm_settings = build_llm_settings(base_url, LlmProviderProtocol::ChatCompletions, false);

    let result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "/ask 请读取 notes.md 并总结",
        conversation: &conversation,
        conversation_state: None,
        prompts_settings: &prompts_settings,
        rag_settings: &rag_settings,
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: None,
    })
    .await
    .expect("question backend request should succeed");

    server_handle.abort();

    assert_eq!(result.status, ExecutionStatus::Success);
    assert_eq!(
        result.primary_text.as_deref(),
        Some("结论：第一行和第二行已经被读取。")
    );
    assert!(result
        .secondary_text
        .as_deref()
        .is_some_and(|text| text.contains("已执行 1 次工具调用")));

    let structured = result
        .structured_payload
        .expect("question backend should return structured payload");
    let citations = structured
        .get("citations")
        .and_then(Value::as_array)
        .expect("citations should be an array");
    assert_eq!(citations.len(), 1);
    assert!(citations[0]["absolutePath"]
        .as_str()
        .is_some_and(|path| path.ends_with("/notes.md")));
    assert!(citations[0]["snippet"]
        .as_str()
        .is_some_and(|snippet| snippet.contains("alpha line")));

    let tool_calls = structured
        .get("tools")
        .and_then(|value| value.get("calls"))
        .and_then(Value::as_array)
        .expect("tool call list should exist");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0]["name"], "wabity.read_file_lines");

    let recorded_requests = requests.lock().expect("failed to lock recorded requests");
    assert_eq!(recorded_requests.len(), 2);

    let first_tools = recorded_requests[0]
        .get("tools")
        .and_then(Value::as_array)
        .expect("first request should contain builtin tools");
    assert!(first_tools.iter().any(|tool| {
        tool.get("function")
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            == Some("wabity.read_file_lines")
    }));

    let second_messages = recorded_requests[1]
        .get("messages")
        .and_then(Value::as_array)
        .expect("second request should contain tool output message");
    assert!(second_messages.iter().any(|message| {
        message.get("role").and_then(Value::as_str) == Some("tool")
            && message
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.contains("alpha line"))
    }));
}

#[tokio::test]
async fn question_answer_backend_falls_back_to_non_streaming_chat_provider() {
    let workspace_root = temp_test_root("chat-stream-fallback");
    fs::create_dir_all(&workspace_root)
        .await
        .expect("failed to create temp workspace");

    let (base_url, requests, server_handle) =
        spawn_chat_completion_server(MockScenario::ChatRejectsStreaming).await;
    let llm_settings = build_llm_settings(base_url, LlmProviderProtocol::ChatCompletions, false);
    let progress_event_tx = Arc::new(|_event: ExecutionProgressEvent| {});

    let result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "/ask 直接回答即可",
        conversation: &[],
        conversation_state: None,
        prompts_settings: &Default::default(),
        rag_settings: &RagSettings::default(),
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: Some(progress_event_tx),
    })
    .await
    .expect("question backend should fall back to non-stream chat request");

    server_handle.abort();

    assert_eq!(result.status, ExecutionStatus::Success);
    assert_eq!(
        result.primary_text.as_deref(),
        Some("已回退到非流式 chat/completions。")
    );

    let recorded_requests = requests.lock().expect("failed to lock recorded requests");
    assert_eq!(recorded_requests.len(), 2);
    assert_eq!(recorded_requests[0]["stream"], Value::Bool(true));
    assert_eq!(recorded_requests[1]["stream"], Value::Bool(false));
}

#[tokio::test]
async fn question_answer_backend_falls_back_to_non_streaming_responses_provider() {
    let workspace_root = temp_test_root("responses-stream-fallback");
    fs::create_dir_all(&workspace_root)
        .await
        .expect("failed to create temp workspace");

    let (base_url, requests, server_handle) =
        spawn_responses_server(MockScenario::ResponsesRejectStreaming).await;
    let llm_settings = build_llm_settings(base_url, LlmProviderProtocol::Responses, false);
    let progress_event_tx = Arc::new(|_event: ExecutionProgressEvent| {});

    let result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "/ask 直接回答即可",
        conversation: &[],
        conversation_state: None,
        prompts_settings: &Default::default(),
        rag_settings: &RagSettings::default(),
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: Some(progress_event_tx),
    })
    .await
    .expect("question backend should fall back to non-stream responses request");

    server_handle.abort();

    assert_eq!(result.status, ExecutionStatus::Success);
    assert_eq!(
        result.primary_text.as_deref(),
        Some("已回退到非流式 responses。")
    );

    let recorded_requests = requests.lock().expect("failed to lock recorded requests");
    assert_eq!(recorded_requests.len(), 2);
    assert_eq!(recorded_requests[0]["stream"], Value::Bool(true));
    assert_eq!(recorded_requests[1]["stream"], Value::Bool(false));
}

#[tokio::test]
async fn question_answer_backend_supports_direct_answer_without_tool_calls() {
    let workspace_root = temp_test_root("direct-answer");
    fs::create_dir_all(&workspace_root)
        .await
        .expect("failed to create temp workspace");

    let (base_url, requests, server_handle) =
        spawn_chat_completion_server(MockScenario::DirectAnswerOnly).await;
    let prompts_settings = Default::default();
    let rag_settings = RagSettings::default();
    let llm_settings = build_llm_settings(base_url, LlmProviderProtocol::ChatCompletions, false);

    let result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "什么是这个测试场景？",
        conversation: &[],
        conversation_state: None,
        prompts_settings: &prompts_settings,
        rag_settings: &rag_settings,
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: None,
    })
    .await
    .expect("direct answer question should succeed");

    server_handle.abort();

    assert_eq!(result.status, ExecutionStatus::Success);
    assert_eq!(
        result.primary_text.as_deref(),
        Some("这是直接回答，不需要读取任何文件。")
    );
    assert!(result
        .secondary_text
        .as_deref()
        .is_some_and(|text| text.contains("未调用工具")));

    let structured = result
        .structured_payload
        .expect("direct answer should still return structured payload");
    assert_eq!(structured["citations"], json!([]));
    assert_eq!(structured["tools"]["calls"], json!([]));

    let recorded_requests = requests.lock().expect("failed to lock recorded requests");
    assert_eq!(recorded_requests.len(), 1);
}

#[tokio::test]
async fn question_answer_backend_preserves_chat_reasoning_separately_from_final_answer() {
    let workspace_root = temp_test_root("chat-reasoning");
    fs::create_dir_all(&workspace_root)
        .await
        .expect("failed to create temp workspace");

    let (base_url, _requests, server_handle) =
        spawn_chat_completion_server(MockScenario::DirectAnswerWithReasoning).await;
    let prompts_settings = Default::default();
    let rag_settings = RagSettings::default();
    let llm_settings = build_llm_settings(base_url, LlmProviderProtocol::ChatCompletions, false);

    let result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "给我一个简短答案",
        conversation: &[],
        conversation_state: None,
        prompts_settings: &prompts_settings,
        rag_settings: &rag_settings,
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: None,
    })
    .await
    .expect("chat reasoning question should succeed");

    server_handle.abort();

    assert_eq!(result.status, ExecutionStatus::Success);
    assert_eq!(result.primary_text.as_deref(), Some("最终答案在这里。"));
    assert_eq!(
        result
            .structured_payload
            .as_ref()
            .and_then(|payload| payload.get("reasoning"))
            .and_then(Value::as_str),
        Some("先检查问题范围，再整理回答结构。")
    );
}

#[tokio::test]
async fn question_answer_backend_extracts_thinking_embedded_in_chat_content() {
    let workspace_root = temp_test_root("chat-content-thinking");
    fs::create_dir_all(&workspace_root)
        .await
        .expect("failed to create temp workspace");

    let (base_url, _requests, server_handle) =
        spawn_chat_completion_server(MockScenario::DirectAnswerWithEmbeddedThinkingContent).await;
    let prompts_settings = Default::default();
    let rag_settings = RagSettings::default();
    let llm_settings = build_llm_settings(base_url, LlmProviderProtocol::ChatCompletions, false);

    let result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "给我一个简短答案",
        conversation: &[],
        conversation_state: None,
        prompts_settings: &prompts_settings,
        rag_settings: &rag_settings,
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: None,
    })
    .await
    .expect("chat content thinking question should succeed");

    server_handle.abort();

    assert_eq!(result.status, ExecutionStatus::Success);
    assert_eq!(result.primary_text.as_deref(), Some("最终答案在这里。"));
    assert_eq!(
        result
            .structured_payload
            .as_ref()
            .and_then(|payload| payload.get("reasoning"))
            .and_then(Value::as_str),
        Some("先枚举问题范围，再决定回答结构。")
    );
}

#[tokio::test]
async fn question_answer_backend_passes_multi_turn_history_to_model() {
    let workspace_root = temp_test_root("history");
    fs::create_dir_all(&workspace_root)
        .await
        .expect("failed to create temp workspace");

    let (base_url, requests, server_handle) =
        spawn_chat_completion_server(MockScenario::HistoryAwareDirectAnswer).await;
    let prompts_settings = Default::default();
    let rag_settings = RagSettings::default();
    let llm_settings = build_llm_settings(base_url, LlmProviderProtocol::ChatCompletions, false);
    let conversation = [
        ExecutionConversationTurn {
            role: ExecutionConversationRole::User,
            content: "第一个问题".to_string(),
        },
        ExecutionConversationTurn {
            role: ExecutionConversationRole::Assistant,
            content: "之前的答案".to_string(),
        },
    ];

    let result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "/ask 基于上文继续追问",
        conversation: &conversation,
        conversation_state: None,
        prompts_settings: &prompts_settings,
        rag_settings: &rag_settings,
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: None,
    })
    .await
    .expect("history-aware question should succeed");

    server_handle.abort();

    assert_eq!(result.status, ExecutionStatus::Success);
    assert_eq!(
        result.primary_text.as_deref(),
        Some("我看到了历史上下文：之前的答案")
    );

    let recorded_requests = requests.lock().expect("failed to lock recorded requests");
    assert_eq!(recorded_requests.len(), 1);
    let messages = recorded_requests[0]
        .get("messages")
        .and_then(Value::as_array)
        .expect("history request should include messages");
    assert!(messages.iter().any(|message| {
        message.get("role").and_then(Value::as_str) == Some("assistant")
            && message.get("content").and_then(Value::as_str) == Some("之前的答案")
    }));
}

#[tokio::test]
async fn question_answer_backend_reports_builtin_tool_error_for_outside_workspace_read() {
    let workspace_root = temp_test_root("outside-read");
    fs::create_dir_all(&workspace_root)
        .await
        .expect("failed to create temp workspace");

    let (base_url, requests, server_handle) =
        spawn_chat_completion_server(MockScenario::RejectOutsideWorkspaceRead).await;
    let prompts_settings = Default::default();
    let rag_settings = RagSettings::default();
    let llm_settings = build_llm_settings(base_url, LlmProviderProtocol::ChatCompletions, false);

    let result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "/ask 请读取上级目录的 secret.txt",
        conversation: &[],
        conversation_state: None,
        prompts_settings: &prompts_settings,
        rag_settings: &rag_settings,
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: None,
    })
    .await
    .expect("outside workspace read flow should still complete");

    server_handle.abort();

    assert_eq!(result.status, ExecutionStatus::Success);
    assert_eq!(
        result.primary_text.as_deref(),
        Some("读取被拒绝：目标文件不在允许目录内。")
    );

    let structured = result
        .structured_payload
        .expect("tool error answer should return structured payload");
    let tool_calls = structured["tools"]["calls"]
        .as_array()
        .expect("tool calls should be an array");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0]["status"], "error");

    let recorded_requests = requests.lock().expect("failed to lock recorded requests");
    assert_eq!(recorded_requests.len(), 2);
    let tool_messages = recorded_requests[1]
        .get("messages")
        .and_then(Value::as_array)
        .expect("second request should include tool result");
    let tool_message = tool_messages
        .iter()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .expect("second request should include at least one tool role message");
    let tool_payload: Value = serde_json::from_str(
        tool_message
            .get("content")
            .and_then(Value::as_str)
            .expect("tool content should be encoded as JSON string"),
    )
    .expect("tool content should be valid JSON");
    assert_eq!(tool_payload["ok"], false);
    assert!(tool_payload["error"]
        .as_str()
        .is_some_and(|error| !error.trim().is_empty()));
}

#[tokio::test]
async fn question_answer_backend_supports_responses_stateless_tool_loop() {
    let workspace_root = temp_test_root("responses-stateless-tool");
    fs::create_dir_all(&workspace_root)
        .await
        .expect("failed to create temp workspace");
    fs::write(
        workspace_root.join("notes.md"),
        "alpha line\nbeta line\ngamma line\n",
    )
    .await
    .expect("failed to write notes fixture");

    let (base_url, requests, server_handle) =
        spawn_responses_server(MockScenario::ResponsesStatelessReadWorkspaceFile).await;
    let prompts_settings = Default::default();
    let rag_settings = RagSettings::default();
    let llm_settings = build_llm_settings(base_url, LlmProviderProtocol::Responses, false);

    let result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "/ask 用 responses 读取 notes.md 并总结",
        conversation: &[],
        conversation_state: None,
        prompts_settings: &prompts_settings,
        rag_settings: &rag_settings,
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: None,
    })
    .await
    .expect("responses stateless tool loop should succeed");

    server_handle.abort();

    assert_eq!(result.status, ExecutionStatus::Success);
    assert_eq!(
        result.primary_text.as_deref(),
        Some("结论：responses 已读取第一行和第二行。")
    );
    assert!(result.secondary_text.as_deref().is_some_and(|text| text
        .contains("responses(stateless)")
        && text.contains("已执行 1 次工具调用")));

    let structured = result
        .structured_payload
        .expect("responses stateless tool loop should return structured payload");
    assert_eq!(
        structured["tools"]["calls"][0]["name"],
        "wabity.read_file_lines"
    );

    let recorded_requests = requests.lock().expect("failed to lock recorded requests");
    assert_eq!(recorded_requests.len(), 2);
    assert!(recorded_requests[0].get("previous_response_id").is_none());
    assert_eq!(
        recorded_requests[1]["input"][0]["type"].as_str(),
        Some("function_call_output")
    );
}

#[tokio::test]
async fn question_answer_backend_responses_stateless_follow_up_uses_explicit_history() {
    let workspace_root = temp_test_root("responses-stateless-follow-up");
    fs::create_dir_all(&workspace_root)
        .await
        .expect("failed to create temp workspace");

    let (base_url, requests, server_handle) =
        spawn_responses_server(MockScenario::ResponsesStatelessFollowUp).await;
    let prompts_settings = Default::default();
    let rag_settings = RagSettings::default();
    let llm_settings = build_llm_settings(base_url, LlmProviderProtocol::Responses, false);

    let first_result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "/ask 第一次提问",
        conversation: &[],
        conversation_state: None,
        prompts_settings: &prompts_settings,
        rag_settings: &rag_settings,
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: None,
    })
    .await
    .expect("first stateless request should succeed");

    let first_state: ExecutionConversationState = serde_json::from_value(
        first_result
            .structured_payload
            .expect("first stateless response should have structured payload")["conversationState"]
            .clone(),
    )
    .expect("conversation state should deserialize");

    let conversation = [
        ExecutionConversationTurn {
            role: ExecutionConversationRole::User,
            content: "第一次提问".to_string(),
        },
        ExecutionConversationTurn {
            role: ExecutionConversationRole::Assistant,
            content: "第一次回答，不使用 stateful 续链。".to_string(),
        },
    ];
    let second_result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "/ask 继续追问 stateless 行为",
        conversation: &conversation,
        conversation_state: Some(&first_state),
        prompts_settings: &prompts_settings,
        rag_settings: &rag_settings,
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: None,
    })
    .await
    .expect("stateless follow-up should succeed");

    server_handle.abort();

    assert_eq!(second_result.status, ExecutionStatus::Success);
    assert_eq!(
        second_result.primary_text.as_deref(),
        Some("stateless 追问保留显式历史，且没有发送 previous_response_id。")
    );

    let recorded_requests = requests.lock().expect("failed to lock recorded requests");
    assert_eq!(recorded_requests.len(), 2);
    assert!(recorded_requests[1].get("previous_response_id").is_none());
    let second_input = recorded_requests[1]["input"]
        .as_array()
        .expect("second stateless request should send explicit input history");
    assert_eq!(second_input.len(), 3);
}

#[tokio::test]
async fn question_answer_backend_responses_stateful_follow_up_uses_previous_response_id() {
    let workspace_root = temp_test_root("responses-stateful-follow-up");
    fs::create_dir_all(&workspace_root)
        .await
        .expect("failed to create temp workspace");

    let (base_url, requests, server_handle) =
        spawn_responses_server(MockScenario::ResponsesStatefulFollowUp).await;
    let prompts_settings = Default::default();
    let rag_settings = RagSettings::default();
    let llm_settings = build_llm_settings(base_url, LlmProviderProtocol::Responses, true);

    let first_result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "/ask 第一次 stateful 提问",
        conversation: &[],
        conversation_state: None,
        prompts_settings: &prompts_settings,
        rag_settings: &rag_settings,
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: None,
    })
    .await
    .expect("first stateful request should succeed");

    let first_state: ExecutionConversationState = serde_json::from_value(
        first_result
            .structured_payload
            .expect("first stateful response should have structured payload")["conversationState"]
            .clone(),
    )
    .expect("conversation state should deserialize");

    let conversation = [
        ExecutionConversationTurn {
            role: ExecutionConversationRole::User,
            content: "第一次 stateful 提问".to_string(),
        },
        ExecutionConversationTurn {
            role: ExecutionConversationRole::Assistant,
            content: "第一次回答，后续应通过 response chain 继续。".to_string(),
        },
    ];
    let second_result = answer_question(QuestionAnswerBackendRequest {
        data_dir: &workspace_root,
        workspace_root: &workspace_root,
        raw_text: "/ask 继续追问 stateful 行为",
        conversation: &conversation,
        conversation_state: Some(&first_state),
        prompts_settings: &prompts_settings,
        rag_settings: &rag_settings,
        llm_settings: &llm_settings,
        mcp_servers: &[],
        progress_event_tx: None,
    })
    .await
    .expect("stateful follow-up should succeed");

    server_handle.abort();

    assert_eq!(second_result.status, ExecutionStatus::Success);
    assert_eq!(
        second_result.primary_text.as_deref(),
        Some("stateful 追问已发送 previous_response_id，且只携带最新问题。")
    );
    assert!(second_result
        .secondary_text
        .as_deref()
        .is_some_and(|text| text.contains("responses(stateful)")));

    let recorded_requests = requests.lock().expect("failed to lock recorded requests");
    assert_eq!(recorded_requests.len(), 2);
    assert_eq!(
        recorded_requests[1]["previous_response_id"].as_str(),
        Some("resp-stateful-1")
    );
    let second_input = recorded_requests[1]["input"]
        .as_array()
        .expect("second stateful request should send latest question only");
    assert_eq!(second_input.len(), 1);
}
