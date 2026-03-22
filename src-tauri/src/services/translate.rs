use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde_json::{json, Value};

use crate::domain::{
    execution::ExecutionResult,
    settings::{LlmProviderConfig, LlmProviderProtocol, LlmSettings, PromptsSettings},
};
use crate::infrastructure::openai_compatible::{
    extract_chat_completions_text, extract_provider_error_message, extract_responses_text,
    normalize_base_url, parse_json_or_sse_payload, parse_json_payload,
};

const TRANSLATE_COMMAND_ALIASES: [&str; 3] = ["/translate", "/fy", "/tr"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranslationProtocol {
    Responses,
    ChatCompletions,
}

pub fn execute_translation(
    raw_text: &str,
    prompts_settings: &PromptsSettings,
    llm_settings: &LlmSettings,
) -> Result<ExecutionResult> {
    let payload = translation_payload(raw_text);
    if payload.is_empty() {
        bail!("请输入要翻译的内容");
    }

    let provider = resolve_translation_provider(llm_settings)?;
    let base_url = normalize_base_url(&provider.base_url, "LLM provider base URL")?;
    let api_key = provider.api_key.trim();
    let model = provider
        .llm_model_name()
        .context("翻译使用的 LLM 模型不能为空")?;
    let protocol = translation_protocol(provider);

    let client = Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .context("failed to build HTTP client for translation")?;

    let (translated, used_protocol) = match request_translation(
        protocol,
        &client,
        &base_url,
        api_key,
        model,
        &prompts_settings.translation_prompt,
        payload,
    ) {
        Ok(result) => result,
        Err(primary_error) => {
            let fallback_protocol = alternate_translation_protocol(protocol);
            if !should_retry_with_protocol(protocol, &primary_error) {
                return Err(primary_error);
            }

            match request_translation(
                fallback_protocol,
                &client,
                &base_url,
                api_key,
                model,
                &prompts_settings.translation_prompt,
                payload,
            ) {
                Ok(result) => result,
                Err(_) => return Err(primary_error),
            }
        }
    };

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

    if provider.llm_model_name().is_none() {
        bail!("翻译使用的 LLM 模型不能为空");
    }

    Ok(())
}

fn translation_protocol(provider: &LlmProviderConfig) -> TranslationProtocol {
    match provider.protocol {
        LlmProviderProtocol::Responses => TranslationProtocol::Responses,
        LlmProviderProtocol::ChatCompletions => TranslationProtocol::ChatCompletions,
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

fn alternate_translation_protocol(protocol: TranslationProtocol) -> TranslationProtocol {
    match protocol {
        TranslationProtocol::Responses => TranslationProtocol::ChatCompletions,
        TranslationProtocol::ChatCompletions => TranslationProtocol::Responses,
    }
}

fn request_translation(
    protocol: TranslationProtocol,
    client: &Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
    payload: &str,
) -> Result<(String, TranslationProtocol)> {
    let translated = match protocol {
        TranslationProtocol::Responses => {
            translate_with_responses(client, base_url, api_key, model, prompt, payload)?
        }
        TranslationProtocol::ChatCompletions => {
            translate_with_chat_completions(client, base_url, api_key, model, prompt, payload)?
        }
    };

    Ok((translated, protocol))
}

fn should_retry_with_protocol(protocol: TranslationProtocol, error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    match protocol {
        TranslationProtocol::Responses => {
            message.contains("route protocol chat")
                || message.contains("stateless responses requests")
                || retryable_endpoint_mismatch(&message, "responses")
        }
        TranslationProtocol::ChatCompletions => {
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

fn translate_with_responses(
    client: &Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
    payload: &str,
) -> Result<String> {
    let mut request = client.post(format!("{base_url}/responses"));
    if !api_key.is_empty() {
        request = request.bearer_auth(api_key);
    }

    let response = request
        .json(&json!({
            "model": model,
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
        }))
        .send()
        .context("failed to request translation from responses API")?;

    extract_response_text(
        response.status(),
        response
            .text()
            .context("failed to read translation response body")?,
        extract_responses_text,
        "responses",
    )
}

fn translate_with_chat_completions(
    client: &Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
    payload: &str,
) -> Result<String> {
    let mut request = client.post(format!("{base_url}/chat/completions"));
    if !api_key.is_empty() {
        request = request.bearer_auth(api_key);
    }

    let response = request
        .json(&json!({
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
            "stream": false,
        }))
        .send()
        .context("failed to request translation from chat/completions API")?;

    extract_response_text(
        response.status(),
        response
            .text()
            .context("failed to read translation response body")?,
        extract_chat_completions_text,
        "chat/completions",
    )
}

fn extract_response_text(
    status: reqwest::StatusCode,
    body: String,
    extractor: fn(&Value) -> Option<String>,
    endpoint: &str,
) -> Result<String> {
    if !status.is_success() {
        let message = extract_provider_error_message(&body);
        bail!("LLM provider {endpoint} 请求失败 ({status}): {message}");
    }

    let payload = if endpoint == "responses" {
        parse_json_or_sse_payload(&body, "translation response from responses")?
    } else {
        parse_json_payload(&body, "translation response from chat/completions")?
    };
    extractor(&payload).with_context(|| format!("LLM provider {endpoint} 未返回可识别的译文"))
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
        alternate_translation_protocol, execute_translation, extract_translate_payload,
        resolve_translation_provider, should_retry_with_protocol, TranslationProtocol,
    };
    use crate::domain::settings::{
        default_translation_prompt, LlmModelType, LlmProviderConfig, LlmProviderProtocol,
        LlmSettings, PromptsSettings,
    };
    use crate::infrastructure::openai_compatible::{
        extract_chat_completions_text, extract_responses_text,
    };
    use serde_json::json;

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
        let error = execute_translation(
            "/tr",
            &PromptsSettings {
                translation_prompt: default_translation_prompt(),
                ..PromptsSettings::default()
            },
            &LlmSettings::default(),
        )
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
    fn protocol_retry_switches_to_other_endpoint() {
        assert_eq!(
            alternate_translation_protocol(TranslationProtocol::Responses),
            TranslationProtocol::ChatCompletions
        );
        assert_eq!(
            alternate_translation_protocol(TranslationProtocol::ChatCompletions),
            TranslationProtocol::Responses
        );
    }

    #[test]
    fn protocol_retry_detects_chat_only_route_mismatch() {
        let error = anyhow::anyhow!(
            "LLM provider responses 请求失败 (400 Bad Request): route protocol chat does not support stateless responses requests"
        );
        assert!(should_retry_with_protocol(
            TranslationProtocol::Responses,
            &error
        ));
        assert!(!should_retry_with_protocol(
            TranslationProtocol::ChatCompletions,
            &error
        ));
    }

    #[test]
    fn protocol_retry_detects_responses_only_route_mismatch() {
        let error = anyhow::anyhow!(
            "LLM provider chat/completions 请求失败 (400 Bad Request): route protocol responses does not support chat/completions requests"
        );
        assert!(should_retry_with_protocol(
            TranslationProtocol::ChatCompletions,
            &error
        ));
        assert!(!should_retry_with_protocol(
            TranslationProtocol::Responses,
            &error
        ));
    }

    #[test]
    fn protocol_retry_detects_missing_responses_endpoint_by_status() {
        let error = anyhow::anyhow!("LLM provider responses 请求失败 (404 Not Found): not found");

        assert!(should_retry_with_protocol(
            TranslationProtocol::Responses,
            &error
        ));
    }
}
