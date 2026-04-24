use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use reqwest::Client as AsyncHttpClient;
use serde_json::{json, Value};

use crate::domain::{
    execution::ExecutionResult,
    settings::{LlmProviderProtocol, LlmSettings, PromptsSettings, ResolvedLlmModelBinding},
};
use crate::infrastructure::openai_compatible::{
    describe_chat_completions_response_issue, extract_chat_completions_message_parts,
    extract_text_content, OpenAiCompatibleClient, OpenAiCompatibleResponseFormat,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct TranslationOutputParts {
    content: String,
    reasoning: Option<String>,
}

#[derive(Debug, Default)]
struct TranslationStreamFilter {
    raw: String,
    emitted_visible: String,
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

    let binding = resolve_translation_model_binding(llm_settings)?;
    let model = binding
        .model_name()
        .context("翻译使用的 LLM 模型不能为空")?;
    let protocol = translation_protocol(binding);

    let http_client = shared_translation_http_client()?;
    let client = OpenAiCompatibleClient::new_async(
        &http_client,
        &binding.provider().base_url,
        &binding.provider().api_key,
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
        provider_id = %binding.provider().id,
        model_id = %binding.model().id,
        protocol = %translation_protocol_label(used_protocol),
        payload_chars = payload.chars().count(),
        elapsed_ms = started_at.elapsed().as_millis(),
        "translation request completed"
    );

    Ok(ExecutionResult::success(
        Some(translated.content),
        Some(format!(
            "已使用 {} 进行翻译",
            translation_provider_label(binding, used_protocol)
        )),
        translation_structured_payload(translated.reasoning),
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

fn resolve_translation_model_binding(
    llm_settings: &LlmSettings,
) -> Result<ResolvedLlmModelBinding<'_>> {
    let model_id = llm_settings
        .translation_model_id
        .as_deref()
        .context("没有配置翻译 LLM，请先在 AI 功能页选择一个条目")?;
    let binding = llm_settings
        .find_model_binding(model_id)
        .with_context(|| format!("翻译 LLM 模型不存在: {model_id}"))?;
    validate_translation_provider(binding)?;
    Ok(binding)
}

#[cfg(test)]
fn resolve_translation_provider(llm_settings: &LlmSettings) -> Result<ResolvedLlmModelBinding<'_>> {
    resolve_translation_model_binding(llm_settings)
}

fn validate_translation_provider(binding: ResolvedLlmModelBinding<'_>) -> Result<()> {
    if binding.provider().base_url.trim().is_empty() {
        bail!("翻译使用的 LLM provider base URL 不能为空");
    }

    if !binding.can_handle_ai_task() {
        bail!("翻译使用的 LLM 模型不能为空");
    }

    Ok(())
}

fn translation_protocol(binding: ResolvedLlmModelBinding<'_>) -> TranslationProtocol {
    match binding.kind() {
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
            match binding.provider().protocol {
                LlmProviderProtocol::Responses => TranslationProtocol::Responses,
                LlmProviderProtocol::ChatCompletions => TranslationProtocol::ChatCompletions,
            }
        }
    }
}

fn translation_provider_label(
    binding: ResolvedLlmModelBinding<'_>,
    protocol: TranslationProtocol,
) -> String {
    let provider_label = binding
        .provider()
        .name
        .trim()
        .split('\n')
        .next()
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| binding.model().model.trim().to_string());
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
) -> Result<(TranslationOutputParts, TranslationProtocol)> {
    let translated = if let Some(on_text_delta) = callbacks.on_text_delta {
        let mut stream_filter = TranslationStreamFilter::default();
        let mut filtered_on_text_delta = |delta: &str| {
            if let Some(visible_delta) = stream_filter.push(delta) {
                on_text_delta(visible_delta);
            }
        };
        let translated = match protocol {
            TranslationProtocol::Responses => {
                translate_with_responses(
                    client,
                    model,
                    prompt,
                    payload,
                    true,
                    Some(&mut filtered_on_text_delta),
                )
                .await
            }
            TranslationProtocol::ChatCompletions => {
                translate_with_chat_completions(
                    client,
                    model,
                    prompt,
                    payload,
                    true,
                    Some(&mut filtered_on_text_delta),
                )
                .await
            }
        };

        match translated {
            Ok(translated) => translated,
            Err(error) if should_retry_without_stream(true, &error) => {
                tracing::warn!(
                    error = format_args!("{:#}", error),
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
) -> Result<TranslationOutputParts> {
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
    extract_responses_translation_parts(&payload)
}

async fn translate_with_chat_completions(
    client: &OpenAiCompatibleClient<'_, AsyncHttpClient>,
    model: &str,
    prompt: &str,
    payload: &str,
    streaming_enabled: bool,
    on_text_delta: Option<&mut (dyn FnMut(&str) + Send)>,
) -> Result<TranslationOutputParts> {
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
    extract_chat_completions_translation_parts(&payload)
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
    let stream_related =
        message.contains("stream") || message.contains("sse") || message.contains("response body");
    if !stream_related {
        return false;
    }

    let stream_unsupported = [
        "unsupported",
        "not supported",
        "does not support",
        "disabled",
        "invalid",
        "unexpected",
    ]
    .iter()
    .any(|needle| message.contains(needle));
    if stream_unsupported {
        return true;
    }

    error
        .chain()
        .filter_map(|source| source.downcast_ref::<reqwest::Error>())
        .any(reqwest::Error::is_timeout)
        || message.contains("timed out")
        || message.contains("timeout")
}

fn translation_structured_payload(reasoning: Option<String>) -> Option<Value> {
    reasoning.map(|reasoning| {
        json!({
            "kind": "translation_result",
            "reasoning": reasoning,
        })
    })
}

fn extract_responses_translation_parts(payload: &Value) -> Result<TranslationOutputParts> {
    let mut content_segments = Vec::new();
    let mut reasoning_segments = Vec::new();

    if let Some(output_text) = trim_to_owned(payload.get("output_text").and_then(Value::as_str)) {
        content_segments.push(output_text);
    }

    for output in payload
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for content in output
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(segment) = extract_text_content(content)
                .as_deref()
                .and_then(|value| trim_to_owned(Some(value)))
            else {
                continue;
            };

            if content
                .get("type")
                .and_then(Value::as_str)
                .map(is_reasoning_content_type)
                .unwrap_or(false)
            {
                reasoning_segments.push(segment);
            } else {
                content_segments.push(segment);
            }
        }
    }

    let (content, reasoning) = normalize_translation_content_and_reasoning(
        join_non_empty_segments(content_segments),
        join_non_empty_segments(reasoning_segments),
    );

    build_translation_output_parts(
        content,
        reasoning,
        "LLM provider responses 未返回可识别的译文",
        "LLM provider responses 返回了思考内容，但没有可识别的译文",
    )
}

fn extract_chat_completions_translation_parts(payload: &Value) -> Result<TranslationOutputParts> {
    let missing_content_error = format!(
        "LLM provider chat/completions 未返回可识别的译文: {}",
        describe_chat_completions_response_issue(payload)
    );
    let parts = extract_chat_completions_message_parts(payload)
        .with_context(|| missing_content_error.clone())?;

    let (content, reasoning) = normalize_translation_content_and_reasoning(
        trim_to_owned(parts.content.as_deref()),
        trim_to_owned(parts.reasoning.as_deref()),
    );

    build_translation_output_parts(
        content,
        reasoning,
        &missing_content_error,
        "LLM provider chat/completions 返回了思考内容，但没有可识别的译文",
    )
}

fn build_translation_output_parts(
    content: Option<String>,
    reasoning: Option<String>,
    missing_content_error: &str,
    reasoning_only_error: &str,
) -> Result<TranslationOutputParts> {
    let content = match content {
        Some(content) => content,
        None if reasoning.is_some() => bail!(reasoning_only_error.to_string()),
        None => bail!(missing_content_error.to_string()),
    };

    Ok(TranslationOutputParts { content, reasoning })
}

fn trim_to_owned(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn join_non_empty_segments(segments: Vec<String>) -> Option<String> {
    let filtered = segments
        .into_iter()
        .map(|segment| segment.trim().to_string())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    (!filtered.is_empty()).then(|| filtered.join("\n"))
}

fn normalize_translation_content_and_reasoning(
    content: Option<String>,
    reasoning: Option<String>,
) -> (Option<String>, Option<String>) {
    let content = trim_to_owned(content.as_deref());
    let reasoning = trim_to_owned(reasoning.as_deref());

    let Some(content) = content else {
        return (None, reasoning);
    };

    if let Some(extracted_content) = extract_visible_translation_candidate(&content) {
        let merged_reasoning = join_non_empty_segments(
            [reasoning, Some(content)]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
        );
        return (Some(extracted_content), merged_reasoning);
    }

    if looks_like_translation_reasoning_scaffold(&content) {
        let merged_reasoning = join_non_empty_segments(
            [reasoning, Some(content)]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
        );
        return (None, merged_reasoning);
    }

    (Some(content), reasoning)
}

fn extract_visible_translation_candidate(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    for label in [
        "最终译文：",
        "最终译文:",
        "最终翻译：",
        "最终翻译:",
        "译文：",
        "译文:",
        "翻译：",
        "翻译:",
        "final translation:",
        "final translation：",
        "finaltranslation:",
        "finaltranslation：",
        "translated text:",
        "translated text：",
        "translatedtext:",
        "translatedtext：",
    ] {
        let lower = trimmed.to_ascii_lowercase();
        let label_lower = label.to_ascii_lowercase();
        let Some(index) = lower.rfind(&label_lower) else {
            continue;
        };

        let start = index + label_lower.len();
        let Some(suffix) = trimmed.get(start..) else {
            continue;
        };
        let candidate = trim_to_owned(Some(suffix.trim_start_matches(['*', '-', ' ', '\t'])));
        if let Some(candidate) =
            candidate.filter(|value| !looks_like_translation_reasoning_scaffold(value))
        {
            return Some(candidate);
        }
    }

    None
}

fn looks_like_translation_reasoning_scaffold(value: &str) -> bool {
    let normalized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(|character| character.to_lowercase())
        .collect::<String>();

    if normalized.is_empty() {
        return false;
    }

    if normalized.starts_with("thinkingprocess")
        || normalized.starts_with("thoughtprocess")
        || normalized.starts_with("analysis")
    {
        return true;
    }

    let markers = [
        "analyzetherequest",
        "analyzethesourcetext",
        "determinethetargettranslation",
        "returnonlythetranslation",
        "inputtext",
        "partofspeech",
        "simplifiedchinesetranslation",
        "commonmedicalcontext",
        "processfieldofstudy",
        "likelymedicaltechnicalorcomputerrelated",
    ];

    markers
        .iter()
        .filter(|marker| normalized.contains(**marker))
        .count()
        >= 2
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

impl TranslationStreamFilter {
    fn push<'a>(&'a mut self, delta: &str) -> Option<&'a str> {
        if delta.is_empty() {
            return None;
        }

        self.raw.push_str(delta);
        let next_visible = if let Some(candidate) = extract_visible_translation_candidate(&self.raw)
        {
            candidate
        } else if looks_like_translation_reasoning_scaffold(&self.raw) {
            String::new()
        } else {
            self.raw.clone()
        };

        if next_visible.len() <= self.emitted_visible.len()
            || !next_visible.starts_with(&self.emitted_visible)
        {
            if next_visible.is_empty() {
                self.emitted_visible.clear();
            }
            return None;
        }

        let suffix_start = self.emitted_visible.len();
        self.emitted_visible = next_visible;
        Some(&self.emitted_visible[suffix_start..])
    }
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
        execute_translation_with_callbacks, extract_chat_completions_translation_parts,
        extract_responses_translation_parts, extract_translate_payload,
        extract_visible_translation_candidate, looks_like_translation_reasoning_scaffold,
        resolve_translation_provider, should_retry_without_stream, TranslationCallbacks,
        TranslationStreamFilter,
    };
    use crate::domain::settings::{
        default_translation_prompt, LlmModelType, LlmProviderConfig, LlmProviderProtocol,
        LlmSettings, PromptsSettings,
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
        ChatReturnsContentAndReasoning,
        ChatReturnsReasoningOnly,
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
            models: vec![crate::domain::settings::LlmModelConfig {
                id: "default".to_string(),
                model_type: LlmModelType::Llm,
                model: "gpt-4.1-mini".to_string(),
                supports_multimodal: false,
                ..crate::domain::settings::LlmModelConfig::default()
            }],
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
            "protocol": protocol_name,
            "models": [{
                "id": "translate-provider",
                "modelType": "llm",
                "model": "mock-model"
            }]
        }))
        .expect("failed to deserialize translation test provider");

        serde_json::from_value(json!({
            "providers": [provider],
            "translationModelId": "translate-provider"
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
            (MockTranslationScenario::ChatReturnsContentAndReasoning, _) => (
                StatusCode::OK,
                Json(json!({
                    "id": "chatcmpl-translate-reasoning",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "最终译文",
                            "reasoning_content": "先判断语言方向，再保留原文语气。"
                        },
                        "finish_reason": "stop"
                    }]
                })),
            ),
            (MockTranslationScenario::ChatReturnsReasoningOnly, _) => (
                StatusCode::OK,
                Json(json!({
                    "id": "chatcmpl-translate-reasoning-only",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "",
                            "reasoning_content": "这里只有思考，没有译文。"
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
            translation_model_id: Some(provider.models[0].id.clone()),
            ..LlmSettings::default()
        };

        let resolved = resolve_translation_provider(&settings).unwrap();
        assert_eq!(resolved.provider().id, provider.id);
    }

    #[test]
    fn resolve_translation_provider_rejects_embedding_provider() {
        let mut provider = provider();
        provider.models[0].model_type = LlmModelType::Embedding;
        provider.models[0].model = "text-embedding-3-small".to_string();
        let settings = LlmSettings {
            providers: vec![provider.clone()],
            translation_model_id: Some(provider.models[0].id.clone()),
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
            translation_model_id: Some(provider.models[0].id.clone()),
            ..LlmSettings::default()
        };

        let resolved = resolve_translation_provider(&settings).unwrap();
        assert_eq!(resolved.provider().id, provider.id);
    }

    #[test]
    fn extract_responses_translation_parts_separates_reasoning() {
        let payload = json!({
            "output": [
                {
                    "type": "message",
                    "content": [
                        {
                            "type": "reasoning",
                            "text": "reasoning text"
                        },
                        {
                            "type": "output_text",
                            "text": "translated text"
                        }
                    ]
                }
            ]
        });

        let parts = extract_responses_translation_parts(&payload)
            .expect("responses translation payload should parse");

        assert_eq!(parts.content, "translated text");
        assert_eq!(parts.reasoning.as_deref(), Some("reasoning text"));
    }

    #[test]
    fn extract_chat_completions_translation_parts_reads_string_content() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "translated text",
                        "reasoning_content": "reasoning text"
                    }
                }
            ]
        });

        let parts = extract_chat_completions_translation_parts(&payload)
            .expect("chat/completions translation payload should parse");

        assert_eq!(parts.content, "translated text");
        assert_eq!(parts.reasoning.as_deref(), Some("reasoning text"));
    }

    #[test]
    fn extract_chat_completions_translation_parts_reads_array_content() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": [
                            {
                                "type": "text",
                                "text": "translated text"
                            },
                            {
                                "type": "thinking",
                                "text": "reasoning text"
                            }
                        ]
                    }
                }
            ]
        });

        let parts = extract_chat_completions_translation_parts(&payload)
            .expect("chat/completions translation payload should parse");

        assert_eq!(parts.content, "translated text");
        assert_eq!(parts.reasoning.as_deref(), Some("reasoning text"));
    }

    #[test]
    fn extract_chat_completions_translation_parts_rejects_reasoning_only_payload() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "reasoning_content": "only reasoning"
                    }
                }
            ]
        });

        let error = extract_chat_completions_translation_parts(&payload)
            .expect_err("reasoning-only payload must be rejected");

        assert_eq!(
            error.to_string(),
            "LLM provider chat/completions 返回了思考内容，但没有可识别的译文"
        );
    }

    #[test]
    fn reasoning_scaffold_detector_catches_plain_text_thinking() {
        let content = "ThinkingProcess:1.**AnalyzetheRequest:**Inputtext:\"diagnostics\"\
            2.**AnalyzetheSourceText:**PartofSpeech:Noun\
            3.**DeterminetheTargetTranslation:**SimplifiedChinesetranslation:";

        assert!(looks_like_translation_reasoning_scaffold(content));
    }

    #[test]
    fn visible_translation_candidate_extracts_labeled_suffix() {
        let content = "Thinking process...\nFinal translation: 诊断";

        assert_eq!(
            extract_visible_translation_candidate(content).as_deref(),
            Some("诊断")
        );
    }

    #[test]
    fn visible_translation_candidate_extracts_ocr_collapsed_label_suffix() {
        let content = "Thinking process...\nFinaltranslation: 诊断";

        assert_eq!(
            extract_visible_translation_candidate(content).as_deref(),
            Some("诊断")
        );
    }

    #[test]
    fn translation_stream_filter_suppresses_reasoning_scaffold_chunks() {
        let mut filter = TranslationStreamFilter::default();

        assert_eq!(filter.push("ThinkingProcess:"), None);
        assert_eq!(filter.push("1.**AnalyzetheRequest:**"), None);
        assert_eq!(filter.push("Inputtext:\"diagnostics\""), None);
    }

    #[test]
    fn translation_stream_filter_extracts_ocr_collapsed_label_chunks() {
        let mut filter = TranslationStreamFilter::default();

        assert_eq!(filter.push("ThinkingProcess:"), None);
        assert_eq!(filter.push("Finaltranslation:"), None);
        assert_eq!(filter.push("诊"), Some("诊"));
        assert_eq!(filter.push("断"), Some("断"));
    }

    #[test]
    fn translation_stream_filter_passes_plain_translation_chunks() {
        let mut filter = TranslationStreamFilter::default();

        assert_eq!(filter.push("诊"), Some("诊"));
        assert_eq!(filter.push("断"), Some("断"));
    }

    #[test]
    fn should_retry_without_stream_for_stream_timeout() {
        let error = anyhow::anyhow!(
            "failed to read translation from chat/completions API response stream: error decoding response body: request or response body error: operation timed out"
        );

        assert!(should_retry_without_stream(true, &error));
    }

    #[test]
    fn should_not_retry_without_stream_for_non_stream_timeout() {
        let error = anyhow::anyhow!(
            "failed to request translation from chat/completions API: operation timed out"
        );

        assert!(!should_retry_without_stream(true, &error));
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

    #[tokio::test]
    async fn translation_keeps_reasoning_as_structured_secondary_content() {
        let (base_url, _requests, server_handle) =
            spawn_chat_translation_server(MockTranslationScenario::ChatReturnsContentAndReasoning)
                .await;
        let llm_settings = llm_settings(base_url, LlmProviderProtocol::ChatCompletions);

        let result = execute_translation_with_callbacks(
            "hello",
            &PromptsSettings::default(),
            &llm_settings,
            TranslationCallbacks {
                on_text_delta: None,
            },
        )
        .await
        .expect("translation should preserve reasoning as secondary content");

        server_handle.abort();

        assert_eq!(result.primary_text.as_deref(), Some("最终译文"));
        assert_eq!(
            result.structured_payload,
            Some(json!({
                "kind": "translation_result",
                "reasoning": "先判断语言方向，再保留原文语气。"
            }))
        );
    }

    #[tokio::test]
    async fn translation_rejects_reasoning_only_chat_response() {
        let (base_url, _requests, server_handle) =
            spawn_chat_translation_server(MockTranslationScenario::ChatReturnsReasoningOnly).await;
        let llm_settings = llm_settings(base_url, LlmProviderProtocol::ChatCompletions);

        let error = execute_translation_with_callbacks(
            "hello",
            &PromptsSettings::default(),
            &llm_settings,
            TranslationCallbacks {
                on_text_delta: None,
            },
        )
        .await
        .expect_err("reasoning-only chat response must be rejected");

        server_handle.abort();

        assert_eq!(
            error.to_string(),
            "LLM provider chat/completions 返回了思考内容，但没有可识别的译文"
        );
    }
}
