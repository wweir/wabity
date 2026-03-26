use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, RwLock as StdRwLock,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use reqwest::blocking::Client as BlockingHttpClient;
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
    question_answer_backend,
    rag::RagIndexService,
    rag_mcp::RagMcpServerService,
    translate,
};
use crate::{
    domain::{
        acp::{
            AcpAgentCatalog, AcpAgentConfig, AcpMcpServerCatalog, AcpMcpServerConfig,
            AcpNameValuePair, AcpRestoreNotice, AcpSessionDetail, AcpSessionSummary,
        },
        execution::{ExecutionProgressEvent, ExecutionRequest, ExecutionResult},
        rag::{BuiltinRagMcpServerStatus, RagRuntimeStatus, RagScanResult},
        settings::{
            builtin_llm_provider_templates, find_builtin_llm_provider_template, AppSettings,
            BuiltinLlmProviderTemplate, BuiltinLlmTemplateModelProtocol,
            BuiltinLlmTemplateModelType, LlmProviderConfig, LlmProviderModelEntry, LlmSettings,
            OcrProviderKind, OcrSettings, RagSettings,
        },
        skills::PublicSkillCatalog,
        workspace::WorkspaceState,
    },
    infrastructure::config::{
        default_workspace_root, display_home_as_tilde, home_workspace_root,
        normalize_workspace_root, AppConfig, ConfigStore, SavedAcpSession, ShortcutConfig,
        ShortcutKey, WorkspaceHistory, RECENT_WORKSPACE_LIMIT,
    },
    infrastructure::openai_compatible::{extract_model_entries, OpenAiCompatibleClient},
    services::public_skills::PublicSkillService,
};

const SHORTCUT_PRESS_STALE_AFTER: Duration = Duration::from_millis(750);

#[derive(Debug, Clone, Copy, Default)]
struct ShortcutPressGate {
    pressed: bool,
    pressed_at: Option<Instant>,
}

#[derive(Clone)]
pub struct AppState {
    matcher: MatcherService,
    executor: ExecutorService,
    application: ApplicationService,
    file_search: FileSearchService,
    acp: AcpService,
    ocr_provider: Arc<StdRwLock<Arc<dyn OcrProvider>>>,
    rag_index: RagIndexService,
    rag_mcp: RagMcpServerService,
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
        let rag_index = RagIndexService::new(ConfigStore::data_dir()?);
        rag_index
            .apply_settings(config.rag.clone(), config.llm.clone())
            .await;
        let rag_mcp = RagMcpServerService::new(ConfigStore::data_dir()?, config_store.clone());
        rag_mcp.start().await;

        Ok(Self {
            matcher,
            executor,
            application: ApplicationService::new()?,
            file_search: FileSearchService::new()?,
            acp: AcpService::new(),
            ocr_provider: Arc::new(StdRwLock::new(build_ocr_provider(&config.ocr, &config.llm))),
            rag_index,
            rag_mcp,
            config_store,
            workspace_state: Arc::new(AsyncRwLock::new(initial_workspace)),
        })
    }

    pub fn matcher(&self) -> &MatcherService {
        &self.matcher
    }

    pub async fn execute_action_with_progress(
        &self,
        request: ExecutionRequest,
        progress_event_tx: Option<Arc<dyn Fn(ExecutionProgressEvent) + Send + Sync>>,
    ) -> Result<ExecutionResult> {
        if request.action_id == "translate_text" {
            let settings = self.app_settings().await?;
            return tokio::task::spawn_blocking(move || {
                translate::execute_translation(
                    &request.query.raw_text,
                    &settings.prompts,
                    &settings.llm,
                )
            })
            .await
            .context("failed to join translation task")?;
        }

        if request.action_id == "rag_answer" {
            let settings = self.app_settings().await?;
            let data_dir = ConfigStore::data_dir()?;
            let workspace = self.workspace().await?;
            let workspace_root = normalize_workspace_root(&workspace.root_path)?;
            let mcp_servers = self.acp_mcp_servers().await?.servers;
            return question_answer_backend::answer_question(
                question_answer_backend::QuestionAnswerBackendRequest {
                    data_dir: &data_dir,
                    workspace_root: &workspace_root,
                    raw_text: &request.query.raw_text,
                    conversation: &request.conversation,
                    conversation_state: request.conversation_state.as_ref(),
                    prompts_settings: &settings.prompts,
                    rag_settings: &settings.rag,
                    llm_settings: &settings.llm,
                    mcp_servers: &mcp_servers,
                    progress_event_tx,
                },
            )
            .await;
        }

        self.executor.execute(&request)
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
            prompts: config.prompts,
            llm: config.llm,
            ocr: config.ocr,
            rag: config.rag,
        })
    }

    pub async fn list_llm_provider_models(
        &self,
        provider: LlmProviderConfig,
    ) -> Result<Vec<LlmProviderModelEntry>> {
        tokio::task::spawn_blocking(move || fetch_llm_provider_models(&provider))
            .await
            .context("failed to join LLM model list task")?
    }

    pub async fn builtin_llm_provider_templates(&self) -> Vec<BuiltinLlmProviderTemplate> {
        builtin_llm_provider_templates()
    }

    pub async fn public_skill_catalog(&self) -> Result<PublicSkillCatalog> {
        PublicSkillService::load_catalog()
    }

    pub async fn update_shortcut(&self, key: ShortcutKey, shortcut: &str) -> Result<()> {
        let store = self.config_store.write().await;
        let mut config = store.load().await?;
        config.shortcuts.set(key, shortcut);
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
        config.prompts = settings.prompts.clone();
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
            prompts: config.prompts,
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
        self.rag_index
            .scan_sources(&rag_settings, &llm_settings)
            .await
    }

    pub async fn rag_runtime_status(&self) -> RagRuntimeStatus {
        self.rag_index.runtime_status().await
    }

    pub async fn builtin_rag_mcp_server_status(&self) -> BuiltinRagMcpServerStatus {
        self.rag_mcp.status().await
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
                if !provider.has_responses_model() {
                    anyhow::bail!("OCR 选择的 LLM 配置缺少 responses 模型")
                }
                if !provider.supports_multimodal() {
                    anyhow::bail!("OCR 选择的 LLM 配置未启用多模态能力")
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

    validate_llm_provider_reference(
        settings,
        settings.translation_provider_id.as_deref(),
        "翻译 LLM provider",
    )?;
    validate_llm_provider_reference(
        settings,
        settings.question_answer_provider_id.as_deref(),
        "问答 LLM provider",
    )?;

    Ok(())
}

fn validate_llm_provider_reference(
    settings: &LlmSettings,
    provider_id: Option<&str>,
    label: &str,
) -> Result<()> {
    let Some(provider_id) = provider_id else {
        return Ok(());
    };

    if settings
        .providers
        .iter()
        .any(|provider| provider.id == provider_id && provider.has_llm_model())
    {
        return Ok(());
    }

    anyhow::bail!("{label} 不存在: {provider_id}");
}

fn validate_ocr_settings(settings: &OcrSettings, llm_settings: &LlmSettings) -> Result<()> {
    match settings.provider {
        OcrProviderKind::Disabled => Ok(()),
        OcrProviderKind::System => validate_system_ocr_settings(),
        OcrProviderKind::LlmOcr => {
            let provider = resolve_llm_provider(settings, llm_settings)?;
            if !provider.has_responses_model() {
                anyhow::bail!("OCR 选择的 LLM 配置缺少 responses 模型");
            }
            if !provider.supports_multimodal() {
                anyhow::bail!("OCR 选择的 LLM 配置未启用多模态能力");
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
        if !provider.has_embedding_model() {
            anyhow::bail!("RAG 只接受启用了 embedding 能力的 provider");
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
    if provider.model_name().is_empty() {
        anyhow::bail!("LLM provider model 不能为空");
    }
    if provider.supports_multimodal && !provider.has_responses_model() {
        anyhow::bail!("多模态开关当前只能和 responses 协议一起使用");
    }
    if provider.supports_stateful && !provider.has_responses_model() {
        anyhow::bail!("stateful 开关只能和 responses 协议一起使用");
    }
    validate_builtin_llm_provider_binding(provider)?;

    Ok(())
}

fn validate_builtin_llm_provider_binding(provider: &LlmProviderConfig) -> Result<()> {
    let Some(template_id) = provider.builtin_preset_id.as_deref() else {
        return Ok(());
    };
    let template = find_builtin_llm_provider_template(template_id)
        .with_context(|| format!("未知内置 LLM 模板: {template_id}"))?;

    if provider.managed_base_url && provider.base_url.trim() != template.default_base_url {
        anyhow::bail!("内置模板条目的 Base URL 必须与模板默认值一致，或先关闭模板管理");
    }

    let model_id = provider
        .builtin_preset_model_id
        .as_deref()
        .context("内置模板条目缺少模型目录绑定")?;
    let template_model = template
        .models
        .iter()
        .find(|candidate| candidate.id == model_id)
        .with_context(|| format!("内置模板模型不存在: {model_id}"))?;

    if !template_model.selectable_in_current_app {
        anyhow::bail!("内置模板模型当前不可在 Wabity 中使用: {model_id}");
    }
    if provider.model.trim() != template_model.model {
        anyhow::bail!("内置模板条目的模型必须来自模板白名单");
    }

    match template_model.model_type {
        BuiltinLlmTemplateModelType::Llm => {
            if !provider.is_llm_model() {
                anyhow::bail!("内置模板模型类型与 provider 类型不一致");
            }
        }
        BuiltinLlmTemplateModelType::Embedding => {
            if !provider.is_embedding_model() {
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
    if provider.supports_multimodal != expected_multimodal {
        anyhow::bail!("内置模板条目的多模态能力与模板目录不一致");
    }

    let expected_stateful = template_model.supports_stateful
        && expected_protocol == crate::domain::settings::LlmProviderProtocol::Responses;
    if provider.supports_stateful != expected_stateful {
        anyhow::bail!("内置模板条目的 stateful 能力与模板目录不一致");
    }

    Ok(())
}

fn fetch_llm_provider_models(provider: &LlmProviderConfig) -> Result<Vec<LlmProviderModelEntry>> {
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

fn normalize_mcp_remote_url(url: &str, label: &str) -> Result<String> {
    let normalized = url.trim();
    if normalized.is_empty() {
        anyhow::bail!("{label} 为空");
    }

    let parsed =
        reqwest::Url::parse(normalized).with_context(|| format!("{label} 不是合法 URL"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(normalized.to_string()),
        _ => anyhow::bail!("{label} 只支持 http:// 或 https://"),
    }
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
                let url = normalize_mcp_remote_url(&config.url, &format!("{label} 的 http url"))?;

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
                let url = normalize_mcp_remote_url(&config.url, &format!("{label} 的 sse url"))?;

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

#[derive(Clone)]
pub struct ShortcutRuntimeState {
    launcher_shortcut: Arc<StdRwLock<Option<Shortcut>>>,
    ocr_shortcut: Arc<StdRwLock<Option<Shortcut>>>,
    ocr_translate_shortcut: Arc<StdRwLock<Option<Shortcut>>>,
    launcher_visible: Arc<AtomicBool>,
    launcher_blur_auto_hide_enabled: Arc<AtomicBool>,
    launcher_resize_reposition_until: Arc<StdRwLock<Option<Instant>>>,
    launcher_blur_auto_hide_suppressed_until: Arc<StdRwLock<Option<Instant>>>,
    launcher_blur_auto_hide_sequence: Arc<AtomicUsize>,
    launcher_shortcut_pressed: Arc<StdRwLock<ShortcutPressGate>>,
    ocr_shortcut_pressed: Arc<StdRwLock<ShortcutPressGate>>,
    ocr_translate_shortcut_pressed: Arc<StdRwLock<ShortcutPressGate>>,
    ocr_capture_active: Arc<AtomicBool>,
    transient_window_interactions: Arc<AtomicUsize>,
}

impl Default for ShortcutRuntimeState {
    fn default() -> Self {
        Self {
            launcher_shortcut: Arc::new(StdRwLock::new(None)),
            ocr_shortcut: Arc::new(StdRwLock::new(None)),
            ocr_translate_shortcut: Arc::new(StdRwLock::new(None)),
            launcher_visible: Arc::new(AtomicBool::new(false)),
            launcher_blur_auto_hide_enabled: Arc::new(AtomicBool::new(true)),
            launcher_resize_reposition_until: Arc::new(StdRwLock::new(None)),
            launcher_blur_auto_hide_suppressed_until: Arc::new(StdRwLock::new(None)),
            launcher_blur_auto_hide_sequence: Arc::new(AtomicUsize::new(0)),
            launcher_shortcut_pressed: Arc::new(StdRwLock::new(ShortcutPressGate::default())),
            ocr_shortcut_pressed: Arc::new(StdRwLock::new(ShortcutPressGate::default())),
            ocr_translate_shortcut_pressed: Arc::new(StdRwLock::new(ShortcutPressGate::default())),
            ocr_capture_active: Arc::new(AtomicBool::new(false)),
            transient_window_interactions: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    ToggleLauncher,
    OcrCapture,
    OcrTranslate,
}

impl ShortcutRuntimeState {
    pub fn shortcut_action(&self, shortcut: Shortcut) -> Option<ShortcutAction> {
        for (key, action) in [
            (ShortcutKey::ToggleLauncher, ShortcutAction::ToggleLauncher),
            (ShortcutKey::OcrCapture, ShortcutAction::OcrCapture),
            (ShortcutKey::OcrTranslate, ShortcutAction::OcrTranslate),
        ] {
            if self.current_shortcut(key) == Some(shortcut) {
                return Some(action);
            }
        }

        None
    }

    pub fn current_shortcut(&self, key: ShortcutKey) -> Option<Shortcut> {
        match key {
            ShortcutKey::ToggleLauncher => *self.launcher_shortcut.read().unwrap(),
            ShortcutKey::OcrCapture => *self.ocr_shortcut.read().unwrap(),
            ShortcutKey::OcrTranslate => *self.ocr_translate_shortcut.read().unwrap(),
        }
    }

    pub fn set_shortcut(&self, key: ShortcutKey, shortcut: Option<Shortcut>) {
        match key {
            ShortcutKey::ToggleLauncher => *self.launcher_shortcut.write().unwrap() = shortcut,
            ShortcutKey::OcrCapture => *self.ocr_shortcut.write().unwrap() = shortcut,
            ShortcutKey::OcrTranslate => {
                *self.ocr_translate_shortcut.write().unwrap() = shortcut;
            }
        }
    }

    pub fn is_launcher_visible(&self) -> bool {
        self.launcher_visible.load(Ordering::SeqCst)
    }

    pub fn set_launcher_visible(&self, visible: bool) {
        self.launcher_visible.store(visible, Ordering::SeqCst);
    }

    pub fn is_launcher_blur_auto_hide_enabled(&self) -> bool {
        self.launcher_blur_auto_hide_enabled.load(Ordering::SeqCst)
    }

    pub fn set_launcher_blur_auto_hide_enabled(&self, enabled: bool) {
        self.launcher_blur_auto_hide_enabled
            .store(enabled, Ordering::SeqCst);
    }

    pub fn arm_launcher_resize_reposition(&self, duration: Duration) {
        *self.launcher_resize_reposition_until.write().unwrap() = Some(Instant::now() + duration);
    }

    pub fn clear_launcher_resize_reposition(&self) {
        *self.launcher_resize_reposition_until.write().unwrap() = None;
    }

    pub fn arm_launcher_blur_auto_hide_suppression(&self, duration: Duration) -> Instant {
        let deadline = Instant::now() + duration;
        *self
            .launcher_blur_auto_hide_suppressed_until
            .write()
            .unwrap() = Some(deadline);
        deadline
    }

    pub fn clear_launcher_blur_auto_hide_suppression(&self) {
        *self
            .launcher_blur_auto_hide_suppressed_until
            .write()
            .unwrap() = None;
    }

    pub fn launcher_blur_auto_hide_delay(&self) -> Option<Duration> {
        self.launcher_blur_auto_hide_suppressed_until
            .read()
            .unwrap()
            .and_then(|deadline| deadline.checked_duration_since(Instant::now()))
    }

    pub fn arm_launcher_blur_auto_hide_confirmation(&self) -> usize {
        self.launcher_blur_auto_hide_sequence
            .fetch_add(1, Ordering::SeqCst)
            + 1
    }

    pub fn cancel_launcher_blur_auto_hide_confirmation(&self) -> usize {
        self.launcher_blur_auto_hide_sequence
            .fetch_add(1, Ordering::SeqCst)
            + 1
    }

    pub fn should_execute_launcher_blur_auto_hide(&self, sequence: usize) -> bool {
        self.launcher_blur_auto_hide_sequence.load(Ordering::SeqCst) == sequence
    }

    pub fn should_reposition_launcher_on_resize(&self) -> bool {
        if !self.is_launcher_visible() {
            return true;
        }

        self.launcher_resize_reposition_until
            .read()
            .unwrap()
            .is_some_and(|deadline| Instant::now() <= deadline)
    }

    pub fn begin_shortcut_press(&self, action: ShortcutAction) -> bool {
        Self::begin_shortcut_press_gate(match action {
            ShortcutAction::ToggleLauncher => &self.launcher_shortcut_pressed,
            ShortcutAction::OcrCapture => &self.ocr_shortcut_pressed,
            ShortcutAction::OcrTranslate => &self.ocr_translate_shortcut_pressed,
        })
    }

    pub fn end_shortcut_press(&self, action: ShortcutAction) {
        Self::end_shortcut_press_gate(match action {
            ShortcutAction::ToggleLauncher => &self.launcher_shortcut_pressed,
            ShortcutAction::OcrCapture => &self.ocr_shortcut_pressed,
            ShortcutAction::OcrTranslate => &self.ocr_translate_shortcut_pressed,
        });
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

    fn begin_shortcut_press_gate(gate: &StdRwLock<ShortcutPressGate>) -> bool {
        let mut gate = gate.write().unwrap();
        let now = Instant::now();

        if gate.pressed
            && gate.pressed_at.is_some_and(|pressed_at| {
                now.duration_since(pressed_at) < SHORTCUT_PRESS_STALE_AFTER
            })
        {
            return false;
        }

        gate.pressed = true;
        gate.pressed_at = Some(now);
        true
    }

    fn end_shortcut_press_gate(gate: &StdRwLock<ShortcutPressGate>) {
        let mut gate = gate.write().unwrap();
        gate.pressed = false;
        gate.pressed_at = None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        normalize_mcp_remote_url, validate_llm_provider_config, validate_rag_settings,
        ShortcutAction, ShortcutRuntimeState, SHORTCUT_PRESS_STALE_AFTER,
    };
    use crate::domain::settings::{
        LlmProviderConfig, LlmProviderModelEntry, LlmSettings, RagSettings,
    };
    use crate::infrastructure::openai_compatible::{extract_model_entries, extract_model_ids};
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
    fn ocr_translate_shortcut_press_only_triggers_once_until_release() {
        let state = ShortcutRuntimeState::default();

        assert!(state.begin_shortcut_press(ShortcutAction::OcrTranslate));
        assert!(!state.begin_shortcut_press(ShortcutAction::OcrTranslate));

        state.end_shortcut_press(ShortcutAction::OcrTranslate);

        assert!(state.begin_shortcut_press(ShortcutAction::OcrTranslate));
    }

    #[test]
    fn shortcut_press_recovers_after_missing_release() {
        let state = ShortcutRuntimeState::default();

        assert!(state.begin_shortcut_press(ShortcutAction::ToggleLauncher));
        assert!(!state.begin_shortcut_press(ShortcutAction::ToggleLauncher));

        std::thread::sleep(SHORTCUT_PRESS_STALE_AFTER + Duration::from_millis(50));

        assert!(state.begin_shortcut_press(ShortcutAction::ToggleLauncher));
    }

    #[test]
    fn hidden_launcher_always_repositions_on_resize() {
        let state = ShortcutRuntimeState::default();

        assert!(state.should_reposition_launcher_on_resize());
    }

    #[test]
    fn visible_launcher_only_repositions_within_grace_period() {
        let state = ShortcutRuntimeState::default();

        state.set_launcher_visible(true);
        assert!(!state.should_reposition_launcher_on_resize());

        state.arm_launcher_resize_reposition(Duration::from_millis(20));
        assert!(state.should_reposition_launcher_on_resize());

        std::thread::sleep(Duration::from_millis(30));
        assert!(!state.should_reposition_launcher_on_resize());
    }

    #[test]
    fn launcher_blur_auto_hide_suppression_expires() {
        let state = ShortcutRuntimeState::default();

        state.arm_launcher_blur_auto_hide_suppression(Duration::from_millis(20));
        assert!(state.launcher_blur_auto_hide_delay().is_some());

        std::thread::sleep(Duration::from_millis(30));
        assert!(state.launcher_blur_auto_hide_delay().is_none());

        state.clear_launcher_blur_auto_hide_suppression();
        assert!(state.launcher_blur_auto_hide_delay().is_none());
    }

    #[test]
    fn launcher_blur_auto_hide_enable_flag_round_trips() {
        let state = ShortcutRuntimeState::default();

        assert!(state.is_launcher_blur_auto_hide_enabled());
        state.set_launcher_blur_auto_hide_enabled(false);
        assert!(!state.is_launcher_blur_auto_hide_enabled());
        state.set_launcher_blur_auto_hide_enabled(true);
        assert!(state.is_launcher_blur_auto_hide_enabled());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn newer_blur_auto_hide_confirmation_cancels_older_one() {
        let state = ShortcutRuntimeState::default();

        let first = state.arm_launcher_blur_auto_hide_confirmation();
        let second = state.arm_launcher_blur_auto_hide_confirmation();

        assert!(!state.should_execute_launcher_blur_auto_hide(first));
        assert!(state.should_execute_launcher_blur_auto_hide(second));

        state.cancel_launcher_blur_auto_hide_confirmation();
        assert!(!state.should_execute_launcher_blur_auto_hide(second));
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

        let models = extract_model_ids(&payload);

        assert_eq!(
            models,
            vec![
                "gpt-4.1-mini".to_string(),
                "text-embedding-3-small".to_string()
            ]
        );
    }

    #[test]
    fn extract_llm_provider_models_reads_identity_hints_from_digest_fields() {
        let payload = json!({
            "data": [
                {
                    "id": "text-embedding-3-small",
                    "digest": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                }
            ]
        });

        assert_eq!(
            extract_model_entries(&payload),
            vec![LlmProviderModelEntry {
                id: "text-embedding-3-small".to_string(),
                identity_hint: Some(
                    "digest:sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                        .to_string()
                ),
            }]
        );
    }

    #[test]
    fn missing_responses_model_rejects_multimodal_flag() {
        let provider = LlmProviderConfig {
            id: "embedding".to_string(),
            name: "Embedding".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: String::new(),
            model_type: crate::domain::settings::LlmModelType::Embedding,
            model: "text-embedding-3-small".to_string(),
            supports_multimodal: true,
            ..LlmProviderConfig::default()
        };

        let error =
            validate_llm_provider_config(&provider).expect_err("multimodal requires responses");
        assert!(error.to_string().contains("responses 协议"));
    }

    #[test]
    fn missing_responses_model_rejects_stateful_flag() {
        let provider = LlmProviderConfig {
            id: "embedding".to_string(),
            name: "Embedding".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: String::new(),
            model_type: crate::domain::settings::LlmModelType::Embedding,
            model: "text-embedding-3-small".to_string(),
            supports_stateful: true,
            ..LlmProviderConfig::default()
        };

        let error =
            validate_llm_provider_config(&provider).expect_err("stateful requires responses");
        assert!(error.to_string().contains("responses 协议"));
    }

    #[test]
    fn builtin_provider_rejects_models_outside_whitelist() {
        let provider = LlmProviderConfig {
            id: "zhipu".to_string(),
            name: "智谱 AI".to_string(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
            api_key: "key".to_string(),
            model: "glm-4.9".to_string(),
            protocol: crate::domain::settings::LlmProviderProtocol::ChatCompletions,
            builtin_preset_id: Some("zhipu".to_string()),
            builtin_preset_model_id: Some("glm-4.7-flash".to_string()),
            managed_base_url: true,
            ..LlmProviderConfig::default()
        };

        let error = validate_llm_provider_config(&provider)
            .expect_err("builtin provider must stay inside whitelist");
        assert!(error.to_string().contains("白名单"));
    }

    #[test]
    fn builtin_provider_rejects_unselectable_catalog_model() {
        let provider = LlmProviderConfig {
            id: "zhipu".to_string(),
            name: "智谱 AI".to_string(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
            api_key: "key".to_string(),
            model: "cogview-3-flash".to_string(),
            protocol: crate::domain::settings::LlmProviderProtocol::ChatCompletions,
            builtin_preset_id: Some("zhipu".to_string()),
            builtin_preset_model_id: Some("cogview-3-flash".to_string()),
            managed_base_url: true,
            ..LlmProviderConfig::default()
        };

        let error = validate_llm_provider_config(&provider)
            .expect_err("unsupported builtin model must not be selectable");
        assert!(error.to_string().contains("当前不可在 Wabity 中使用"));
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
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                model_type: crate::domain::settings::LlmModelType::Llm,
                model: "gpt-4.1-mini".to_string(),
                supports_multimodal: false,
                ..LlmProviderConfig::default()
            }],
            ..LlmSettings::default()
        };

        let error = validate_rag_settings(&rag_settings, &llm_settings)
            .expect_err("RAG must reject non-embedding providers");

        assert!(error.to_string().contains("embedding 能力"));
    }

    #[test]
    fn rag_settings_accept_chat_provider_with_embedding_capability() {
        let rag_settings = RagSettings {
            source_directories: Vec::new(),
            ignore_globs: Vec::new(),
            embedding_provider_id: Some("chat".to_string()),
        };
        let llm_settings = LlmSettings {
            providers: vec![LlmProviderConfig {
                id: "chat".to_string(),
                name: "Chat".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                model_type: crate::domain::settings::LlmModelType::Embedding,
                model: "text-embedding-3-small".to_string(),
                supports_multimodal: false,
                ..LlmProviderConfig::default()
            }],
            ..LlmSettings::default()
        };

        validate_rag_settings(&rag_settings, &llm_settings)
            .expect("RAG should accept providers with embedding capability");
    }

    #[test]
    fn normalize_mcp_remote_url_trims_valid_http_url() {
        let normalized = normalize_mcp_remote_url("  https://example.com/mcp  ", "MCP URL")
            .expect("valid MCP URL should pass");

        assert_eq!(normalized, "https://example.com/mcp");
    }

    #[test]
    fn normalize_mcp_remote_url_rejects_invalid_scheme() {
        let error = normalize_mcp_remote_url("ws://example.com/mcp", "MCP URL")
            .expect_err("unsupported scheme should fail");

        assert!(error.to_string().contains("只支持 http:// 或 https://"));
    }

    #[test]
    fn normalize_mcp_remote_url_rejects_incomplete_url() {
        let error =
            normalize_mcp_remote_url("foo", "MCP URL").expect_err("incomplete URL should fail");

        assert!(error.to_string().contains("不是合法 URL"));
    }
}
