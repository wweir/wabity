use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, RwLock as StdRwLock,
    },
    time::Duration,
};

use anyhow::{Context, Result};
use serde_json::Value;
use tauri_plugin_global_shortcut::Shortcut;
use tokio::sync::RwLock as AsyncRwLock;

#[cfg(target_os = "macos")]
use crate::services::ocr::MacOsVisionOcrProvider;
use crate::services::{
    acp::AcpService,
    application::ApplicationService,
    executor::ExecutorService,
    file_search::FileSearchService,
    matcher::MatcherService,
    ocr::{OcrProvider, OpenAiCompatibleOcrProvider, UnavailableOcrProvider},
    rag::{self, RagIndexService},
};
use crate::{
    domain::{
        acp::{
            AcpAgentCatalog, AcpAgentConfig, AcpMcpServerCatalog, AcpMcpServerConfig,
            AcpNameValuePair, AcpRestoreNotice, AcpSessionDetail, AcpSessionSummary,
        },
        rag::RagScanResult,
        settings::{
            AppSettings, LlmProviderConfig, LlmSettings, OcrProviderKind, OcrSettings, RagSettings,
        },
        skills::PublicSkillCatalog,
        workspace::WorkspaceState,
    },
    infrastructure::config::{
        default_workspace_root, display_home_as_tilde, home_workspace_root,
        normalize_workspace_root, AppConfig, ConfigStore, SavedAcpSession, ShortcutConfig,
        WorkspaceHistory, RECENT_WORKSPACE_LIMIT,
    },
    services::public_skills::PublicSkillService,
};

#[derive(Clone)]
pub struct AppState {
    matcher: MatcherService,
    executor: ExecutorService,
    application: ApplicationService,
    file_search: FileSearchService,
    acp: AcpService,
    ocr_provider: Arc<StdRwLock<Arc<dyn OcrProvider>>>,
    rag_index: RagIndexService,
    config_store: Arc<AsyncRwLock<ConfigStore>>,
    workspace_state: Arc<AsyncRwLock<WorkspaceState>>,
}

impl AppState {
    pub async fn new(matcher: MatcherService, executor: ExecutorService) -> Result<Self> {
        let config_store = Arc::new(AsyncRwLock::new(ConfigStore::new()?));
        let (config, history) = {
            let store = config_store.read().await;
            let config = store.load().await?;
            let history = store.load_workspace_history().await?;
            (config, history)
        };
        let root_path = normalize_workspace_root(&config.workspace.root_path)
            .ok()
            .or_else(default_workspace_root)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()))
            .to_string_lossy()
            .into_owned();
        let initial_workspace = WorkspaceState {
            root_path: root_path.clone(),
            recent_roots: normalize_recent_roots(&history, &root_path),
            home_path: home_workspace_root().map(|path| path.to_string_lossy().into_owned()),
            display_home_as_tilde: display_home_as_tilde(),
        };
        let rag_index = RagIndexService::new(ConfigStore::config_dir()?);
        rag_index
            .apply_settings(config.rag.clone(), config.llm.clone())
            .await;

        Ok(Self {
            matcher,
            executor,
            application: ApplicationService::new()?,
            file_search: FileSearchService::new()?,
            acp: AcpService::new(),
            ocr_provider: Arc::new(StdRwLock::new(build_ocr_provider(&config.ocr, &config.llm))),
            rag_index,
            config_store,
            workspace_state: Arc::new(AsyncRwLock::new(initial_workspace)),
        })
    }

    pub fn matcher(&self) -> &MatcherService {
        &self.matcher
    }

    pub fn executor(&self) -> &ExecutorService {
        &self.executor
    }

    pub fn file_search(&self) -> &FileSearchService {
        &self.file_search
    }

    pub fn application(&self) -> &ApplicationService {
        &self.application
    }

    pub fn acp(&self) -> &AcpService {
        &self.acp
    }

    pub fn ocr_provider(&self) -> Arc<dyn OcrProvider> {
        self.ocr_provider.read().unwrap().clone()
    }

    pub async fn app_config(&self) -> Result<AppConfig> {
        let store = self.config_store.read().await;
        store.load().await
    }

    pub async fn config(&self) -> Result<ShortcutConfig> {
        Ok(self.app_config().await?.shortcuts)
    }

    pub async fn workspace(&self) -> Result<WorkspaceState> {
        Ok(self.workspace_state.read().await.clone())
    }

    pub async fn acp_agents(&self) -> Result<AcpAgentCatalog> {
        let config = self.app_config().await?;
        Ok(AcpAgentCatalog {
            agents: config.acp.agents,
            default_agent_id: config.acp.default_agent_id,
        })
    }

    pub async fn acp_mcp_servers(&self) -> Result<AcpMcpServerCatalog> {
        let config = self.app_config().await?;
        Ok(AcpMcpServerCatalog {
            servers: config.acp.mcp_servers,
        })
    }

    pub async fn app_settings(&self) -> Result<AppSettings> {
        let config = self.app_config().await?;
        Ok(AppSettings {
            general: config.general,
            appearance: config.appearance,
            llm: config.llm,
            ocr: config.ocr,
            rag: config.rag,
        })
    }

    pub async fn list_llm_provider_models(
        &self,
        provider: LlmProviderConfig,
    ) -> Result<Vec<String>> {
        tokio::task::spawn_blocking(move || fetch_llm_provider_models(&provider))
            .await
            .context("failed to join LLM model list task")?
    }

    pub async fn public_skill_catalog(&self) -> Result<PublicSkillCatalog> {
        PublicSkillService::load_catalog()
    }

    pub async fn update_shortcut(&self, key: &str, shortcut: &str) -> Result<()> {
        let store = self.config_store.write().await;
        let mut config = store.load().await?;
        match key {
            "toggle_launcher" => config.shortcuts.toggle_launcher = shortcut.to_string(),
            "ocr_capture" => config.shortcuts.ocr_capture = shortcut.to_string(),
            other => anyhow::bail!("unknown shortcut key: {other}"),
        }
        store.save(&config).await?;
        Ok(())
    }

    pub async fn update_workspace_root(&self, root_path: &str) -> Result<WorkspaceState> {
        let normalized = normalize_workspace_root(root_path)?;
        let normalized_string = normalized.to_string_lossy().into_owned();
        let mut next_workspace = self.workspace_state.read().await.clone();
        next_workspace.root_path = normalized_string.clone();
        next_workspace
            .recent_roots
            .retain(|root| root != &normalized_string);
        next_workspace
            .recent_roots
            .insert(0, normalized_string.clone());
        next_workspace.recent_roots.truncate(RECENT_WORKSPACE_LIMIT);
        next_workspace.home_path =
            home_workspace_root().map(|path| path.to_string_lossy().into_owned());
        next_workspace.display_home_as_tilde = display_home_as_tilde();

        {
            let store = self.config_store.write().await;
            let mut config = store.load().await?;
            config.workspace.root_path = normalized_string;
            store.save(&config).await?;
            store
                .save_workspace_history(&WorkspaceHistory {
                    recent_roots: next_workspace.recent_roots.clone(),
                })
                .await?;
        }

        *self.workspace_state.write().await = next_workspace.clone();

        Ok(next_workspace)
    }

    pub async fn update_acp_agents(&self, catalog: AcpAgentCatalog) -> Result<AcpAgentCatalog> {
        let store = self.config_store.write().await;
        let mut config = store.load().await?;
        let normalized_catalog = normalize_acp_agent_catalog(catalog)?;
        config.acp.agents = normalized_catalog.agents.clone();
        config.acp.default_agent_id = normalized_catalog.default_agent_id.clone();
        store.save(&config).await?;
        Ok(normalized_catalog)
    }

    pub async fn update_acp_mcp_servers(
        &self,
        catalog: AcpMcpServerCatalog,
    ) -> Result<AcpMcpServerCatalog> {
        let store = self.config_store.write().await;
        let mut config = store.load().await?;
        let normalized_catalog = normalize_acp_mcp_server_catalog(catalog)?;
        config.acp.mcp_servers = normalized_catalog.servers.clone();
        store.save(&config).await?;
        Ok(normalized_catalog)
    }

    pub async fn update_app_settings(&self, settings: AppSettings) -> Result<AppSettings> {
        validate_llm_settings(&settings.llm)?;
        validate_ocr_settings(&settings.ocr, &settings.llm)?;
        validate_rag_settings(&settings.rag, &settings.llm)?;
        let store = self.config_store.write().await;
        let mut config = store.load().await?;
        config.general = settings.general.clone();
        config.appearance = settings.appearance.clone();
        config.llm = settings.llm.clone();
        config.ocr = settings.ocr.clone();
        config.rag = settings.rag.clone();
        config.normalize();
        store.save(&config).await?;
        *self.ocr_provider.write().unwrap() = build_ocr_provider(&config.ocr, &config.llm);
        self.rag_index
            .apply_settings(config.rag.clone(), config.llm.clone())
            .await;
        Ok(AppSettings {
            general: config.general,
            appearance: config.appearance,
            llm: config.llm,
            ocr: config.ocr,
            rag: config.rag,
        })
    }

    pub async fn scan_rag_sources(
        &self,
        rag_settings: RagSettings,
        llm_settings: LlmSettings,
    ) -> Result<RagScanResult> {
        validate_rag_settings(&rag_settings, &llm_settings)?;
        let config_dir = ConfigStore::config_dir()?;
        rag::scan_rag_sources(&config_dir, &rag_settings, &llm_settings).await
    }

    pub async fn create_acp_session(&self, agent_id: Option<String>) -> Result<AcpSessionDetail> {
        let workspace = self.workspace().await?;
        let agent = self.resolve_acp_agent(agent_id.as_deref()).await?;
        let mcp_servers = self.acp_mcp_servers().await?.servers;
        let runtime_agent = hydrate_runtime_agent(agent.clone(), mcp_servers.clone());
        let workspace_root = normalize_workspace_root(&workspace.root_path)?;
        let detail = self
            .acp
            .create_session(workspace_root, runtime_agent.clone(), mcp_servers.clone())
            .await?;
        self.save_session_snapshot(&detail.session, &agent, &mcp_servers)
            .await?;
        Ok(detail)
    }

    pub async fn activate_acp_session(
        &self,
        session_id: Option<String>,
    ) -> Result<Vec<AcpSessionSummary>> {
        let summaries = self.acp.activate_session(session_id.clone()).await;
        let store = self.config_store.write().await;
        let mut config = store.load().await?;
        config.acp.active_session_id = session_id;
        store.save(&config).await?;
        Ok(summaries)
    }

    pub async fn send_acp_prompt(
        &self,
        session_id: &str,
        prompt: String,
    ) -> Result<AcpSessionDetail> {
        let detail = self.acp.send_prompt(session_id, prompt).await?;
        let session_runtime_config = self
            .acp
            .session_runtime_config(session_id)
            .await
            .with_context(|| format!("unknown ACP session: {session_id}"))?;
        self.save_session_snapshot(
            &detail.session,
            &session_runtime_config.agent,
            &session_runtime_config.mcp_servers,
        )
        .await?;
        Ok(detail)
    }

    pub async fn close_acp_session(&self, session_id: &str) -> Result<()> {
        self.acp.close_session(session_id).await?;
        self.remove_session_snapshot(session_id).await?;
        Ok(())
    }

    pub async fn take_acp_restore_notices(&self) -> Vec<AcpRestoreNotice> {
        self.acp.take_restore_notices().await
    }

    pub async fn restore_acp_sessions(&self) -> Result<Vec<AcpSessionDetail>> {
        let config = self.app_config().await?;
        let snapshots = config.acp.saved_sessions.clone();
        let active_session_id = config.acp.active_session_id.clone();
        let mut restored = Vec::new();
        let mut retained = Vec::new();

        for snapshot in snapshots {
            let result = self.acp.restore_session(snapshot.clone()).await;
            if result.keep_snapshot {
                retained.push(snapshot.clone());
            }
            if let Some(detail) = result.restored {
                restored.push(detail);
            }
        }

        let store = self.config_store.write().await;
        let mut next_config = store.load().await?;
        next_config.acp.saved_sessions = retained;
        next_config.acp.active_session_id = active_session_id.filter(|session_id| {
            restored
                .iter()
                .any(|detail| detail.session.session_id == *session_id)
        });
        store.save(&next_config).await?;

        if let Some(active_session_id) = next_config.acp.active_session_id.clone() {
            let _ = self.acp.activate_session(Some(active_session_id)).await;
        }

        Ok(restored)
    }

    async fn save_session_snapshot(
        &self,
        summary: &AcpSessionSummary,
        agent: &AcpAgentConfig,
        mcp_servers: &[AcpMcpServerConfig],
    ) -> Result<()> {
        let store = self.config_store.write().await;
        let mut config = store.load().await?;
        let snapshot = SavedAcpSession {
            session_id: summary.session_id.clone(),
            workspace_root: summary.workspace_root.clone(),
            title: summary.title.clone(),
            agent_id: Some(agent.id.clone()),
            agent_name: agent.name.clone(),
            agent_program: agent.program.clone(),
            agent_args: agent.args.clone(),
            agent_shell_command: agent.shell_command.clone(),
            mcp_servers: mcp_servers.to_vec(),
            last_updated_at_ms: summary.last_updated_at_ms,
        };
        config
            .acp
            .saved_sessions
            .retain(|item| item.session_id != snapshot.session_id);
        config.acp.saved_sessions.push(snapshot);
        config.acp.saved_sessions.sort_by(|left, right| {
            right
                .last_updated_at_ms
                .cmp(&left.last_updated_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        config.acp.saved_sessions.truncate(16);
        store.save(&config).await?;
        Ok(())
    }

    async fn remove_session_snapshot(&self, session_id: &str) -> Result<()> {
        let store = self.config_store.write().await;
        let mut config = store.load().await?;
        config
            .acp
            .saved_sessions
            .retain(|item| item.session_id != session_id);
        if config.acp.active_session_id.as_deref() == Some(session_id) {
            config.acp.active_session_id = None;
        }
        store.save(&config).await?;
        Ok(())
    }

    async fn resolve_acp_agent(&self, agent_id: Option<&str>) -> Result<AcpAgentConfig> {
        let catalog = self.acp_agents().await?;
        let selected_agent_id = agent_id
            .map(ToOwned::to_owned)
            .or(catalog.default_agent_id.clone());
        let Some(selected_agent_id) = selected_agent_id else {
            anyhow::bail!("ACP agent 未配置");
        };

        catalog
            .agents
            .into_iter()
            .find(|agent| agent.id == selected_agent_id)
            .with_context(|| format!("unknown ACP agent: {selected_agent_id}"))
    }
}

fn build_ocr_provider(settings: &OcrSettings, llm_settings: &LlmSettings) -> Arc<dyn OcrProvider> {
    match settings.provider {
        OcrProviderKind::Disabled => Arc::new(UnavailableOcrProvider::new(
            "ocr provider is disabled for image path",
        )),
        OcrProviderKind::System => build_system_ocr_provider(),
        OcrProviderKind::LlmOcr => {
            match resolve_llm_provider(settings, llm_settings).and_then(|provider| {
                if provider.protocol != crate::domain::settings::LlmProviderProtocolKind::Chat {
                    anyhow::bail!("OCR 当前只支持 OpenAI Chat 协议的 LLM provider")
                }
                if !provider.supports_multimodal {
                    anyhow::bail!("OCR 选择的 LLM provider 未启用多模态能力")
                }
                OpenAiCompatibleOcrProvider::from_config(provider)
            }) {
                Ok(provider) => Arc::new(provider),
                Err(error) => Arc::new(UnavailableOcrProvider::new(format!(
                    "llm_ocr provider is misconfigured ({error:#}) for image path"
                ))),
            }
        }
    }
}

fn validate_llm_settings(settings: &LlmSettings) -> Result<()> {
    for (index, provider) in settings.providers.iter().enumerate() {
        validate_llm_provider_config(provider)
            .with_context(|| format!("第 {} 个 LLM provider 配置非法", index + 1))?;
    }

    if let Some(default_provider_id) = settings.default_provider_id.as_deref() {
        if !settings
            .providers
            .iter()
            .any(|provider| provider.id == default_provider_id)
        {
            anyhow::bail!("默认 LLM provider 不存在: {default_provider_id}");
        }
    }

    Ok(())
}

fn validate_ocr_settings(settings: &OcrSettings, llm_settings: &LlmSettings) -> Result<()> {
    match settings.provider {
        OcrProviderKind::Disabled => Ok(()),
        OcrProviderKind::System => validate_system_ocr_settings(),
        OcrProviderKind::LlmOcr => {
            let provider = resolve_llm_provider(settings, llm_settings)?;
            if provider.protocol != crate::domain::settings::LlmProviderProtocolKind::Chat {
                anyhow::bail!("OCR 当前只支持 OpenAI Chat 协议的 LLM provider");
            }
            if !provider.supports_multimodal {
                anyhow::bail!("OCR 选择的 LLM provider 未启用多模态能力");
            }
            OpenAiCompatibleOcrProvider::from_config(provider).map(|_| ())
        }
    }
}

fn validate_rag_settings(settings: &RagSettings, llm_settings: &LlmSettings) -> Result<()> {
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

    if !settings.source_directories.is_empty() && settings.embedding_provider_id.is_none() {
        anyhow::bail!("RAG 配置了扫描目录时，必须选择一个 embedding provider");
    }

    if let Some(provider_id) = settings.embedding_provider_id.as_deref() {
        let provider = llm_settings
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .with_context(|| format!("RAG 选择的 embedding provider 不存在: {provider_id}"))?;
        if provider.protocol != crate::domain::settings::LlmProviderProtocolKind::Embedding {
            anyhow::bail!("RAG 只接受 OpenAI Embedding 协议的 provider");
        }
        validate_llm_provider_config(provider)
            .with_context(|| format!("RAG 选择的 embedding provider 配置非法: {provider_id}"))?;
    }

    Ok(())
}

fn resolve_llm_provider<'a>(
    ocr_settings: &OcrSettings,
    llm_settings: &'a LlmSettings,
) -> Result<&'a LlmProviderConfig> {
    let provider_id = ocr_settings
        .llm_provider_id
        .as_deref()
        .context("OCR provider 为 llm_ocr 时必须选择一个 LLM provider")?;

    llm_settings
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .with_context(|| format!("OCR 选择的 LLM provider 不存在: {provider_id}"))
}

fn validate_llm_provider_config(provider: &LlmProviderConfig) -> Result<()> {
    if provider.id.trim().is_empty() {
        anyhow::bail!("LLM provider id 不能为空");
    }
    if provider.name.trim().is_empty() {
        anyhow::bail!("LLM provider 名称不能为空");
    }
    if provider.base_url.trim().is_empty() {
        anyhow::bail!("LLM provider base URL 不能为空");
    }
    if provider.model.trim().is_empty() {
        anyhow::bail!("LLM provider model 不能为空");
    }
    if provider.protocol == crate::domain::settings::LlmProviderProtocolKind::Embedding
        && provider.supports_multimodal
    {
        anyhow::bail!("OpenAI Embedding 协议不允许启用多模态");
    }

    Ok(())
}

fn fetch_llm_provider_models(provider: &LlmProviderConfig) -> Result<Vec<String>> {
    let base_url = normalize_llm_provider_base_url(&provider.base_url)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to build HTTP client for LLM model listing")?;
    let mut request = client.get(format!("{base_url}/models"));
    let api_key = provider.api_key.trim();
    if !api_key.is_empty() {
        request = request.bearer_auth(api_key);
    }

    let response = request
        .send()
        .context("failed to request LLM model list from provider")?;
    let status = response.status();
    let body = response
        .text()
        .context("failed to read LLM model list response body")?;

    if !status.is_success() {
        let message = extract_llm_provider_error_message(&body);
        anyhow::bail!("LLM provider /models 请求失败 ({status}): {message}");
    }

    let parsed: Value =
        serde_json::from_str(&body).context("failed to parse LLM model list response JSON")?;
    let models = extract_llm_provider_models(&parsed);
    if models.is_empty() {
        anyhow::bail!("LLM provider /models 未返回可识别的模型列表");
    }

    Ok(models)
}

fn normalize_llm_provider_base_url(base_url: &str) -> Result<String> {
    let normalized = base_url.trim().trim_end_matches('/');
    if normalized.is_empty() {
        anyhow::bail!("LLM provider base URL 不能为空");
    }

    reqwest::Url::parse(normalized)
        .with_context(|| format!("invalid LLM provider base URL: {normalized}"))?;

    Ok(normalized.to_string())
}

fn extract_llm_provider_error_message(body: &str) -> String {
    let parsed = serde_json::from_str::<Value>(body).ok();
    parsed
        .as_ref()
        .and_then(|json| {
            json.get("error")
                .and_then(|value| {
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .or_else(|| value.as_str())
                })
                .or_else(|| json.get("message").and_then(Value::as_str))
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            body.lines()
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("unknown error")
                .to_string()
        })
}

fn extract_llm_provider_models(payload: &Value) -> Vec<String> {
    let mut models = BTreeSet::new();

    if let Some(items) = payload.get("data").and_then(Value::as_array) {
        for item in items {
            if let Some(model_id) = item
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                models.insert(model_id.to_string());
            }
        }
    }

    models.into_iter().collect()
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

fn normalize_recent_roots(history: &WorkspaceHistory, current_root: &str) -> Vec<String> {
    let mut roots = history
        .recent_roots
        .iter()
        .filter_map(|root| normalize_workspace_root(root).ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    roots.retain(|root| root != current_root);
    roots.insert(0, current_root.to_string());
    roots.dedup();
    roots.truncate(RECENT_WORKSPACE_LIMIT);
    roots
}

fn normalize_acp_agent_catalog(catalog: AcpAgentCatalog) -> Result<AcpAgentCatalog> {
    let mut agents = Vec::with_capacity(catalog.agents.len());

    for (index, agent) in catalog.agents.into_iter().enumerate() {
        let name = agent.name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("第 {} 个 ACP agent 名称为空", index + 1);
        }

        let shell_command = agent
            .shell_command
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let program = agent.program.trim().to_string();
        if shell_command.is_none() && program.is_empty() {
            anyhow::bail!("第 {} 个 ACP agent 命令为空", index + 1);
        }

        agents.push(AcpAgentConfig {
            id: agent.id.trim().to_string(),
            name,
            program,
            args: agent
                .args
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect(),
            shell_command,
            mcp_servers: Vec::new(),
        });
    }

    let default_agent_id = if agents.is_empty() {
        None
    } else {
        let requested_default = catalog
            .default_agent_id
            .filter(|default_agent_id| agents.iter().any(|agent| agent.id == *default_agent_id));
        requested_default.or_else(|| agents.first().map(|agent| agent.id.clone()))
    };

    Ok(AcpAgentCatalog {
        agents,
        default_agent_id,
    })
}

fn normalize_acp_mcp_server_catalog(catalog: AcpMcpServerCatalog) -> Result<AcpMcpServerCatalog> {
    Ok(AcpMcpServerCatalog {
        servers: normalize_mcp_servers(catalog.servers, "全局 MCP server")?,
    })
}

fn normalize_mcp_servers(
    servers: Vec<AcpMcpServerConfig>,
    owner_label: &str,
) -> Result<Vec<AcpMcpServerConfig>> {
    let mut normalized = Vec::with_capacity(servers.len());

    for (server_index, server) in servers.into_iter().enumerate() {
        let label = format!("{owner_label} 的第 {} 个 MCP server", server_index + 1);
        let server = match server {
            AcpMcpServerConfig::Stdio(config) => {
                let name = config.name.trim().to_string();
                if name.is_empty() {
                    anyhow::bail!("{label} 名称为空");
                }
                let command = config.command.trim().to_string();
                if command.is_empty() {
                    anyhow::bail!("{label} 的 stdio command 为空");
                }

                AcpMcpServerConfig::Stdio(crate::domain::acp::AcpMcpServerStdioConfig {
                    name,
                    command,
                    args: config
                        .args
                        .into_iter()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                        .collect(),
                    env: normalize_name_value_pairs(config.env, &format!("{label} 的 env"))?,
                })
            }
            AcpMcpServerConfig::Http(config) => {
                let name = config.name.trim().to_string();
                if name.is_empty() {
                    anyhow::bail!("{label} 名称为空");
                }
                let url = config.url.trim().to_string();
                if url.is_empty() {
                    anyhow::bail!("{label} 的 http url 为空");
                }

                AcpMcpServerConfig::Http(crate::domain::acp::AcpMcpServerHttpConfig {
                    name,
                    url,
                    headers: normalize_name_value_pairs(
                        config.headers,
                        &format!("{label} 的 headers"),
                    )?,
                })
            }
            AcpMcpServerConfig::Sse(config) => {
                let name = config.name.trim().to_string();
                if name.is_empty() {
                    anyhow::bail!("{label} 名称为空");
                }
                let url = config.url.trim().to_string();
                if url.is_empty() {
                    anyhow::bail!("{label} 的 sse url 为空");
                }

                AcpMcpServerConfig::Sse(crate::domain::acp::AcpMcpServerSseConfig {
                    name,
                    url,
                    headers: normalize_name_value_pairs(
                        config.headers,
                        &format!("{label} 的 headers"),
                    )?,
                })
            }
        };

        normalized.push(server);
    }

    Ok(normalized)
}

fn hydrate_runtime_agent(
    agent: AcpAgentConfig,
    mcp_servers: Vec<AcpMcpServerConfig>,
) -> AcpAgentConfig {
    AcpAgentConfig {
        mcp_servers,
        ..agent
    }
}

fn normalize_name_value_pairs(
    pairs: Vec<AcpNameValuePair>,
    label: &str,
) -> Result<Vec<AcpNameValuePair>> {
    let mut normalized = Vec::with_capacity(pairs.len());

    for (index, pair) in pairs.into_iter().enumerate() {
        let name = pair.name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("{label} 第 {} 项 key 为空", index + 1);
        }

        normalized.push(AcpNameValuePair {
            name,
            value: pair.value.trim().to_string(),
        });
    }

    Ok(normalized)
}

#[derive(Clone, Default)]
pub struct ShortcutRuntimeState {
    launcher_shortcut: Arc<StdRwLock<Option<Shortcut>>>,
    ocr_shortcut: Arc<StdRwLock<Option<Shortcut>>>,
    launcher_visible: Arc<AtomicBool>,
    launcher_shown_once: Arc<AtomicBool>,
    launcher_shortcut_pressed: Arc<AtomicBool>,
    ocr_shortcut_pressed: Arc<AtomicBool>,
    ocr_capture_active: Arc<AtomicBool>,
    transient_window_interactions: Arc<AtomicUsize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    ToggleLauncher,
    OcrCapture,
}

impl ShortcutRuntimeState {
    pub fn shortcut_action(&self, shortcut: Shortcut) -> Option<ShortcutAction> {
        if self.current_launcher_shortcut() == Some(shortcut) {
            return Some(ShortcutAction::ToggleLauncher);
        }

        if self.current_ocr_shortcut() == Some(shortcut) {
            return Some(ShortcutAction::OcrCapture);
        }

        None
    }

    pub fn current_launcher_shortcut(&self) -> Option<Shortcut> {
        *self.launcher_shortcut.read().unwrap()
    }

    pub fn set_launcher_shortcut(&self, shortcut: Option<Shortcut>) {
        *self.launcher_shortcut.write().unwrap() = shortcut;
    }

    pub fn current_ocr_shortcut(&self) -> Option<Shortcut> {
        *self.ocr_shortcut.read().unwrap()
    }

    pub fn set_ocr_shortcut(&self, shortcut: Option<Shortcut>) {
        *self.ocr_shortcut.write().unwrap() = shortcut;
    }

    pub fn is_launcher_visible(&self) -> bool {
        self.launcher_visible.load(Ordering::SeqCst)
    }

    pub fn set_launcher_visible(&self, visible: bool) {
        self.launcher_visible.store(visible, Ordering::SeqCst);
    }

    pub fn has_launcher_been_shown(&self) -> bool {
        self.launcher_shown_once.load(Ordering::SeqCst)
    }

    pub fn mark_launcher_shown(&self) {
        self.launcher_shown_once.store(true, Ordering::SeqCst);
    }

    pub fn begin_shortcut_press(&self, action: ShortcutAction) -> bool {
        match action {
            ShortcutAction::ToggleLauncher => {
                !self.launcher_shortcut_pressed.swap(true, Ordering::SeqCst)
            }
            ShortcutAction::OcrCapture => !self.ocr_shortcut_pressed.swap(true, Ordering::SeqCst),
        }
    }

    pub fn end_shortcut_press(&self, action: ShortcutAction) {
        match action {
            ShortcutAction::ToggleLauncher => {
                self.launcher_shortcut_pressed
                    .store(false, Ordering::SeqCst);
            }
            ShortcutAction::OcrCapture => {
                self.ocr_shortcut_pressed.store(false, Ordering::SeqCst);
            }
        }
    }

    pub fn begin_ocr_capture(&self) -> bool {
        !self.ocr_capture_active.swap(true, Ordering::SeqCst)
    }

    pub fn end_ocr_capture(&self) {
        self.ocr_capture_active.store(false, Ordering::SeqCst);
    }

    pub fn begin_transient_window_interaction(&self) {
        self.transient_window_interactions
            .fetch_add(1, Ordering::SeqCst);
    }

    pub fn end_transient_window_interaction(&self) {
        let _ = self.transient_window_interactions.fetch_update(
            Ordering::SeqCst,
            Ordering::SeqCst,
            |count| Some(count.saturating_sub(1)),
        );
    }

    pub fn is_transient_window_interaction_active(&self) -> bool {
        self.transient_window_interactions.load(Ordering::SeqCst) > 0
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_llm_provider_models, validate_llm_provider_config, validate_rag_settings,
        ShortcutAction, ShortcutRuntimeState,
    };
    use crate::domain::settings::{
        LlmProviderConfig, LlmProviderProtocolKind, LlmSettings, RagSettings,
    };
    use serde_json::json;

    #[test]
    fn shortcut_press_only_triggers_once_until_release() {
        let state = ShortcutRuntimeState::default();

        assert!(state.begin_shortcut_press(ShortcutAction::ToggleLauncher));
        assert!(!state.begin_shortcut_press(ShortcutAction::ToggleLauncher));

        state.end_shortcut_press(ShortcutAction::ToggleLauncher);

        assert!(state.begin_shortcut_press(ShortcutAction::ToggleLauncher));
    }

    #[test]
    fn launcher_shown_flag_persists_after_marking() {
        let state = ShortcutRuntimeState::default();

        assert!(!state.has_launcher_been_shown());

        state.mark_launcher_shown();

        assert!(state.has_launcher_been_shown());
    }

    #[test]
    fn ocr_capture_only_allows_one_active_flow() {
        let state = ShortcutRuntimeState::default();

        assert!(state.begin_ocr_capture());
        assert!(!state.begin_ocr_capture());

        state.end_ocr_capture();

        assert!(state.begin_ocr_capture());
    }

    #[test]
    fn transient_window_interaction_handles_nesting_and_underflow() {
        let state = ShortcutRuntimeState::default();

        assert!(!state.is_transient_window_interaction_active());

        state.begin_transient_window_interaction();
        state.begin_transient_window_interaction();
        assert!(state.is_transient_window_interaction_active());

        state.end_transient_window_interaction();
        assert!(state.is_transient_window_interaction_active());

        state.end_transient_window_interaction();
        assert!(!state.is_transient_window_interaction_active());

        state.end_transient_window_interaction();
        assert!(!state.is_transient_window_interaction_active());
    }

    #[test]
    fn extract_llm_provider_models_reads_ids_from_data_array() {
        let payload = json!({
            "data": [
                { "id": "gpt-4.1-mini" },
                { "id": "text-embedding-3-small" },
                { "id": "gpt-4.1-mini" }
            ]
        });

        let models = extract_llm_provider_models(&payload);

        assert_eq!(
            models,
            vec![
                "gpt-4.1-mini".to_string(),
                "text-embedding-3-small".to_string()
            ]
        );
    }

    #[test]
    fn embedding_protocol_rejects_multimodal_flag() {
        let provider = LlmProviderConfig {
            id: "embedding".to_string(),
            name: "Embedding".to_string(),
            protocol: LlmProviderProtocolKind::Embedding,
            base_url: "https://api.example.com/v1".to_string(),
            api_key: String::new(),
            model: "text-embedding-3-small".to_string(),
            supports_multimodal: true,
        };

        let error =
            validate_llm_provider_config(&provider).expect_err("embedding must reject multimodal");
        assert!(error.to_string().contains("不允许启用多模态"));
    }

    #[test]
    fn rag_settings_require_embedding_provider_when_sources_exist() {
        let rag_settings = RagSettings {
            source_directories: vec!["/tmp".to_string()],
            ignore_globs: Vec::new(),
            embedding_provider_id: None,
        };
        let llm_settings = LlmSettings::default();

        let error = validate_rag_settings(&rag_settings, &llm_settings)
            .expect_err("RAG sources must require an embedding provider");

        assert!(error
            .to_string()
            .contains("必须选择一个 embedding provider"));
    }

    #[test]
    fn rag_settings_reject_non_embedding_provider() {
        let rag_settings = RagSettings {
            source_directories: Vec::new(),
            ignore_globs: Vec::new(),
            embedding_provider_id: Some("chat".to_string()),
        };
        let llm_settings = LlmSettings {
            providers: vec![LlmProviderConfig {
                id: "chat".to_string(),
                name: "Chat".to_string(),
                protocol: LlmProviderProtocolKind::Chat,
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                model: "gpt-4.1-mini".to_string(),
                supports_multimodal: false,
            }],
            default_provider_id: None,
        };

        let error = validate_rag_settings(&rag_settings, &llm_settings)
            .expect_err("RAG must reject non-embedding providers");

        assert!(error.to_string().contains("OpenAI Embedding"));
    }
}
