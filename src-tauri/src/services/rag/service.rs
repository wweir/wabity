use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::{Context, Result};
use notify::{event::ModifyKind, Event, EventKind, RecursiveMode, Watcher};
use tauri::AppHandle;
use tokio::{
    sync::{mpsc, Mutex as AsyncMutex, RwLock as AsyncRwLock},
    task::JoinHandle,
};

use crate::domain::{
    rag::{RagRuntimePhase, RagRuntimeStatus, RagScanResult},
    settings::{LlmSettings, RagSettings},
};

use super::{
    config::{
        classify_rag_runtime_start, rag_database_path, rag_metadata_database_path,
        rag_settings_disabled, resolve_rag_config, PathStartsWithAny,
    },
    indexing::{
        build_path_update_plan, execute_path_update_plans, initialize_runtime_storage,
        rebuild_index_locked,
    },
    model::{RagRuntimeContext, RagRuntimeInputs, RagRuntimeStartMode, WATCH_DEBOUNCE_WINDOW},
    status::{set_runtime_status, set_runtime_status_for_generation},
    storage::{clear_index, clear_metadata_store, load_metadata_records_for_paths},
};

#[derive(Clone)]
pub struct RagIndexService {
    app_handle: Option<AppHandle>,
    data_dir: PathBuf,
    pub(super) runtime: Arc<AsyncRwLock<Option<JoinHandle<()>>>>,
    runtime_inputs: Arc<AsyncRwLock<Option<RagRuntimeInputs>>>,
    runtime_status: Arc<AsyncRwLock<RagRuntimeStatus>>,
    pub(super) runtime_generation: Arc<AtomicU64>,
    storage_lock: Arc<AsyncMutex<()>>,
}

impl RagIndexService {
    pub fn new(data_dir: PathBuf) -> Self {
        Self::new_with_app_handle(None, data_dir)
    }

    pub fn new_with_app_handle(app_handle: Option<AppHandle>, data_dir: PathBuf) -> Self {
        Self {
            app_handle,
            data_dir,
            runtime: Arc::new(AsyncRwLock::new(None)),
            runtime_inputs: Arc::new(AsyncRwLock::new(None)),
            runtime_status: Arc::new(AsyncRwLock::new(RagRuntimeStatus::default())),
            runtime_generation: Arc::new(AtomicU64::new(0)),
            storage_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    pub async fn apply_settings(&self, settings: RagSettings, llm_settings: LlmSettings) {
        let next_inputs = RagRuntimeInputs::from_settings(&settings, &llm_settings);
        let previous_inputs = self.runtime_inputs.read().await.clone();
        if previous_inputs.as_ref() == Some(&next_inputs) {
            let runtime_is_active = self
                .runtime
                .read()
                .await
                .as_ref()
                .is_some_and(|handle| !handle.is_finished());
            if runtime_is_active {
                return;
            }

            let runtime_in_error = self.runtime_status.read().await.phase == RagRuntimePhase::Error;
            if !runtime_in_error {
                return;
            }
        }

        let mut runtime = self.runtime.write().await;
        if let Some(handle) = runtime.take() {
            if !handle.is_finished() {
                handle.abort();
            }
        }
        *self.runtime_inputs.write().await = Some(next_inputs.clone());

        let data_dir = self.data_dir.clone();
        let runtime_status = self.runtime_status.clone();
        let runtime_generation = self.runtime_generation.clone();
        let generation = runtime_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let status_app_handle = self.app_handle.clone();
        let runtime_context = RagRuntimeContext {
            app_handle: self.app_handle.clone(),
            runtime_status: runtime_status.clone(),
            runtime_generation: runtime_generation.clone(),
            storage_lock: self.storage_lock.clone(),
            generation,
        };
        let start_mode = classify_rag_runtime_start(previous_inputs.as_ref(), &next_inputs);
        *runtime = Some(tokio::spawn(async move {
            if let Err(error) = run_watch_loop(
                data_dir,
                settings,
                llm_settings,
                runtime_context,
                start_mode,
            )
            .await
            {
                set_runtime_status_for_generation(
                    status_app_handle.as_ref(),
                    &runtime_status,
                    &runtime_generation,
                    generation,
                    RagRuntimePhase::Error,
                    Default::default(),
                    Some(error.to_string()),
                )
                .await;
                tracing::error!(?error, "RAG watcher loop exited unexpectedly");
            }
        }));
    }

    pub async fn runtime_status(&self) -> RagRuntimeStatus {
        self.runtime_status.read().await.clone()
    }

    pub async fn scan_sources(
        &self,
        settings: &RagSettings,
        llm_settings: &LlmSettings,
    ) -> Result<RagScanResult> {
        let resolved = resolve_rag_config(settings, llm_settings)?;
        let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
        let _storage_guard = self.storage_lock.lock().await;
        rebuild_index_locked(
            self.app_handle.as_ref(),
            &self.data_dir,
            &resolved,
            &runtime_status,
            None,
        )
        .await
    }
}

pub(super) async fn run_watch_loop(
    data_dir: PathBuf,
    settings: RagSettings,
    llm_settings: LlmSettings,
    runtime_context: RagRuntimeContext,
    start_mode: RagRuntimeStartMode,
) -> Result<()> {
    let database_path = rag_database_path(&data_dir);
    let metadata_path = rag_metadata_database_path(&data_dir);
    let resolved = match resolve_rag_config(&settings, &llm_settings) {
        Ok(resolved) => resolved,
        Err(error) => {
            if rag_settings_disabled(&settings) {
                clear_index(&database_path).await?;
                clear_metadata_store(&metadata_path).await?;
                set_runtime_status_for_generation(
                    runtime_context.app_handle.as_ref(),
                    &runtime_context.runtime_status,
                    &runtime_context.runtime_generation,
                    runtime_context.generation,
                    RagRuntimePhase::Idle,
                    Default::default(),
                    None,
                )
                .await;
                return Ok(());
            }
            set_runtime_status_for_generation(
                runtime_context.app_handle.as_ref(),
                &runtime_context.runtime_status,
                &runtime_context.runtime_generation,
                runtime_context.generation,
                RagRuntimePhase::Error,
                Default::default(),
                Some(error.to_string()),
            )
            .await;
            return Err(error);
        }
    };

    initialize_runtime_storage(
        &data_dir,
        &metadata_path,
        &resolved,
        runtime_context.app_handle.as_ref(),
        &runtime_context.runtime_status,
        Some((
            &runtime_context.runtime_generation,
            runtime_context.generation,
        )),
        &runtime_context.storage_lock,
        start_mode,
    )
    .await?;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(move |result| {
        let _ = event_tx.send(result);
    })
    .context("failed to create RAG file watcher")?;

    for source_root in &resolved.source_roots {
        watcher
            .watch(source_root, RecursiveMode::Recursive)
            .with_context(|| {
                format!(
                    "failed to watch RAG source directory: {}",
                    source_root.display()
                )
            })?;
    }

    loop {
        let Some(first_event) = event_rx.recv().await else {
            return Ok(());
        };

        let mut events = vec![first_event];
        while let Ok(next_event) =
            tokio::time::timeout(WATCH_DEBOUNCE_WINDOW, event_rx.recv()).await
        {
            let Some(next_event) = next_event else {
                break;
            };
            events.push(next_event);
        }

        if let Err(error) = process_event_batch(
            runtime_context.app_handle.as_ref(),
            &data_dir,
            &resolved,
            &runtime_context.runtime_status,
            Some((
                &runtime_context.runtime_generation,
                runtime_context.generation,
            )),
            &runtime_context.storage_lock,
            events,
        )
        .await
        {
            set_runtime_status_for_generation(
                runtime_context.app_handle.as_ref(),
                &runtime_context.runtime_status,
                &runtime_context.runtime_generation,
                runtime_context.generation,
                RagRuntimePhase::Error,
                Default::default(),
                Some(error.to_string()),
            )
            .await;
            tracing::warn!(?error, "failed to process RAG watcher events");
        }
    }
}

async fn process_event_batch(
    app_handle: Option<&AppHandle>,
    data_dir: &Path,
    resolved: &super::model::ResolvedRagConfig,
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_guard: Option<(&Arc<AtomicU64>, u64)>,
    storage_lock: &Arc<AsyncMutex<()>>,
    events: Vec<notify::Result<Event>>,
) -> Result<()> {
    let _storage_guard = storage_lock.lock().await;
    let database_path = rag_database_path(data_dir);
    let metadata_path = rag_metadata_database_path(data_dir);
    let mut full_rescan = false;
    let mut changed_paths = BTreeSet::new();

    for event in events {
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(?error, "RAG watcher reported an invalid event");
                continue;
            }
        };

        if should_ignore_event_for_indexing(&event) {
            continue;
        }

        if should_force_full_rescan_for_event(&event) {
            full_rescan = true;
        }

        for path in event.paths {
            if !path.starts_with_any(&resolved.source_roots) {
                continue;
            }

            if path.exists() && path.is_dir() {
                if should_force_full_rescan_for_existing_directory(event.kind) {
                    full_rescan = true;
                }
                continue;
            }

            changed_paths.insert(path);
        }
    }

    if full_rescan {
        rebuild_index_locked(
            app_handle,
            data_dir,
            resolved,
            runtime_status,
            runtime_guard,
        )
        .await?;
        return Ok(());
    }

    if !changed_paths.is_empty() {
        set_runtime_status(
            app_handle,
            runtime_status,
            runtime_guard,
            RagRuntimePhase::Scanning,
            super::model::RuntimeProgress {
                scanned_file_count: changed_paths.len(),
                total_file_count: changed_paths.len(),
                ..Default::default()
            },
            None,
        )
        .await;
    }

    if changed_paths.is_empty() {
        set_runtime_status(
            app_handle,
            runtime_status,
            runtime_guard,
            RagRuntimePhase::Idle,
            Default::default(),
            None,
        )
        .await;
        return Ok(());
    }

    let changed_paths = changed_paths.into_iter().collect::<Vec<_>>();
    let stored_records = tokio::task::spawn_blocking({
        let metadata_path = metadata_path.clone();
        let absolute_paths = changed_paths
            .iter()
            .map(|path| super::config::normalize_path_string(path))
            .collect::<Vec<_>>();
        move || load_metadata_records_for_paths(&metadata_path, &absolute_paths)
    })
    .await
    .context("failed to join RAG metadata batch lookup task")??;
    let resolved_for_batch = resolved.clone();
    let plans = tokio::task::spawn_blocking(move || {
        changed_paths
            .into_iter()
            .map(|path| {
                let normalized_path = super::config::normalize_path_string(&path);
                let stored_record = stored_records.get(&normalized_path).cloned();
                build_path_update_plan(&resolved_for_batch, &path, stored_record)
                    .map(|plan| (path, plan))
            })
            .collect::<Result<Vec<_>>>()
    })
    .await
    .context("failed to join RAG path batch planning task")??;

    execute_path_update_plans(
        app_handle,
        &database_path,
        &metadata_path,
        resolved,
        runtime_status,
        runtime_guard,
        plans,
    )
    .await?;

    set_runtime_status(
        app_handle,
        runtime_status,
        runtime_guard,
        RagRuntimePhase::Idle,
        Default::default(),
        None,
    )
    .await;
    Ok(())
}

pub(super) fn should_force_full_rescan_for_event(event: &Event) -> bool {
    matches!(event.kind, EventKind::Any)
        || (matches!(event.kind, EventKind::Other) && event.paths.is_empty())
}

pub(super) fn should_ignore_event_for_indexing(event: &Event) -> bool {
    matches!(event.kind, EventKind::Access(_))
        || matches!(event.kind, EventKind::Modify(ModifyKind::Metadata(_)))
}

pub(super) fn should_force_full_rescan_for_existing_directory(kind: EventKind) -> bool {
    matches!(kind, EventKind::Modify(ModifyKind::Name(_)))
}
