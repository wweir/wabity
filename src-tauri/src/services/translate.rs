use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use reqwest::Client as AsyncHttpClient;
use serde_json::{json, Value};

use crate::domain::{
    execution::ExecutionResult,
    settings::{LlmProviderConfig, LlmProviderProtocol, LlmSettings, PromptsSettings},
};
use crate::infrastructure::openai_compatible::{
    describe_chat_completions_response_issue, extract_chat_completions_text,
    extract_responses_text, OpenAiCompatibleClient, OpenAiCompatibleResponseFormat,
};

const TRANSLATE_COMMAND_ALIASES: [&str; 3] = ["/translate", "/fy", "/tr"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranslationProtocol {
    Responses,
    ChatCompletions,
}

pub struct TranslationCallbacks<'a> {
    pub on_text_delta: Option<&'a mut (dyn FnMut(&str) + Send)>,
}

fn shared_translation_http_client() -> Result<AsyncHttpClient> {
    static TRANSLATION_HTTP_CLIENT: OnceLock<Result<AsyncHttpClient, String>> = OnceLock::new();

    TRANSLATION_HTTP_CLIENT
        .get_or_init(|| {
            AsyncHttpClient::builder()
                .timeout(Duration::from_secs(45))
                .build()
                .map_err(|error| format!("failed to build HTTP client for translation: {error}"))
        })
        .clone()
        .map_err(|message| anyhow::anyhow!(message.clone()))
}

pub async fn execute_translation_with_callbacks(
    raw_text: &str,
    prompts_settings: &PromptsSettings,
    llm_settings: &LlmSettings,
    callbacks: TranslationCallbacks<'_>,
) -> Result<ExecutionResult> {
    let started_at = Instant::now();
    let payload = translation_payload(raw_text);
    if payload.is_empty() {
        bail!("请输入要翻译的内容");
    }

    let provider = resolve_translation_provider(llm_settings)?;
    let model = provider
        .llm_model_name()
        .context("翻译使用的 LLM 模型不能为空")?;
    let protocol = translation_protocol(provider);

    let http_client = shared_translation_http_client()?;
    let client = OpenAiCompatibleClient::new_async(
        &http_client,
        &provider.base_url,
        &provider.api_key,
        "LLM provider base URL",
    )?;

    let (translated, used_protocol) = request_translation(
        protocol,
        &client,
        model,
        &prompts_settings.translation_prompt,
        payload,
        callbacks,
    )
    .await?;
    tracing::info!(
        provider_id = %provider.id,
        protocol = %translation_protocol_label(used_protocol),
        payload_chars = payload.chars().count(),
        elapsed_ms = started_at.elapsed().as_millis(),
        "translation request completed"
    );

    Ok(ExecutionResult::success(
        Some(translated),
        Some(format!(
            "已使用 {} 进行翻译",
            translation_provider_label(provider, used_protocol)
        )),
        None,
        vec!["copy_text"],
        false,
    ))
}

pub fn extract_translate_payload(raw_text: &str) -> Option<&str> {
    extract_prefixed_payload(raw_text, &TRANSLATE_COMMAND_ALIASES)
}

fn translation_payload(raw_text: &str) -> &str {
    let trimmed = raw_text.trim_start();
    if trimmed.starts_with('/') {
        return extract_translate_payload(raw_text).unwrap_or("").trim();
    }

    raw_text.trim()
}

fn resolve_translation_provider(llm_settings: &LlmSettings) -> Result<&LlmProviderConfig> {
    let provider_id = llm_settings
        .translation_provider_id
        .as_deref()
        .context("没有配置翻译 LLM，请先在 AI 功能页选择一个条目")?;
    let provider = llm_settings
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .with_context(|| format!("翻译 LLM provider 不存在: {provider_id}"))?;
    validate_translation_provider(provider)?;
    Ok(provider)
}

fn validate_translation_provider(provider: &LlmProviderConfig) -> Result<()> {
    if provider.base_url.trim().is_empty() {
        bail!("翻译使用的 LLM provider base URL 不能为空");
    }

    if !provider.resolved_profile().can_handle_ai_task() {
        bail!("翻译使用的 LLM 模型不能为空");
    }

    Ok(())
}

fn translation_protocol(provider: &LlmProviderConfig) -> TranslationProtocol {
    match provider.resolved_profile().kind() {
        crate::domain::settings::ResolvedLlmProviderKind::Responses {
            configured: true, ..
        } => TranslationProtocol::Responses,
        crate::domain::settings::ResolvedLlmProviderKind::ChatCompletions { configured: true } => {
            TranslationProtocol::ChatCompletions
        }
        crate::domain::settings::ResolvedLlmProviderKind::Responses {
            configured: false, ..
        }
        | crate::domain::settings::ResolvedLlmProviderKind::ChatCompletions { configured: false }
        | crate::domain::settings::ResolvedLlmProviderKind::Embedding { .. } => {
            match provider.protocol {
                LlmProviderProtocol::Responses => TranslationProtocol::Responses,
                LlmProviderProtocol::ChatCompletions => TranslationProtocol::ChatCompletions,
            }
        }
    }
}

fn translation_provider_label(
    provider: &LlmProviderConfig,
    protocol: TranslationProtocol,
) -> String {
    let provider_label = provider
        .name
        .trim()
        .split('\n')
        .next()
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| provider.model_name().to_string());
    format!(
        "{provider_label} ({})",
        translation_protocol_label(protocol)
    )
}

fn translation_protocol_label(protocol: TranslationProtocol) -> &'static str {
    match protocol {
        TranslationProtocol::Responses => "responses",
        TranslationProtocol::ChatCompletions => "chat/completions",
    }
}

async fn request_translation(
    protocol: TranslationProtocol,
    client: &OpenAiCompatibleClient<'_, AsyncHttpClient>,
    model: &str,
    prompt: &str,
    payload: &str,
    callbacks: TranslationCallbacks<'_>,
) -> Result<(String, TranslationProtocol)> {
    let translated = if let Some(on_text_delta) = callbacks.on_text_delta {
        let translated = match protocol {
            TranslationProtocol::Responses => {
                translate_with_responses(client, model, prompt, payload, true, Some(on_text_delta))
                    .await
            }
            TranslationProtocol::ChatCompletions => {
                translate_with_chat_completions(
                    client,
                    model,
                    prompt,
                    payload,
                    true,
                    Some(on_text_delta),
                )
                .await
            }
        };

        match translated {
            Ok(translated) => translated,
            Err(error) if should_retry_without_stream(true, &error) => {
                tracing::warn!(
                    ?error,
                    protocol = translation_protocol_label(protocol),
                    "translation provider rejected streaming output, retrying without stream"
                );
                match protocol {
                    TranslationProtocol::Responses => {
                        translate_with_responses(client, model, prompt, payload, false, None)
                            .await?
                    }
                    TranslationProtocol::ChatCompletions => {
                        translate_with_chat_completions(client, model, prompt, payload, false, None)
                            .await?
                    }
                }
            }
            Err(error) => return Err(error),
        }
    } else {
        match protocol {
            TranslationProtocol::Responses => {
                translate_with_responses(client, model, prompt, payload, false, None).await?
            }
            TranslationProtocol::ChatCompletions => {
                translate_with_chat_completions(client, model, prompt, payload, false, None).await?
            }
        }
    };

    Ok((translated, protocol))
}

async fn translate_with_responses(
    client: &OpenAiCompatibleClient<'_, AsyncHttpClient>,
    model: &str,
    prompt: &str,
    payload: &str,
    streaming_enabled: bool,
    on_text_delta: Option<&mut (dyn FnMut(&str) + Send)>,
) -> Result<String> {
    let request_body =
        build_responses_translation_request_body(model, prompt, payload, streaming_enabled);
    let payload: Value = if streaming_enabled {
        client
            .post_json_with_text_stream(
                "/responses",
                &request_body,
                "translation from responses API",
                OpenAiCompatibleResponseFormat::JsonOrSse,
                on_text_delta.context("translation streaming callback is missing")?,
            )
            .await?
    } else {
        client
            .post_json(
                "/responses",
                &request_body,
                "translation from responses API",
                OpenAiCompatibleResponseFormat::JsonOrSse,
            )
            .await?
    };
    extract_responses_text(&payload).context("LLM provider responses 未返回可识别的译文")
}

async fn translate_with_chat_completions(
    client: &OpenAiCompatibleClient<'_, AsyncHttpClient>,
    model: &str,
    prompt: &str,
    payload: &str,
    streaming_enabled: bool,
    on_text_delta: Option<&mut (dyn FnMut(&str) + Send)>,
) -> Result<String> {
    let request_body =
        build_chat_completions_translation_request_body(model, prompt, payload, streaming_enabled);
    let payload: Value = if streaming_enabled {
        client
            .post_json_with_text_stream(
                "/chat/completions",
                &request_body,
                "translation from chat/completions API",
                OpenAiCompatibleResponseFormat::JsonOrSse,
                on_text_delta.context("translation streaming callback is missing")?,
            )
            .await?
    } else {
        client
            .post_json(
                "/chat/completions",
                &request_body,
                "translation from chat/completions API",
                OpenAiCompatibleResponseFormat::JsonOrSse,
            )
            .await?
    };
    extract_chat_completions_text(&payload).with_context(|| {
        format!(
            "LLM provider chat/completions 未返回可识别的译文: {}",
            describe_chat_completions_response_issue(&payload)
        )
    })
}

fn build_responses_translation_request_body(
    model: &str,
    prompt: &str,
    payload: &str,
    streaming_enabled: bool,
) -> Value {
    json!({
        "model": model,
        "stream": streaming_enabled,
        "thinking": {
            "type": "disabled",
        },
        "input": [
            {
                "role": "system",
                "content": [
                    { "type": "input_text", "text": prompt }
                ],
            },
            {
                "role": "user",
                "content": [
                    { "type": "input_text", "text": payload }
                ],
            },
        ],
    })
}

fn build_chat_completions_translation_request_body(
    model: &str,
    prompt: &str,
    payload: &str,
    streaming_enabled: bool,
) -> Value {
    json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": prompt,
            },
            {
                "role": "user",
                "content": payload,
            },
        ],
        "stream": streaming_enabled,
        "thinking": {
            "type": "disabled",
        },
    })
}

fn should_retry_without_stream(streaming_enabled: bool, error: &anyhow::Error) -> bool {
    if !streaming_enabled {
        return false;
    }

    let message = error.to_string().to_ascii_lowercase();
    (message.contains("stream") || message.contains("sse"))
        && [
            "unsupported",
            "not supported",
            "does not support",
            "disabled",
            "invalid",
            "unexpected",
        ]
        .iter()
        .any(|needle| message.contains(needle))
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

#[cfg(test)]
mod tests {
    use super::{
        build_chat_completions_translation_request_body, build_responses_translation_request_body,
        execute_translation_with_callbacks, extract_translate_payload,
        resolve_translation_provider, TranslationCallbacks,
    };
    use crate::domain::settings::{
        default_translation_prompt, LlmModelType, LlmProviderConfig, LlmProviderProtocol,
        LlmSettings, PromptsSettings,
    };
    use crate::infrastructure::openai_compatible::{
        extract_chat_completions_text, extract_responses_text,
    };
    use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use tokio::{net::TcpListener, task::JoinHandle};

    #[derive(Clone)]
    enum MockTranslationScenario {
        ChatRejectsStreaming,
        ResponsesRejectsStreaming,
    }

    #[derive(Clone)]
    struct MockTranslationState {
        round: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<serde_json::Value>>>,
        scenario: MockTranslationScenario,
    }

    fn provider() -> LlmProviderConfig {
        LlmProviderConfig {
            id: "default".to_string(),
            name: "Default".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: String::new(),
            model_type: LlmModelType::Llm,
            model: "gpt-4.1-mini".to_string(),
            supports_multimodal: false,
            ..LlmProviderConfig::default()
        }
    }

    fn llm_settings(base_url: String, protocol: LlmProviderProtocol) -> LlmSettings {
        let protocol_name = match protocol {
            LlmProviderProtocol::Responses => "responses",
            LlmProviderProtocol::ChatCompletions => "chat_completions",
        };
        let provider: LlmProviderConfig = serde_json::from_value(json!({
            "id": "translate-provider",
            "name": "Translate Provider",
            "baseUrl": base_url,
            "apiKey": "test-key",
            "modelType": "llm",
            "protocol": protocol_name,
            "model": "mock-model"
        }))
        .expect("failed to deserialize translation test provider");

        serde_json::from_value(json!({
            "providers": [provider],
            "translationProviderId": "translate-provider"
        }))
        .expect("failed to deserialize translation test settings")
    }

    async fn spawn_chat_translation_server(
        scenario: MockTranslationScenario,
    ) -> (String, Arc<Mutex<Vec<serde_json::Value>>>, JoinHandle<()>) {
        let state = MockTranslationState {
            round: Arc::new(AtomicUsize::new(0)),
            requests: Arc::new(Mutex::new(Vec::new())),
            scenario,
        };
        let shared_requests = state.requests.clone();
        let app = Router::new()
            .route("/v1/chat/completions", post(mock_chat_translation))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind chat translation server");
        let address = listener
            .local_addr()
            .expect("failed to read server address");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("chat translation server exited unexpectedly");
        });

        (format!("http://{address}/v1"), shared_requests, handle)
    }

    async fn spawn_responses_translation_server(
        scenario: MockTranslationScenario,
    ) -> (String, Arc<Mutex<Vec<serde_json::Value>>>, JoinHandle<()>) {
        let state = MockTranslationState {
            round: Arc::new(AtomicUsize::new(0)),
            requests: Arc::new(Mutex::new(Vec::new())),
            scenario,
        };
        let shared_requests = state.requests.clone();
        let app = Router::new()
            .route("/v1/responses", post(mock_responses_translation))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind responses translation server");
        let address = listener
            .local_addr()
            .expect("failed to read server address");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("responses translation server exited unexpectedly");
        });

        (format!("http://{address}/v1"), shared_requests, handle)
    }

    async fn mock_chat_translation(
        State(state): State<MockTranslationState>,
        Json(payload): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        state
            .requests
            .lock()
            .expect("failed to lock chat translation requests")
            .push(payload.clone());
        let round = state.round.fetch_add(1, Ordering::SeqCst);

        match (&state.scenario, round) {
            (MockTranslationScenario::ChatRejectsStreaming, 0)
                if payload.get("stream") == Some(&json!(true)) =>
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
            (MockTranslationScenario::ChatRejectsStreaming, _) => (
                StatusCode::OK,
                Json(json!({
                    "id": "chatcmpl-translate-fallback",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "已回退到非流式 chat 翻译。"
                        },
                        "finish_reason": "stop"
                    }]
                })),
            ),
            _ => unreachable!("unexpected chat translation scenario"),
        }
    }

    async fn mock_responses_translation(
        State(state): State<MockTranslationState>,
        Json(payload): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        state
            .requests
            .lock()
            .expect("failed to lock responses translation requests")
            .push(payload.clone());
        let round = state.round.fetch_add(1, Ordering::SeqCst);

        match (&state.scenario, round) {
            (MockTranslationScenario::ResponsesRejectsStreaming, 0)
                if payload.get("stream") == Some(&json!(true)) =>
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
            (MockTranslationScenario::ResponsesRejectsStreaming, _) => (
                StatusCode::OK,
                Json(json!({
                    "id": "responses-translate-fallback",
                    "output": [{
                        "type": "message",
                        "content": [{
                            "type": "output_text",
                            "text": "已回退到非流式 responses 翻译。"
                        }]
                    }]
                })),
            ),
            _ => unreachable!("unexpected responses translation scenario"),
        }
    }

    #[test]
    fn translate_command_extracts_payload() {
        assert_eq!(extract_translate_payload("/translate hello"), Some("hello"));
        assert_eq!(extract_translate_payload("/trhello"), Some("hello"));
        assert_eq!(extract_translate_payload("/fy 你好"), Some("你好"));
    }

    #[test]
    fn resolve_translation_provider_reads_explicit_translation_provider() {
        let provider = provider();
        let settings = LlmSettings {
            providers: vec![provider.clone()],
            translation_provider_id: Some(provider.id.clone()),
            ..LlmSettings::default()
        };

        let resolved = resolve_translation_provider(&settings).unwrap();
        assert_eq!(resolved.id, provider.id);
    }

    #[test]
    fn resolve_translation_provider_rejects_embedding_provider() {
        let mut provider = provider();
        provider.model_type = LlmModelType::Embedding;
        provider.model = "text-embedding-3-small".to_string();
        let settings = LlmSettings {
            providers: vec![provider.clone()],
            translation_provider_id: Some(provider.id.clone()),
            ..LlmSettings::default()
        };

        let error = resolve_translation_provider(&settings).unwrap_err();
        assert_eq!(error.to_string(), "翻译使用的 LLM 模型不能为空");
    }

    #[test]
    fn execute_translation_rejects_empty_payload() {
        let error = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                execute_translation_with_callbacks(
                    "/tr",
                    &PromptsSettings {
                        translation_prompt: default_translation_prompt(),
                        ..PromptsSettings::default()
                    },
                    &LlmSettings::default(),
                    TranslationCallbacks {
                        on_text_delta: None,
                    },
                )
                .await
            })
            .unwrap_err();

        assert_eq!(error.to_string(), "请输入要翻译的内容");
    }

    #[test]
    fn resolve_translation_provider_accepts_chat_completions_provider() {
        let mut provider = provider();
        provider.protocol = LlmProviderProtocol::ChatCompletions;
        let settings = LlmSettings {
            providers: vec![provider.clone()],
            translation_provider_id: Some(provider.id.clone()),
            ..LlmSettings::default()
        };

        let resolved = resolve_translation_provider(&settings).unwrap();
        assert_eq!(resolved.id, provider.id);
    }

    #[test]
    fn extract_responses_text_reads_output_text_fallback() {
        let payload = json!({
            "output": [
                {
                    "type": "message",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "translated text"
                        }
                    ]
                }
            ]
        });

        assert_eq!(
            extract_responses_text(&payload).as_deref(),
            Some("translated text")
        );
    }

    #[test]
    fn extract_chat_completions_text_reads_string_content() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "translated text"
                    }
                }
            ]
        });

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("translated text")
        );
    }

    #[test]
    fn extract_chat_completions_text_reads_array_content() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": [
                            {
                                "type": "text",
                                "text": "translated text"
                            }
                        ]
                    }
                }
            ]
        });

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("translated text")
        );
    }

    #[test]
    fn chat_translation_payload_uses_requested_stream_flag() {
        let payload = build_chat_completions_translation_request_body(
            "gpt-4.1-mini",
            "translate",
            "hello",
            false,
        );

        assert_eq!(
            payload.get("thinking"),
            Some(&json!({
                "type": "disabled",
            }))
        );
        assert_eq!(payload.get("stream"), Some(&json!(false)));
    }

    #[test]
    fn responses_translation_payload_uses_requested_stream_flag() {
        let payload =
            build_responses_translation_request_body("gpt-4.1-mini", "translate", "hello", false);

        assert_eq!(
            payload.get("thinking"),
            Some(&json!({
                "type": "disabled",
            }))
        );
        assert_eq!(payload.get("stream"), Some(&json!(false)));
    }

    #[tokio::test]
    async fn translation_falls_back_to_non_streaming_chat_provider() {
        let (base_url, requests, server_handle) =
            spawn_chat_translation_server(MockTranslationScenario::ChatRejectsStreaming).await;
        let llm_settings = llm_settings(base_url, LlmProviderProtocol::ChatCompletions);
        let mut ignore_delta = |_delta: &str| {};

        let result = execute_translation_with_callbacks(
            "hello",
            &PromptsSettings::default(),
            &llm_settings,
            TranslationCallbacks {
                on_text_delta: Some(&mut ignore_delta),
            },
        )
        .await
        .expect("translation should fall back to non-stream chat request");

        server_handle.abort();

        assert_eq!(
            result.primary_text.as_deref(),
            Some("已回退到非流式 chat 翻译。")
        );

        let recorded_requests = requests.lock().expect("failed to lock recorded requests");
        assert_eq!(recorded_requests.len(), 2);
        assert_eq!(recorded_requests[0]["stream"], json!(true));
        assert_eq!(recorded_requests[1]["stream"], json!(false));
    }

    #[tokio::test]
    async fn translation_falls_back_to_non_streaming_responses_provider() {
        let (base_url, requests, server_handle) =
            spawn_responses_translation_server(MockTranslationScenario::ResponsesRejectsStreaming)
                .await;
        let llm_settings = llm_settings(base_url, LlmProviderProtocol::Responses);
        let mut ignore_delta = |_delta: &str| {};

        let result = execute_translation_with_callbacks(
            "hello",
            &PromptsSettings::default(),
            &llm_settings,
            TranslationCallbacks {
                on_text_delta: Some(&mut ignore_delta),
            },
        )
        .await
        .expect("translation should fall back to non-stream responses request");

        server_handle.abort();

        assert_eq!(
            result.primary_text.as_deref(),
            Some("已回退到非流式 responses 翻译。")
        );

        let recorded_requests = requests.lock().expect("failed to lock recorded requests");
        assert_eq!(recorded_requests.len(), 2);
        assert_eq!(recorded_requests[0]["stream"], json!(true));
        assert_eq!(recorded_requests[1]["stream"], json!(false));
    }
}
