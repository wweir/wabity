use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{atomic::AtomicU64, Arc},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use reqwest::Client as HttpClient;
use tauri::AppHandle;
use tokio::{
    sync::{mpsc, Mutex as AsyncMutex, RwLock as AsyncRwLock, Semaphore},
    task::JoinSet,
};

use crate::{
    domain::rag::{RagRuntimePhase, RagRuntimeStatus, RagScanResult},
    services::document_extract::{
        classify_document_kind, extract_document_from_bytes, extractor_fingerprint_for_path,
        is_supported_document_file, DocumentKind,
    },
};

use super::{
    chunking::split_extracted_document_for_path,
    config::{
        normalize_path_string, now_unix_ms, rag_database_path, rag_sqlite_database_path,
        resolve_source_root_for_path, should_skip_path, system_time_to_unix_ms,
    },
    embedding::{
        build_embedding_client, request_embeddings_with_stats, text_fingerprint,
        EmbeddingBatchPlanner,
    },
    model::{
        PreparedRagChunk, PreparedRagFile, RagChunk, RagChunkState, RagIndexedFileRecord,
        RagIndexedFileVersion, RagRuntimeStartMode, ResolvedRagConfig, RuntimeProgress,
        CHUNK_MAX_CHARS, CHUNK_OVERLAP_CHARS, MAX_STREAMING_REINDEX_CONCURRENCY,
        MAX_TEXT_FILE_BYTES_DOCX, MAX_TEXT_FILE_BYTES_MARKDOWN, MAX_TEXT_FILE_BYTES_PDF,
        MAX_TEXT_FILE_BYTES_PLAIN_TEXT,
    },
    status::{set_runtime_status, RuntimeStatusUpdate},
    storage::{
        delete_rag_file_records_for_paths, delete_vectors_for_exact_paths,
        delete_vectors_for_exact_paths_in_state, delete_vectors_for_prefix_paths,
        delete_vectors_with_filter, escape_sql_literal,
        finalize_rag_file_record_and_replace_lexical_chunks, load_rag_file_records,
        prepare_index_storage, refresh_projection_for_rag_file_records, upsert_rag_file_records,
        RagVectorStore,
    },
};

#[derive(Clone, Copy)]
pub(super) struct PathUpdateRuntimeContext<'a> {
    pub(super) app_handle: Option<&'a AppHandle>,
    pub(super) runtime_status: &'a Arc<AsyncRwLock<RagRuntimeStatus>>,
    pub(super) runtime_guard: Option<(&'a Arc<AtomicU64>, u64)>,
}

const MAX_REPORTED_RAG_WARNINGS: usize = 5;

#[derive(Debug, Default)]
struct RebuildPlan {
    scanned_file_count: usize,
    indexed_file_count: usize,
    skipped_file_count: usize,
    chunk_count: usize,
    warning_count: usize,
    recent_warnings: Vec<String>,
    staged_cleanup_paths: BTreeSet<String>,
    stale_paths: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub(super) struct RebuildScanEvent {
    pub(super) scanned_file_count: usize,
    pub(super) indexed_file_count: usize,
    pub(super) skipped_file_count: usize,
    pub(super) chunk_count: usize,
    pub(super) warning_count: usize,
    pub(super) recent_warnings: Vec<String>,
    pub(super) staged_cleanup_paths: Vec<String>,
    pub(super) stale_paths: Vec<String>,
    pub(super) rag_file_record_refresh: Option<RagIndexedFileRecord>,
    pub(super) projection_refresh: Option<RagIndexedFileRecord>,
    pub(super) file_to_index: Option<PreparedRagFile>,
}

#[derive(Debug)]
struct IndexedPreparedFile {
    file: PreparedRagFile,
    chunks: Vec<RagChunk>,
    vectors: Vec<Vec<f32>>,
}

#[derive(Debug, Clone, Copy, Default)]
struct RagFileObservation {
    bytes_read: usize,
    extracted_text_bytes: usize,
    prepared_chunk_count: usize,
    prepared_chunk_text_bytes: usize,
    vector_count: usize,
    vector_bytes_estimate: usize,
}

pub(super) fn push_recent_rag_warning(recent_warnings: &mut Vec<String>, warning: String) {
    if warning.trim().is_empty() {
        return;
    }
    if recent_warnings.len() == MAX_REPORTED_RAG_WARNINGS {
        recent_warnings.remove(0);
    }
    recent_warnings.push(warning);
}

pub(super) fn extend_recent_rag_warnings(
    recent_warnings: &mut Vec<String>,
    warnings: impl IntoIterator<Item = String>,
) {
    for warning in warnings {
        push_recent_rag_warning(recent_warnings, warning);
    }
}

pub(super) fn format_rag_warning_for_path(path: &Path, warning: &str) -> String {
    format!("{}: {warning}", path.display())
}

pub(super) fn format_error_chain(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" -> ")
}

#[derive(Debug)]
pub(super) enum InspectPathOutcome {
    Skip,
    Unchanged {
        record: RagIndexedFileRecord,
        refresh_rag_file_record: bool,
        clear_staged: bool,
        refresh_projection: bool,
    },
    Reindex(PreparedRagFile),
}

#[derive(Debug)]
pub(super) enum PathUpdatePlan {
    Noop,
    Delete {
        delete_descendants: bool,
    },
    RefreshRagFileRecord {
        record: RagIndexedFileRecord,
        clear_staged: bool,
        refresh_projection: bool,
    },
    Reindex(PreparedRagFile),
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn initialize_runtime_storage(
    data_dir: &Path,
    _sqlite_path: &Path,
    resolved: &ResolvedRagConfig,
    app_handle: Option<&AppHandle>,
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_guard: Option<(&Arc<AtomicU64>, u64)>,
    storage_lock: &Arc<AsyncMutex<()>>,
    start_mode: RagRuntimeStartMode,
) -> Result<()> {
    let _storage_guard = storage_lock.lock().await;
    match start_mode {
        RagRuntimeStartMode::ReuseIndex => {
            tracing::info!(
                "running incremental RAG startup scan to reconcile offline file changes"
            );
            reconcile_runtime_storage_locked(
                app_handle,
                data_dir,
                resolved,
                runtime_status,
                runtime_guard,
            )
            .await?;
        }
        RagRuntimeStartMode::RebuildIndex => {
            rebuild_index_locked(
                app_handle,
                data_dir,
                resolved,
                runtime_status,
                runtime_guard,
            )
            .await?;
        }
    }
    Ok(())
}

async fn reconcile_runtime_storage_locked(
    app_handle: Option<&AppHandle>,
    data_dir: &Path,
    resolved: &ResolvedRagConfig,
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_guard: Option<(&Arc<AtomicU64>, u64)>,
) -> Result<()> {
    set_runtime_status(
        app_handle,
        runtime_status,
        runtime_guard,
        RagRuntimePhase::Scanning,
        RuntimeProgress::default(),
        RuntimeStatusUpdate::default(),
    )
    .await;
    tokio::fs::create_dir_all(data_dir).await.with_context(|| {
        format!(
            "failed to create RAG data directory: {}",
            data_dir.display()
        )
    })?;

    let database_path = rag_database_path(data_dir);
    let sqlite_path = rag_sqlite_database_path(data_dir);
    tokio::fs::create_dir_all(&database_path)
        .await
        .with_context(|| {
            format!(
                "failed to create RAG database directory: {}",
                database_path.display()
            )
        })?;

    prepare_index_storage(&database_path, &sqlite_path, resolved, true).await?;
    let stored_records = tokio::task::spawn_blocking({
        let sqlite_path = sqlite_path.clone();
        move || load_rag_file_records(&sqlite_path)
    })
    .await
    .context("failed to join RAG sqlite load task")??;
    let resolved_for_planning = resolved.clone();
    let (plans, planning_update) = tokio::task::spawn_blocking(move || {
        plan_startup_reconciliation(&resolved_for_planning, &stored_records)
    })
    .await
    .context("failed to join RAG startup reconciliation planning task")??;
    let plans_is_empty = plans.is_empty();
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

    if plans_is_empty {
        let mut vector_store = RagVectorStore::open(&database_path).await?;
        vector_store.ensure_index().await?;
    }

    set_runtime_status(
        app_handle,
        runtime_status,
        runtime_guard,
        RagRuntimePhase::Idle,
        RuntimeProgress::default(),
        status_update,
    )
    .await;
    Ok(())
}

pub(super) async fn rebuild_index_locked(
    app_handle: Option<&AppHandle>,
    data_dir: &Path,
    resolved: &ResolvedRagConfig,
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_guard: Option<(&Arc<AtomicU64>, u64)>,
) -> Result<RagScanResult> {
    let rebuild_started_at = Instant::now();
    set_runtime_status(
        app_handle,
        runtime_status,
        runtime_guard,
        RagRuntimePhase::Scanning,
        RuntimeProgress::default(),
        RuntimeStatusUpdate::default(),
    )
    .await;
    tokio::fs::create_dir_all(data_dir).await.with_context(|| {
        format!(
            "failed to create RAG data directory: {}",
            data_dir.display()
        )
    })?;

    let database_path = rag_database_path(data_dir);
    let sqlite_path = rag_sqlite_database_path(data_dir);
    tokio::fs::create_dir_all(&database_path)
        .await
        .with_context(|| {
            format!(
                "failed to create RAG database directory: {}",
                database_path.display()
            )
        })?;

    prepare_index_storage(&database_path, &sqlite_path, resolved, true).await?;
    let stored_records = tokio::task::spawn_blocking({
        let sqlite_path = sqlite_path.clone();
        move || load_rag_file_records(&sqlite_path)
    })
    .await
    .context("failed to join RAG sqlite load task")??;
    let client = build_embedding_client()?;
    let mut vector_store = RagVectorStore::open(&database_path).await?;
    let (scan_tx, mut scan_rx) =
        mpsc::channel::<RebuildScanEvent>(streaming_reindex_concurrency().saturating_mul(2));
    let resolved_for_scan = resolved.clone();
    let scan_handle = tokio::task::spawn_blocking(move || {
        stream_rebuild_scan(&resolved_for_scan, &stored_records, scan_tx)
    });
    let semaphore = Arc::new(Semaphore::new(streaming_reindex_concurrency()));
    let mut join_set = JoinSet::new();
    let mut pending_files = VecDeque::new();
    let mut plan = RebuildPlan::default();
    let mut completed_file_count = 0usize;
    let mut pending_file_count = 0usize;
    let mut scan_completed = false;

    loop {
        spawn_streaming_reindex_tasks(
            &mut join_set,
            &mut pending_files,
            &semaphore,
            &sqlite_path,
            resolved,
            &client,
            &vector_store,
        )
        .await?;

        if scan_completed && pending_files.is_empty() && join_set.is_empty() {
            break;
        }

        tokio::select! {
            maybe_event = scan_rx.recv(), if !scan_completed => {
                let Some(event) = maybe_event else {
                    scan_completed = true;
                    continue;
                };

                plan.scanned_file_count = plan
                    .scanned_file_count
                    .saturating_add(event.scanned_file_count);
                plan.indexed_file_count = plan
                    .indexed_file_count
                    .saturating_add(event.indexed_file_count);
                plan.skipped_file_count = plan
                    .skipped_file_count
                    .saturating_add(event.skipped_file_count);
                plan.chunk_count = plan.chunk_count.saturating_add(event.chunk_count);
                plan.warning_count = plan.warning_count.saturating_add(event.warning_count);
                extend_recent_rag_warnings(
                    &mut plan.recent_warnings,
                    event.recent_warnings,
                );
                plan.staged_cleanup_paths.extend(event.staged_cleanup_paths);
                plan.stale_paths.extend(event.stale_paths);

                if let Some(record) = event.rag_file_record_refresh {
                    write_rag_file_records(
                        &sqlite_path,
                        vec![record.clone()],
                        "failed to join streamed RAG file record refresh task",
                    )
                    .await?;
                }
                if let Some(record) = event.projection_refresh {
                    refresh_projection_records(&database_path, &sqlite_path, vec![record]).await?;
                }

                if let Some(file) = event.file_to_index {
                    pending_file_count = pending_file_count.saturating_add(1);
                    pending_files.push_back(file);
                    tracing::info!(
                        pending_files_len = pending_files.len(),
                        pending_file_count,
                        "rag pending file queue updated"
                    );
                } else {
                    completed_file_count =
                        completed_file_count.saturating_add(event.indexed_file_count);
                }

                set_rebuild_runtime_status(
                    app_handle,
                    runtime_status,
                    runtime_guard,
                    plan.scanned_file_count,
                    completed_file_count,
                    pending_file_count,
                    RuntimeStatusUpdate {
                        warning_count: plan.warning_count,
                        recent_warnings: plan.recent_warnings.clone(),
                        ..RuntimeStatusUpdate::default()
                    },
                )
                .await;
            }
            maybe_indexed = join_set.join_next(), if !join_set.is_empty() => {
                let indexed = maybe_indexed
                    .context("streaming RAG reindex task queue ended unexpectedly")?
                    .context("failed to join streaming RAG reindex task")??;
                persist_indexed_file(&mut vector_store, &sqlite_path, indexed).await?;
                pending_file_count = pending_file_count.saturating_sub(1);
                completed_file_count = completed_file_count.saturating_add(1);
                set_rebuild_runtime_status(
                    app_handle,
                    runtime_status,
                    runtime_guard,
                    plan.scanned_file_count,
                    completed_file_count,
                    pending_file_count,
                    RuntimeStatusUpdate {
                        warning_count: plan.warning_count,
                        recent_warnings: plan.recent_warnings.clone(),
                        ..RuntimeStatusUpdate::default()
                    },
                )
                .await;
            }
        }
    }

    scan_handle
        .await
        .context("failed to join RAG scan task")??;

    if !plan.staged_cleanup_paths.is_empty() {
        let staged_cleanup_paths = plan
            .staged_cleanup_paths
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        delete_vectors_for_exact_paths_in_state(
            &mut vector_store,
            &staged_cleanup_paths,
            RagChunkState::Staged,
        )
        .await?;
    }

    if !plan.stale_paths.is_empty() {
        let stale_paths = plan.stale_paths.iter().cloned().collect::<Vec<_>>();
        delete_vectors_for_exact_paths(&mut vector_store, &stale_paths).await?;
        tokio::task::spawn_blocking({
            let sqlite_path = sqlite_path.clone();
            move || delete_rag_file_records_for_paths(&sqlite_path, &stale_paths, false)
        })
        .await
        .context("failed to join RAG sqlite cleanup task")??;
    }

    vector_store.ensure_index().await?;

    set_runtime_status(
        app_handle,
        runtime_status,
        runtime_guard,
        RagRuntimePhase::Idle,
        RuntimeProgress::default(),
        RuntimeStatusUpdate {
            warning_count: plan.warning_count,
            recent_warnings: plan.recent_warnings.clone(),
            ..RuntimeStatusUpdate::default()
        },
    )
    .await;
    tracing::info!(
        elapsed_ms = rebuild_started_at.elapsed().as_millis(),
        scanned_file_count = plan.scanned_file_count,
        indexed_file_count = plan.indexed_file_count,
        skipped_file_count = plan.skipped_file_count,
        chunk_count = plan.chunk_count,
        "rag rebuild completed"
    );
    Ok(build_scan_result(&database_path, resolved, &plan))
}

fn streaming_reindex_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| {
            parallelism
                .get()
                .clamp(1, MAX_STREAMING_REINDEX_CONCURRENCY)
        })
        .unwrap_or(2)
}

pub(super) fn choose_scanned_file_path(
    resolved: &ResolvedRagConfig,
    original_path: &Path,
    canonical_path: &Path,
) -> PathBuf {
    if resolve_source_root_for_path(&resolved.source_roots, canonical_path).is_some() {
        canonical_path.to_path_buf()
    } else {
        original_path.to_path_buf()
    }
}

pub(super) fn scanned_file_lookup_keys(
    original_path: &Path,
    canonical_path: &Path,
    scan_path: &Path,
) -> Vec<String> {
    let mut keys = Vec::new();
    for key in [
        normalize_path_string(scan_path),
        normalize_path_string(original_path),
        normalize_path_string(canonical_path),
    ] {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

pub(super) fn find_stored_record_by_path_alias<'a>(
    stored_records: &'a HashMap<String, RagIndexedFileRecord>,
    keys: &[String],
) -> Option<(&'a RagIndexedFileRecord, String)> {
    keys.iter()
        .find_map(|key| stored_records.get(key).map(|record| (record, key.clone())))
}

pub(super) fn stream_rebuild_scan(
    resolved: &ResolvedRagConfig,
    stored_records: &HashMap<String, RagIndexedFileRecord>,
    scan_tx: mpsc::Sender<RebuildScanEvent>,
) -> Result<()> {
    let mut visited_paths = HashSet::new();

    for source_root in &resolved.source_roots {
        let mut walker = WalkBuilder::new(source_root);
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
                        "failed to walk RAG source entry"
                    );
                    if scan_tx
                        .blocking_send(RebuildScanEvent {
                            skipped_file_count: 1,
                            warning_count: 1,
                            recent_warnings: vec![format!("walk source entry failed: {error}")],
                            ..RebuildScanEvent::default()
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                    continue;
                }
            };

            let path = entry.path();
            if path == source_root
                || !entry
                    .file_type()
                    .map(|kind| kind.is_file())
                    .unwrap_or(false)
            {
                continue;
            }

            let canonical_path = match path.canonicalize() {
                Ok(path) => path,
                Err(error) => {
                    tracing::warn!(error = format_args!("{:#}", error), path = %path.display(), "failed to canonicalize RAG file path");
                    let fallback_path = normalize_path_string(path);
                    if stored_records.contains_key(&fallback_path) {
                        visited_paths.insert(fallback_path);
                    }
                    if scan_tx
                        .blocking_send(RebuildScanEvent {
                            skipped_file_count: 1,
                            warning_count: 1,
                            recent_warnings: vec![format_rag_warning_for_path(
                                path,
                                &format!("failed to canonicalize path: {error}"),
                            )],
                            ..RebuildScanEvent::default()
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                    continue;
                }
            };
            let scan_path = choose_scanned_file_path(resolved, path, &canonical_path);
            let lookup_keys = scanned_file_lookup_keys(path, &canonical_path, &scan_path);
            let normalized_path = lookup_keys
                .first()
                .cloned()
                .unwrap_or_else(|| normalize_path_string(&scan_path));
            if !visited_paths.insert(normalized_path.clone()) {
                continue;
            }

            let mut event = RebuildScanEvent {
                scanned_file_count: 1,
                ..RebuildScanEvent::default()
            };
            let matched_stored_path =
                find_stored_record_by_path_alias(stored_records, &lookup_keys).map(
                    |(record, matched_key)| {
                        visited_paths.insert(matched_key.clone());
                        (record, matched_key)
                    },
                );
            let stored_record = matched_stored_path.as_ref().map(|(record, _)| *record);
            match inspect_path_for_index(resolved, &scan_path, stored_record) {
                Ok(InspectPathOutcome::Skip) => {
                    event.skipped_file_count = 1;
                    if let Some((_, matched_path)) = matched_stored_path {
                        event.stale_paths.push(matched_path);
                    }
                }
                Ok(InspectPathOutcome::Unchanged {
                    record,
                    refresh_rag_file_record,
                    clear_staged,
                    refresh_projection,
                }) => {
                    event.indexed_file_count = 1;
                    event.chunk_count = record.current_chunk_count();
                    if refresh_rag_file_record {
                        event.rag_file_record_refresh = Some(record.clone());
                    }
                    if refresh_projection {
                        event.projection_refresh = Some(record);
                    }
                    if clear_staged {
                        let cleanup_path = matched_stored_path
                            .as_ref()
                            .map(|(_, matched_path)| matched_path.clone())
                            .unwrap_or(normalized_path);
                        event.staged_cleanup_paths.push(cleanup_path);
                    }
                }
                Ok(InspectPathOutcome::Reindex(file)) => {
                    event.indexed_file_count = 1;
                    event.chunk_count = file.chunk_count;
                    event.warning_count = file.warnings.len();
                    extend_recent_rag_warnings(
                        &mut event.recent_warnings,
                        file.warnings
                            .iter()
                            .map(|warning| format_rag_warning_for_path(&scan_path, warning)),
                    );
                    event.file_to_index = Some(file);
                }
                Err(error) => {
                    tracing::warn!(error = format_args!("{:#}", error), path = %scan_path.display(), "failed to inspect RAG file");
                    event.skipped_file_count = 1;
                    event.warning_count = 1;
                    push_recent_rag_warning(
                        &mut event.recent_warnings,
                        format_rag_warning_for_path(&scan_path, &format_error_chain(&error)),
                    );
                    let has_active_chunks = stored_record
                        .and_then(|r| r.active.as_ref())
                        .map(|v| v.chunk_count > 0)
                        .unwrap_or(false);
                    if !has_active_chunks {
                        event.rag_file_record_refresh =
                            build_skip_marker_record(resolved, &scan_path);
                    }
                }
            }

            if scan_tx.blocking_send(event).is_err() {
                return Ok(());
            }
        }
    }

    let stale_paths = stored_records
        .keys()
        .filter(|absolute_path| !visited_paths.contains(*absolute_path))
        .cloned()
        .collect::<Vec<_>>();
    if !stale_paths.is_empty() {
        let _ = scan_tx.blocking_send(RebuildScanEvent {
            stale_paths,
            ..RebuildScanEvent::default()
        });
    }

    Ok(())
}

fn plan_startup_reconciliation(
    resolved: &ResolvedRagConfig,
    stored_records: &HashMap<String, RagIndexedFileRecord>,
) -> Result<(Vec<(PathBuf, PathUpdatePlan)>, RuntimeStatusUpdate)> {
    let mut visited_paths = HashSet::new();
    let mut plans = Vec::new();
    let mut planning_update = RuntimeStatusUpdate::default();

    for source_root in &resolved.source_roots {
        let mut walker = WalkBuilder::new(source_root);
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
                        "failed to walk RAG source entry"
                    );
                    planning_update.warning_count = planning_update.warning_count.saturating_add(1);
                    push_recent_rag_warning(
                        &mut planning_update.recent_warnings,
                        format!("walk source entry failed: {error}"),
                    );
                    continue;
                }
            };

            let path = entry.path();
            if path == source_root
                || !entry
                    .file_type()
                    .map(|kind| kind.is_file())
                    .unwrap_or(false)
            {
                continue;
            }

            let canonical_path = match path.canonicalize() {
                Ok(path) => path,
                Err(error) => {
                    tracing::warn!(error = format_args!("{:#}", error), path = %path.display(), "failed to canonicalize RAG file path");
                    let fallback_path = normalize_path_string(path);
                    if stored_records.contains_key(&fallback_path) {
                        visited_paths.insert(fallback_path);
                    }
                    planning_update.warning_count = planning_update.warning_count.saturating_add(1);
                    push_recent_rag_warning(
                        &mut planning_update.recent_warnings,
                        format_rag_warning_for_path(
                            path,
                            &format!("failed to canonicalize path: {error}"),
                        ),
                    );
                    continue;
                }
            };
            let scan_path = choose_scanned_file_path(resolved, path, &canonical_path);
            let lookup_keys = scanned_file_lookup_keys(path, &canonical_path, &scan_path);
            let normalized_path = lookup_keys
                .first()
                .cloned()
                .unwrap_or_else(|| normalize_path_string(&scan_path));
            if !visited_paths.insert(normalized_path.clone()) {
                continue;
            }

            let stored_record = find_stored_record_by_path_alias(stored_records, &lookup_keys).map(
                |(record, matched_key)| {
                    visited_paths.insert(matched_key);
                    record.clone()
                },
            );
            match build_path_update_plan(resolved, &scan_path, stored_record.clone()) {
                Ok(PathUpdatePlan::Noop) => {}
                Ok(PathUpdatePlan::Delete { .. }) if stored_record.is_none() => {}
                Ok(plan @ PathUpdatePlan::Delete { .. }) => {
                    let plan_path = stored_record
                        .as_ref()
                        .map(|record| PathBuf::from(&record.absolute_path))
                        .unwrap_or_else(|| scan_path.clone());
                    plans.push((plan_path, plan));
                }
                Ok(plan) => plans.push((scan_path, plan)),
                Err(error) => {
                    tracing::warn!(error = format_args!("{:#}", error), path = %scan_path.display(), "failed to inspect RAG file during startup reconciliation");
                    planning_update.warning_count = planning_update.warning_count.saturating_add(1);
                    push_recent_rag_warning(
                        &mut planning_update.recent_warnings,
                        format_rag_warning_for_path(&scan_path, &format_error_chain(&error)),
                    );
                    // Only write a skip marker when there are no active chunks
                    // to avoid orphaning existing vector data in the chunk store.
                    let has_active_chunks = stored_record
                        .as_ref()
                        .and_then(|r| r.active.as_ref())
                        .map(|v| v.chunk_count > 0)
                        .unwrap_or(false);
                    if !has_active_chunks {
                        if let Some(marker) = build_skip_marker_record(resolved, &scan_path) {
                            plans.push((
                                scan_path,
                                PathUpdatePlan::RefreshRagFileRecord {
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
    }

    for absolute_path in stored_records.keys() {
        if visited_paths.contains(absolute_path) {
            continue;
        }
        plans.push((
            PathBuf::from(absolute_path),
            PathUpdatePlan::Delete {
                delete_descendants: false,
            },
        ));
    }

    Ok((plans, planning_update))
}

async fn spawn_streaming_reindex_tasks(
    join_set: &mut JoinSet<Result<IndexedPreparedFile>>,
    pending_files: &mut VecDeque<PreparedRagFile>,
    semaphore: &Arc<Semaphore>,
    sqlite_path: &Path,
    resolved: &ResolvedRagConfig,
    client: &HttpClient,
    vector_store: &RagVectorStore,
) -> Result<()> {
    while let Some(file) = pending_files.pop_front() {
        let Ok(permit) = semaphore.clone().try_acquire_owned() else {
            pending_files.push_front(file);
            break;
        };
        write_rag_file_records(
            sqlite_path,
            vec![file.record.clone()],
            "failed to join streamed RAG file record stage task",
        )
        .await?;
        let resolved = resolved.clone();
        let client = client.clone();
        let vector_store = vector_store.clone();
        join_set.spawn(async move {
            let _permit = permit;
            index_prepared_file(&resolved, &client, &vector_store, file).await
        });
    }

    Ok(())
}

async fn set_rebuild_runtime_status(
    app_handle: Option<&AppHandle>,
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_guard: Option<(&Arc<AtomicU64>, u64)>,
    scanned_file_count: usize,
    completed_file_count: usize,
    pending_file_count: usize,
    update: RuntimeStatusUpdate,
) {
    let total_file_count = completed_file_count.saturating_add(pending_file_count);
    set_runtime_status(
        app_handle,
        runtime_status,
        runtime_guard,
        if pending_file_count == 0 {
            RagRuntimePhase::Scanning
        } else {
            RagRuntimePhase::Indexing
        },
        RuntimeProgress {
            scanned_file_count,
            completed_file_count,
            total_file_count,
            pending_file_count,
        },
        update,
    )
    .await;
}

async fn write_rag_file_records(
    sqlite_path: &Path,
    records: Vec<RagIndexedFileRecord>,
    join_error_message: &'static str,
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }

    let started_at = Instant::now();
    let record_count = records.len();
    tokio::task::spawn_blocking({
        let sqlite_path = sqlite_path.to_path_buf();
        move || upsert_rag_file_records(&sqlite_path, &records)
    })
    .await
    .context(join_error_message)??;
    tracing::info!(
        record_count,
        elapsed_ms = started_at.elapsed().as_millis(),
        "rag file records persisted"
    );
    Ok(())
}

async fn refresh_projection_records(
    database_path: &Path,
    sqlite_path: &Path,
    records: Vec<RagIndexedFileRecord>,
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }

    tokio::task::spawn_blocking({
        let database_path = database_path.to_path_buf();
        let sqlite_path = sqlite_path.to_path_buf();
        move || refresh_projection_for_rag_file_records(&database_path, &sqlite_path, &records)
    })
    .await
    .context("failed to join RAG projection refresh task")??;
    Ok(())
}

pub(super) async fn execute_path_update_plans(
    runtime_context: PathUpdateRuntimeContext<'_>,
    database_path: &Path,
    sqlite_path: &Path,
    resolved: &ResolvedRagConfig,
    plans: Vec<(PathBuf, PathUpdatePlan)>,
    mut status_update: RuntimeStatusUpdate,
) -> Result<RuntimeStatusUpdate> {
    if plans.is_empty() {
        return Ok(status_update);
    }

    let mut vector_store = RagVectorStore::open(database_path).await?;
    let mut delete_exact_paths = Vec::new();
    let mut delete_prefix_paths = Vec::new();
    let mut staged_cleanup_paths = Vec::new();
    let mut rag_file_record_refreshes = Vec::new();
    let mut projection_refreshes = Vec::new();
    let mut files_to_index = Vec::new();

    for (path, plan) in plans {
        match plan {
            PathUpdatePlan::Noop => {}
            PathUpdatePlan::Delete { delete_descendants } => {
                if delete_descendants {
                    delete_prefix_paths.push(normalize_path_string(&path));
                } else {
                    delete_exact_paths.push(normalize_path_string(&path));
                }
            }
            PathUpdatePlan::RefreshRagFileRecord {
                record,
                clear_staged,
                refresh_projection,
            } => {
                if clear_staged {
                    staged_cleanup_paths.push(record.absolute_path.clone());
                }
                if refresh_projection {
                    projection_refreshes.push(record.clone());
                }
                rag_file_record_refreshes.push(record);
            }
            PathUpdatePlan::Reindex(file) => files_to_index.push(file),
        }
    }

    if !delete_exact_paths.is_empty() {
        delete_vectors_for_exact_paths(&mut vector_store, &delete_exact_paths).await?;
        tokio::task::spawn_blocking({
            let sqlite_path = sqlite_path.to_path_buf();
            let delete_exact_paths = delete_exact_paths.clone();
            move || delete_rag_file_records_for_paths(&sqlite_path, &delete_exact_paths, false)
        })
        .await
        .context("failed to join batched RAG file record delete task")??;
    }

    if !delete_prefix_paths.is_empty() {
        delete_vectors_for_prefix_paths(&mut vector_store, &delete_prefix_paths).await?;
        tokio::task::spawn_blocking({
            let sqlite_path = sqlite_path.to_path_buf();
            let delete_prefix_paths = delete_prefix_paths.clone();
            move || delete_rag_file_records_for_paths(&sqlite_path, &delete_prefix_paths, true)
        })
        .await
        .context("failed to join descendant RAG file record delete task")??;
    }

    if !staged_cleanup_paths.is_empty() {
        delete_vectors_for_exact_paths_in_state(
            &mut vector_store,
            &staged_cleanup_paths,
            RagChunkState::Staged,
        )
        .await?;
    }

    if !rag_file_record_refreshes.is_empty() {
        tokio::task::spawn_blocking({
            let sqlite_path = sqlite_path.to_path_buf();
            let rag_file_record_refreshes = rag_file_record_refreshes.clone();
            move || upsert_rag_file_records(&sqlite_path, &rag_file_record_refreshes)
        })
        .await
        .context("failed to join batched RAG file record refresh task")??;
    }

    if !projection_refreshes.is_empty() {
        refresh_projection_records(database_path, sqlite_path, projection_refreshes).await?;
    }

    for file in &files_to_index {
        status_update.warning_count = status_update
            .warning_count
            .saturating_add(file.warnings.len());
        extend_recent_rag_warnings(
            &mut status_update.recent_warnings,
            file.warnings
                .iter()
                .map(|warning| format_rag_warning_for_path(&file.path, warning)),
        );
    }

    if !files_to_index.is_empty() {
        let client = build_embedding_client()?;
        set_runtime_status(
            runtime_context.app_handle,
            runtime_context.runtime_status,
            runtime_context.runtime_guard,
            RagRuntimePhase::Indexing,
            RuntimeProgress {
                scanned_file_count: files_to_index.len(),
                total_file_count: files_to_index.len(),
                pending_file_count: files_to_index.len(),
                ..RuntimeProgress::default()
            },
            status_update.clone(),
        )
        .await;
        for (index, file) in files_to_index.iter().enumerate() {
            write_rag_file_records(
                sqlite_path,
                vec![file.record.clone()],
                "failed to join batched RAG file record stage task",
            )
            .await?;
            reindex_prepared_file(&mut vector_store, sqlite_path, resolved, &client, file).await?;
            let remaining = files_to_index.len().saturating_sub(index + 1);
            set_runtime_status(
                runtime_context.app_handle,
                runtime_context.runtime_status,
                runtime_context.runtime_guard,
                if remaining == 0 {
                    RagRuntimePhase::Scanning
                } else {
                    RagRuntimePhase::Indexing
                },
                RuntimeProgress {
                    scanned_file_count: files_to_index.len(),
                    completed_file_count: index + 1,
                    total_file_count: files_to_index.len(),
                    pending_file_count: remaining,
                },
                status_update.clone(),
            )
            .await;
        }
    }

    vector_store.ensure_index().await?;
    Ok(status_update)
}

async fn reindex_prepared_file(
    vector_store: &mut RagVectorStore,
    sqlite_path: &Path,
    resolved: &ResolvedRagConfig,
    client: &HttpClient,
    file: &PreparedRagFile,
) -> Result<()> {
    let indexed = index_prepared_file_for_store(vector_store, resolved, client, file).await?;
    persist_indexed_file(vector_store, sqlite_path, indexed).await
}

async fn index_prepared_file(
    resolved: &ResolvedRagConfig,
    client: &HttpClient,
    vector_store: &RagVectorStore,
    file: PreparedRagFile,
) -> Result<IndexedPreparedFile> {
    build_indexed_file_output(resolved, client, vector_store, file).await
}

async fn index_prepared_file_for_store(
    vector_store: &RagVectorStore,
    resolved: &ResolvedRagConfig,
    client: &HttpClient,
    file: &PreparedRagFile,
) -> Result<IndexedPreparedFile> {
    build_indexed_file_output(resolved, client, vector_store, file.clone()).await
}

async fn build_indexed_file_output(
    resolved: &ResolvedRagConfig,
    client: &HttpClient,
    vector_store: &RagVectorStore,
    file: PreparedRagFile,
) -> Result<IndexedPreparedFile> {
    let reusable_vectors = vector_store
        .load_chunk_vectors_for_file(
            &file.record.absolute_path,
            RagChunkState::Active,
            &resolved.embedding_fingerprint,
        )
        .await?;
    let observation = observe_prepared_file(&file);
    let chunks = build_chunks_for_prepared_file(&file, &file.prepared_chunks);
    let vectors =
        resolve_chunk_vectors(resolved, client, vector_store, &chunks, &reusable_vectors).await?;
    log_rag_file_observation(
        &file,
        RagFileObservation {
            vector_count: vectors.len(),
            vector_bytes_estimate: vectors
                .iter()
                .map(|vector| vector.len().saturating_mul(std::mem::size_of::<f32>()))
                .sum(),
            ..observation
        },
    );
    Ok(IndexedPreparedFile {
        file,
        chunks,
        vectors,
    })
}

async fn persist_indexed_file(
    vector_store: &mut RagVectorStore,
    sqlite_path: &Path,
    indexed: IndexedPreparedFile,
) -> Result<()> {
    let persist_started_at = Instant::now();
    let IndexedPreparedFile {
        file,
        chunks,
        vectors,
    } = indexed;
    let chunk_count = chunks.len();

    delete_vectors_with_filter(
        vector_store,
        &format!(
            "absolute_path = '{}' AND chunk_state = '{}'",
            escape_sql_literal(&file.record.absolute_path),
            RagChunkState::Staged.as_str()
        ),
    )
    .await?;
    if !chunks.is_empty() {
        vector_store.add_chunks(&chunks, &vectors).await?;
    }

    let activate_filter = format!(
        "absolute_path = '{}' AND version_id = '{}' AND chunk_state = '{}'",
        escape_sql_literal(&file.record.absolute_path),
        escape_sql_literal(&file.version_id),
        RagChunkState::Staged.as_str()
    );
    vector_store
        .update_where(&activate_filter, RagChunkState::Active)
        .await?;
    if let Some(active_version) = file.record.active.as_ref() {
        delete_vectors_with_filter(
            vector_store,
            &format!(
                "absolute_path = '{}' AND version_id = '{}' AND chunk_state = '{}'",
                escape_sql_literal(&file.record.absolute_path),
                escape_sql_literal(&active_version.version_id),
                RagChunkState::Active.as_str()
            ),
        )
        .await?;
    }

    tokio::task::spawn_blocking({
        let sqlite_path = sqlite_path.to_path_buf();
        move || {
            finalize_rag_file_record_and_replace_lexical_chunks(
                &sqlite_path,
                &file.record,
                file.prepared_chunks.len(),
                &file.prepared_chunks,
            )
        }
    })
    .await
    .context("failed to join RAG file record finalize task")??;
    tracing::info!(
        chunk_count,
        elapsed_ms = persist_started_at.elapsed().as_millis(),
        "rag indexed file persisted"
    );
    Ok(())
}

pub(super) async fn resolve_chunk_vectors(
    resolved: &ResolvedRagConfig,
    client: &HttpClient,
    vector_store: &RagVectorStore,
    chunks: &[RagChunk],
    reusable_vectors: &HashMap<String, Vec<f32>>,
) -> Result<Vec<Vec<f32>>> {
    if chunks.is_empty() {
        return Ok(Vec::new());
    }

    let mut resolved_vectors = vec![None; chunks.len()];
    let mut missing_indexes = Vec::new();
    let mut missing_inputs = Vec::new();

    for (index, chunk) in chunks.iter().enumerate() {
        if let Some(vector) = reusable_vectors.get(&chunk.chunk_reuse_key) {
            resolved_vectors[index] = Some(vector.clone());
        } else {
            missing_indexes.push(index);
            missing_inputs.push(chunk.text.clone());
        }
    }

    let cached_vectors = vector_store
        .load_cached_vectors_for_texts(&resolved.embedding_fingerprint, &missing_inputs)
        .await?;
    let mut remote_missing_indexes = Vec::new();
    let mut remote_missing_inputs = Vec::new();
    for (offset, text) in missing_inputs.into_iter().enumerate() {
        let chunk_index = missing_indexes[offset];
        if let Some(vector) = cached_vectors.get(&text) {
            resolved_vectors[chunk_index] = Some(vector.clone());
            continue;
        }
        remote_missing_indexes.push(chunk_index);
        remote_missing_inputs.push(text);
    }

    let mut batch_planner = EmbeddingBatchPlanner::default();
    let mut start = 0usize;
    while start < remote_missing_inputs.len() {
        let end = batch_planner.next_batch_end(&remote_missing_inputs, start);
        let inputs = &remote_missing_inputs[start..end];
        let embedding_started_at = Instant::now();
        let (vectors, embedding_stats) =
            request_embeddings_with_stats(client, &resolved.provider, inputs).await?;
        tracing::info!(
            batch_size = inputs.len(),
            provider_id = %resolved.provider.id,
            elapsed_ms = embedding_started_at.elapsed().as_millis(),
            "rag embedding batch resolved for chunk vectors"
        );
        for (offset, vector) in vectors.into_iter().enumerate() {
            let chunk_index = remote_missing_indexes[start + offset];
            resolved_vectors[chunk_index] = Some(vector);
        }
        batch_planner.record_success(inputs.len(), embedding_stats);
        start = end;
    }

    resolved_vectors
        .into_iter()
        .map(|vector| vector.context("missing vector for prepared RAG chunk"))
        .collect()
}

pub(super) fn build_path_update_plan(
    resolved: &ResolvedRagConfig,
    path: &Path,
    stored_record: Option<RagIndexedFileRecord>,
) -> Result<PathUpdatePlan> {
    if !(path.exists() && path.is_file()) {
        return Ok(PathUpdatePlan::Delete {
            delete_descendants: !path.exists(),
        });
    }

    match inspect_path_for_index(resolved, path, stored_record.as_ref())? {
        InspectPathOutcome::Skip => Ok(PathUpdatePlan::Delete {
            delete_descendants: false,
        }),
        InspectPathOutcome::Unchanged {
            record,
            refresh_rag_file_record,
            clear_staged,
            refresh_projection,
        } => {
            if refresh_rag_file_record {
                Ok(PathUpdatePlan::RefreshRagFileRecord {
                    record,
                    clear_staged,
                    refresh_projection,
                })
            } else {
                Ok(PathUpdatePlan::Noop)
            }
        }
        InspectPathOutcome::Reindex(file) => Ok(PathUpdatePlan::Reindex(file)),
    }
}

pub(super) fn inspect_path_for_index(
    resolved: &ResolvedRagConfig,
    path: &Path,
    stored_record: Option<&RagIndexedFileRecord>,
) -> Result<InspectPathOutcome> {
    let source_root = resolve_source_root_for_path(&resolved.source_roots, path)
        .with_context(|| format!("path is outside configured RAG roots: {}", path.display()))?;
    let source_root_string = normalize_path_string(source_root);
    let relative_path = path
        .strip_prefix(source_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    if should_skip_path(source_root, path, resolved.ignore_globs.as_ref().as_ref()) {
        return Ok(InspectPathOutcome::Skip);
    }
    if !is_supported_document_file(path) {
        return Ok(InspectPathOutcome::Skip);
    }
    let Some(document_kind) = classify_document_kind(path) else {
        return Ok(InspectPathOutcome::Skip);
    };

    let file_metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to read file metadata: {}", path.display()))?;
    if file_metadata.len() > max_indexable_file_bytes(document_kind) {
        return Ok(InspectPathOutcome::Skip);
    }

    let size_bytes = i64::try_from(file_metadata.len()).with_context(|| {
        format!(
            "file is too large to track in rag sqlite store: {}",
            path.display()
        )
    })?;
    let modified_at_ms = file_metadata
        .modified()
        .ok()
        .and_then(system_time_to_unix_ms);
    let active_version = stored_record.and_then(|record| record.active.as_ref());
    let extractor_fingerprint = extractor_fingerprint_for_path(path)
        .map(str::to_string)
        .context("unsupported document type for extractor fingerprint")?;
    let same_embedding_and_extractor = stored_record
        .map(|record| {
            record.embedding_fingerprint == resolved.embedding_fingerprint
                && record.extractor_fingerprint == extractor_fingerprint
        })
        .unwrap_or(false);
    let same_projection = stored_record
        .map(|record| {
            record.source_root == source_root_string && record.relative_path == relative_path
        })
        .unwrap_or(false);

    if let Some(stored_record) = stored_record {
        if same_embedding_and_extractor
            && active_version
                .map(|version| {
                    version.size_bytes == size_bytes && version.modified_at_ms == modified_at_ms
                })
                .unwrap_or(false)
        {
            let clear_staged = stored_record.has_pending();
            let refresh_projection = !same_projection;
            return Ok(InspectPathOutcome::Unchanged {
                record: refreshed_record_with_current_path(
                    stored_record,
                    &source_root_string,
                    &relative_path,
                    modified_at_ms,
                    size_bytes,
                ),
                refresh_rag_file_record: clear_staged || refresh_projection,
                clear_staged,
                refresh_projection,
            });
        }
    }

    tracing::debug!(
        path = %path.display(),
        has_stored_record = stored_record.is_some(),
        has_active_version = active_version.is_some(),
        same_embedding_and_extractor,
        stored_size = active_version.map(|v| v.size_bytes),
        current_size = size_bytes,
        stored_mtime = active_version.and_then(|v| v.modified_at_ms),
        current_mtime = modified_at_ms,
        "rag file metadata fast path missed, will extract and inspect content"
    );

    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))?;
    let extracted = extract_document_from_bytes(path, &bytes)?;
    tracing::info!(
        path = %path.display(),
        document_kind = document_kind.as_str(),
        bytes_read = bytes.len(),
        extracted_text_bytes = extracted.normalized_text.len(),
        "rag file extracted for indexing"
    );
    let same_index_target = stored_record
        .map(|record| {
            record.source_root == source_root_string
                && record.relative_path == relative_path
                && record.embedding_fingerprint == resolved.embedding_fingerprint
                && record.extractor_fingerprint == extracted.extractor_fingerprint
        })
        .unwrap_or(false);

    if let Some(stored_record) = stored_record {
        if same_index_target
            && active_version
                .map(|version| {
                    version.size_bytes == size_bytes && version.modified_at_ms == modified_at_ms
                })
                .unwrap_or(false)
        {
            let clear_staged = stored_record.has_pending();
            return Ok(InspectPathOutcome::Unchanged {
                record: if clear_staged {
                    stored_record.refresh_active_version(modified_at_ms, size_bytes)
                } else {
                    stored_record.clone()
                },
                refresh_rag_file_record: clear_staged,
                clear_staged,
                refresh_projection: false,
            });
        }
    }

    let content_md5 = format!("{:x}", md5::compute(&bytes));

    if let Some(stored_record) = stored_record {
        if same_embedding_and_extractor
            && active_version
                .map(|version| version.content_md5 == content_md5)
                .unwrap_or(false)
        {
            let clear_staged = stored_record.has_pending();
            let refresh_projection = !same_projection;
            return Ok(InspectPathOutcome::Unchanged {
                record: refreshed_record_with_current_path(
                    stored_record,
                    &source_root_string,
                    &relative_path,
                    modified_at_ms,
                    size_bytes,
                ),
                refresh_rag_file_record: true,
                clear_staged,
                refresh_projection,
            });
        }
    }
    if extracted.normalized_text.trim().is_empty() && extracted.blocks.is_empty() {
        return Ok(build_empty_content_outcome(
            path,
            &source_root_string,
            &relative_path,
            &extractor_fingerprint,
            resolved,
            modified_at_ms,
            size_bytes,
            stored_record,
        ));
    }
    let prepared_chunks =
        split_extracted_document_for_path(path, &extracted, CHUNK_MAX_CHARS, CHUNK_OVERLAP_CHARS)?;
    if prepared_chunks.is_empty() {
        return Ok(build_empty_content_outcome(
            path,
            &source_root_string,
            &relative_path,
            &extractor_fingerprint,
            resolved,
            modified_at_ms,
            size_bytes,
            stored_record,
        ));
    }
    let chunk_count = prepared_chunks.len();

    let version_id = make_chunk_version_id(path);
    let pending_version = RagIndexedFileVersion {
        version_id: version_id.clone(),
        content_md5,
        modified_at_ms,
        size_bytes,
        chunk_count: i64::try_from(chunk_count).context("chunk count exceeds i64 range")?,
        indexed_at_ms: now_unix_ms(),
    };

    Ok(InspectPathOutcome::Reindex(PreparedRagFile {
        path: path.to_path_buf(),
        record: RagIndexedFileRecord {
            source_root: source_root_string,
            absolute_path: normalize_path_string(path),
            relative_path,
            embedding_fingerprint: resolved.embedding_fingerprint.clone(),
            extractor_fingerprint: extracted.extractor_fingerprint.clone(),
            active: stored_record.and_then(|record| record.active.clone()),
            pending: Some(pending_version),
        },
        prepared_chunks,
        version_id,
        chunk_count,
        warnings: extracted.warnings,
    }))
}

/// Build a skip marker record for a file that cannot produce indexable content.
/// Stores size+mtime so the next startup fast path can skip re-extraction.
pub(super) fn build_skip_marker_record(
    resolved: &ResolvedRagConfig,
    path: &Path,
) -> Option<RagIndexedFileRecord> {
    let source_root = resolve_source_root_for_path(&resolved.source_roots, path)?;
    let source_root_string = normalize_path_string(source_root);
    let relative_path = path
        .strip_prefix(source_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let extractor_fingerprint = extractor_fingerprint_for_path(path)?;
    let file_metadata = std::fs::metadata(path).ok()?;
    let size_bytes = i64::try_from(file_metadata.len()).ok()?;
    let modified_at_ms = file_metadata
        .modified()
        .ok()
        .and_then(system_time_to_unix_ms);
    Some(RagIndexedFileRecord {
        source_root: source_root_string,
        absolute_path: normalize_path_string(path),
        relative_path,
        embedding_fingerprint: resolved.embedding_fingerprint.clone(),
        extractor_fingerprint: extractor_fingerprint.to_string(),
        active: Some(RagIndexedFileVersion {
            version_id: format!("skip-{:x}", md5::compute(normalize_path_string(path))),
            content_md5: String::new(),
            modified_at_ms,
            size_bytes,
            chunk_count: 0,
            indexed_at_ms: now_unix_ms(),
        }),
        pending: None,
    })
}

/// Build an outcome for files that exist but produce no indexable content
/// (empty text, no viable chunks). If the file was previously indexed with
/// real chunks, returns `Skip` so the caller deletes stale vector data first;
/// the skip marker will be written on the next scan once the old data is gone.
/// Otherwise persists a skip marker record so the next startup can
/// short-circuit via the size+mtime fast path.
#[allow(clippy::too_many_arguments)]
fn build_empty_content_outcome(
    path: &Path,
    source_root: &str,
    relative_path: &str,
    extractor_fingerprint: &str,
    resolved: &ResolvedRagConfig,
    modified_at_ms: Option<i64>,
    size_bytes: i64,
    stored_record: Option<&RagIndexedFileRecord>,
) -> InspectPathOutcome {
    // When the stored record has active chunks, return Skip so the caller
    // runs Delete and cleans up stale vector data in the chunk store.
    let has_active_chunks = stored_record
        .and_then(|r| r.active.as_ref())
        .map(|v| v.chunk_count > 0)
        .unwrap_or(false);
    if has_active_chunks {
        return InspectPathOutcome::Skip;
    }

    let record = RagIndexedFileRecord {
        source_root: source_root.to_string(),
        absolute_path: normalize_path_string(path),
        relative_path: relative_path.to_string(),
        embedding_fingerprint: resolved.embedding_fingerprint.clone(),
        extractor_fingerprint: extractor_fingerprint.to_string(),
        active: Some(RagIndexedFileVersion {
            version_id: format!("skip-{:x}", md5::compute(normalize_path_string(path))),
            content_md5: String::new(),
            modified_at_ms,
            size_bytes,
            chunk_count: 0,
            indexed_at_ms: now_unix_ms(),
        }),
        pending: None,
    };
    let clear_staged = stored_record.map(|r| r.has_pending()).unwrap_or(false);
    InspectPathOutcome::Unchanged {
        record,
        refresh_rag_file_record: true,
        clear_staged,
        refresh_projection: false,
    }
}

fn refreshed_record_with_current_path(
    stored_record: &RagIndexedFileRecord,
    source_root: &str,
    relative_path: &str,
    modified_at_ms: Option<i64>,
    size_bytes: i64,
) -> RagIndexedFileRecord {
    let mut refreshed = stored_record.refresh_active_version(modified_at_ms, size_bytes);
    refreshed.source_root = source_root.to_string();
    refreshed.relative_path = relative_path.to_string();
    refreshed
}

#[cfg(test)]
pub(super) fn collect_chunks_for_path(
    resolved: &ResolvedRagConfig,
    path: &Path,
) -> Result<Vec<RagChunk>> {
    match inspect_path_for_index(resolved, path, None)? {
        InspectPathOutcome::Skip => Ok(Vec::new()),
        InspectPathOutcome::Unchanged { .. } => Ok(Vec::new()),
        InspectPathOutcome::Reindex(file) => {
            Ok(build_chunks_for_prepared_file(&file, &file.prepared_chunks))
        }
    }
}

fn observe_prepared_file(file: &PreparedRagFile) -> RagFileObservation {
    let bytes_read = file
        .record
        .pending
        .as_ref()
        .map(|version| version.size_bytes.max(0) as usize)
        .unwrap_or_default();
    let prepared_chunk_text_bytes = file
        .prepared_chunks
        .iter()
        .map(|chunk| chunk.text.len())
        .sum();
    RagFileObservation {
        bytes_read,
        extracted_text_bytes: prepared_chunk_text_bytes,
        prepared_chunk_count: file.prepared_chunks.len(),
        prepared_chunk_text_bytes,
        ..RagFileObservation::default()
    }
}

pub(super) fn build_chunks_for_prepared_file(
    file: &PreparedRagFile,
    prepared_chunks: &[PreparedRagChunk],
) -> Vec<RagChunk> {
    prepared_chunks
        .iter()
        .map(|chunk| RagChunk {
            id: format!(
                "{}#{}:{}:{}:{}:{}:{}",
                file.record.absolute_path,
                file.version_id,
                chunk.chunk_index,
                chunk.page_start.unwrap_or_default(),
                chunk.page_end.unwrap_or_default(),
                chunk.line_start.unwrap_or_default(),
                chunk.line_end.unwrap_or_default(),
            ),
            source_root: file.record.source_root.clone(),
            absolute_path: file.record.absolute_path.clone(),
            version_id: file.version_id.clone(),
            embedding_fingerprint: file.record.embedding_fingerprint.clone(),
            document_kind: chunk.document_kind,
            chunk_state: RagChunkState::Staged,
            chunk_index: chunk.chunk_index,
            line_start: chunk.line_start,
            line_end: chunk.line_end,
            paragraph_line_start: chunk.paragraph_line_start,
            page_start: chunk.page_start,
            page_end: chunk.page_end,
            heading_path: chunk.heading_path.clone(),
            anchor_label: chunk.anchor_label.clone(),
            chunk_reuse_key: chunk.chunk_reuse_key.clone(),
            text_fingerprint: text_fingerprint(&chunk.text),
            text: chunk.text.clone(),
        })
        .collect()
}

fn max_indexable_file_bytes(document_kind: DocumentKind) -> u64 {
    match document_kind {
        DocumentKind::PlainText => MAX_TEXT_FILE_BYTES_PLAIN_TEXT,
        DocumentKind::Markdown => MAX_TEXT_FILE_BYTES_MARKDOWN,
        DocumentKind::Docx => MAX_TEXT_FILE_BYTES_DOCX,
        DocumentKind::Pdf => MAX_TEXT_FILE_BYTES_PDF,
    }
}

fn log_rag_file_observation(file: &PreparedRagFile, observation: RagFileObservation) {
    tracing::info!(
        path = %file.path.display(),
        bytes_read = observation.bytes_read,
        extracted_text_bytes = observation.extracted_text_bytes,
        prepared_chunk_count = observation.prepared_chunk_count,
        prepared_chunk_text_bytes = observation.prepared_chunk_text_bytes,
        vector_count = observation.vector_count,
        vector_bytes_estimate = observation.vector_bytes_estimate,
        "rag file indexing observation"
    );
}

fn make_chunk_version_id(path: &Path) -> String {
    let path_hash = format!("{:x}", md5::compute(normalize_path_string(path)));
    let timestamp_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{timestamp_nanos:x}-{path_hash}")
}

fn build_scan_result(
    database_path: &Path,
    resolved: &ResolvedRagConfig,
    plan: &RebuildPlan,
) -> RagScanResult {
    RagScanResult {
        database_path: database_path.to_string_lossy().into_owned(),
        source_count: resolved.source_roots.len(),
        scanned_file_count: plan.scanned_file_count,
        indexed_file_count: plan.indexed_file_count,
        skipped_file_count: plan.skipped_file_count,
        chunk_count: plan.chunk_count,
        warning_count: plan.warning_count,
        recent_warnings: plan.recent_warnings.clone(),
        finished_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    }
}
