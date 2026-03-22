use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize};

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
    [
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
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub general: GeneralSettings,
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
