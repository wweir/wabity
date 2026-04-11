use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
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
use crate::services::document_extract::is_supported_document_file;

use super::{
    config::{
        classify_rag_runtime_start, rag_database_path, rag_settings_disabled,
        rag_sqlite_database_path, resolve_rag_config, PathStartsWithAny,
    },
    indexing::{
        build_path_update_plan, build_skip_marker_record, choose_scanned_file_path,
        execute_path_update_plans, extend_recent_rag_warnings, find_stored_record_by_path_alias,
        format_error_chain, format_rag_warning_for_path, initialize_runtime_storage,
        rebuild_index_locked, scanned_file_lookup_keys, PathUpdateRuntimeContext,
    },
    model::{RagRuntimeContext, RagRuntimeInputs, RagRuntimeStartMode, WATCH_DEBOUNCE_WINDOW},
    status::{set_runtime_status, set_runtime_status_for_generation, RuntimeStatusUpdate},
    storage::{clear_index, clear_sqlite_store, load_rag_file_records_for_paths},
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
            let runtime_in_error = self.runtime_status.read().await.phase == RagRuntimePhase::Error;
            let runtime_is_active = self
                .runtime
                .read()
                .await
                .as_ref()
                .is_some_and(|handle| !handle.is_finished());
            if runtime_is_active && !runtime_in_error {
                return;
            }

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
                    RuntimeStatusUpdate {
                        last_error: Some(error.to_string()),
                        ..RuntimeStatusUpdate::default()
                    },
                )
                .await;
                tracing::error!(
                    error = format_args!("{:#}", error),
                    "RAG watcher loop exited unexpectedly"
                );
            }
        }));
    }

    pub async fn runtime_status(&self) -> RagRuntimeStatus {
        self.runtime_status.read().await.clone()
    }

    #[cfg(test)]
    pub(super) async fn seed_runtime_state_for_test(
        &self,
        inputs: RagRuntimeInputs,
        phase: RagRuntimePhase,
        handle: JoinHandle<()>,
    ) {
        *self.runtime_inputs.write().await = Some(inputs);
        *self.runtime_status.write().await = RagRuntimeStatus {
            phase,
            ..RagRuntimeStatus::default()
        };
        *self.runtime.write().await = Some(handle);
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
    let sqlite_path = rag_sqlite_database_path(&data_dir);
    let resolved = match resolve_rag_config(&settings, &llm_settings) {
        Ok(resolved) => resolved,
        Err(error) => {
            if rag_settings_disabled(&settings) {
                clear_index(&database_path).await?;
                clear_sqlite_store(&sqlite_path).await?;
                set_runtime_status_for_generation(
                    runtime_context.app_handle.as_ref(),
                    &runtime_context.runtime_status,
                    &runtime_context.runtime_generation,
                    runtime_context.generation,
                    RagRuntimePhase::Idle,
                    Default::default(),
                    RuntimeStatusUpdate::default(),
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
                RuntimeStatusUpdate {
                    last_error: Some(error.to_string()),
                    ..RuntimeStatusUpdate::default()
                },
            )
            .await;
            return Err(error);
        }
    };

    initialize_runtime_storage(
        &data_dir,
        &sqlite_path,
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
                RuntimeStatusUpdate {
                    last_error: Some(error.to_string()),
                    ..RuntimeStatusUpdate::default()
                },
            )
            .await;
            tracing::warn!(
                error = format_args!("{:#}", error),
                "failed to process RAG watcher events"
            );
        }
    }
}

pub(super) async fn process_event_batch(
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
    let sqlite_path = rag_sqlite_database_path(data_dir);
    let mut full_rescan = false;
    let mut changed_paths = BTreeMap::new();

    for event in events {
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(
                    error = format_args!("{:#}", error),
                    "RAG watcher reported an invalid event"
                );
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
            let stable_path = normalize_event_path_for_planning(&path);
            if !stable_path.starts_with_any(&resolved.source_roots) {
                continue;
            }

            if stable_path.exists() && stable_path.is_dir() {
                if should_force_full_rescan_for_existing_directory(event.kind) {
                    full_rescan = true;
                } else if matches!(
                    event.kind,
                    EventKind::Create(notify::event::CreateKind::Folder)
                ) {
                    for indexed_path in collect_indexable_paths_in_directory(&stable_path) {
                        changed_paths
                            .entry(indexed_path.clone())
                            .or_insert(indexed_path);
                    }
                }
                continue;
            }

            changed_paths.entry(stable_path).or_insert(path);
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
            RuntimeStatusUpdate::default(),
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
            RuntimeStatusUpdate::default(),
        )
        .await;
        return Ok(());
    }

    let changed_paths = changed_paths.into_iter().collect::<Vec<_>>();
    let changed_path_inputs = changed_paths
        .iter()
        .map(|(stable_path, original_path)| {
            let canonical_path = stable_path
                .canonicalize()
                .unwrap_or_else(|_| stable_path.clone());
            let scan_path =
                choose_scanned_file_path(resolved, original_path.as_path(), &canonical_path);
            let lookup_keys =
                scanned_file_lookup_keys(original_path.as_path(), &canonical_path, &scan_path);
            (stable_path.clone(), scan_path, lookup_keys)
        })
        .collect::<Vec<_>>();
    let stored_records = tokio::task::spawn_blocking({
        let sqlite_path = sqlite_path.clone();
        let absolute_paths = changed_path_inputs
            .iter()
            .flat_map(|(_, _, lookup_keys)| lookup_keys.iter().cloned())
            .collect::<Vec<_>>();
        move || load_rag_file_records_for_paths(&sqlite_path, &absolute_paths)
    })
    .await
    .context("failed to join RAG file records batch lookup task")??;
    let resolved_for_batch = resolved.clone();
    let (plans, planning_update) = tokio::task::spawn_blocking(move || {
        let mut plans = Vec::new();
        let mut planning_update = RuntimeStatusUpdate::default();
        for (_, scan_path, lookup_keys) in changed_path_inputs {
            let stored_record = find_stored_record_by_path_alias(&stored_records, &lookup_keys)
                .map(|(record, _)| record.clone());
            match build_path_update_plan(&resolved_for_batch, &scan_path, stored_record.clone()) {
                Ok(plan @ super::indexing::PathUpdatePlan::Delete { .. }) => {
                    let plan_path = stored_record
                        .as_ref()
                        .map(|record| PathBuf::from(&record.absolute_path))
                        .unwrap_or_else(|| scan_path.clone());
                    plans.push((plan_path, plan));
                }
                Ok(plan) => plans.push((scan_path, plan)),
                Err(error) => {
                    tracing::warn!(
                        error = format_args!("{:#}", error),
                        path = %scan_path.display(),
                        "failed to build RAG watcher update plan"
                    );
                    planning_update.warning_count = planning_update.warning_count.saturating_add(1);
                    extend_recent_rag_warnings(
                        &mut planning_update.recent_warnings,
                        [format_rag_warning_for_path(
                            &scan_path,
                            &format_error_chain(&error),
                        )],
                    );
                    let has_active_chunks = stored_record
                        .as_ref()
                        .and_then(|r| r.active.as_ref())
                        .map(|v| v.chunk_count > 0)
                        .unwrap_or(false);
                    if !has_active_chunks {
                        if let Some(marker) =
                            build_skip_marker_record(&resolved_for_batch, &scan_path)
                        {
                            plans.push((
                                scan_path,
                                super::indexing::PathUpdatePlan::RefreshRagFileRecord {
                                    record: marker,
                                    clear_staged: false,
                                    refresh_projection: false,
                                },
                            ));
                        }
                    }
                }
            }
        }
        (plans, planning_update)
    })
    .await
    .context("failed to join RAG path batch planning task")?;

    let status_update = execute_path_update_plans(
        PathUpdateRuntimeContext {
            app_handle,
            runtime_status,
            runtime_guard,
        },
        &database_path,
        &sqlite_path,
        resolved,
        plans,
        planning_update,
    )
    .await?;

    set_runtime_status(
        app_handle,
        runtime_status,
        runtime_guard,
        RagRuntimePhase::Idle,
        Default::default(),
        status_update,
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

pub(super) fn normalize_event_path_for_planning(path: &Path) -> PathBuf {
    if let Ok(canonical_path) = path.canonicalize() {
        return canonical_path;
    }

    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name()) {
        if let Ok(canonical_parent) = parent.canonicalize() {
            return canonical_parent.join(file_name);
        }
    }

    lexical_normalize_path(path)
}

fn lexical_normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

pub(super) fn collect_indexable_paths_in_directory(path: &Path) -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::new();
    let mut walker = WalkBuilder::new(path);
    walker
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false);

    for entry in walker.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::warn!(
                    error = format_args!("{:#}", error),
                    path = %path.display(),
                    "failed to walk created RAG directory"
                );
                continue;
            }
        };
        let entry_path = entry.path();
        if entry_path == path
            || !entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
            || !is_supported_document_file(entry_path)
        {
            continue;
        }
        paths.insert(entry_path.to_path_buf());
    }

    paths
}
