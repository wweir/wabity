use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize};

use crate::domain::notification::NotificationSettings;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralSettings {
    pub auto_start: bool,
    pub show_in_dock: bool,
    pub language: String,
    #[serde(default, skip_serializing, alias = "translationPrompt")]
    legacy_translation_prompt: Option<String>,
}

impl GeneralSettings {
    pub fn normalize(&mut self) {}

    pub fn take_legacy_translation_prompt(&mut self) -> Option<String> {
        self.legacy_translation_prompt.take()
    }
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            auto_start: false,
            show_in_dock: true,
            language: "zh-CN".to_string(),
            legacy_translation_prompt: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptsSettings {
    #[serde(default = "default_translation_prompt")]
    pub translation_prompt: String,
    #[serde(default = "default_rag_answer_system_prompt")]
    pub rag_answer_system_prompt: String,
}

impl PromptsSettings {
    pub fn normalize(&mut self) {
        let normalized_translation_prompt = self.translation_prompt.trim();
        if normalized_translation_prompt.is_empty() {
            self.translation_prompt = default_translation_prompt();
        } else {
            self.translation_prompt = normalized_translation_prompt.to_string();
        }

        let normalized_rag_answer_prompt = self.rag_answer_system_prompt.trim();
        if normalized_rag_answer_prompt.is_empty() {
            self.rag_answer_system_prompt = default_rag_answer_system_prompt();
        } else {
            self.rag_answer_system_prompt = normalized_rag_answer_prompt.to_string();
        }
    }

    pub fn adopt_legacy_translation_prompt(&mut self, legacy_prompt: Option<String>) {
        let Some(legacy_prompt) = legacy_prompt else {
            return;
        };
        let trimmed_prompt = legacy_prompt.trim();
        if trimmed_prompt.is_empty() || self.translation_prompt != default_translation_prompt() {
            return;
        }

        self.translation_prompt = trimmed_prompt.to_string();
    }
}

impl Default for PromptsSettings {
    fn default() -> Self {
        Self {
            translation_prompt: default_translation_prompt(),
            rag_answer_system_prompt: default_rag_answer_system_prompt(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSettings {
    pub theme: String,
    pub font_size: String,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: "auto".to_string(),
            font_size: "medium".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrProviderKind {
    Disabled,
    System,
    LlmOcr,
}

#[allow(clippy::derivable_impls)]
impl Default for OcrProviderKind {
    fn default() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::System
        }

        #[cfg(not(target_os = "macos"))]
        {
            Self::Disabled
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyLlmProviderProtocolKind {
    #[serde(rename = "openai_chat", alias = "openai_compatible")]
    Chat,
    #[serde(rename = "openai_responses")]
    Responses,
    #[serde(rename = "openai_embedding")]
    Embedding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderProtocol {
    #[default]
    Responses,
    ChatCompletions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmModelType {
    #[default]
    Llm,
    Embedding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinLlmTemplateModelType {
    Llm,
    Embedding,
    ImageGeneration,
    VideoGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinLlmTemplateModelProtocol {
    Responses,
    ChatCompletions,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinLlmTemplateUseCase {
    Translation,
    RagAnswer,
    Ocr,
    Embedding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinLlmProviderTemplateModel {
    pub id: String,
    pub display_name: String,
    pub model: String,
    pub model_type: BuiltinLlmTemplateModelType,
    pub protocol: BuiltinLlmTemplateModelProtocol,
    pub supports_multimodal: bool,
    pub supports_stateful: bool,
    pub recommended_for: Vec<BuiltinLlmTemplateUseCase>,
    pub summary: String,
    pub selectable_in_current_app: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinLlmProviderTemplate {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub registration_label: String,
    pub registration_url: String,
    pub api_key_label: String,
    pub api_key_url: String,
    pub docs_label: String,
    pub docs_url: String,
    pub default_base_url: String,
    pub supports_model_listing: bool,
    pub models: Vec<BuiltinLlmProviderTemplateModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderConfig {
    pub id: String,
    pub name: String,
    #[serde(default = "default_openai_compatible_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model_type: LlmModelType,
    #[serde(default)]
    pub protocol: LlmProviderProtocol,
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_identity_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_preset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_preset_model_id: Option<String>,
    #[serde(default)]
    pub managed_base_url: bool,
    #[serde(default)]
    pub supports_multimodal: bool,
    #[serde(default)]
    pub supports_stateful: bool,
    #[serde(default, skip_serializing, rename = "protocol")]
    pub(crate) legacy_protocol: Option<LegacyLlmProviderProtocolKind>,
    #[serde(default, skip_serializing, rename = "supportsEmbedding")]
    pub(crate) legacy_supports_embedding: bool,
    #[serde(default, skip_serializing, rename = "responsesModel")]
    pub(crate) legacy_responses_model: String,
    #[serde(default, skip_serializing, rename = "embeddingModel")]
    pub(crate) legacy_embedding_model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawLlmProviderConfig {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default = "default_openai_compatible_base_url")]
    base_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    model_type: LlmModelType,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    model_identity_hint: Option<String>,
    #[serde(default)]
    builtin_preset_id: Option<String>,
    #[serde(default)]
    builtin_preset_model_id: Option<String>,
    #[serde(default)]
    managed_base_url: bool,
    #[serde(default)]
    supports_multimodal: bool,
    #[serde(default)]
    supports_stateful: bool,
    #[serde(default, rename = "supportsEmbedding")]
    legacy_supports_embedding: bool,
    #[serde(default, rename = "responsesModel")]
    legacy_responses_model: String,
    #[serde(default, rename = "embeddingModel")]
    legacy_embedding_model: String,
}

impl LlmProviderConfig {
    pub fn is_llm_model(&self) -> bool {
        self.model_type == LlmModelType::Llm
    }

    pub fn is_embedding_model(&self) -> bool {
        self.model_type == LlmModelType::Embedding
    }

    pub fn is_llm_responses_protocol(&self) -> bool {
        self.is_llm_model() && self.protocol == LlmProviderProtocol::Responses
    }

    pub fn model_name(&self) -> &str {
        self.model.trim()
    }

    pub fn has_llm_model(&self) -> bool {
        self.is_llm_model() && !self.model_name().is_empty()
    }

    pub fn llm_model_name(&self) -> Option<&str> {
        self.has_llm_model().then_some(self.model_name())
    }

    pub fn has_responses_model(&self) -> bool {
        self.is_llm_responses_protocol() && !self.model_name().is_empty()
    }

    pub fn supports_multimodal(&self) -> bool {
        self.has_responses_model() && self.supports_multimodal
    }

    pub fn supports_stateful(&self) -> bool {
        self.has_responses_model() && self.supports_stateful
    }

    pub fn has_embedding_model(&self) -> bool {
        self.is_embedding_model() && !self.model_name().is_empty()
    }

    pub fn embedding_model_name(&self) -> Option<&str> {
        self.has_embedding_model().then_some(self.model_name())
    }

    pub fn legacy_responses_model_name(&self) -> &str {
        self.legacy_responses_model.trim()
    }

    pub fn legacy_embedding_model_name(&self) -> &str {
        self.legacy_embedding_model.trim()
    }

    pub fn legacy_model_type(&self) -> Option<LlmModelType> {
        self.legacy_protocol
            .as_ref()
            .map(|protocol| match protocol {
                LegacyLlmProviderProtocolKind::Embedding => LlmModelType::Embedding,
                LegacyLlmProviderProtocolKind::Chat | LegacyLlmProviderProtocolKind::Responses => {
                    LlmModelType::Llm
                }
            })
    }

    pub fn legacy_llm_protocol(&self) -> Option<LlmProviderProtocol> {
        self.legacy_protocol
            .as_ref()
            .and_then(|protocol| match protocol {
                LegacyLlmProviderProtocolKind::Chat => Some(LlmProviderProtocol::ChatCompletions),
                LegacyLlmProviderProtocolKind::Responses => Some(LlmProviderProtocol::Responses),
                LegacyLlmProviderProtocolKind::Embedding => None,
            })
    }
}

impl<'de> Deserialize<'de> for LlmProviderConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawLlmProviderConfig::deserialize(deserializer)?;
        let protocol = match raw.protocol.as_deref().map(str::trim) {
            None | Some("") | Some("responses") | Some("openai_responses") => {
                LlmProviderProtocol::Responses
            }
            Some("chat_completions") => LlmProviderProtocol::ChatCompletions,
            Some("openai_chat") | Some("openai_compatible") => LlmProviderProtocol::ChatCompletions,
            Some("openai_embedding") => LlmProviderProtocol::Responses,
            Some(other) => {
                return Err(D::Error::custom(format!(
                    "unknown LLM provider protocol: {other}"
                )))
            }
        };
        let legacy_protocol = match raw.protocol.as_deref().map(str::trim) {
            Some("openai_chat") | Some("openai_compatible") => {
                Some(LegacyLlmProviderProtocolKind::Chat)
            }
            Some("openai_responses") => Some(LegacyLlmProviderProtocolKind::Responses),
            Some("openai_embedding") => Some(LegacyLlmProviderProtocolKind::Embedding),
            _ => None,
        };

        Ok(Self {
            id: raw.id,
            name: raw.name,
            base_url: raw.base_url,
            api_key: raw.api_key,
            model_type: raw.model_type,
            protocol,
            model: raw.model,
            model_identity_hint: raw.model_identity_hint,
            builtin_preset_id: raw.builtin_preset_id,
            builtin_preset_model_id: raw.builtin_preset_model_id,
            managed_base_url: raw.managed_base_url,
            supports_multimodal: raw.supports_multimodal,
            supports_stateful: raw.supports_stateful,
            legacy_protocol,
            legacy_supports_embedding: raw.legacy_supports_embedding,
            legacy_responses_model: raw.legacy_responses_model,
            legacy_embedding_model: raw.legacy_embedding_model,
        })
    }
}

impl Default for LlmProviderConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            base_url: default_openai_compatible_base_url(),
            api_key: String::new(),
            model_type: LlmModelType::Llm,
            protocol: LlmProviderProtocol::Responses,
            model: String::new(),
            model_identity_hint: None,
            builtin_preset_id: None,
            builtin_preset_model_id: None,
            managed_base_url: false,
            supports_multimodal: false,
            supports_stateful: false,
            legacy_protocol: None,
            legacy_supports_embedding: false,
            legacy_responses_model: String::new(),
            legacy_embedding_model: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettings {
    #[serde(default)]
    pub providers: Vec<LlmProviderConfig>,
    #[serde(default)]
    pub translation_provider_id: Option<String>,
    #[serde(default)]
    pub question_answer_provider_id: Option<String>,
    #[serde(default, skip_serializing, alias = "defaultProviderId")]
    pub(crate) legacy_default_provider_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderModelEntry {
    pub id: String,
    #[serde(default)]
    pub identity_hint: Option<String>,
}

pub fn builtin_llm_provider_templates() -> Vec<BuiltinLlmProviderTemplate> {
    vec![
        BuiltinLlmProviderTemplate {
            id: "openai".to_string(),
            display_name: "OpenAI".to_string(),
            description:
                "官方 API 模板。先登录 OpenAI 平台、创建 API Key，再从常用 Responses / Embedding 模型里选择。"
                    .to_string(),
            registration_label: "注册 / 登录".to_string(),
            registration_url: "https://platform.openai.com/signup".to_string(),
            api_key_label: "API Key 页面".to_string(),
            api_key_url: "https://platform.openai.com/api-keys".to_string(),
            docs_label: "模型与 API 文档".to_string(),
            docs_url: "https://platform.openai.com/docs/overview".to_string(),
            default_base_url: "https://api.openai.com/v1".to_string(),
            supports_model_listing: true,
            models: vec![
                BuiltinLlmProviderTemplateModel {
                    id: "gpt-5.4-mini".to_string(),
                    display_name: "GPT-5.4-mini".to_string(),
                    model: "gpt-5.4-mini".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::Responses,
                    supports_multimodal: true,
                    supports_stateful: true,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                        BuiltinLlmTemplateUseCase::Ocr,
                    ],
                    summary:
                        "通用小型模型，适合翻译、问答和截图理解，默认成本比旗舰档更低。"
                            .to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "gpt-5.4".to_string(),
                    display_name: "GPT-5.4".to_string(),
                    model: "gpt-5.4".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::Responses,
                    supports_multimodal: true,
                    supports_stateful: true,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                        BuiltinLlmTemplateUseCase::Ocr,
                    ],
                    summary: "旗舰通用模型，适合高质量翻译、复杂问答和多模态理解。"
                        .to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "gpt-5.4-nano".to_string(),
                    display_name: "GPT-5.4-nano".to_string(),
                    model: "gpt-5.4-nano".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::Responses,
                    supports_multimodal: true,
                    supports_stateful: true,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                        BuiltinLlmTemplateUseCase::Ocr,
                    ],
                    summary: "更轻量的通用模型，适合低延迟翻译和基础问答。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "text-embedding-3-small".to_string(),
                    display_name: "text-embedding-3-small".to_string(),
                    model: "text-embedding-3-small".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Embedding,
                    protocol: BuiltinLlmTemplateModelProtocol::Responses,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::Embedding],
                    summary: "常用 Embedding 模型，适合给 RAG 建立通用文本向量索引。"
                        .to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "text-embedding-3-large".to_string(),
                    display_name: "text-embedding-3-large".to_string(),
                    model: "text-embedding-3-large".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Embedding,
                    protocol: BuiltinLlmTemplateModelProtocol::Responses,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::Embedding],
                    summary:
                        "更高质量的 Embedding 模型，适合更重视召回质量的 RAG 场景。"
                            .to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
            ],
        },
        BuiltinLlmProviderTemplate {
            id: "openrouter".to_string(),
            display_name: "OpenRouter".to_string(),
            description:
                "聚合网关模板。先登录 OpenRouter、创建 API Key，再按当前账号可见的远端模型目录选择模型。"
                    .to_string(),
            registration_label: "注册 / 登录".to_string(),
            registration_url: "https://openrouter.ai/".to_string(),
            api_key_label: "API Key 页面".to_string(),
            api_key_url: "https://openrouter.ai/settings/keys".to_string(),
            docs_label: "官方文档".to_string(),
            docs_url: "https://openrouter.ai/docs/quickstart".to_string(),
            default_base_url: "https://openrouter.ai/api/v1".to_string(),
            supports_model_listing: true,
            models: vec![],
        },
        BuiltinLlmProviderTemplate {
            id: "deepseek".to_string(),
            display_name: "DeepSeek".to_string(),
            description:
                "官方 API 模板。先登录 DeepSeek 平台、创建 API Key，再从常用聊天或推理模型里选择。"
                    .to_string(),
            registration_label: "注册 / 登录".to_string(),
            registration_url: "https://platform.deepseek.com/".to_string(),
            api_key_label: "API Key 页面".to_string(),
            api_key_url: "https://platform.deepseek.com/api_keys".to_string(),
            docs_label: "官方文档".to_string(),
            docs_url: "https://api-docs.deepseek.com/".to_string(),
            default_base_url: "https://api.deepseek.com".to_string(),
            supports_model_listing: true,
            models: vec![
                BuiltinLlmProviderTemplateModel {
                    id: "deepseek-chat".to_string(),
                    display_name: "DeepSeek-Chat".to_string(),
                    model: "deepseek-chat".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "通用对话模型，适合翻译、日常问答和轻量文本生成。"
                        .to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "deepseek-reasoner".to_string(),
                    display_name: "DeepSeek-Reasoner".to_string(),
                    model: "deepseek-reasoner".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::RagAnswer],
                    summary: "推理模型，适合复杂问答、分析和需要多步思考的场景。"
                        .to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
            ],
        },
        BuiltinLlmProviderTemplate {
            id: "ollama".to_string(),
            display_name: "Ollama".to_string(),
            description:
                "本地 OpenAI-compatible 模板。先安装 Ollama 并 pull 模型；默认连接本机 `http://localhost:11434/v1`，API Key 可以留空。"
                    .to_string(),
            registration_label: "下载 / 安装".to_string(),
            registration_url: "https://ollama.com/download".to_string(),
            api_key_label: "OpenAI 兼容说明".to_string(),
            api_key_url: "https://docs.ollama.com/openai".to_string(),
            docs_label: "模型与文档".to_string(),
            docs_url: "https://docs.ollama.com/".to_string(),
            default_base_url: "http://localhost:11434/v1".to_string(),
            supports_model_listing: true,
            models: vec![
                BuiltinLlmProviderTemplateModel {
                    id: "qwen3-8b".to_string(),
                    display_name: "Qwen3 8B".to_string(),
                    model: "qwen3:8b".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::Responses,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "常见本地文本模型，适合日常翻译、问答和低成本试配。"
                        .to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "gpt-oss-20b".to_string(),
                    display_name: "gpt-oss 20B".to_string(),
                    model: "gpt-oss:20b".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::RagAnswer],
                    summary: "常见本地推理模型，适合复杂问答、解释和代码辅助场景。"
                        .to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "qwen3-vl-8b".to_string(),
                    display_name: "Qwen3-VL 8B".to_string(),
                    model: "qwen3-vl:8b".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: true,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::RagAnswer],
                    summary:
                        "常见本地图像理解模型，适合截图和文档理解；当前不会进入 OCR 列表。"
                            .to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "embeddinggemma".to_string(),
                    display_name: "EmbeddingGemma".to_string(),
                    model: "embeddinggemma".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Embedding,
                    protocol: BuiltinLlmTemplateModelProtocol::Responses,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::Embedding],
                    summary: "常见本地 Embedding 模型，适合给 RAG 建立向量索引。"
                        .to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
            ],
        },
        BuiltinLlmProviderTemplate {
            id: "zhipu".to_string(),
            display_name: "智谱 AI".to_string(),
            description:
                "官方免费模型目录模板。先注册智谱开放平台、创建 API Key，再从白名单里选择模型。"
                    .to_string(),
            registration_label: "注册 / 登录".to_string(),
            registration_url:
                "https://bigmodel.cn/login?redirect=%2Fusercenter%2Fproj-mgmt%2Fapikeys".to_string(),
            api_key_label: "API Key 页面".to_string(),
            api_key_url: "https://bigmodel.cn/login?redirect=%2Fusercenter%2Fproj-mgmt%2Fapikeys"
                .to_string(),
            docs_label: "官方文档".to_string(),
            docs_url: "https://docs.bigmodel.cn/cn/guide/start/quick-start".to_string(),
            default_base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
            supports_model_listing: true,
            models: vec![
                BuiltinLlmProviderTemplateModel {
                    id: "glm-4.7-flash".to_string(),
                    display_name: "GLM-4.7-Flash".to_string(),
                    model: "glm-4.7-flash".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "免费文本模型，适合翻译、问答和通用长文本任务。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "glm-4.6v-flash".to_string(),
                    display_name: "GLM-4.6V-Flash".to_string(),
                    model: "glm-4.6v-flash".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: true,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "免费视觉理解模型，擅长图像、视频和文件理解。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "glm-4.1v-thinking-flash".to_string(),
                    display_name: "GLM-4.1V-Thinking-Flash".to_string(),
                    model: "glm-4.1v-thinking-flash".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: true,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "免费视觉推理模型，适合图表、GUI 和网页理解场景。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "glm-4-flash-250414".to_string(),
                    display_name: "GLM-4-Flash-250414".to_string(),
                    model: "glm-4-flash-250414".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "免费轻量文本模型，适合通用对话、翻译和基础问答。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "glm-4v-flash".to_string(),
                    display_name: "GLM-4V-Flash".to_string(),
                    model: "glm-4v-flash".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: true,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "免费图像理解模型，适合图像识别、问答和视觉推理。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "cogview-3-flash".to_string(),
                    display_name: "CogView-3-Flash".to_string(),
                    model: "cogview-3-flash".to_string(),
                    model_type: BuiltinLlmTemplateModelType::ImageGeneration,
                    protocol: BuiltinLlmTemplateModelProtocol::Unsupported,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: Vec::new(),
                    summary: "免费图像生成模型，适合根据文本快速生成图片。".to_string(),
                    selectable_in_current_app: false,
                    disabled_reason: Some(
                        "当前 Wabity 没有图像生成链路，不能当普通 LLM 使用。".to_string(),
                    ),
                },
                BuiltinLlmProviderTemplateModel {
                    id: "cogvideox-flash".to_string(),
                    display_name: "CogVideoX-Flash".to_string(),
                    model: "cogvideox-flash".to_string(),
                    model_type: BuiltinLlmTemplateModelType::VideoGeneration,
                    protocol: BuiltinLlmTemplateModelProtocol::Unsupported,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: Vec::new(),
                    summary: "免费视频生成模型，适合根据文本指令生成短视频。".to_string(),
                    selectable_in_current_app: false,
                    disabled_reason: Some(
                        "当前 Wabity 没有视频生成链路，不能当普通 LLM 使用。".to_string(),
                    ),
                },
            ],
        },
        BuiltinLlmProviderTemplate {
            id: "siliconflow".to_string(),
            display_name: "SiliconFlow".to_string(),
            description:
                "官方免费语言模型目录模板。先注册 SiliconFlow、创建 API Key，再从白名单里选择模型。"
                    .to_string(),
            registration_label: "注册 / 登录".to_string(),
            registration_url: "https://account.siliconflow.cn".to_string(),
            api_key_label: "API Key 页面".to_string(),
            api_key_url: "https://cloud.siliconflow.cn/account/ak".to_string(),
            docs_label: "官方文档".to_string(),
            docs_url:
                "https://docs.siliconflow.cn/cn/api-reference/chat-completions/chat-completions"
                    .to_string(),
            default_base_url: "https://api.siliconflow.cn/v1".to_string(),
            supports_model_listing: true,
            models: vec![
                BuiltinLlmProviderTemplateModel {
                    id: "qwen3.5-4b-instruct-2507".to_string(),
                    display_name: "Qwen3.5-4B-Instruct-2507".to_string(),
                    model: "Qwen/Qwen3.5-4B-Instruct-2507".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "免费轻量指令模型，适合低成本翻译、问答和日常文本任务。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "paddleocr-vl-1.5".to_string(),
                    display_name: "PaddleOCR-VL-1.5".to_string(),
                    model: "PaddlePaddle/PaddleOCR-VL-1.5".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: true,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::RagAnswer],
                    summary: "免费文档理解模型，适合票据、表格和复杂版面 OCR 识别。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "deepseek-r1-distill-qwen-7b".to_string(),
                    display_name: "DeepSeek-R1-Distill-Qwen-7B".to_string(),
                    model: "deepseek-ai/DeepSeek-R1-Distill-Qwen-7B".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "免费轻量推理模型，适合分析、问答和需要推理的文本任务。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "glm-4.1v-9b-thinking".to_string(),
                    display_name: "GLM-4.1V-9B-Thinking".to_string(),
                    model: "THUDM/GLM-4.1V-9B-Thinking".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: true,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::RagAnswer],
                    summary: "免费视觉推理模型，适合图表、截图和复杂图像理解。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "paddleocr-vl".to_string(),
                    display_name: "PaddleOCR-VL".to_string(),
                    model: "PaddlePaddle/PaddleOCR-VL".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: true,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::RagAnswer],
                    summary: "免费 OCR / 文档解析模型，适合表格、票据和富版面内容提取。"
                        .to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "deepseek-ocr".to_string(),
                    display_name: "DeepSeek-OCR".to_string(),
                    model: "deepseek-ai/DeepSeek-OCR".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: true,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::RagAnswer],
                    summary: "免费 OCR 模型，适合截图、扫描件和文档文字提取。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "qwen3-8b".to_string(),
                    display_name: "Qwen3-8B".to_string(),
                    model: "Qwen/Qwen3-8B".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "免费通用文本模型，适合对话、翻译和基础问答。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "hunyuan-mt-7b".to_string(),
                    display_name: "Hunyuan-MT-7B".to_string(),
                    model: "tencent/Hunyuan-MT-7B".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::Translation],
                    summary: "免费机器翻译模型，适合中英文和多语种翻译场景。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "deepseek-r1-0528-qwen3-8b".to_string(),
                    display_name: "DeepSeek-R1-0528-Qwen3-8B".to_string(),
                    model: "deepseek-ai/DeepSeek-R1-0528-Qwen3-8B".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::RagAnswer],
                    summary: "免费推理模型，适合复杂问答和需要多步分析的任务。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "glm-z1-9b-0414".to_string(),
                    display_name: "GLM-Z1-9B-0414".to_string(),
                    model: "THUDM/GLM-Z1-9B-0414".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![BuiltinLlmTemplateUseCase::RagAnswer],
                    summary: "免费推理模型，适合代码解释、复杂问答和长链思考。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "qwen2.5-7b-instruct".to_string(),
                    display_name: "Qwen2.5-7B-Instruct".to_string(),
                    model: "Qwen/Qwen2.5-7B-Instruct".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "免费通用指令模型，适合日常问答、改写和轻量生成。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "glm-4-9b-0414".to_string(),
                    display_name: "GLM-4-9B-0414".to_string(),
                    model: "THUDM/GLM-4-9B-0414".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "免费通用文本模型，适合对话、翻译和基础知识问答。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
                BuiltinLlmProviderTemplateModel {
                    id: "internlm2-5-7b-chat".to_string(),
                    display_name: "internlm2_5-7b-chat".to_string(),
                    model: "internlm/internlm2_5-7b-chat".to_string(),
                    model_type: BuiltinLlmTemplateModelType::Llm,
                    protocol: BuiltinLlmTemplateModelProtocol::ChatCompletions,
                    supports_multimodal: false,
                    supports_stateful: false,
                    recommended_for: vec![
                        BuiltinLlmTemplateUseCase::Translation,
                        BuiltinLlmTemplateUseCase::RagAnswer,
                    ],
                    summary: "免费聊天模型，适合日常问答和轻量文本生成。".to_string(),
                    selectable_in_current_app: true,
                    disabled_reason: None,
                },
            ],
        },
    ]
}

pub fn find_builtin_llm_provider_template(template_id: &str) -> Option<BuiltinLlmProviderTemplate> {
    builtin_llm_provider_templates()
        .into_iter()
        .find(|template| template.id == template_id)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OcrSettings {
    #[serde(default)]
    pub provider: OcrProviderKind,
    #[serde(default)]
    pub llm_provider_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagSettings {
    #[serde(default)]
    pub source_directories: Vec<String>,
    #[serde(default = "default_rag_ignore_globs")]
    pub ignore_globs: Vec<String>,
    #[serde(default)]
    pub embedding_provider_id: Option<String>,
}

impl Default for RagSettings {
    fn default() -> Self {
        Self {
            source_directories: Vec::new(),
            ignore_globs: default_rag_ignore_globs(),
            embedding_provider_id: None,
        }
    }
}

fn default_openai_compatible_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

pub fn default_translation_prompt() -> String {
    [
        "You are an expert translation engine specialized in English ↔ Simplified Chinese.",
        "",
        "Translate the following text accurately, naturally, and fluently.",
        "",
        "Rules:",
        "- If the user specifies a target language, follow it exactly.",
        "- If no target language is specified:",
        "  - Primarily Simplified Chinese → English",
        "  - Primarily English → Simplified Chinese",
        "  - Other languages → Simplified Chinese",
        "- Preserve original meaning, tone, style, and all formatting (Markdown, code blocks, URLs, proper nouns, etc.).",
        "- Return ONLY the translation. No explanations, notes, or extra text.",
    ]
    .join("\n")
}

pub fn default_rag_answer_system_prompt() -> String {
    [
        "You are a precise tool-augmented assistant. Answer questions using only the tools available.",
        "",
        "Core Rules:",
        "- Always ground your answers in tool results. Never assert repository-specific, document-specific, or system-specific facts without first using RAG/search/file-reading/MCP tools to gather evidence.",
        "- Use tools in multiple rounds if needed: start broad, then drill down to exact files and line ranges until evidence is sufficient.",
        "- For exact file content, always call the file reading tool with the precise path and line window. Do not guess.",
        "- For broader context, use RAG or MCP tools instead of assuming.",
        "- In your final answer, cite concrete file paths and line numbers when tool results provide them.",
        "- Clearly distinguish facts from inferences. Label any inference explicitly.",
        "- You may add concise general background knowledge when helpful, but never fabricate file paths, APIs, behaviors, code, or configuration values.",
        "- If tool results are conflicting, incomplete, or insufficient, state it clearly and explain what is missing.",
        "",
        "Return Markdown only.",
    ]
    .join("\n")
}

pub fn default_rag_ignore_globs() -> Vec<String> {
    fixed_rag_ignore_globs()
        .iter()
        .copied()
        .map(str::to_string)
        .collect()
}

pub const fn fixed_rag_ignore_globs() -> &'static [&'static str] {
    &[
        "**/.git/**",
        "**/node_modules/**",
        "**/vendor/**",
        "**/Pods/**",
        "**/target/**",
        "**/dist/**",
        "**/build/**",
        "**/out/**",
        "**/.next/**",
        "**/.nuxt/**",
        "**/.svelte-kit/**",
        "**/.turbo/**",
        "**/.cache/**",
        "**/coverage/**",
        "**/.venv/**",
        "**/venv/**",
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub general: GeneralSettings,
    #[serde(default)]
    pub notification: NotificationSettings,
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(default)]
    pub prompts: PromptsSettings,
    #[serde(default)]
    pub llm: LlmSettings,
    #[serde(default)]
    pub ocr: OcrSettings,
    #[serde(default)]
    pub rag: RagSettings,
}
