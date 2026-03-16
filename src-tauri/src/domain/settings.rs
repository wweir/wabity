use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralSettings {
    pub auto_start: bool,
    pub show_in_dock: bool,
    pub language: String,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            auto_start: false,
            show_in_dock: true,
            language: "zh-CN".to_string(),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderProtocolKind {
    #[default]
    #[serde(rename = "openai_chat", alias = "openai_compatible")]
    Chat,
    #[serde(rename = "openai_responses")]
    Responses,
    #[serde(rename = "openai_embedding")]
    Embedding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub protocol: LlmProviderProtocolKind,
    #[serde(default = "default_openai_compatible_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub supports_multimodal: bool,
}

impl Default for LlmProviderConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            protocol: LlmProviderProtocolKind::Chat,
            base_url: default_openai_compatible_base_url(),
            api_key: String::new(),
            model: String::new(),
            supports_multimodal: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettings {
    #[serde(default)]
    pub providers: Vec<LlmProviderConfig>,
    #[serde(default)]
    pub default_provider_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OcrSettings {
    #[serde(default)]
    pub provider: OcrProviderKind,
    #[serde(default)]
    pub llm_provider_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RagSettings {
    #[serde(default)]
    pub source_directories: Vec<String>,
    #[serde(default)]
    pub ignore_globs: Vec<String>,
    #[serde(default)]
    pub embedding_provider_id: Option<String>,
}

fn default_openai_compatible_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub general: GeneralSettings,
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(default)]
    pub llm: LlmSettings,
    #[serde(default)]
    pub ocr: OcrSettings,
    #[serde(default)]
    pub rag: RagSettings,
}
