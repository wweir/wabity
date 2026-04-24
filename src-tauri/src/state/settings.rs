use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use reqwest::blocking::Client as BlockingHttpClient;

#[cfg(target_os = "macos")]
use crate::services::ocr::MacOsVisionOcrProvider;
use crate::{
    domain::settings::{
        find_builtin_llm_provider_template, BuiltinLlmTemplateModelProtocol,
        BuiltinLlmTemplateModelType, LlmModelConfig, LlmProviderConfig, LlmProviderModelEntry,
        LlmProviderProtocol, LlmSettings, OcrProviderKind, OcrSettings, RagSettings,
        ResolvedLlmModelBinding,
    },
    infrastructure::openai_compatible::{extract_model_entries, OpenAiCompatibleClient},
    services::ocr::{OcrProvider, OpenAiCompatibleOcrProvider, UnavailableOcrProvider},
};

pub(super) fn build_ocr_provider(
    settings: &OcrSettings,
    llm_settings: &LlmSettings,
) -> Arc<dyn OcrProvider> {
    match settings.provider {
        OcrProviderKind::Disabled => Arc::new(UnavailableOcrProvider::new(
            "ocr provider is disabled for image path",
        )),
        OcrProviderKind::System => build_system_ocr_provider(),
        OcrProviderKind::LlmOcr => {
            match resolve_ocr_model_binding(settings, llm_settings).and_then(|binding| {
                if !binding.can_handle_ocr() {
                    anyhow::bail!("OCR 选择的 LLM 配置未启用多模态能力")
                }
                OpenAiCompatibleOcrProvider::from_binding(binding)
            }) {
                Ok(provider) => Arc::new(provider),
                Err(error) => Arc::new(UnavailableOcrProvider::new(format!(
                    "llm_ocr provider is misconfigured ({error:#}) for image path"
                ))),
            }
        }
    }
}

pub(super) fn validate_llm_settings(settings: &LlmSettings) -> Result<()> {
    let mut seen_model_ids = std::collections::HashSet::new();
    for (index, provider) in settings.providers.iter().enumerate() {
        validate_llm_provider_config(provider)
            .with_context(|| format!("第 {} 个 LLM provider 配置非法", index + 1))?;
        for model in &provider.models {
            if !seen_model_ids.insert(model.id.as_str()) {
                anyhow::bail!("LLM model id 不能重复: {}", model.id);
            }
        }
    }

    validate_llm_model_reference(
        settings,
        settings.translation_model_id.as_deref(),
        "翻译 LLM 模型",
        |binding| binding.can_handle_ai_task(),
    )?;
    validate_llm_model_reference(
        settings,
        settings.question_answer_model_id.as_deref(),
        "问答 LLM 模型",
        |binding| binding.can_handle_ai_task(),
    )?;

    Ok(())
}

pub(super) fn validate_ocr_settings(
    settings: &OcrSettings,
    llm_settings: &LlmSettings,
) -> Result<()> {
    match settings.provider {
        OcrProviderKind::Disabled => Ok(()),
        OcrProviderKind::System => validate_system_ocr_settings(),
        OcrProviderKind::LlmOcr => {
            let binding = resolve_ocr_model_binding(settings, llm_settings)?;
            if !binding.can_handle_ocr() {
                anyhow::bail!("OCR 选择的 LLM 配置未启用多模态能力");
            }
            OpenAiCompatibleOcrProvider::from_binding(binding).map(|_| ())
        }
    }
}

pub(super) fn validate_rag_settings(
    settings: &RagSettings,
    llm_settings: &LlmSettings,
) -> Result<()> {
    for (index, directory) in settings.source_directories.iter().enumerate() {
        let trimmed = directory.trim();
        if trimmed.is_empty() {
            anyhow::bail!("第 {} 个 RAG 扫描目录不能为空", index + 1);
        }

        let path = std::path::Path::new(trimmed);
        if !path.exists() {
            anyhow::bail!("第 {} 个 RAG 扫描目录不存在: {trimmed}", index + 1);
        }
        if !path.is_dir() {
            anyhow::bail!("第 {} 个 RAG 扫描路径不是目录: {trimmed}", index + 1);
        }
    }

    for (index, pattern) in settings.ignore_globs.iter().enumerate() {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            anyhow::bail!("第 {} 个 RAG 忽略模式不能为空", index + 1);
        }

        globset::Glob::new(trimmed)
            .with_context(|| format!("第 {} 个 RAG 忽略模式非法: {trimmed}", index + 1))?;
    }

    if !settings.source_directories.is_empty() && settings.embedding_model_id.is_none() {
        anyhow::bail!("RAG 配置了扫描目录时，必须选择一个 embedding 模型");
    }

    if let Some(model_id) = settings.embedding_model_id.as_deref() {
        let binding = llm_settings
            .find_model_binding(model_id)
            .with_context(|| format!("RAG 选择的 embedding 模型不存在: {model_id}"))?;
        if !binding.can_handle_embedding() {
            anyhow::bail!("RAG 只接受启用了 embedding 能力的模型");
        }
        validate_llm_provider_config(binding.provider())
            .with_context(|| format!("RAG 选择的 embedding 模型配置非法: {model_id}"))?;
    }

    Ok(())
}

pub(super) fn fetch_llm_provider_models(
    provider: &LlmProviderConfig,
) -> Result<Vec<LlmProviderModelEntry>> {
    let http_client = BlockingHttpClient::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to build HTTP client for LLM model listing")?;
    let client = OpenAiCompatibleClient::new_blocking(
        &http_client,
        &provider.base_url,
        &provider.api_key,
        "LLM provider base URL",
    )?;
    let parsed: serde_json::Value = client.get_json("/models", "LLM model list from provider")?;
    let models = extract_model_entries(&parsed);
    if models.is_empty() {
        anyhow::bail!("LLM provider /models 未返回可识别的模型列表");
    }

    Ok(models)
}

fn validate_llm_model_reference(
    settings: &LlmSettings,
    model_id: Option<&str>,
    label: &str,
    predicate: impl for<'a> Fn(ResolvedLlmModelBinding<'a>) -> bool,
) -> Result<()> {
    let Some(model_id) = model_id else {
        return Ok(());
    };

    if settings
        .find_model_binding(model_id)
        .filter(|binding| predicate(*binding))
        .is_some()
    {
        return Ok(());
    }

    anyhow::bail!("{label} 不存在或能力不匹配: {model_id}");
}

fn resolve_ocr_model_binding<'a>(
    ocr_settings: &OcrSettings,
    llm_settings: &'a LlmSettings,
) -> Result<ResolvedLlmModelBinding<'a>> {
    let model_id = ocr_settings
        .llm_model_id
        .as_deref()
        .context("OCR provider 为 llm_ocr 时必须选择一个 LLM 模型")?;

    llm_settings
        .find_model_binding(model_id)
        .with_context(|| format!("OCR 选择的 LLM 模型不存在: {model_id}"))
}

pub(super) fn validate_llm_provider_config(provider: &LlmProviderConfig) -> Result<()> {
    if provider.id.trim().is_empty() {
        anyhow::bail!("LLM provider id 不能为空");
    }
    if provider.name.trim().is_empty() {
        anyhow::bail!("LLM provider 名称不能为空");
    }
    if provider.base_url.trim().is_empty() {
        anyhow::bail!("LLM provider base URL 不能为空");
    }
    if provider.models.is_empty() {
        anyhow::bail!("LLM provider 至少需要配置一个模型");
    }

    let mut seen_model_ids = std::collections::HashSet::new();
    for model in &provider.models {
        validate_llm_model_config(provider, model)?;
        if !seen_model_ids.insert(model.id.as_str()) {
            anyhow::bail!("同一个 LLM provider 下的 model id 不能重复: {}", model.id);
        }
    }

    Ok(())
}

fn validate_llm_model_config(provider: &LlmProviderConfig, model: &LlmModelConfig) -> Result<()> {
    if model.id.trim().is_empty() {
        anyhow::bail!("LLM model id 不能为空");
    }
    let binding = ResolvedLlmModelBinding::new(provider, model);
    if binding.model_name().is_none() {
        anyhow::bail!("LLM model 不能为空");
    }
    if model.model_type == crate::domain::settings::LlmModelType::Embedding
        && provider.protocol != LlmProviderProtocol::Responses
    {
        anyhow::bail!("Embedding 模型当前只能和 responses provider 一起使用");
    }
    if model.supports_multimodal
        && model.model_type == crate::domain::settings::LlmModelType::Llm
        && !binding.uses_responses_api()
    {
        anyhow::bail!("LLM 多模态开关当前只能和 responses 协议一起使用");
    }
    if model.supports_stateful && !binding.uses_responses_api() {
        anyhow::bail!("stateful 开关只能和 responses 协议一起使用");
    }
    validate_builtin_llm_provider_binding(provider, model)?;

    Ok(())
}

fn validate_builtin_llm_provider_binding(
    provider: &LlmProviderConfig,
    model: &LlmModelConfig,
) -> Result<()> {
    let Some(template_id) = provider.builtin_preset_id.as_deref() else {
        return Ok(());
    };
    let template = find_builtin_llm_provider_template(template_id)
        .with_context(|| format!("未知内置 LLM 模板: {template_id}"))?;

    if provider.managed_base_url && provider.base_url.trim() != template.default_base_url {
        anyhow::bail!("内置模板条目的 Base URL 必须与模板默认值一致，或先关闭模板管理");
    }

    let Some(model_id) = model.builtin_preset_model_id.as_deref() else {
        return Ok(());
    };
    let template_model = template
        .models
        .iter()
        .find(|candidate| candidate.id == model_id)
        .with_context(|| format!("内置模板模型不存在: {model_id}"))?;

    if !template_model.selectable_in_current_app {
        anyhow::bail!("内置模板模型当前不可在 Wabity 中使用: {model_id}");
    }
    if model.model.trim() != template_model.model {
        anyhow::bail!("内置模板条目的模型必须来自模板白名单");
    }

    match template_model.model_type {
        BuiltinLlmTemplateModelType::Llm => {
            if model.model_type != crate::domain::settings::LlmModelType::Llm {
                anyhow::bail!("内置模板模型类型与 provider 类型不一致");
            }
        }
        BuiltinLlmTemplateModelType::Embedding => {
            if model.model_type != crate::domain::settings::LlmModelType::Embedding {
                anyhow::bail!("内置模板模型类型与 provider 类型不一致");
            }
        }
        BuiltinLlmTemplateModelType::ImageGeneration
        | BuiltinLlmTemplateModelType::VideoGeneration => {
            anyhow::bail!("当前不支持把图像或视频生成模型保存为 LLM provider");
        }
    }

    let expected_protocol = match template_model.protocol {
        BuiltinLlmTemplateModelProtocol::Responses => {
            crate::domain::settings::LlmProviderProtocol::Responses
        }
        BuiltinLlmTemplateModelProtocol::ChatCompletions => {
            crate::domain::settings::LlmProviderProtocol::ChatCompletions
        }
        BuiltinLlmTemplateModelProtocol::Unsupported => {
            anyhow::bail!("当前内置模板模型没有可用的 LLM 协议");
        }
    };
    if provider.protocol != expected_protocol {
        anyhow::bail!("内置模板条目的协议与模板目录不一致");
    }

    let expected_multimodal = template_model.supports_multimodal
        && expected_protocol == crate::domain::settings::LlmProviderProtocol::Responses;
    if model.supports_multimodal != expected_multimodal {
        anyhow::bail!("内置模板条目的多模态能力与模板目录不一致");
    }

    let expected_stateful = template_model.supports_stateful
        && expected_protocol == crate::domain::settings::LlmProviderProtocol::Responses;
    if model.supports_stateful != expected_stateful {
        anyhow::bail!("内置模板条目的 stateful 能力与模板目录不一致");
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn build_system_ocr_provider() -> Arc<dyn OcrProvider> {
    Arc::new(MacOsVisionOcrProvider)
}

#[cfg(target_os = "macos")]
fn validate_system_ocr_settings() -> Result<()> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn build_system_ocr_provider() -> Arc<dyn OcrProvider> {
    Arc::new(UnavailableOcrProvider::new(
        "system OCR provider is only available on macOS for image path",
    ))
}

#[cfg(not(target_os = "macos"))]
fn validate_system_ocr_settings() -> Result<()> {
    anyhow::bail!("system OCR provider is only available on macOS")
}
