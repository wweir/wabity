use std::{
    collections::HashSet,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, RwLock as StdRwLock,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use tauri_plugin_global_shortcut::Shortcut;
use tokio::sync::RwLock as AsyncRwLock;

mod acp_catalog;
mod session_snapshot;
mod settings;

use self::acp_catalog::{
    effective_mcp_servers, normalize_acp_agent_catalog, normalize_acp_mcp_server_catalog,
    reconcile_saved_session_builtin_mcp,
};
use self::session_snapshot::merge_restored_session_snapshots;
use self::settings::{
    build_ocr_provider, fetch_llm_provider_models, validate_llm_settings, validate_ocr_settings,
    validate_rag_settings,
};
use crate::services::{
    acp::{AcpService, AgentSessionRuntimeConfig},
    application::ApplicationService,
    builtin_mcp::BuiltinMcpServerService,
    clipboard::ClipboardService,
    executor::ExecutorService,
    file_search::FileSearchService,
    matcher::MatcherService,
    notification::NotificationService,
    ocr::OcrProvider,
    open_target::OpenTargetService,
    process::ProcessService,
    question_answer_backend,
    rag::RagIndexService,
    translate,
};
use crate::{
    domain::{
        acp::{
            AcpAgentCatalog, AcpAgentConfig, AcpMcpServerCatalog, AcpMcpServerConfig,
            AcpRestoreNotice, AcpSessionDetail, AcpSessionSummary, BuiltinMcpServerStatus,
        },
        execution::{ExecutionProgressEvent, ExecutionRequest, ExecutionResult},
        rag::{RagRuntimeStatus, RagScanResult},
        settings::{
            builtin_llm_provider_templates, AppSettings, BuiltinLlmProviderTemplate,
            LlmProviderConfig, LlmProviderModelEntry, LlmSettings, RagSettings,
            ResolvedLlmModelBinding,
        },
        workspace::WorkspaceState,
    },
    infrastructure::config::{
        default_workspace_root, display_home_as_tilde, home_workspace_root,
        normalize_workspace_root, AppConfig, ConfigStore, ShortcutConfig, ShortcutKey,
        WorkspaceHistory, RECENT_WORKSPACE_LIMIT,
    },
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
    open_target: OpenTargetService,
    application: ApplicationService,
    process: ProcessService,
    file_search: FileSearchService,
    clipboard: ClipboardService,
    notification: NotificationService,
    acp: AcpService,
    ocr_provider: Arc<StdRwLock<Arc<dyn OcrProvider>>>,
    rag_index: RagIndexService,
    builtin_mcp: BuiltinMcpServerService,
    config_store: Arc<AsyncRwLock<ConfigStore>>,
    workspace_state: Arc<AsyncRwLock<WorkspaceState>>,
}

impl AppState {
    pub async fn new(
        app_handle: tauri::AppHandle,
        shortcut_state: ShortcutRuntimeState,
        matcher: MatcherService,
        executor: ExecutorService,
    ) -> Result<Self> {
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
        let rag_index = RagIndexService::new_with_app_handle(
            Some(app_handle.clone()),
            ConfigStore::data_dir()?,
        );
        rag_index
            .apply_settings(config.rag.clone(), config.llm.clone())
            .await;
        let clipboard = ClipboardService::new(app_handle.clone()).await?;
        let builtin_mcp = BuiltinMcpServerService::new(
            ConfigStore::data_dir()?,
            normalize_workspace_root(&root_path)?,
            config.rag.clone(),
            config.llm.clone(),
            config.acp.builtin_mcp.clone(),
        );
        builtin_mcp.start().await;
        clipboard.start();
        let notification =
            NotificationService::new(app_handle, shortcut_state, config_store.clone());

        Ok(Self {
            matcher,
            executor,
            open_target: OpenTargetService::new(),
            application: ApplicationService::new()?,
            process: ProcessService::new(),
            file_search: FileSearchService::new()?,
            clipboard,
            notification: notification.clone(),
            acp: AcpService::new(notification),
            ocr_provider: Arc::new(StdRwLock::new(build_ocr_provider(&config.ocr, &config.llm))),
            rag_index,
            builtin_mcp,
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
        match request.action_id.as_str() {
            "open_target" | "open_url" => {
                let workspace_root = self
                    .workspace()
                    .await
                    .ok()
                    .and_then(|workspace| normalize_workspace_root(&workspace.root_path).ok());
                self.open_target
                    .open_action(&request.query.raw_text, workspace_root.as_deref())
            }
            "kill_process" => self.process.kill_action(&request.query.raw_text),
            "translate_text" => {
                let settings = self.app_settings().await?;
                {
                    let mut partial_text = String::new();
                    let mut on_text_delta = |delta: &str| {
                        partial_text.push_str(delta);
                        if let Some(progress_event_tx) = progress_event_tx.as_ref() {
                            progress_event_tx(ExecutionProgressEvent {
                                action_id: "translate_text".to_string(),
                                status_text: "模型响应中 · 正在输出译文".to_string(),
                                partial_text: Some(partial_text.clone()),
                            });
                        }
                    };
                    translate::execute_translation_with_callbacks(
                        &request.query.raw_text,
                        &settings.prompts,
                        &settings.llm,
                        translate::TranslationCallbacks {
                            on_text_delta: Some(&mut on_text_delta),
                        },
                    )
                    .await
                }
            }
            "rag_answer" => {
                let settings = self.app_settings().await?;
                let data_dir = ConfigStore::data_dir()?;
                let workspace = self.workspace().await?;
                let workspace_root = normalize_workspace_root(&workspace.root_path)?;
                let mcp_servers = self.effective_acp_mcp_servers().await?;
                let result = question_answer_backend::answer_question(
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
                match &result {
                    Ok(output) => {
                        self.notification
                            .notify_question_answer_success(output.primary_text.clone())
                            .await;
                    }
                    Err(error) => {
                        self.notification
                            .notify_question_answer_failure(Some(error.to_string()))
                            .await;
                    }
                }
                result
            }
            _ => self.executor.execute(&request),
        }
    }

    pub fn file_search(&self) -> &FileSearchService {
        &self.file_search
    }

    pub fn application(&self) -> &ApplicationService {
        &self.application
    }

    pub fn process(&self) -> &ProcessService {
        &self.process
    }

    pub fn clipboard(&self) -> &ClipboardService {
        &self.clipboard
    }

    pub fn open_document_path(&self, path: &Path) -> Result<()> {
        self.open_target.open_path(path)
    }

    pub fn acp(&self) -> &AcpService {
        &self.acp
    }

    pub fn ocr_provider(&self) -> Arc<dyn OcrProvider> {
        read_runtime_lock(&self.ocr_provider, "ocr provider").clone()
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
            builtin: config.acp.builtin_mcp,
        })
    }

    pub async fn effective_acp_mcp_servers(&self) -> Result<Vec<AcpMcpServerConfig>> {
        let catalog = self.acp_mcp_servers().await?;
        let builtin_running = self.builtin_mcp.status().await.running;
        Ok(effective_mcp_servers(&catalog, builtin_running))
    }

    pub async fn app_settings(&self) -> Result<AppSettings> {
        let config = self.app_config().await?;
        Ok(AppSettings {
            general: config.general,
            notification: config.notification,
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
        let settings = self.app_settings().await?;
        let builtin_config = self.acp_mcp_servers().await?.builtin;
        self.builtin_mcp
            .apply_runtime_config(
                normalize_workspace_root(&next_workspace.root_path)?,
                settings.rag,
                settings.llm,
                builtin_config,
            )
            .await;

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
        config.acp.builtin_mcp = normalized_catalog.builtin.clone();
        store.save(&config).await?;
        let workspace = self.workspace().await?;
        self.builtin_mcp
            .apply_runtime_config(
                normalize_workspace_root(&workspace.root_path)?,
                config.rag.clone(),
                config.llm.clone(),
                config.acp.builtin_mcp.clone(),
            )
            .await;
        Ok(normalized_catalog)
    }

    pub async fn update_app_settings(&self, settings: AppSettings) -> Result<AppSettings> {
        let store = self.config_store.write().await;
        let mut config = store.load().await?;
        apply_app_settings_to_config(&mut config, settings)?;
        store.save(&config).await?;
        *write_runtime_lock(&self.ocr_provider, "ocr provider") =
            build_ocr_provider(&config.ocr, &config.llm);
        self.rag_index
            .apply_settings(config.rag.clone(), config.llm.clone())
            .await;
        let workspace = self.workspace().await?;
        self.builtin_mcp
            .apply_runtime_config(
                normalize_workspace_root(&workspace.root_path)?,
                config.rag.clone(),
                config.llm.clone(),
                config.acp.builtin_mcp.clone(),
            )
            .await;
        Ok(AppSettings {
            general: config.general,
            notification: config.notification,
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

    pub async fn builtin_mcp_server_status(&self) -> BuiltinMcpServerStatus {
        self.builtin_mcp.status().await
    }

    pub async fn create_acp_session(&self, _agent_id: Option<String>) -> Result<AcpSessionDetail> {
        let workspace = self.workspace().await?;
        let workspace_root = normalize_workspace_root(&workspace.root_path)?;
        let config = self.app_config().await?;
        let runtime_config = resolve_agent_session_runtime_config(&config);
        self.acp
            .create_session(workspace_root, pi_agent_config(), runtime_config)
            .await
    }

    pub async fn activate_acp_session(
        &self,
        session_id: Option<String>,
    ) -> Result<Vec<AcpSessionSummary>> {
        let requested_session_id = normalize_optional_id(session_id.as_deref());
        let summaries = self
            .acp
            .activate_session(requested_session_id.clone())
            .await;
        let persisted_active_session_id =
            normalized_existing_session_id(requested_session_id, &summaries);
        let store = self.config_store.write().await;
        let mut config = store.load().await?;
        config.acp.active_session_id = persisted_active_session_id;
        store.save(&config).await?;
        Ok(summaries)
    }

    pub async fn acp_session_detail(&self, session_id: &str) -> Result<Option<AcpSessionDetail>> {
        let session_id = normalize_required_id("Pi Agent session id", session_id)?;
        Ok(self.acp.session_detail(&session_id).await)
    }

    pub async fn send_acp_prompt(
        &self,
        session_id: &str,
        prompt: String,
    ) -> Result<AcpSessionDetail> {
        let session_id = normalize_required_id("Pi Agent session id", session_id)?;
        self.acp.send_prompt(&session_id, prompt).await
    }

    pub async fn set_acp_session_mode(
        &self,
        session_id: &str,
        mode_id: String,
    ) -> Result<AcpSessionDetail> {
        let session_id = normalize_required_id("Pi Agent session id", session_id)?;
        self.acp.set_session_mode(&session_id, mode_id).await
    }

    pub async fn set_acp_session_config_option(
        &self,
        session_id: &str,
        config_id: String,
        value_id: String,
    ) -> Result<AcpSessionDetail> {
        let session_id = normalize_required_id("Pi Agent session id", session_id)?;
        self.acp
            .set_session_config_option(&session_id, config_id, value_id)
            .await
    }

    pub async fn close_acp_session(&self, session_id: &str) -> Result<()> {
        let session_id = normalize_required_id("Pi Agent session id", session_id)?;
        self.acp.close_session(&session_id).await?;
        self.remove_session_snapshot(&session_id).await?;
        Ok(())
    }

    pub async fn take_acp_restore_notices(&self) -> Vec<AcpRestoreNotice> {
        self.acp.take_restore_notices().await
    }

    pub async fn restore_acp_sessions(&self) -> Result<Vec<AcpSessionDetail>> {
        let config = self.app_config().await?;
        let snapshots = config.acp.saved_sessions.clone();
        let active_session_id = config.acp.active_session_id.clone();
        let initial_snapshot_ids = snapshots
            .iter()
            .map(|snapshot| snapshot.session_id.clone())
            .collect::<HashSet<_>>();
        let builtin_catalog = self.acp_mcp_servers().await?;
        let builtin_running = self.builtin_mcp.status().await.running;
        let mut restored = Vec::new();
        let mut retained = Vec::new();

        for snapshot in snapshots {
            let snapshot =
                reconcile_saved_session_builtin_mcp(snapshot, &builtin_catalog, builtin_running);
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
        next_config.acp.saved_sessions = merge_restored_session_snapshots(
            next_config.acp.saved_sessions.clone(),
            &initial_snapshot_ids,
            retained,
        );
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
}

fn pi_agent_config() -> AcpAgentConfig {
    AcpAgentConfig {
        id: "pi-agent".to_string(),
        name: "Agent".to_string(),
        program: String::new(),
        args: Vec::new(),
        shell_command: None,
        launch_mode: crate::domain::acp::AcpAgentLaunchMode::Direct,
        mcp_servers: Vec::new(),
    }
}

fn resolve_agent_session_runtime_config(config: &AppConfig) -> AgentSessionRuntimeConfig {
    let Some(binding) = resolve_agent_model_binding(&config.llm) else {
        return AgentSessionRuntimeConfig::default();
    };
    let Some(provider) = resolve_pi_provider_id(binding.provider()) else {
        return AgentSessionRuntimeConfig::default();
    };
    let Some(model) = binding.model_name().map(str::to_string) else {
        return AgentSessionRuntimeConfig::default();
    };

    AgentSessionRuntimeConfig {
        provider: Some(provider),
        model: Some(model),
        api_key: non_empty_string(&binding.provider().api_key),
        ..AgentSessionRuntimeConfig::default()
    }
}

fn resolve_agent_model_binding(llm_settings: &LlmSettings) -> Option<ResolvedLlmModelBinding<'_>> {
    llm_settings
        .question_answer_model_id
        .as_deref()
        .and_then(|model_id| llm_settings.find_model_binding(model_id))
        .filter(|binding| binding.can_handle_ai_task())
        .or_else(|| {
            llm_settings
                .translation_model_id
                .as_deref()
                .and_then(|model_id| llm_settings.find_model_binding(model_id))
                .filter(|binding| binding.can_handle_ai_task())
        })
        .or_else(|| {
            llm_settings
                .iter_model_bindings()
                .find(|binding| binding.can_handle_ai_task())
        })
}

fn resolve_pi_provider_id(provider: &LlmProviderConfig) -> Option<String> {
    if let Some(provider_id) = provider
        .builtin_preset_id
        .as_deref()
        .and_then(resolve_pi_provider_id_from_template)
    {
        return Some(provider_id.to_string());
    }

    resolve_pi_provider_id_from_base_url(&provider.base_url).map(ToOwned::to_owned)
}

fn resolve_pi_provider_id_from_template(template_id: &str) -> Option<&'static str> {
    match template_id.trim() {
        "openai" => Some("openai"),
        "openrouter" => Some("openrouter"),
        "deepseek" => Some("deepseek"),
        "ollama" => Some("ollama"),
        "siliconflow" => Some("siliconflow-cn"),
        _ => None,
    }
}

fn resolve_pi_provider_id_from_base_url(base_url: &str) -> Option<&'static str> {
    let normalized = base_url.trim().trim_end_matches('/').to_ascii_lowercase();
    match normalized.as_str() {
        "https://api.openai.com/v1" => Some("openai"),
        "https://openrouter.ai/api/v1" => Some("openrouter"),
        "https://api.deepseek.com" | "https://api.deepseek.com/v1" => Some("deepseek"),
        "http://localhost:11434/v1" | "http://127.0.0.1:11434/v1" => Some("ollama"),
        "https://api.siliconflow.cn/v1" => Some("siliconflow-cn"),
        "https://api.siliconflow.com/v1" => Some("siliconflow"),
        _ => None,
    }
}

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn apply_app_settings_to_config(config: &mut AppConfig, settings: AppSettings) -> Result<()> {
    config.general = settings.general;
    config.notification = settings.notification;
    config.appearance = settings.appearance;
    config.prompts = settings.prompts;
    config.llm = settings.llm;
    config.ocr = settings.ocr;
    config.rag = settings.rag;
    config.normalize()?;
    validate_llm_settings(&config.llm)?;
    validate_ocr_settings(&config.ocr, &config.llm)?;
    validate_rag_settings(&config.rag, &config.llm)?;
    Ok(())
}

fn read_runtime_lock<'a, T>(
    lock: &'a StdRwLock<T>,
    label: &str,
) -> std::sync::RwLockReadGuard<'a, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                resource = label,
                "runtime state read lock poisoned; recovering state"
            );
            poisoned.into_inner()
        }
    }
}

fn write_runtime_lock<'a, T>(
    lock: &'a StdRwLock<T>,
    label: &str,
) -> std::sync::RwLockWriteGuard<'a, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                resource = label,
                "runtime state write lock poisoned; recovering state"
            );
            poisoned.into_inner()
        }
    }
}

fn normalize_optional_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_required_id(label: &str, value: &str) -> Result<String> {
    normalize_optional_id(Some(value)).ok_or_else(|| anyhow::anyhow!("{label} 不能为空"))
}

fn normalized_existing_session_id(
    session_id: Option<String>,
    sessions: &[AcpSessionSummary],
) -> Option<String> {
    session_id.filter(|session_id| sessions.iter().any(|item| item.session_id == *session_id))
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

#[derive(Clone)]
pub struct ShortcutRuntimeState {
    launcher_shortcut: Arc<StdRwLock<Option<Shortcut>>>,
    ocr_translate_shortcut: Arc<StdRwLock<Option<Shortcut>>>,
    open_clipboard_history_shortcut: Arc<StdRwLock<Option<Shortcut>>>,
    launcher_shortcut_status: Arc<StdRwLock<ShortcutRegistrationStatus>>,
    ocr_translate_shortcut_status: Arc<StdRwLock<ShortcutRegistrationStatus>>,
    open_clipboard_history_shortcut_status: Arc<StdRwLock<ShortcutRegistrationStatus>>,
    launcher_view_mode: Arc<StdRwLock<LauncherWindowViewMode>>,
    launcher_main_window_size: Arc<StdRwLock<Option<LauncherWindowSize>>>,
    launcher_clipboard_window_size: Arc<StdRwLock<Option<LauncherWindowSize>>>,
    launcher_visible: Arc<AtomicBool>,
    clipboard_history_visible: Arc<AtomicBool>,
    clipboard_window_preserves_launcher_focus: Arc<AtomicBool>,
    launcher_pinned: Arc<AtomicBool>,
    launcher_blur_auto_hide_enabled: Arc<AtomicBool>,
    launcher_resize_reposition_until: Arc<StdRwLock<Option<Instant>>>,
    launcher_blur_auto_hide_suppressed_until: Arc<StdRwLock<Option<Instant>>>,
    launcher_blur_auto_hide_sequence: Arc<AtomicUsize>,
    launcher_shortcut_pressed: Arc<StdRwLock<ShortcutPressGate>>,
    ocr_translate_shortcut_pressed: Arc<StdRwLock<ShortcutPressGate>>,
    open_clipboard_history_shortcut_pressed: Arc<StdRwLock<ShortcutPressGate>>,
    ocr_translate_active: Arc<AtomicBool>,
    transient_window_interactions: Arc<AtomicUsize>,
    #[cfg(target_os = "macos")]
    clipboard_external_paste_target_pid: Arc<StdRwLock<Option<i32>>>,
}

impl Default for ShortcutRuntimeState {
    fn default() -> Self {
        Self {
            launcher_shortcut: Arc::new(StdRwLock::new(None)),
            ocr_translate_shortcut: Arc::new(StdRwLock::new(None)),
            open_clipboard_history_shortcut: Arc::new(StdRwLock::new(None)),
            launcher_shortcut_status: Arc::new(StdRwLock::new(
                ShortcutRegistrationStatus::default(),
            )),
            ocr_translate_shortcut_status: Arc::new(StdRwLock::new(
                ShortcutRegistrationStatus::default(),
            )),
            open_clipboard_history_shortcut_status: Arc::new(StdRwLock::new(
                ShortcutRegistrationStatus::default(),
            )),
            launcher_view_mode: Arc::new(StdRwLock::new(LauncherWindowViewMode::Main)),
            launcher_main_window_size: Arc::new(StdRwLock::new(None)),
            launcher_clipboard_window_size: Arc::new(StdRwLock::new(None)),
            launcher_visible: Arc::new(AtomicBool::new(false)),
            clipboard_history_visible: Arc::new(AtomicBool::new(false)),
            clipboard_window_preserves_launcher_focus: Arc::new(AtomicBool::new(false)),
            launcher_pinned: Arc::new(AtomicBool::new(false)),
            launcher_blur_auto_hide_enabled: Arc::new(AtomicBool::new(true)),
            launcher_resize_reposition_until: Arc::new(StdRwLock::new(None)),
            launcher_blur_auto_hide_suppressed_until: Arc::new(StdRwLock::new(None)),
            launcher_blur_auto_hide_sequence: Arc::new(AtomicUsize::new(0)),
            launcher_shortcut_pressed: Arc::new(StdRwLock::new(ShortcutPressGate::default())),
            ocr_translate_shortcut_pressed: Arc::new(StdRwLock::new(ShortcutPressGate::default())),
            open_clipboard_history_shortcut_pressed: Arc::new(StdRwLock::new(
                ShortcutPressGate::default(),
            )),
            ocr_translate_active: Arc::new(AtomicBool::new(false)),
            transient_window_interactions: Arc::new(AtomicUsize::new(0)),
            #[cfg(target_os = "macos")]
            clipboard_external_paste_target_pid: Arc::new(StdRwLock::new(None)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    ToggleLauncher,
    OcrTranslate,
    OpenClipboardHistory,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutRuntimeStatusEntry {
    pub configured_shortcut: String,
    pub registered: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ShortcutRuntimeStatusSnapshot {
    pub toggle_launcher: ShortcutRuntimeStatusEntry,
    pub ocr_translate: ShortcutRuntimeStatusEntry,
    pub open_clipboard_history: ShortcutRuntimeStatusEntry,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ShortcutRegistrationStatus {
    configured_shortcut: String,
    registered: bool,
    message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherWindowViewMode {
    Main,
    ClipboardHistory,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LauncherWindowSize {
    pub width: f64,
    pub height: f64,
}

impl ShortcutRuntimeState {
    pub fn shortcut_action(&self, shortcut: Shortcut) -> Option<ShortcutAction> {
        for (key, action) in [
            (ShortcutKey::ToggleLauncher, ShortcutAction::ToggleLauncher),
            (ShortcutKey::OcrTranslate, ShortcutAction::OcrTranslate),
            (
                ShortcutKey::OpenClipboardHistory,
                ShortcutAction::OpenClipboardHistory,
            ),
        ] {
            if self.current_shortcut(key) == Some(shortcut) {
                return Some(action);
            }
        }

        None
    }

    pub fn current_shortcut(&self, key: ShortcutKey) -> Option<Shortcut> {
        match key {
            ShortcutKey::ToggleLauncher => {
                *read_runtime_lock(&self.launcher_shortcut, "launcher shortcut")
            }
            ShortcutKey::OcrTranslate => {
                *read_runtime_lock(&self.ocr_translate_shortcut, "ocr translate shortcut")
            }
            ShortcutKey::OpenClipboardHistory => *read_runtime_lock(
                &self.open_clipboard_history_shortcut,
                "clipboard history shortcut",
            ),
        }
    }

    pub fn set_shortcut(&self, key: ShortcutKey, shortcut: Option<Shortcut>) {
        match key {
            ShortcutKey::ToggleLauncher => {
                *write_runtime_lock(&self.launcher_shortcut, "launcher shortcut") = shortcut
            }
            ShortcutKey::OcrTranslate => {
                *write_runtime_lock(&self.ocr_translate_shortcut, "ocr translate shortcut") =
                    shortcut;
            }
            ShortcutKey::OpenClipboardHistory => {
                *write_runtime_lock(
                    &self.open_clipboard_history_shortcut,
                    "clipboard history shortcut",
                ) = shortcut;
            }
        }
    }

    pub fn set_shortcut_registration_status(
        &self,
        key: ShortcutKey,
        configured_shortcut: impl Into<String>,
        registered: bool,
        message: Option<String>,
    ) {
        let status = ShortcutRegistrationStatus {
            configured_shortcut: configured_shortcut.into(),
            registered,
            message,
        };

        match key {
            ShortcutKey::ToggleLauncher => {
                *write_runtime_lock(&self.launcher_shortcut_status, "launcher shortcut status") =
                    status
            }
            ShortcutKey::OcrTranslate => {
                *write_runtime_lock(
                    &self.ocr_translate_shortcut_status,
                    "ocr translate shortcut status",
                ) = status;
            }
            ShortcutKey::OpenClipboardHistory => {
                *write_runtime_lock(
                    &self.open_clipboard_history_shortcut_status,
                    "clipboard history shortcut status",
                ) = status;
            }
        }
    }

    pub fn shortcut_runtime_status(&self) -> ShortcutRuntimeStatusSnapshot {
        ShortcutRuntimeStatusSnapshot {
            toggle_launcher: Self::read_shortcut_status(&self.launcher_shortcut_status),
            ocr_translate: Self::read_shortcut_status(&self.ocr_translate_shortcut_status),
            open_clipboard_history: Self::read_shortcut_status(
                &self.open_clipboard_history_shortcut_status,
            ),
        }
    }

    fn read_shortcut_status(
        status: &StdRwLock<ShortcutRegistrationStatus>,
    ) -> ShortcutRuntimeStatusEntry {
        let status = read_runtime_lock(status, "shortcut registration status").clone();
        ShortcutRuntimeStatusEntry {
            configured_shortcut: status.configured_shortcut,
            registered: status.registered,
            message: status.message,
        }
    }

    pub fn is_launcher_visible(&self) -> bool {
        self.launcher_visible.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn launcher_view_mode(&self) -> LauncherWindowViewMode {
        *read_runtime_lock(&self.launcher_view_mode, "launcher view mode")
    }

    pub fn set_launcher_view_mode(&self, mode: LauncherWindowViewMode) {
        *write_runtime_lock(&self.launcher_view_mode, "launcher view mode") = mode;
    }

    pub fn cached_launcher_window_size(
        &self,
        mode: LauncherWindowViewMode,
    ) -> Option<LauncherWindowSize> {
        match mode {
            LauncherWindowViewMode::Main => {
                *read_runtime_lock(&self.launcher_main_window_size, "launcher main window size")
            }
            LauncherWindowViewMode::ClipboardHistory => *read_runtime_lock(
                &self.launcher_clipboard_window_size,
                "clipboard window size",
            ),
        }
    }

    pub fn remember_launcher_window_size(
        &self,
        mode: LauncherWindowViewMode,
        size: LauncherWindowSize,
    ) {
        match mode {
            LauncherWindowViewMode::Main => {
                *write_runtime_lock(&self.launcher_main_window_size, "launcher main window size") =
                    Some(size)
            }
            LauncherWindowViewMode::ClipboardHistory => {
                *write_runtime_lock(
                    &self.launcher_clipboard_window_size,
                    "clipboard window size",
                ) = Some(size);
            }
        }
    }

    #[cfg(test)]
    pub fn remember_current_launcher_window_size(&self, size: LauncherWindowSize) {
        self.remember_launcher_window_size(self.launcher_view_mode(), size);
    }

    pub fn set_launcher_visible(&self, visible: bool) {
        self.launcher_visible.store(visible, Ordering::SeqCst);
    }

    pub fn is_clipboard_history_visible(&self) -> bool {
        self.clipboard_history_visible.load(Ordering::SeqCst)
    }

    pub fn set_clipboard_history_visible(&self, visible: bool) {
        self.clipboard_history_visible
            .store(visible, Ordering::SeqCst);
    }

    pub fn clipboard_window_preserves_launcher_focus(&self) -> bool {
        self.clipboard_window_preserves_launcher_focus
            .load(Ordering::SeqCst)
    }

    pub fn set_clipboard_window_preserves_launcher_focus(&self, preserve: bool) {
        self.clipboard_window_preserves_launcher_focus
            .store(preserve, Ordering::SeqCst);
    }

    pub fn is_launcher_pinned(&self) -> bool {
        self.launcher_pinned.load(Ordering::SeqCst)
    }

    pub fn set_launcher_pinned(&self, pinned: bool) {
        self.launcher_pinned.store(pinned, Ordering::SeqCst);
    }

    pub fn is_launcher_blur_auto_hide_enabled(&self) -> bool {
        self.launcher_blur_auto_hide_enabled.load(Ordering::SeqCst)
    }

    pub fn set_launcher_blur_auto_hide_enabled(&self, enabled: bool) {
        self.launcher_blur_auto_hide_enabled
            .store(enabled, Ordering::SeqCst);
    }

    pub fn arm_launcher_resize_reposition(&self, duration: Duration) {
        *write_runtime_lock(
            &self.launcher_resize_reposition_until,
            "launcher resize reposition deadline",
        ) = Some(Instant::now() + duration);
    }

    pub fn clear_launcher_resize_reposition(&self) {
        *write_runtime_lock(
            &self.launcher_resize_reposition_until,
            "launcher resize reposition deadline",
        ) = None;
    }

    pub fn arm_launcher_blur_auto_hide_suppression(&self, duration: Duration) -> Instant {
        let deadline = Instant::now() + duration;
        *write_runtime_lock(
            &self.launcher_blur_auto_hide_suppressed_until,
            "launcher blur auto hide suppression deadline",
        ) = Some(deadline);
        deadline
    }

    pub fn clear_launcher_blur_auto_hide_suppression(&self) {
        *write_runtime_lock(
            &self.launcher_blur_auto_hide_suppressed_until,
            "launcher blur auto hide suppression deadline",
        ) = None;
    }

    pub fn launcher_blur_auto_hide_delay(&self) -> Option<Duration> {
        read_runtime_lock(
            &self.launcher_blur_auto_hide_suppressed_until,
            "launcher blur auto hide suppression deadline",
        )
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

        read_runtime_lock(
            &self.launcher_resize_reposition_until,
            "launcher resize reposition deadline",
        )
        .is_some_and(|deadline| Instant::now() <= deadline)
    }

    pub fn begin_shortcut_press(&self, action: ShortcutAction) -> bool {
        Self::begin_shortcut_press_gate(match action {
            ShortcutAction::ToggleLauncher => &self.launcher_shortcut_pressed,
            ShortcutAction::OcrTranslate => &self.ocr_translate_shortcut_pressed,
            ShortcutAction::OpenClipboardHistory => &self.open_clipboard_history_shortcut_pressed,
        })
    }

    pub fn end_shortcut_press(&self, action: ShortcutAction) {
        Self::end_shortcut_press_gate(match action {
            ShortcutAction::ToggleLauncher => &self.launcher_shortcut_pressed,
            ShortcutAction::OcrTranslate => &self.ocr_translate_shortcut_pressed,
            ShortcutAction::OpenClipboardHistory => &self.open_clipboard_history_shortcut_pressed,
        });
    }

    pub fn begin_ocr_translate_flow(&self) -> bool {
        !self.ocr_translate_active.swap(true, Ordering::SeqCst)
    }

    pub fn end_ocr_translate_flow(&self) {
        self.ocr_translate_active.store(false, Ordering::SeqCst);
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

    #[cfg(target_os = "macos")]
    pub fn remember_clipboard_external_paste_target_pid(&self, pid: Option<i32>) {
        *write_runtime_lock(
            &self.clipboard_external_paste_target_pid,
            "clipboard external paste target pid",
        ) = pid;
    }

    #[cfg(target_os = "macos")]
    pub fn take_clipboard_external_paste_target_pid(&self) -> Option<i32> {
        write_runtime_lock(
            &self.clipboard_external_paste_target_pid,
            "clipboard external paste target pid",
        )
        .take()
    }

    fn begin_shortcut_press_gate(gate: &StdRwLock<ShortcutPressGate>) -> bool {
        let mut gate = write_runtime_lock(gate, "shortcut press gate");
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
        let mut gate = write_runtime_lock(gate, "shortcut press gate");
        gate.pressed = false;
        gate.pressed_at = None;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::Duration;

    use super::{
        acp_catalog::{
            effective_mcp_servers, normalize_acp_agent_catalog, normalize_mcp_remote_url,
            reconcile_saved_session_builtin_mcp,
        },
        apply_app_settings_to_config, merge_restored_session_snapshots, normalize_optional_id,
        normalize_required_id, normalized_existing_session_id,
        resolve_agent_session_runtime_config, resolve_pi_provider_id_from_base_url,
        session_snapshot::{apply_session_snapshot, SnapshotWriteMode},
        settings::{validate_llm_provider_config, validate_rag_settings},
        LauncherWindowSize, LauncherWindowViewMode, ShortcutAction, ShortcutRuntimeState,
        SHORTCUT_PRESS_STALE_AFTER,
    };
    use crate::domain::{
        acp::{
            AcpAgentCatalog, AcpAgentConfig, AcpAgentLaunchMode, AcpMcpServerCatalog,
            AcpMcpServerConfig, AcpMcpServerHttpConfig, AcpSessionStatus, AcpSessionSummary,
            BuiltinMcpConfig, BuiltinMcpModuleKey,
        },
        settings::{
            AppSettings, LlmModelConfig, LlmModelType, LlmProviderConfig, LlmProviderModelEntry,
            LlmProviderProtocol, LlmSettings, OcrProviderKind, RagSettings,
        },
    };
    use crate::infrastructure::config::{AppConfig, SavedAcpSession};
    use crate::infrastructure::openai_compatible::{extract_model_entries, extract_model_ids};
    use crate::services::builtin_mcp;
    use serde_json::json;

    fn test_agent_provider(id: &str, builtin_preset_id: Option<&str>) -> LlmProviderConfig {
        LlmProviderConfig {
            id: id.to_string(),
            name: id.to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            protocol: LlmProviderProtocol::Responses,
            models: vec![LlmModelConfig {
                id: format!("{id}-model"),
                model_type: LlmModelType::Llm,
                model: "test-model".to_string(),
                model_identity_hint: None,
                builtin_preset_model_id: None,
                supports_multimodal: false,
                supports_stateful: true,
            }],
            builtin_preset_id: builtin_preset_id.map(ToOwned::to_owned),
            managed_base_url: false,
        }
    }

    #[test]
    fn agent_runtime_config_reuses_question_answer_model_when_provider_maps_to_pi() {
        let provider = test_agent_provider("qa", Some("openrouter"));
        let model_id = provider.models[0].id.clone();
        let config = AppConfig {
            llm: LlmSettings {
                providers: vec![provider],
                translation_model_id: None,
                question_answer_model_id: Some(model_id),
            },
            ..AppConfig::default()
        };

        let runtime = resolve_agent_session_runtime_config(&config);

        assert_eq!(runtime.provider.as_deref(), Some("openrouter"));
        assert_eq!(runtime.model.as_deref(), Some("test-model"));
        assert_eq!(runtime.api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn agent_runtime_config_falls_back_for_unmapped_provider() {
        let provider = test_agent_provider("custom", None);
        let model_id = provider.models[0].id.clone();
        let config = AppConfig {
            llm: LlmSettings {
                providers: vec![provider],
                translation_model_id: None,
                question_answer_model_id: Some(model_id),
            },
            ..AppConfig::default()
        };

        let runtime = resolve_agent_session_runtime_config(&config);

        assert!(runtime.provider.is_none());
        assert!(runtime.model.is_none());
        assert!(runtime.api_key.is_none());
    }

    #[test]
    fn pi_provider_base_url_mapping_accepts_ollama_localhost_alias() {
        assert_eq!(
            resolve_pi_provider_id_from_base_url("http://localhost:11434/v1/"),
            Some("ollama")
        );
    }

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
    fn clipboard_history_shortcut_press_only_triggers_once_until_release() {
        let state = ShortcutRuntimeState::default();

        assert!(state.begin_shortcut_press(ShortcutAction::OpenClipboardHistory));
        assert!(!state.begin_shortcut_press(ShortcutAction::OpenClipboardHistory));

        state.end_shortcut_press(ShortcutAction::OpenClipboardHistory);

        assert!(state.begin_shortcut_press(ShortcutAction::OpenClipboardHistory));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clipboard_external_paste_target_pid_round_trips_once() {
        let state = ShortcutRuntimeState::default();

        state.remember_clipboard_external_paste_target_pid(Some(4242));

        assert_eq!(state.take_clipboard_external_paste_target_pid(), Some(4242));
        assert_eq!(state.take_clipboard_external_paste_target_pid(), None);
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
    fn launcher_window_view_mode_defaults_to_main() {
        let state = ShortcutRuntimeState::default();

        assert_eq!(state.launcher_view_mode(), LauncherWindowViewMode::Main);
    }

    #[test]
    fn launcher_window_size_cache_round_trips_per_view_mode() {
        let state = ShortcutRuntimeState::default();
        let main_size = LauncherWindowSize {
            width: 760.0,
            height: 280.0,
        };
        let clipboard_size = LauncherWindowSize {
            width: 452.0,
            height: 398.0,
        };

        state.remember_launcher_window_size(LauncherWindowViewMode::Main, main_size);
        state.remember_launcher_window_size(
            LauncherWindowViewMode::ClipboardHistory,
            clipboard_size,
        );

        assert_eq!(
            state.cached_launcher_window_size(LauncherWindowViewMode::Main),
            Some(main_size)
        );
        assert_eq!(
            state.cached_launcher_window_size(LauncherWindowViewMode::ClipboardHistory),
            Some(clipboard_size)
        );
    }

    #[test]
    fn current_launcher_window_size_cache_follows_current_view_mode() {
        let state = ShortcutRuntimeState::default();
        let clipboard_size = LauncherWindowSize {
            width: 448.0,
            height: 372.0,
        };

        state.set_launcher_view_mode(LauncherWindowViewMode::ClipboardHistory);
        state.remember_current_launcher_window_size(clipboard_size);

        assert_eq!(
            state.cached_launcher_window_size(LauncherWindowViewMode::ClipboardHistory),
            Some(clipboard_size)
        );
        assert_eq!(
            state.cached_launcher_window_size(LauncherWindowViewMode::Main),
            None
        );
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

    #[test]
    fn launcher_pinned_flag_round_trips() {
        let state = ShortcutRuntimeState::default();

        assert!(!state.is_launcher_pinned());
        state.set_launcher_pinned(true);
        assert!(state.is_launcher_pinned());
        state.set_launcher_pinned(false);
        assert!(!state.is_launcher_pinned());
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
    fn ocr_translate_only_allows_one_active_flow() {
        let state = ShortcutRuntimeState::default();

        assert!(state.begin_ocr_translate_flow());
        assert!(!state.begin_ocr_translate_flow());

        state.end_ocr_translate_flow();

        assert!(state.begin_ocr_translate_flow());
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
    fn embedding_provider_allows_multimodal_flag() {
        let provider = LlmProviderConfig {
            id: "embedding".to_string(),
            name: "Embedding".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: String::new(),
            models: vec![crate::domain::settings::LlmModelConfig {
                id: "embedding".to_string(),
                model_type: crate::domain::settings::LlmModelType::Embedding,
                model: "text-embedding-3-small".to_string(),
                supports_multimodal: true,
                ..crate::domain::settings::LlmModelConfig::default()
            }],
            ..LlmProviderConfig::default()
        };

        validate_llm_provider_config(&provider)
            .expect("embedding providers should allow multimodal flag");
    }

    #[test]
    fn missing_responses_model_rejects_stateful_flag() {
        let provider = LlmProviderConfig {
            id: "embedding".to_string(),
            name: "Embedding".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: String::new(),
            models: vec![crate::domain::settings::LlmModelConfig {
                id: "embedding".to_string(),
                model_type: crate::domain::settings::LlmModelType::Embedding,
                model: "text-embedding-3-small".to_string(),
                supports_stateful: true,
                ..crate::domain::settings::LlmModelConfig::default()
            }],
            ..LlmProviderConfig::default()
        };

        let error =
            validate_llm_provider_config(&provider).expect_err("stateful requires responses");
        assert!(error.to_string().contains("responses 协议"));
    }

    #[test]
    fn builtin_provider_allows_custom_model_without_catalog_binding() {
        let provider = LlmProviderConfig {
            id: "zhipu".to_string(),
            name: "智谱 AI".to_string(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
            api_key: "key".to_string(),
            protocol: crate::domain::settings::LlmProviderProtocol::ChatCompletions,
            models: vec![crate::domain::settings::LlmModelConfig {
                id: "zhipu".to_string(),
                model: "glm-4.9".to_string(),
                ..crate::domain::settings::LlmModelConfig::default()
            }],
            builtin_preset_id: Some("zhipu".to_string()),
            managed_base_url: true,
        };

        validate_llm_provider_config(&provider)
            .expect("builtin provider should allow custom model without catalog binding");
    }

    #[test]
    fn builtin_provider_rejects_mismatched_bound_catalog_model() {
        let provider = LlmProviderConfig {
            id: "zhipu".to_string(),
            name: "智谱 AI".to_string(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
            api_key: "key".to_string(),
            protocol: crate::domain::settings::LlmProviderProtocol::ChatCompletions,
            models: vec![crate::domain::settings::LlmModelConfig {
                id: "zhipu".to_string(),
                model: "glm-4.9".to_string(),
                builtin_preset_model_id: Some("glm-4.7-flash".to_string()),
                ..crate::domain::settings::LlmModelConfig::default()
            }],
            builtin_preset_id: Some("zhipu".to_string()),
            managed_base_url: true,
        };

        let error = validate_llm_provider_config(&provider)
            .expect_err("bound builtin model metadata must stay consistent");
        assert!(error.to_string().contains("白名单"));
    }

    #[test]
    fn builtin_provider_rejects_unselectable_catalog_model() {
        let provider = LlmProviderConfig {
            id: "zhipu".to_string(),
            name: "智谱 AI".to_string(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
            api_key: "key".to_string(),
            protocol: crate::domain::settings::LlmProviderProtocol::ChatCompletions,
            models: vec![crate::domain::settings::LlmModelConfig {
                id: "zhipu".to_string(),
                model: "cogview-3-flash".to_string(),
                builtin_preset_model_id: Some("cogview-3-flash".to_string()),
                ..crate::domain::settings::LlmModelConfig::default()
            }],
            builtin_preset_id: Some("zhipu".to_string()),
            managed_base_url: true,
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
            embedding_model_id: None,
        };
        let llm_settings = LlmSettings::default();

        let error = validate_rag_settings(&rag_settings, &llm_settings)
            .expect_err("RAG sources must require an embedding provider");

        assert!(error.to_string().contains("必须选择一个 embedding 模型"));
    }

    #[test]
    fn rag_settings_reject_non_embedding_provider() {
        let rag_settings = RagSettings {
            source_directories: Vec::new(),
            ignore_globs: Vec::new(),
            embedding_model_id: Some("chat".to_string()),
        };
        let llm_settings = LlmSettings {
            providers: vec![LlmProviderConfig {
                id: "chat".to_string(),
                name: "Chat".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                models: vec![crate::domain::settings::LlmModelConfig {
                    id: "chat".to_string(),
                    model_type: crate::domain::settings::LlmModelType::Llm,
                    model: "gpt-4.1-mini".to_string(),
                    supports_multimodal: false,
                    ..crate::domain::settings::LlmModelConfig::default()
                }],
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
            embedding_model_id: Some("chat".to_string()),
        };
        let llm_settings = LlmSettings {
            providers: vec![LlmProviderConfig {
                id: "chat".to_string(),
                name: "Chat".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                models: vec![crate::domain::settings::LlmModelConfig {
                    id: "chat".to_string(),
                    model_type: crate::domain::settings::LlmModelType::Embedding,
                    model: "text-embedding-3-small".to_string(),
                    supports_multimodal: false,
                    ..crate::domain::settings::LlmModelConfig::default()
                }],
                ..LlmProviderConfig::default()
            }],
            ..LlmSettings::default()
        };

        validate_rag_settings(&rag_settings, &llm_settings)
            .expect("RAG should accept providers with embedding capability");
    }

    #[test]
    fn apply_app_settings_repairs_dependent_model_references_before_validation() {
        let mut config = AppConfig::default();
        let settings = AppSettings {
            general: config.general.clone(),
            notification: config.notification.clone(),
            appearance: config.appearance.clone(),
            prompts: config.prompts.clone(),
            llm: LlmSettings {
                providers: vec![
                    LlmProviderConfig {
                        id: "replacement-chat".to_string(),
                        name: "Replacement Chat".to_string(),
                        base_url: "https://api.example.com/v1".to_string(),
                        api_key: String::new(),
                        protocol: LlmProviderProtocol::Responses,
                        models: vec![LlmModelConfig {
                            id: "replacement-chat".to_string(),
                            model_type: LlmModelType::Llm,
                            model: "gpt-4.1-mini".to_string(),
                            supports_multimodal: true,
                            ..LlmModelConfig::default()
                        }],
                        ..LlmProviderConfig::default()
                    },
                    LlmProviderConfig {
                        id: "replacement-embedding".to_string(),
                        name: "Replacement Embedding".to_string(),
                        base_url: "https://api.example.com/v1".to_string(),
                        api_key: String::new(),
                        protocol: LlmProviderProtocol::Responses,
                        models: vec![LlmModelConfig {
                            id: "replacement-embedding".to_string(),
                            model_type: LlmModelType::Embedding,
                            model: "text-embedding-3-small".to_string(),
                            ..LlmModelConfig::default()
                        }],
                        ..LlmProviderConfig::default()
                    },
                ],
                translation_model_id: Some("deleted-chat".to_string()),
                question_answer_model_id: Some("deleted-chat".to_string()),
            },
            ocr: crate::domain::settings::OcrSettings {
                provider: OcrProviderKind::LlmOcr,
                llm_model_id: Some("deleted-ocr".to_string()),
            },
            rag: RagSettings {
                source_directories: Vec::new(),
                ignore_globs: Vec::new(),
                embedding_model_id: Some("deleted-embedding".to_string()),
            },
        };

        apply_app_settings_to_config(&mut config, settings)
            .expect("dependent model references should be repaired before validation");

        assert_eq!(config.llm.translation_model_id, None);
        assert_eq!(config.llm.question_answer_model_id, None);
        assert_eq!(config.ocr.llm_model_id.as_deref(), Some("replacement-chat"));
        assert_eq!(
            config.rag.embedding_model_id.as_deref(),
            Some("replacement-embedding")
        );
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

    #[test]
    fn normalize_acp_agent_catalog_parses_direct_shell_command_into_argv() {
        let catalog = AcpAgentCatalog {
            agents: vec![AcpAgentConfig {
                id: "agent-1".to_string(),
                name: "Direct Agent".to_string(),
                program: String::new(),
                args: Vec::new(),
                shell_command: Some("uvx --from example-agent agent --stdio".to_string()),
                launch_mode: AcpAgentLaunchMode::Direct,
                mcp_servers: Vec::new(),
            }],
            default_agent_id: None,
        };

        let normalized = normalize_acp_agent_catalog(catalog).expect("direct agent should parse");

        assert_eq!(normalized.agents[0].program, "uvx");
        assert_eq!(
            normalized.agents[0].args,
            vec![
                "--from".to_string(),
                "example-agent".to_string(),
                "agent".to_string(),
                "--stdio".to_string()
            ]
        );
    }

    #[test]
    fn normalize_acp_agent_catalog_rejects_invalid_direct_shell_command() {
        let catalog = AcpAgentCatalog {
            agents: vec![AcpAgentConfig {
                id: "agent-1".to_string(),
                name: "Broken Direct Agent".to_string(),
                program: String::new(),
                args: Vec::new(),
                shell_command: Some("\"unterminated".to_string()),
                launch_mode: AcpAgentLaunchMode::Direct,
                mcp_servers: Vec::new(),
            }],
            default_agent_id: None,
        };

        let error = normalize_acp_agent_catalog(catalog)
            .expect_err("invalid direct shell command should fail validation");

        assert!(error.to_string().contains("直连命令解析失败"));
    }

    #[test]
    fn normalize_acp_agent_catalog_fills_missing_and_duplicate_ids() {
        let catalog = AcpAgentCatalog {
            agents: vec![
                AcpAgentConfig {
                    id: "  ".to_string(),
                    name: "Codex".to_string(),
                    program: "codex-acp".to_string(),
                    args: Vec::new(),
                    shell_command: None,
                    launch_mode: AcpAgentLaunchMode::Direct,
                    mcp_servers: Vec::new(),
                },
                AcpAgentConfig {
                    id: "agent".to_string(),
                    name: "Claude".to_string(),
                    program: "claude-agent".to_string(),
                    args: Vec::new(),
                    shell_command: None,
                    launch_mode: AcpAgentLaunchMode::Direct,
                    mcp_servers: Vec::new(),
                },
                AcpAgentConfig {
                    id: "agent".to_string(),
                    name: "OpenCode".to_string(),
                    program: "opencode".to_string(),
                    args: Vec::new(),
                    shell_command: None,
                    launch_mode: AcpAgentLaunchMode::Direct,
                    mcp_servers: Vec::new(),
                },
            ],
            default_agent_id: Some(" agent ".to_string()),
        };

        let normalized = normalize_acp_agent_catalog(catalog).expect("agent ids should normalize");

        assert_eq!(normalized.agents[0].id, "agent-1");
        assert_eq!(normalized.agents[1].id, "agent");
        assert_eq!(normalized.agents[2].id, "agent-2");
        assert_eq!(normalized.default_agent_id.as_deref(), Some("agent"));
    }

    #[test]
    fn normalize_optional_id_treats_blank_value_as_missing() {
        assert_eq!(
            normalize_optional_id(Some(" agent-1 ")).as_deref(),
            Some("agent-1")
        );
        assert_eq!(normalize_optional_id(Some("   ")), None);
        assert_eq!(normalize_optional_id(None), None);
    }

    #[test]
    fn normalize_required_id_rejects_blank_value() {
        assert_eq!(
            normalize_required_id("session", " session-1 ")
                .expect("trimmed session id should be accepted"),
            "session-1"
        );
        assert!(normalize_required_id("session", "   ").is_err());
    }

    #[test]
    fn normalized_existing_session_id_rejects_unknown_or_blank_value() {
        let sessions = vec![AcpSessionSummary {
            session_id: "session-1".to_string(),
            workspace_root: "/tmp/demo".to_string(),
            title: "Demo".to_string(),
            agent_id: Some("agent-1".to_string()),
            agent_name: "Agent".to_string(),
            status: AcpSessionStatus::Idle,
            error_level: None,
            attention: false,
            is_active: false,
            last_error: None,
            last_updated_at_ms: 1,
        }];

        assert_eq!(
            normalized_existing_session_id(Some("session-1".to_string()), &sessions).as_deref(),
            Some("session-1")
        );
        assert_eq!(
            normalized_existing_session_id(Some("missing".to_string()), &sessions),
            None
        );
        assert_eq!(normalized_existing_session_id(None, &sessions), None);
    }

    #[test]
    fn effective_mcp_servers_requires_running_builtin_server() {
        let catalog = AcpMcpServerCatalog {
            servers: vec![AcpMcpServerConfig::Http(AcpMcpServerHttpConfig {
                name: "WebMCP".to_string(),
                url: "https://example.com/mcp".to_string(),
                headers: Vec::new(),
            })],
            builtin: BuiltinMcpConfig {
                enabled: true,
                enabled_modules: vec![BuiltinMcpModuleKey::Document],
            },
        };

        let without_builtin = effective_mcp_servers(&catalog, false);
        let with_builtin = effective_mcp_servers(&catalog, true);

        assert_eq!(without_builtin.len(), 1);
        assert_eq!(with_builtin.len(), 2);
        assert!(with_builtin.iter().any(builtin_mcp::is_builtin_server));
    }

    #[test]
    fn reconcile_saved_session_builtin_mcp_uses_current_runtime_availability() {
        let snapshot = SavedAcpSession {
            session_id: "session-1".to_string(),
            workspace_root: "/tmp/workspace".to_string(),
            title: "workspace".to_string(),
            agent_id: Some("agent-1".to_string()),
            agent_name: "Codex".to_string(),
            agent_program: "codex-acp".to_string(),
            agent_args: Vec::new(),
            agent_shell_command: None,
            agent_launch_mode: AcpAgentLaunchMode::Direct,
            mcp_servers: vec![
                builtin_mcp::builtin_server_config(),
                AcpMcpServerConfig::Http(AcpMcpServerHttpConfig {
                    name: "WebMCP".to_string(),
                    url: "https://example.com/mcp".to_string(),
                    headers: Vec::new(),
                }),
            ],
            last_updated_at_ms: 0,
        };
        let catalog = AcpMcpServerCatalog {
            servers: vec![AcpMcpServerConfig::Http(AcpMcpServerHttpConfig {
                name: "WebMCP".to_string(),
                url: "https://example.com/mcp".to_string(),
                headers: Vec::new(),
            })],
            builtin: BuiltinMcpConfig {
                enabled: true,
                enabled_modules: vec![BuiltinMcpModuleKey::Rag],
            },
        };

        let without_builtin =
            reconcile_saved_session_builtin_mcp(snapshot.clone(), &catalog, false);
        let with_builtin = reconcile_saved_session_builtin_mcp(snapshot, &catalog, true);

        assert_eq!(without_builtin.mcp_servers.len(), 1);
        assert!(!without_builtin
            .mcp_servers
            .iter()
            .any(builtin_mcp::is_builtin_server));
        assert_eq!(with_builtin.mcp_servers.len(), 2);
        assert_eq!(
            with_builtin
                .mcp_servers
                .iter()
                .filter(|server| builtin_mcp::is_builtin_server(server))
                .count(),
            1
        );
    }

    #[test]
    fn merge_restored_session_snapshots_preserves_newer_runtime_persisted_snapshot() {
        let initial_snapshot_ids = HashSet::from(["session-1".to_string()]);
        let current = vec![
            SavedAcpSession {
                session_id: "session-1".to_string(),
                workspace_root: "/tmp/workspace".to_string(),
                title: "runtime-title".to_string(),
                agent_id: Some("agent-1".to_string()),
                agent_name: "Codex".to_string(),
                agent_program: "codex-acp".to_string(),
                agent_args: Vec::new(),
                agent_shell_command: None,
                agent_launch_mode: AcpAgentLaunchMode::Direct,
                mcp_servers: Vec::new(),
                last_updated_at_ms: 20,
            },
            SavedAcpSession {
                session_id: "session-2".to_string(),
                workspace_root: "/tmp/other".to_string(),
                title: "other".to_string(),
                agent_id: Some("agent-2".to_string()),
                agent_name: "Other".to_string(),
                agent_program: "other".to_string(),
                agent_args: Vec::new(),
                agent_shell_command: None,
                agent_launch_mode: AcpAgentLaunchMode::Direct,
                mcp_servers: Vec::new(),
                last_updated_at_ms: 5,
            },
        ];
        let retained = vec![SavedAcpSession {
            session_id: "session-1".to_string(),
            workspace_root: "/tmp/workspace".to_string(),
            title: "stale-title".to_string(),
            agent_id: Some("agent-1".to_string()),
            agent_name: "Codex".to_string(),
            agent_program: "codex-acp".to_string(),
            agent_args: Vec::new(),
            agent_shell_command: None,
            agent_launch_mode: AcpAgentLaunchMode::Direct,
            mcp_servers: Vec::new(),
            last_updated_at_ms: 10,
        }];

        let merged = merge_restored_session_snapshots(current, &initial_snapshot_ids, retained);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].session_id, "session-1");
        assert_eq!(merged[0].title, "runtime-title");
    }

    #[test]
    fn merge_restored_session_snapshots_drops_unretained_initial_snapshot() {
        let initial_snapshot_ids = HashSet::from(["session-1".to_string()]);
        let current = vec![SavedAcpSession {
            session_id: "session-1".to_string(),
            workspace_root: "/tmp/workspace".to_string(),
            title: "old".to_string(),
            agent_id: Some("agent-1".to_string()),
            agent_name: "Codex".to_string(),
            agent_program: "codex-acp".to_string(),
            agent_args: Vec::new(),
            agent_shell_command: None,
            agent_launch_mode: AcpAgentLaunchMode::Direct,
            mcp_servers: Vec::new(),
            last_updated_at_ms: 10,
        }];

        let merged = merge_restored_session_snapshots(current, &initial_snapshot_ids, Vec::new());

        assert!(merged.is_empty());
    }

    #[test]
    fn merge_restored_session_snapshots_preserves_runtime_snapshot_on_equal_timestamp() {
        let initial_snapshot_ids = HashSet::from(["session-1".to_string()]);
        let current = vec![SavedAcpSession {
            session_id: "session-1".to_string(),
            workspace_root: "/tmp/workspace".to_string(),
            title: "runtime-title".to_string(),
            agent_id: Some("agent-1".to_string()),
            agent_name: "Codex".to_string(),
            agent_program: "codex-acp".to_string(),
            agent_args: Vec::new(),
            agent_shell_command: None,
            agent_launch_mode: AcpAgentLaunchMode::Direct,
            mcp_servers: Vec::new(),
            last_updated_at_ms: 10,
        }];
        let retained = vec![SavedAcpSession {
            session_id: "session-1".to_string(),
            workspace_root: "/tmp/workspace".to_string(),
            title: "retained-title".to_string(),
            agent_id: Some("agent-1".to_string()),
            agent_name: "Codex".to_string(),
            agent_program: "codex-acp".to_string(),
            agent_args: Vec::new(),
            agent_shell_command: None,
            agent_launch_mode: AcpAgentLaunchMode::Direct,
            mcp_servers: Vec::new(),
            last_updated_at_ms: 10,
        }];

        let merged = merge_restored_session_snapshots(current, &initial_snapshot_ids, retained);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].session_id, "session-1");
        assert_eq!(merged[0].title, "runtime-title");
    }

    #[test]
    fn apply_session_snapshot_update_only_does_not_recreate_removed_session() {
        let mut saved_sessions = Vec::new();
        let changed = apply_session_snapshot(
            &mut saved_sessions,
            SavedAcpSession {
                session_id: "session-1".to_string(),
                workspace_root: "/tmp/workspace".to_string(),
                title: "runtime-title".to_string(),
                agent_id: Some("agent-1".to_string()),
                agent_name: "Codex".to_string(),
                agent_program: "codex-acp".to_string(),
                agent_args: Vec::new(),
                agent_shell_command: None,
                agent_launch_mode: AcpAgentLaunchMode::Direct,
                mcp_servers: Vec::new(),
                last_updated_at_ms: 10,
            },
            SnapshotWriteMode::UpdateOnly,
        );

        assert!(!changed);
        assert!(saved_sessions.is_empty());
    }

    #[test]
    fn apply_session_snapshot_insert_or_update_replaces_existing_session() {
        let mut saved_sessions = vec![SavedAcpSession {
            session_id: "session-1".to_string(),
            workspace_root: "/tmp/workspace".to_string(),
            title: "old-title".to_string(),
            agent_id: Some("agent-1".to_string()),
            agent_name: "Codex".to_string(),
            agent_program: "codex-acp".to_string(),
            agent_args: Vec::new(),
            agent_shell_command: None,
            agent_launch_mode: AcpAgentLaunchMode::Direct,
            mcp_servers: Vec::new(),
            last_updated_at_ms: 1,
        }];
        let changed = apply_session_snapshot(
            &mut saved_sessions,
            SavedAcpSession {
                session_id: "session-1".to_string(),
                workspace_root: "/tmp/workspace".to_string(),
                title: "new-title".to_string(),
                agent_id: Some("agent-1".to_string()),
                agent_name: "Codex".to_string(),
                agent_program: "codex-acp".to_string(),
                agent_args: Vec::new(),
                agent_shell_command: None,
                agent_launch_mode: AcpAgentLaunchMode::Direct,
                mcp_servers: Vec::new(),
                last_updated_at_ms: 10,
            },
            SnapshotWriteMode::InsertOrUpdate,
        );

        assert!(changed);
        assert_eq!(saved_sessions.len(), 1);
        assert_eq!(saved_sessions[0].title, "new-title");
        assert_eq!(saved_sessions[0].last_updated_at_ms, 10);
    }

    #[test]
    fn apply_session_snapshot_ignores_stale_update_for_existing_session() {
        let mut saved_sessions = vec![SavedAcpSession {
            session_id: "session-1".to_string(),
            workspace_root: "/tmp/workspace".to_string(),
            title: "new-title".to_string(),
            agent_id: Some("agent-1".to_string()),
            agent_name: "Codex".to_string(),
            agent_program: "codex-acp".to_string(),
            agent_args: Vec::new(),
            agent_shell_command: None,
            agent_launch_mode: AcpAgentLaunchMode::Direct,
            mcp_servers: Vec::new(),
            last_updated_at_ms: 10,
        }];
        let changed = apply_session_snapshot(
            &mut saved_sessions,
            SavedAcpSession {
                session_id: "session-1".to_string(),
                workspace_root: "/tmp/workspace".to_string(),
                title: "stale-title".to_string(),
                agent_id: Some("agent-1".to_string()),
                agent_name: "Codex".to_string(),
                agent_program: "codex-acp".to_string(),
                agent_args: Vec::new(),
                agent_shell_command: None,
                agent_launch_mode: AcpAgentLaunchMode::Direct,
                mcp_servers: Vec::new(),
                last_updated_at_ms: 9,
            },
            SnapshotWriteMode::UpdateOnly,
        );

        assert!(!changed);
        assert_eq!(saved_sessions.len(), 1);
        assert_eq!(saved_sessions[0].title, "new-title");
        assert_eq!(saved_sessions[0].last_updated_at_ms, 10);
    }

    #[test]
    fn apply_session_snapshot_preserves_existing_session_on_equal_timestamp() {
        let mut saved_sessions = vec![SavedAcpSession {
            session_id: "session-1".to_string(),
            workspace_root: "/tmp/workspace".to_string(),
            title: "runtime-title".to_string(),
            agent_id: Some("agent-1".to_string()),
            agent_name: "Codex".to_string(),
            agent_program: "codex-acp".to_string(),
            agent_args: Vec::new(),
            agent_shell_command: None,
            agent_launch_mode: AcpAgentLaunchMode::Direct,
            mcp_servers: Vec::new(),
            last_updated_at_ms: 10,
        }];
        let changed = apply_session_snapshot(
            &mut saved_sessions,
            SavedAcpSession {
                session_id: "session-1".to_string(),
                workspace_root: "/tmp/workspace".to_string(),
                title: "late-title".to_string(),
                agent_id: Some("agent-1".to_string()),
                agent_name: "Codex".to_string(),
                agent_program: "codex-acp".to_string(),
                agent_args: Vec::new(),
                agent_shell_command: None,
                agent_launch_mode: AcpAgentLaunchMode::Direct,
                mcp_servers: Vec::new(),
                last_updated_at_ms: 10,
            },
            SnapshotWriteMode::UpdateOnly,
        );

        assert!(!changed);
        assert_eq!(saved_sessions.len(), 1);
        assert_eq!(saved_sessions[0].title, "runtime-title");
        assert_eq!(saved_sessions[0].last_updated_at_ms, 10);
    }
}
