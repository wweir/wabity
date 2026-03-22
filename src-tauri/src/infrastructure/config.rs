use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::acp::{
    AcpAgentConfig, AcpMcpServerConfig, AcpMcpServerHttpConfig, AcpMcpServerSseConfig,
    AcpMcpServerStdioConfig, AcpNameValuePair,
};
use crate::domain::settings::{
    AppearanceSettings, GeneralSettings, LlmModelType, LlmProviderConfig, LlmProviderProtocol,
    LlmSettings, OcrSettings, PromptsSettings, RagSettings,
};

const CONFIG_FILE_NAME: &str = "config.toml";
const WORKSPACE_HISTORY_FILE_NAME: &str = "workspace-history.toml";
pub const RECENT_WORKSPACE_LIMIT: usize = 3;
#[cfg(target_os = "macos")]
const LEGACY_DEFAULT_OCR_SHORTCUT: &str = "Cmd+Ctrl+Shift+Space";
#[cfg(not(target_os = "macos"))]
const LEGACY_DEFAULT_OCR_SHORTCUT: &str = "Ctrl+Alt+Shift+Space";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub shortcuts: ShortcutConfig,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
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
    #[serde(default)]
    pub acp: AcpConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutConfig {
    /// Format: "modifiers+key", e.g., "Alt+Space" or "Ctrl+Shift+Space"
    pub toggle_launcher: String,
    /// Format: "modifiers+key", e.g., "Alt+R"
    pub ocr_capture: String,
    /// Format: "modifiers+key", e.g., "Alt+D"
    pub ocr_translate: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutKey {
    ToggleLauncher,
    OcrCapture,
    OcrTranslate,
}

impl ShortcutKey {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "toggle_launcher" => Ok(Self::ToggleLauncher),
            "ocr_capture" => Ok(Self::OcrCapture),
            "ocr_translate" => Ok(Self::OcrTranslate),
            other => anyhow::bail!("unknown shortcut key: {other}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToggleLauncher => "toggle_launcher",
            Self::OcrCapture => "ocr_capture",
            Self::OcrTranslate => "ocr_translate",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::ToggleLauncher => "launcher",
            Self::OcrCapture => "OCR capture",
            Self::OcrTranslate => "OCR translate",
        }
    }
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            toggle_launcher: "Alt+Space".to_string(),
            ocr_capture: "Alt+R".to_string(),
            ocr_translate: "Alt+D".to_string(),
        }
    }
}

impl ShortcutConfig {
    pub fn get(&self, key: ShortcutKey) -> &str {
        match key {
            ShortcutKey::ToggleLauncher => &self.toggle_launcher,
            ShortcutKey::OcrCapture => &self.ocr_capture,
            ShortcutKey::OcrTranslate => &self.ocr_translate,
        }
    }

    pub fn set(&mut self, key: ShortcutKey, shortcut: impl Into<String>) {
        match key {
            ShortcutKey::ToggleLauncher => self.toggle_launcher = shortcut.into(),
            ShortcutKey::OcrCapture => self.ocr_capture = shortcut.into(),
            ShortcutKey::OcrTranslate => self.ocr_translate = shortcut.into(),
        }
    }

    pub fn default_value(key: ShortcutKey) -> &'static str {
        match key {
            ShortcutKey::ToggleLauncher => "Alt+Space",
            ShortcutKey::OcrCapture => "Alt+R",
            ShortcutKey::OcrTranslate => "Alt+D",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceConfig {
    pub root_path: String,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        let root_path = default_workspace_root()
            .unwrap_or_else(|| PathBuf::from("."))
            .to_string_lossy()
            .into_owned();

        Self { root_path }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WorkspaceHistory {
    pub recent_roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AcpConfig {
    #[serde(default)]
    pub agents: Vec<AcpAgentConfig>,
    #[serde(default)]
    pub default_agent_id: Option<String>,
    #[serde(default)]
    pub mcp_servers: Vec<AcpMcpServerConfig>,
    #[serde(default)]
    pub saved_sessions: Vec<SavedAcpSession>,
    #[serde(default)]
    pub active_session_id: Option<String>,
    #[serde(default, skip_serializing, alias = "program")]
    legacy_program: String,
    #[serde(default, skip_serializing, alias = "args")]
    legacy_args: Vec<String>,
    #[serde(default, skip_serializing, alias = "shell_command")]
    legacy_shell_command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedAcpSession {
    pub session_id: String,
    pub workspace_root: String,
    pub title: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_name: String,
    pub agent_program: String,
    #[serde(default)]
    pub agent_args: Vec<String>,
    #[serde(default)]
    pub agent_shell_command: Option<String>,
    #[serde(default)]
    pub mcp_servers: Vec<AcpMcpServerConfig>,
    #[serde(default)]
    pub last_updated_at_ms: u64,
}

impl AcpConfig {
    fn normalize(&mut self) {
        if self.agents.is_empty() {
            let legacy_shell_command = self
                .legacy_shell_command
                .as_ref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            let legacy_program = self.legacy_program.trim().to_string();

            if legacy_shell_command.is_some()
                || !legacy_program.is_empty()
                || !self.legacy_args.is_empty()
            {
                let name = derive_agent_name(legacy_shell_command.as_deref(), &legacy_program);
                self.agents.push(AcpAgentConfig {
                    id: make_agent_id(&name, 0, &HashSet::new()),
                    name,
                    program: legacy_program,
                    args: self.legacy_args.clone(),
                    shell_command: legacy_shell_command,
                    mcp_servers: Vec::new(),
                });
            }
        }

        let mut used_ids = HashSet::new();
        for (index, agent) in self.agents.iter_mut().enumerate() {
            agent.name =
                normalize_agent_name(&agent.name, &agent.program, agent.shell_command.as_deref());
            agent.program = agent.program.trim().to_string();
            agent.args = agent
                .args
                .iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect();
            agent.shell_command = agent
                .shell_command
                .as_ref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            agent.mcp_servers.clear();
            agent.id = make_agent_id(&agent.id, index, &used_ids);
            used_ids.insert(agent.id.clone());
        }

        self.mcp_servers = sanitize_mcp_servers(&self.mcp_servers);

        if !self
            .default_agent_id
            .as_ref()
            .map(|id| self.agents.iter().any(|agent| &agent.id == id))
            .unwrap_or(false)
        {
            self.default_agent_id = self.agents.first().map(|agent| agent.id.clone());
        }

        for snapshot in &mut self.saved_sessions {
            if snapshot.agent_name.trim().is_empty() {
                snapshot.agent_name = derive_agent_name(
                    snapshot.agent_shell_command.as_deref(),
                    &snapshot.agent_program,
                );
            }
        }
    }
}

impl AppConfig {
    pub(crate) fn normalize(&mut self) {
        let legacy_translation_prompt = self.general.take_legacy_translation_prompt();
        self.general.normalize();
        self.prompts
            .adopt_legacy_translation_prompt(legacy_translation_prompt);
        self.prompts.normalize();
        let llm_provider_id_mapping = self.llm.normalize();
        self.ocr.normalize(&self.llm, &llm_provider_id_mapping);
        self.rag.normalize(&self.llm, &llm_provider_id_mapping);
        self.acp.normalize();
    }
}

pub struct ConfigStore {
    config_path: PathBuf,
    workspace_history_path: PathBuf,
    cached_config: Mutex<Option<AppConfig>>,
    cached_workspace_history: Mutex<Option<WorkspaceHistory>>,
}

impl ConfigStore {
    pub fn new() -> Result<Self> {
        let config_dir = Self::config_dir()?;
        std::fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join(CONFIG_FILE_NAME);
        let workspace_history_path = config_dir.join(WORKSPACE_HISTORY_FILE_NAME);

        Ok(Self {
            config_path,
            workspace_history_path,
            cached_config: Mutex::new(None),
            cached_workspace_history: Mutex::new(None),
        })
    }

    pub async fn load(&self) -> Result<AppConfig> {
        {
            let cached = self.cached_config.lock().unwrap();
            if let Some(ref config) = *cached {
                return Ok(config.clone());
            }
        }

        if !self.config_path.exists() {
            let default_config = AppConfig::default();
            self.save(&default_config).await?;
            let mut cached = self.cached_config.lock().unwrap();
            *cached = Some(default_config.clone());
            return Ok(default_config);
        }

        let content = tokio::fs::read_to_string(&self.config_path)
            .await
            .with_context(|| format!("failed to read config file: {:?}", self.config_path))?;

        let mut config = parse_config_content(&content)?;
        if migrate_legacy_shortcuts(&mut config) {
            self.save(&config).await?;
        }

        let mut cached = self.cached_config.lock().unwrap();
        *cached = Some(config.clone());
        Ok(config)
    }

    pub async fn save(&self, config: &AppConfig) -> Result<()> {
        let content = serialize_config_content(config)?;
        safe_write(&self.config_path, &content).await?;

        // Update cache
        let mut cached = self.cached_config.lock().unwrap();
        *cached = Some(config.clone());

        Ok(())
    }

    pub async fn load_workspace_history(&self) -> Result<WorkspaceHistory> {
        {
            let cached = self.cached_workspace_history.lock().unwrap();
            if let Some(ref history) = *cached {
                return Ok(history.clone());
            }
        }

        if !self.workspace_history_path.exists() {
            let default_history = WorkspaceHistory::default();
            self.save_workspace_history(&default_history).await?;
            let mut cached = self.cached_workspace_history.lock().unwrap();
            *cached = Some(default_history.clone());
            return Ok(default_history);
        }

        let content = tokio::fs::read_to_string(&self.workspace_history_path)
            .await
            .with_context(|| {
                format!(
                    "failed to read workspace history file: {:?}",
                    self.workspace_history_path
                )
            })?;
        let history = parse_workspace_history_content(&content)?;

        let mut cached = self.cached_workspace_history.lock().unwrap();
        *cached = Some(history.clone());
        Ok(history)
    }

    pub async fn save_workspace_history(&self, history: &WorkspaceHistory) -> Result<()> {
        let content = serialize_workspace_history_content(history)?;
        safe_write(&self.workspace_history_path, &content).await?;

        let mut cached = self.cached_workspace_history.lock().unwrap();
        *cached = Some(history.clone());
        Ok(())
    }

    pub fn config_dir() -> Result<PathBuf> {
        let config_dir = dirs::config_dir().context("failed to determine config directory")?;
        Ok(config_dir.join("wabity"))
    }

    pub fn data_dir() -> Result<PathBuf> {
        let data_dir = dirs::data_dir().context("failed to determine data directory")?;
        Ok(data_dir.join("wabity"))
    }
}

fn normalize_agent_name(name: &str, program: &str, shell_command: Option<&str>) -> String {
    let trimmed = name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }

    derive_agent_name(shell_command, program)
}

#[derive(Debug, Clone, Default)]
struct LlmProviderIdTargets {
    llm_id: Option<String>,
    embedding_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct LlmProviderIdMapping {
    entries: HashMap<String, LlmProviderIdTargets>,
}

impl LlmProviderIdMapping {
    fn insert(&mut self, original_id: &str, targets: LlmProviderIdTargets) {
        let trimmed = original_id.trim();
        if trimmed.is_empty() {
            return;
        }

        self.entries.insert(trimmed.to_string(), targets);
    }

    fn resolve_llm_id(&self, provider_id: &str) -> Option<String> {
        self.entries
            .get(provider_id)
            .and_then(|targets| targets.llm_id.clone())
    }

    fn resolve_embedding_id(&self, provider_id: &str) -> Option<String> {
        self.entries
            .get(provider_id)
            .and_then(|targets| targets.embedding_id.clone())
    }
}

impl LlmSettings {
    fn normalize(&mut self) -> LlmProviderIdMapping {
        let original_translation_provider_id = self
            .translation_provider_id
            .clone()
            .or_else(|| self.legacy_default_provider_id.clone());
        let original_question_answer_provider_id = self
            .question_answer_provider_id
            .clone()
            .or_else(|| self.legacy_default_provider_id.clone());
        let original_providers = std::mem::take(&mut self.providers);
        let mut normalized_providers = Vec::new();
        let mut used_ids = HashSet::new();
        let mut id_mapping = LlmProviderIdMapping::default();

        for (index, provider) in original_providers.into_iter().enumerate() {
            let original_id = provider.id.clone();
            let expanded_providers = expand_llm_provider(provider);
            let split_from_single_entry = expanded_providers.len() > 1;
            let mut targets = LlmProviderIdTargets::default();

            for mut expanded_provider in expanded_providers {
                expanded_provider.name = normalize_llm_provider_name(
                    &expanded_provider.name,
                    expanded_provider.model_name(),
                    &expanded_provider.base_url,
                    expanded_provider.model_type,
                    split_from_single_entry,
                );
                expanded_provider.base_url = expanded_provider
                    .base_url
                    .trim()
                    .trim_end_matches('/')
                    .to_string();
                expanded_provider.api_key = expanded_provider.api_key.trim().to_string();
                expanded_provider.model = expanded_provider.model_name().to_string();
                if !expanded_provider.has_responses_model() {
                    expanded_provider.supports_multimodal = false;
                    expanded_provider.supports_stateful = false;
                }
                if !expanded_provider.has_llm_model() {
                    expanded_provider.supports_multimodal = false;
                }
                expanded_provider.id = make_llm_provider_id(
                    &build_llm_provider_id_seed(
                        &expanded_provider.id,
                        &expanded_provider.name,
                        expanded_provider.model_type,
                        split_from_single_entry,
                    ),
                    &expanded_provider.name,
                    index,
                    &used_ids,
                );
                used_ids.insert(expanded_provider.id.clone());

                if expanded_provider.has_llm_model() {
                    targets.llm_id = Some(expanded_provider.id.clone());
                }
                if expanded_provider.has_embedding_model() {
                    targets.embedding_id = Some(expanded_provider.id.clone());
                }

                normalized_providers.push(expanded_provider);
            }

            id_mapping.insert(&original_id, targets);
        }

        self.providers = normalized_providers;
        self.translation_provider_id = normalize_llm_provider_reference(
            &self.providers,
            &id_mapping,
            original_translation_provider_id.as_deref(),
        );
        self.question_answer_provider_id = normalize_llm_provider_reference(
            &self.providers,
            &id_mapping,
            original_question_answer_provider_id.as_deref(),
        );

        id_mapping
    }
}

fn normalize_llm_provider_reference(
    providers: &[LlmProviderConfig],
    id_mapping: &LlmProviderIdMapping,
    provider_id: Option<&str>,
) -> Option<String> {
    provider_id
        .and_then(|provider_id| {
            id_mapping
                .resolve_llm_id(provider_id)
                .or_else(|| Some(provider_id.to_string()))
        })
        .filter(|provider_id| {
            providers
                .iter()
                .any(|provider| &provider.id == provider_id && provider.has_llm_model())
        })
}

impl OcrSettings {
    fn normalize(&mut self, llm_settings: &LlmSettings, id_mapping: &LlmProviderIdMapping) {
        let resolved_provider_id = self.llm_provider_id.as_deref().and_then(|provider_id| {
            id_mapping
                .resolve_llm_id(provider_id)
                .or_else(|| Some(provider_id.to_string()))
        });

        if resolved_provider_id
            .as_ref()
            .map(|provider_id| {
                llm_settings
                    .providers
                    .iter()
                    .any(|provider| &provider.id == provider_id && provider.has_responses_model())
            })
            .unwrap_or(false)
        {
            self.llm_provider_id = resolved_provider_id;
            return;
        }

        self.llm_provider_id = llm_settings
            .providers
            .iter()
            .find(|provider| provider.has_responses_model())
            .map(|provider| provider.id.clone());
    }
}

impl RagSettings {
    fn normalize(&mut self, llm_settings: &LlmSettings, id_mapping: &LlmProviderIdMapping) {
        let mut seen_directories = HashSet::new();
        self.source_directories = self
            .source_directories
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .filter(|value| seen_directories.insert(value.clone()))
            .collect();

        let mut seen_globs = HashSet::new();
        self.ignore_globs = self
            .ignore_globs
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .filter(|value| seen_globs.insert(value.clone()))
            .collect();

        let resolved_provider_id = self
            .embedding_provider_id
            .as_deref()
            .and_then(|provider_id| {
                id_mapping
                    .resolve_embedding_id(provider_id)
                    .or_else(|| Some(provider_id.to_string()))
            });

        if resolved_provider_id
            .as_ref()
            .map(|provider_id| {
                llm_settings
                    .providers
                    .iter()
                    .any(|provider| &provider.id == provider_id && provider.has_embedding_model())
            })
            .unwrap_or(false)
        {
            self.embedding_provider_id = resolved_provider_id;
            return;
        }

        self.embedding_provider_id = llm_settings
            .providers
            .iter()
            .find(|provider| provider.has_embedding_model())
            .map(|provider| provider.id.clone());
    }
}

fn sanitize_mcp_servers(servers: &[AcpMcpServerConfig]) -> Vec<AcpMcpServerConfig> {
    servers.iter().map(sanitize_mcp_server).collect()
}

fn expand_llm_provider(provider: LlmProviderConfig) -> Vec<LlmProviderConfig> {
    let explicit_model = provider.model_name().to_string();
    let legacy_responses_model = provider.legacy_responses_model_name().to_string();
    let legacy_embedding_model = provider.legacy_embedding_model_name().to_string();
    let has_legacy_split_fields =
        !legacy_responses_model.is_empty() || !legacy_embedding_model.is_empty();
    let has_legacy_protocol_fields =
        provider.legacy_model_type().is_some() || provider.legacy_supports_embedding;
    let mut llm_model = String::new();
    let mut embedding_model = String::new();
    let mut llm_supports_multimodal = false;
    let mut llm_supports_stateful = false;
    let mut llm_protocol = provider.protocol;

    if has_legacy_split_fields {
        llm_model = legacy_responses_model;
        embedding_model = legacy_embedding_model;
        llm_supports_multimodal = provider.supports_multimodal;
        llm_supports_stateful = provider.supports_stateful;
        llm_protocol = LlmProviderProtocol::Responses;
    } else if has_legacy_protocol_fields {
        match provider.legacy_model_type().unwrap_or_default() {
            LlmModelType::Llm => {
                llm_model = explicit_model.clone();
                llm_supports_multimodal = provider.supports_multimodal;
                llm_supports_stateful = provider.supports_stateful;
                llm_protocol = provider
                    .legacy_llm_protocol()
                    .unwrap_or(LlmProviderProtocol::Responses);
                if provider.legacy_supports_embedding {
                    embedding_model = explicit_model;
                }
            }
            LlmModelType::Embedding => {
                embedding_model = explicit_model;
            }
        }
    } else {
        match provider.model_type {
            LlmModelType::Llm => {
                llm_model = explicit_model;
                llm_supports_multimodal = provider.supports_multimodal;
                llm_supports_stateful = provider.supports_stateful;
                llm_protocol = provider.protocol;
            }
            LlmModelType::Embedding => {
                embedding_model = explicit_model;
            }
        }
    }

    if llm_model.is_empty() && embedding_model.is_empty() {
        let mut fallback_provider = provider;
        fallback_provider.model_type = LlmModelType::Llm;
        fallback_provider.protocol = LlmProviderProtocol::Responses;
        fallback_provider.model.clear();
        fallback_provider.supports_multimodal = false;
        fallback_provider.supports_stateful = false;
        clear_legacy_llm_provider_fields(&mut fallback_provider);
        return vec![fallback_provider];
    }

    let mut expanded_providers = Vec::new();
    if !llm_model.is_empty() {
        let mut llm_provider = provider.clone();
        llm_provider.model_type = LlmModelType::Llm;
        llm_provider.protocol = llm_protocol;
        llm_provider.model = llm_model;
        llm_provider.supports_multimodal = llm_supports_multimodal;
        llm_provider.supports_stateful = llm_supports_stateful;
        clear_legacy_llm_provider_fields(&mut llm_provider);
        expanded_providers.push(llm_provider);
    }
    if !embedding_model.is_empty() {
        let mut embedding_provider = provider;
        embedding_provider.model_type = LlmModelType::Embedding;
        embedding_provider.protocol = LlmProviderProtocol::Responses;
        embedding_provider.model = embedding_model;
        embedding_provider.supports_multimodal = false;
        embedding_provider.supports_stateful = false;
        clear_legacy_llm_provider_fields(&mut embedding_provider);
        expanded_providers.push(embedding_provider);
    }

    expanded_providers
}

fn clear_legacy_llm_provider_fields(provider: &mut LlmProviderConfig) {
    provider.legacy_protocol = None;
    provider.legacy_supports_embedding = false;
    provider.legacy_responses_model.clear();
    provider.legacy_embedding_model.clear();
}

fn build_llm_provider_id_seed(
    current_id: &str,
    name: &str,
    model_type: LlmModelType,
    split_from_single_entry: bool,
) -> String {
    let seed = if current_id.trim().is_empty() {
        name.trim().to_string()
    } else {
        current_id.trim().to_string()
    };
    if !split_from_single_entry || model_type == LlmModelType::Llm {
        return seed;
    }

    if seed.is_empty() {
        "embedding".to_string()
    } else {
        format!("{seed}-embedding")
    }
}

fn normalize_llm_provider_name(
    name: &str,
    model: &str,
    base_url: &str,
    model_type: LlmModelType,
    split_from_single_entry: bool,
) -> String {
    let trimmed = name.trim();
    if !trimmed.is_empty() {
        if split_from_single_entry {
            return format!("{trimmed} · {}", llm_model_type_label(model_type));
        }
        return trimmed.to_string();
    }

    let trimmed_model = model.trim();
    if !trimmed_model.is_empty() {
        return trimmed_model.to_string();
    }

    let trimmed_base_url = base_url.trim();
    if !trimmed_base_url.is_empty() {
        if split_from_single_entry {
            return format!("{trimmed_base_url} · {}", llm_model_type_label(model_type));
        }
        return trimmed_base_url.to_string();
    }

    format!("{} 模型", llm_model_type_label(model_type))
}

fn llm_model_type_label(model_type: LlmModelType) -> &'static str {
    match model_type {
        LlmModelType::Llm => "LLM",
        LlmModelType::Embedding => "Embedding",
    }
}

fn sanitize_mcp_server(server: &AcpMcpServerConfig) -> AcpMcpServerConfig {
    match server {
        AcpMcpServerConfig::Stdio(config) => AcpMcpServerConfig::Stdio(AcpMcpServerStdioConfig {
            name: normalize_mcp_server_name(&config.name, Some(&config.command), None),
            command: config.command.trim().to_string(),
            args: config
                .args
                .iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect(),
            env: sanitize_name_value_pairs(&config.env),
        }),
        AcpMcpServerConfig::Http(config) => AcpMcpServerConfig::Http(AcpMcpServerHttpConfig {
            name: normalize_mcp_server_name(&config.name, None, Some(&config.url)),
            url: config.url.trim().to_string(),
            headers: sanitize_name_value_pairs(&config.headers),
        }),
        AcpMcpServerConfig::Sse(config) => AcpMcpServerConfig::Sse(AcpMcpServerSseConfig {
            name: normalize_mcp_server_name(&config.name, None, Some(&config.url)),
            url: config.url.trim().to_string(),
            headers: sanitize_name_value_pairs(&config.headers),
        }),
    }
}

fn sanitize_name_value_pairs(pairs: &[AcpNameValuePair]) -> Vec<AcpNameValuePair> {
    pairs
        .iter()
        .map(|pair| AcpNameValuePair {
            name: pair.name.trim().to_string(),
            value: pair.value.trim().to_string(),
        })
        .filter(|pair| !pair.name.is_empty())
        .collect()
}

fn normalize_mcp_server_name(name: &str, command: Option<&str>, url: Option<&str>) -> String {
    let trimmed = name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }

    if let Some(command) = command.map(str::trim).filter(|value| !value.is_empty()) {
        let first_token = command.split_whitespace().next().unwrap_or_default();
        let normalized = first_token
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(first_token);
        if !normalized.is_empty() {
            return normalized.to_string();
        }
    }

    if let Some(url) = url.map(str::trim).filter(|value| !value.is_empty()) {
        return url.to_string();
    }

    "MCP Server".to_string()
}

fn derive_agent_name(shell_command: Option<&str>, program: &str) -> String {
    let command = shell_command
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(program.trim());
    let first_token = command.split_whitespace().next().unwrap_or_default();
    let normalized = first_token
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(first_token);
    let lowercase = normalized.to_ascii_lowercase();

    if lowercase.contains("opencode") {
        return "OpenCode".to_string();
    }
    if lowercase.contains("claude-agent") {
        return "Claude Agent".to_string();
    }
    if lowercase.contains("codex") {
        return "Codex".to_string();
    }
    if normalized.is_empty() {
        return "ACP Agent".to_string();
    }

    normalized.to_string()
}

fn make_agent_id(seed: &str, index: usize, used_ids: &HashSet<String>) -> String {
    let mut base = seed
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    base = base.trim_matches('-').to_string();
    if base.is_empty() {
        base = format!("agent-{}", index + 1);
    }

    let mut candidate = base.clone();
    let mut suffix = 2_u32;
    while used_ids.contains(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix = suffix.saturating_add(1);
    }
    candidate
}

fn make_llm_provider_id(
    current_id: &str,
    name: &str,
    index: usize,
    used_ids: &HashSet<String>,
) -> String {
    let seed = if current_id.trim().is_empty() {
        name
    } else {
        current_id
    };

    let mut base = seed
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    base = base.trim_matches('-').to_string();
    if base.is_empty() {
        base = format!("llm-provider-{}", index + 1);
    }

    let mut candidate = base.clone();
    let mut suffix = 2_u32;
    while used_ids.contains(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix = suffix.saturating_add(1);
    }
    candidate
}

fn parse_config_content(content: &str) -> Result<AppConfig> {
    let mut config: AppConfig =
        toml::from_str(content).with_context(|| "failed to parse config file")?;
    config.normalize();
    Ok(config)
}

fn serialize_config_content(config: &AppConfig) -> Result<String> {
    toml::to_string_pretty(config).with_context(|| "failed to serialize config")
}

fn parse_workspace_history_content(content: &str) -> Result<WorkspaceHistory> {
    toml::from_str(content).with_context(|| "failed to parse workspace history file")
}

fn serialize_workspace_history_content(history: &WorkspaceHistory) -> Result<String> {
    toml::to_string_pretty(history).with_context(|| "failed to serialize workspace history")
}

fn migrate_legacy_shortcuts(config: &mut AppConfig) -> bool {
    if config.shortcuts.ocr_capture != LEGACY_DEFAULT_OCR_SHORTCUT {
        return false;
    }

    config.shortcuts.ocr_capture = ShortcutConfig::default().ocr_capture;
    true
}

pub async fn safe_write(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .context("write target must have parent directory")?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("failed to create config directory: {}", parent.display()))?;

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        std::process::id(),
        unique
    ));

    tokio::fs::write(&temp_path, content)
        .await
        .with_context(|| format!("failed to write temp file: {}", temp_path.display()))?;

    match tokio::fs::rename(&temp_path, path).await {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            #[cfg(target_os = "windows")]
            {
                if path.exists() {
                    tokio::fs::remove_file(path).await.with_context(|| {
                        format!("failed to replace existing file: {}", path.display())
                    })?;
                    tokio::fs::rename(&temp_path, path).await.with_context(|| {
                        format!("failed to move temp file into place: {}", path.display())
                    })?;
                    return Ok(());
                }
            }

            let _ = tokio::fs::remove_file(&temp_path).await;
            Err(rename_error)
                .with_context(|| format!("failed to move temp file into place: {}", path.display()))
        }
    }
}

pub fn normalize_workspace_root(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to determine current directory")?
            .join(path)
    };

    let normalized = candidate
        .canonicalize()
        .with_context(|| format!("failed to resolve workspace path: {}", candidate.display()))?;

    if !normalized.is_dir() {
        anyhow::bail!(
            "workspace path is not a directory: {}",
            normalized.display()
        );
    }

    Ok(normalized)
}

pub fn default_workspace_root() -> Option<PathBuf> {
    home_workspace_root().or_else(|| std::env::current_dir().ok())
}

pub fn home_workspace_root() -> Option<PathBuf> {
    dirs::home_dir().and_then(|path| normalize_workspace_root(&path).ok())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn display_home_as_tilde() -> bool {
    true
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn display_home_as_tilde() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_shortcut_config_default() {
        let config = ShortcutConfig::default();
        assert_eq!(config.toggle_launcher, "Alt+Space");
        assert_eq!(config.ocr_capture, "Alt+R");
        assert_eq!(config.ocr_translate, "Alt+D");
    }

    #[test]
    fn shortcut_key_round_trips_config_access() {
        let mut config = ShortcutConfig::default();
        let key = ShortcutKey::parse("ocr_translate").expect("known shortcut key should parse");

        assert_eq!(key.as_str(), "ocr_translate");
        assert_eq!(config.get(key), "Alt+D");

        config.set(key, "Cmd+Alt+D");

        assert_eq!(config.get(key), "Cmd+Alt+D");
        assert_eq!(ShortcutConfig::default_value(key), "Alt+D");
    }

    #[test]
    fn normalize_workspace_root_rejects_missing_directory() {
        let missing = std::env::temp_dir().join("wabity-config-missing");
        let result = normalize_workspace_root(&missing);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn safe_write_replaces_existing_file_atomically() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("wabity-config-safe-write-{unique}"));
        let file_path = root.join("config.toml");

        safe_write(&file_path, "alpha").await.expect("first write");
        safe_write(&file_path, "beta").await.expect("second write");

        let content = tokio::fs::read_to_string(&file_path)
            .await
            .expect("read config");
        assert_eq!(content, "beta");

        let _ = tokio::fs::remove_file(&file_path).await;
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[test]
    fn workspace_history_round_trips_as_toml() {
        let history = WorkspaceHistory {
            recent_roots: vec!["/tmp/demo".to_string()],
        };
        let content = serialize_workspace_history_content(&history)
            .expect("history serialization should succeed");
        let parsed =
            parse_workspace_history_content(&content).expect("history parsing should succeed");

        assert_eq!(parsed.recent_roots, history.recent_roots);
    }

    #[test]
    fn app_config_round_trips_as_toml() {
        let mut config = AppConfig::default();
        config.llm.providers = vec![crate::domain::settings::LlmProviderConfig {
            id: "openai".to_string(),
            name: "OpenAI".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            model_type: crate::domain::settings::LlmModelType::Llm,
            model: "gpt-4.1-mini".to_string(),
            supports_multimodal: true,
            ..crate::domain::settings::LlmProviderConfig::default()
        }];
        config.llm.translation_provider_id = Some("openai".to_string());
        config.llm.question_answer_provider_id = Some("openai".to_string());
        config.ocr.provider = crate::domain::settings::OcrProviderKind::LlmOcr;
        config.ocr.llm_provider_id = Some("openai".to_string());
        config.rag.source_directories = vec!["/tmp/workspace".to_string()];
        config.rag.ignore_globs = vec!["**/*.png".to_string()];
        config.acp.agents.push(AcpAgentConfig {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            program: "codex-acp".to_string(),
            args: Vec::new(),
            shell_command: Some("codex-acp".to_string()),
            mcp_servers: Vec::new(),
        });
        config.acp.mcp_servers = vec![
            AcpMcpServerConfig::Stdio(AcpMcpServerStdioConfig {
                name: "filesystem".to_string(),
                command: "npx".to_string(),
                args: vec![
                    "-y".to_string(),
                    "@modelcontextprotocol/server-filesystem".to_string(),
                ],
                env: vec![AcpNameValuePair {
                    name: "ROOT".to_string(),
                    value: "/tmp".to_string(),
                }],
            }),
            AcpMcpServerConfig::Http(AcpMcpServerHttpConfig {
                name: "remote".to_string(),
                url: "https://example.com/mcp".to_string(),
                headers: vec![AcpNameValuePair {
                    name: "Authorization".to_string(),
                    value: "Bearer token".to_string(),
                }],
            }),
        ];
        let content = serialize_config_content(&config).expect("toml serialization should succeed");
        let parsed = parse_config_content(&content).expect("toml parsing should succeed");

        assert_eq!(
            parsed.shortcuts.toggle_launcher,
            config.shortcuts.toggle_launcher
        );
        assert_eq!(parsed.general.language, config.general.language);
        assert_eq!(parsed.appearance.theme, config.appearance.theme);
        assert_eq!(parsed.prompts, config.prompts);
        assert_eq!(parsed.llm, config.llm);
        assert_eq!(parsed.ocr.provider, config.ocr.provider);
        assert_eq!(parsed.ocr.llm_provider_id, config.ocr.llm_provider_id);
        assert_eq!(parsed.rag, config.rag);
        assert_eq!(parsed.acp.mcp_servers, config.acp.mcp_servers);
    }

    #[test]
    fn parse_config_content_fills_missing_shortcut_fields_from_defaults() {
        let content = r#"
[shortcuts]
toggle_launcher = "Alt+Space"

[workspace]
root_path = "/Users/wweir"

[general]
autoStart = false
showInDock = true
language = "zh-CN"

[appearance]
theme = "auto"
fontSize = "medium"

[acp]
program = ""
args = []
saved_sessions = []
"#;

        let parsed = parse_config_content(content).expect("legacy config should parse");

        assert_eq!(parsed.shortcuts.ocr_capture, "Alt+R");
        assert_eq!(parsed.shortcuts.ocr_translate, "Alt+D");
        assert_eq!(parsed.ocr, OcrSettings::default());
        assert_eq!(parsed.llm, LlmSettings::default());
        assert_eq!(
            parsed.prompts.translation_prompt,
            crate::domain::settings::default_translation_prompt()
        );
        assert!(parsed.acp.agents.is_empty());
    }

    #[test]
    fn parse_config_content_restores_default_translation_prompt_when_blank() {
        let content = r#"
[general]
autoStart = false
showInDock = true
language = "zh-CN"
translationPrompt = "   "
"#;

        let parsed = parse_config_content(content).expect("blank translation prompt should parse");

        assert_eq!(
            parsed.prompts.translation_prompt,
            crate::domain::settings::default_translation_prompt()
        );
    }

    #[test]
    fn parse_config_content_restores_default_rag_answer_prompt_when_blank() {
        let content = r#"
[prompts]
ragAnswerSystemPrompt = "   "
"#;

        let parsed = parse_config_content(content).expect("blank rag answer prompt should parse");

        assert_eq!(
            parsed.prompts.rag_answer_system_prompt,
            crate::domain::settings::default_rag_answer_system_prompt()
        );
    }

    #[test]
    fn parse_config_content_migrates_legacy_translation_prompt_into_prompts_section() {
        let content = r#"
[general]
autoStart = false
showInDock = true
language = "zh-CN"
translationPrompt = "Translate only."
"#;

        let parsed = parse_config_content(content).expect("legacy translation prompt should parse");

        assert_eq!(parsed.prompts.translation_prompt, "Translate only.");
    }

    #[test]
    fn parse_config_content_migrates_legacy_acp_agent() {
        let content = r#"
[acp]
program = "codex-acp"
args = []
shell_command = "codex-acp"
saved_sessions = []
"#;

        let parsed = parse_config_content(content).expect("legacy acp agent should parse");

        assert_eq!(parsed.acp.agents.len(), 1);
        assert_eq!(parsed.acp.agents[0].name, "Codex");
        assert_eq!(parsed.acp.agents[0].program, "codex-acp");
        assert_eq!(
            parsed.acp.default_agent_id.as_deref(),
            Some(parsed.acp.agents[0].id.as_str())
        );
    }

    #[test]
    fn migrate_legacy_shortcuts_updates_old_default_ocr_shortcut() {
        let mut config = AppConfig::default();
        config.shortcuts.ocr_capture = LEGACY_DEFAULT_OCR_SHORTCUT.to_string();

        assert!(migrate_legacy_shortcuts(&mut config));
        assert_eq!(config.shortcuts.ocr_capture, "Alt+R");
    }

    #[test]
    fn migrate_legacy_shortcuts_keeps_custom_ocr_shortcut() {
        let mut config = AppConfig::default();
        config.shortcuts.ocr_capture = "Cmd+Alt+O".to_string();

        assert!(!migrate_legacy_shortcuts(&mut config));
        assert_eq!(config.shortcuts.ocr_capture, "Cmd+Alt+O");
    }

    #[test]
    fn parse_config_content_migrates_legacy_default_llm_provider_id() {
        let content = r#"
[llm]
defaultProviderId = "missing"

[[llm.providers]]
id = ""
name = "OpenAI"
protocol = "openai_compatible"
baseUrl = "https://api.openai.com/v1/"
apiKey = ""
model = "gpt-4.1-mini"
supportsMultimodal = true
"#;

        let parsed = parse_config_content(content).expect("llm config should parse");

        assert_eq!(parsed.llm.providers.len(), 1);
        assert_eq!(parsed.llm.providers[0].id, "openai");
        assert_eq!(
            parsed.llm.providers[0].base_url,
            "https://api.openai.com/v1"
        );
        assert_eq!(parsed.llm.translation_provider_id.as_deref(), None);
        assert_eq!(parsed.llm.question_answer_provider_id.as_deref(), None);
        assert_eq!(parsed.ocr.llm_provider_id, None);
    }

    #[test]
    fn parse_config_content_maps_legacy_chat_protocol_to_chat_completions() {
        let content = r#"
[llm]

[[llm.providers]]
id = "chat"
name = "Chat"
protocol = "openai_compatible"
baseUrl = "https://api.example.com/v1"
apiKey = ""
model = "gpt-4.1-mini"
"#;

        let parsed = parse_config_content(content).expect("legacy chat config should parse");

        assert_eq!(parsed.llm.providers.len(), 1);
        assert_eq!(
            parsed.llm.providers[0].protocol,
            crate::domain::settings::LlmProviderProtocol::ChatCompletions
        );
    }

    #[test]
    fn parse_config_content_splits_combined_provider_and_repairs_references() {
        let content = r#"
[llm]
defaultProviderId = "combo"

[[llm.providers]]
id = "combo"
name = "OpenAI"
baseUrl = "https://api.openai.com/v1/"
apiKey = ""
responsesModel = "gpt-4.1-mini"
supportsMultimodal = true
embeddingModel = "text-embedding-3-small"

[ocr]
provider = "llm_ocr"
llmProviderId = "combo"

[rag]
embeddingProviderId = "combo"
"#;

        let parsed = parse_config_content(content).expect("combined provider config should parse");

        assert_eq!(parsed.llm.providers.len(), 2);
        let llm_provider = parsed
            .llm
            .providers
            .iter()
            .find(|provider| provider.has_responses_model())
            .expect("llm provider should exist");
        let embedding_provider = parsed
            .llm
            .providers
            .iter()
            .find(|provider| provider.has_embedding_model())
            .expect("embedding provider should exist");
        assert_eq!(llm_provider.id, "combo");
        assert_eq!(embedding_provider.id, "combo-embedding");
        assert_eq!(parsed.llm.translation_provider_id.as_deref(), Some("combo"));
        assert_eq!(
            parsed.llm.question_answer_provider_id.as_deref(),
            Some("combo")
        );
        assert_eq!(parsed.ocr.llm_provider_id.as_deref(), Some("combo"));
        assert_eq!(
            parsed.rag.embedding_provider_id.as_deref(),
            Some("combo-embedding")
        );
    }

    #[test]
    fn normalize_rag_settings_deduplicates_inputs_and_repairs_provider_reference() {
        let mut config = AppConfig::default();
        config.llm.providers = vec![
            crate::domain::settings::LlmProviderConfig {
                id: "chat".to_string(),
                name: "Chat".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                model_type: crate::domain::settings::LlmModelType::Llm,
                model: "gpt-4.1-mini".to_string(),
                supports_multimodal: true,
                ..crate::domain::settings::LlmProviderConfig::default()
            },
            crate::domain::settings::LlmProviderConfig {
                id: "embedding".to_string(),
                name: "Embedding".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                model_type: crate::domain::settings::LlmModelType::Embedding,
                model: "text-embedding-3-small".to_string(),
                supports_multimodal: false,
                ..crate::domain::settings::LlmProviderConfig::default()
            },
        ];
        config.rag.source_directories = vec![
            " /tmp/workspace ".to_string(),
            "/tmp/workspace".to_string(),
            "/tmp/notes".to_string(),
        ];
        config.rag.ignore_globs = vec![
            " **/*.png ".to_string(),
            "**/*.png".to_string(),
            " **/node_modules/** ".to_string(),
        ];
        config.rag.embedding_provider_id = Some("missing".to_string());

        config.normalize();

        assert_eq!(
            config.rag.source_directories,
            vec!["/tmp/workspace".to_string(), "/tmp/notes".to_string()]
        );
        assert_eq!(
            config.rag.ignore_globs,
            vec!["**/*.png".to_string(), "**/node_modules/**".to_string()]
        );
        assert_eq!(
            config.rag.embedding_provider_id.as_deref(),
            Some("embedding")
        );
    }

    #[test]
    fn parse_config_content_uses_default_rag_ignore_globs_when_missing() {
        let content = r#"
[rag]
sourceDirectories = ["/tmp/docs"]
"#;

        let parsed = parse_config_content(content).expect("rag config should parse");

        assert_eq!(
            parsed.rag.ignore_globs,
            crate::domain::settings::default_rag_ignore_globs()
        );
    }
}
