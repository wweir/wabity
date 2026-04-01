use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{atomic::AtomicU64, Arc},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use reqwest::Client as HttpClient;
use tokio::{
    sync::{mpsc, Mutex as AsyncMutex, RwLock as AsyncRwLock, Semaphore},
    task::JoinSet,
};

use crate::{
    domain::rag::{RagRuntimePhase, RagRuntimeStatus, RagScanResult},
    services::document_extract::{extract_document_from_bytes, is_supported_document_file},
};

use super::{
    chunking::split_extracted_document_for_path,
    config::{
        normalize_path_string, now_unix_ms, rag_database_path, rag_metadata_database_path,
        resolve_source_root_for_path, should_skip_path, system_time_to_unix_ms,
    },
    embedding::{
        build_embedding_client, request_embeddings_with_stats, text_fingerprint,
        EmbeddingBatchPlanner,
    },
    model::{
        PreparedRagFile, RagChunk, RagChunkState, RagIndexedFileRecord, RagIndexedFileVersion,
        RagRuntimeStartMode, ResolvedRagConfig, RuntimeProgress, CHUNK_MAX_CHARS,
        CHUNK_OVERLAP_CHARS, MAX_STREAMING_REINDEX_CONCURRENCY, MAX_TEXT_FILE_BYTES,
    },
    status::set_runtime_status,
    storage::{
        delete_metadata_for_paths, delete_vectors_for_exact_paths,
        delete_vectors_for_exact_paths_in_state, delete_vectors_with_filter, escape_sql_literal,
        finalize_metadata_record, load_metadata_paths_for_prefixes, load_metadata_records,
        metadata_store_has_active_records, prepare_index_storage, replace_lexical_chunks_for_file,
        upsert_metadata_records, RagVectorStore,
    },
};

#[derive(Debug, Default)]
struct RebuildPlan {
    scanned_file_count: usize,
    indexed_file_count: usize,
    skipped_file_count: usize,
    chunk_count: usize,
    staged_cleanup_paths: BTreeSet<String>,
    stale_paths: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub(super) struct RebuildScanEvent {
    pub(super) scanned_file_count: usize,
    pub(super) indexed_file_count: usize,
    pub(super) skipped_file_count: usize,
    pub(super) chunk_count: usize,
    pub(super) staged_cleanup_paths: Vec<String>,
    pub(super) stale_paths: Vec<String>,
    pub(super) metadata_refresh: Option<RagIndexedFileRecord>,
    pub(super) file_to_index: Option<PreparedRagFile>,
}

#[derive(Debug)]
struct IndexedPreparedFile {
    file: PreparedRagFile,
    chunks: Vec<RagChunk>,
    vectors: Vec<Vec<f32>>,
}

#[derive(Debug)]
pub(super) enum InspectPathOutcome {
    Skip,
    Unchanged {
        record: RagIndexedFileRecord,
        refresh_metadata: bool,
        clear_staged: bool,
    },
    Reindex(PreparedRagFile),
}

#[derive(Debug)]
pub(super) enum PathUpdatePlan {
    Noop,
    Delete {
        delete_descendants: bool,
    },
    RefreshMetadata {
        record: RagIndexedFileRecord,
        clear_staged: bool,
    },
    Reindex(PreparedRagFile),
}

pub(super) async fn initialize_runtime_storage(
    data_dir: &Path,
    metadata_path: &Path,
    resolved: &ResolvedRagConfig,
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_guard: Option<(&Arc<AtomicU64>, u64)>,
    storage_lock: &Arc<AsyncMutex<()>>,
    start_mode: RagRuntimeStartMode,
) -> Result<()> {
    if start_mode == RagRuntimeStartMode::RebuildIndex {
        let _storage_guard = storage_lock.lock().await;
        rebuild_index_locked(data_dir, resolved, runtime_status, runtime_guard).await?;
        return Ok(());
    }

    let database_path = rag_database_path(data_dir);
    prepare_index_storage(&database_path, metadata_path, resolved, true).await?;

    if metadata_store_has_active_records(metadata_path).await? {
        set_runtime_status(
            runtime_status,
            runtime_guard,
            RagRuntimePhase::Idle,
            RuntimeProgress::default(),
            None,
        )
        .await;
        return Ok(());
    }

    let _storage_guard = storage_lock.lock().await;
    rebuild_index_locked(data_dir, resolved, runtime_status, runtime_guard).await?;
    Ok(())
}

pub(super) async fn rebuild_index_locked(
    data_dir: &Path,
    resolved: &ResolvedRagConfig,
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_guard: Option<(&Arc<AtomicU64>, u64)>,
) -> Result<RagScanResult> {
    set_runtime_status(
        runtime_status,
        runtime_guard,
        RagRuntimePhase::Scanning,
        RuntimeProgress::default(),
        None,
    )
    .await;
    tokio::fs::create_dir_all(data_dir).await.with_context(|| {
        format!(
            "failed to create RAG data directory: {}",
            data_dir.display()
        )
    })?;

    let database_path = rag_database_path(data_dir);
    let metadata_path = rag_metadata_database_path(data_dir);
    tokio::fs::create_dir_all(&database_path)
        .await
        .with_context(|| {
            format!(
                "failed to create RAG database directory: {}",
                database_path.display()
            )
        })?;

    prepare_index_storage(&database_path, &metadata_path, resolved, true).await?;
    let stored_records = tokio::task::spawn_blocking({
        let metadata_path = metadata_path.clone();
        move || load_metadata_records(&metadata_path)
    })
    .await
    .context("failed to join RAG metadata load task")??;
    let client = build_embedding_client()?;
    let mut vector_store = RagVectorStore::open(&database_path).await?;
    let (scan_tx, mut scan_rx) =
        mpsc::channel::<RebuildScanEvent>(streaming_reindex_concurrency().saturating_mul(4));
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
            &metadata_path,
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
                plan.staged_cleanup_paths.extend(event.staged_cleanup_paths);
                plan.stale_paths.extend(event.stale_paths);

                if let Some(record) = event.metadata_refresh {
                    write_metadata_records(
                        &metadata_path,
                        vec![record.clone()],
                        "failed to join streamed RAG metadata refresh task",
                    )
                    .await?;
                }

                if let Some(file) = event.file_to_index {
                    pending_file_count = pending_file_count.saturating_add(1);
                    pending_files.push_back(file);
                } else {
                    completed_file_count =
                        completed_file_count.saturating_add(event.indexed_file_count);
                }

                set_rebuild_runtime_status(
                    runtime_status,
                    runtime_guard,
                    plan.scanned_file_count,
                    completed_file_count,
                    pending_file_count,
                )
                .await;
            }
            maybe_indexed = join_set.join_next(), if !join_set.is_empty() => {
                let indexed = maybe_indexed
                    .context("streaming RAG reindex task queue ended unexpectedly")?
                    .context("failed to join streaming RAG reindex task")??;
                persist_indexed_file(&mut vector_store, &metadata_path, indexed).await?;
                pending_file_count = pending_file_count.saturating_sub(1);
                completed_file_count = completed_file_count.saturating_add(1);
                set_rebuild_runtime_status(
                    runtime_status,
                    runtime_guard,
                    plan.scanned_file_count,
                    completed_file_count,
                    pending_file_count,
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
            let metadata_path = metadata_path.clone();
            move || delete_metadata_for_paths(&metadata_path, &stale_paths, false)
        })
        .await
        .context("failed to join RAG metadata cleanup task")??;
    }

    vector_store.ensure_index().await?;

    set_runtime_status(
        runtime_status,
        runtime_guard,
        RagRuntimePhase::Idle,
        RuntimeProgress::default(),
        None,
    )
    .await;
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
                    tracing::warn!(?error, "failed to walk RAG source entry");
                    if scan_tx
                        .blocking_send(RebuildScanEvent {
                            skipped_file_count: 1,
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
                    tracing::warn!(?error, path = %path.display(), "failed to canonicalize RAG file path");
                    if scan_tx
                        .blocking_send(RebuildScanEvent {
                            skipped_file_count: 1,
                            ..RebuildScanEvent::default()
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                    continue;
                }
            };
            let normalized_path = normalize_path_string(&canonical_path);
            if !visited_paths.insert(normalized_path.clone()) {
                continue;
            }

            let mut event = RebuildScanEvent {
                scanned_file_count: 1,
                ..RebuildScanEvent::default()
            };
            let stored_record = stored_records.get(&normalized_path);
            match inspect_path_for_index(resolved, &canonical_path, stored_record) {
                Ok(InspectPathOutcome::Skip) => {
                    event.skipped_file_count = 1;
                    if stored_record.is_some() {
                        event.stale_paths.push(normalized_path);
                    }
                }
                Ok(InspectPathOutcome::Unchanged {
                    record,
                    refresh_metadata,
                    clear_staged,
                }) => {
                    event.indexed_file_count = 1;
                    event.chunk_count = record.current_chunk_count();
                    if refresh_metadata {
                        event.metadata_refresh = Some(record);
                    }
                    if clear_staged {
                        event.staged_cleanup_paths.push(normalized_path);
                    }
                }
                Ok(InspectPathOutcome::Reindex(file)) => {
                    event.indexed_file_count = 1;
                    event.chunk_count = file.chunks.len();
                    event.file_to_index = Some(file);
                }
                Err(error) => {
                    tracing::warn!(?error, path = %canonical_path.display(), "failed to inspect RAG file");
                    event.skipped_file_count = 1;
                    if stored_record.is_some() {
                        event.stale_paths.push(normalized_path);
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

async fn spawn_streaming_reindex_tasks(
    join_set: &mut JoinSet<Result<IndexedPreparedFile>>,
    pending_files: &mut VecDeque<PreparedRagFile>,
    semaphore: &Arc<Semaphore>,
    metadata_path: &Path,
    resolved: &ResolvedRagConfig,
    client: &HttpClient,
    vector_store: &RagVectorStore,
) -> Result<()> {
    while let Some(file) = pending_files.pop_front() {
        let Ok(permit) = semaphore.clone().try_acquire_owned() else {
            pending_files.push_front(file);
            break;
        };
        write_metadata_records(
            metadata_path,
            vec![file.record.clone()],
            "failed to join streamed RAG metadata stage task",
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
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_guard: Option<(&Arc<AtomicU64>, u64)>,
    scanned_file_count: usize,
    completed_file_count: usize,
    pending_file_count: usize,
) {
    let total_file_count = completed_file_count.saturating_add(pending_file_count);
    set_runtime_status(
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
        None,
    )
    .await;
}

async fn write_metadata_records(
    metadata_path: &Path,
    records: Vec<RagIndexedFileRecord>,
    join_error_message: &'static str,
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }

    tokio::task::spawn_blocking({
        let metadata_path = metadata_path.to_path_buf();
        move || upsert_metadata_records(&metadata_path, &records)
    })
    .await
    .context(join_error_message)??;
    Ok(())
}

pub(super) async fn execute_path_update_plans(
    database_path: &Path,
    metadata_path: &Path,
    resolved: &ResolvedRagConfig,
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_guard: Option<(&Arc<AtomicU64>, u64)>,
    plans: Vec<(PathBuf, PathUpdatePlan)>,
) -> Result<()> {
    if plans.is_empty() {
        return Ok(());
    }

    let client = build_embedding_client()?;
    let mut vector_store = RagVectorStore::open(database_path).await?;
    let mut delete_exact_paths = Vec::new();
    let mut delete_prefix_paths = Vec::new();
    let mut staged_cleanup_paths = Vec::new();
    let mut metadata_refreshes = Vec::new();
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
            PathUpdatePlan::RefreshMetadata {
                record,
                clear_staged,
            } => {
                if clear_staged {
                    staged_cleanup_paths.push(record.absolute_path.clone());
                }
                metadata_refreshes.push(record);
            }
            PathUpdatePlan::Reindex(file) => files_to_index.push(file),
        }
    }

    if !delete_exact_paths.is_empty() {
        delete_vectors_for_exact_paths(&mut vector_store, &delete_exact_paths).await?;
        tokio::task::spawn_blocking({
            let metadata_path = metadata_path.to_path_buf();
            let delete_exact_paths = delete_exact_paths.clone();
            move || delete_metadata_for_paths(&metadata_path, &delete_exact_paths, false)
        })
        .await
        .context("failed to join batched RAG metadata delete task")??;
    }

    if !delete_prefix_paths.is_empty() {
        let resolved_delete_paths = tokio::task::spawn_blocking({
            let metadata_path = metadata_path.to_path_buf();
            let delete_prefix_paths = delete_prefix_paths.clone();
            move || load_metadata_paths_for_prefixes(&metadata_path, &delete_prefix_paths)
        })
        .await
        .context("failed to join descendant RAG metadata lookup task")??;
        delete_vectors_for_exact_paths(&mut vector_store, &resolved_delete_paths).await?;
        tokio::task::spawn_blocking({
            let metadata_path = metadata_path.to_path_buf();
            move || delete_metadata_for_paths(&metadata_path, &resolved_delete_paths, false)
        })
        .await
        .context("failed to join descendant RAG metadata delete task")??;
    }

    if !staged_cleanup_paths.is_empty() {
        delete_vectors_for_exact_paths_in_state(
            &mut vector_store,
            &staged_cleanup_paths,
            RagChunkState::Staged,
        )
        .await?;
    }

    if !metadata_refreshes.is_empty() {
        tokio::task::spawn_blocking({
            let metadata_path = metadata_path.to_path_buf();
            let metadata_refreshes = metadata_refreshes.clone();
            move || upsert_metadata_records(&metadata_path, &metadata_refreshes)
        })
        .await
        .context("failed to join batched RAG metadata refresh task")??;
    }

    if !files_to_index.is_empty() {
        set_runtime_status(
            runtime_status,
            runtime_guard,
            RagRuntimePhase::Indexing,
            RuntimeProgress {
                scanned_file_count: files_to_index.len(),
                total_file_count: files_to_index.len(),
                pending_file_count: files_to_index.len(),
                ..RuntimeProgress::default()
            },
            None,
        )
        .await;
        for (index, file) in files_to_index.iter().enumerate() {
            write_metadata_records(
                metadata_path,
                vec![file.record.clone()],
                "failed to join batched RAG metadata stage task",
            )
            .await?;
            reindex_prepared_file(&mut vector_store, metadata_path, resolved, &client, file)
                .await?;
            let remaining = files_to_index.len().saturating_sub(index + 1);
            set_runtime_status(
                runtime_status,
                runtime_guard,
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
                None,
            )
            .await;
        }
    }

    vector_store.ensure_index().await?;
    Ok(())
}

async fn reindex_prepared_file(
    vector_store: &mut RagVectorStore,
    metadata_path: &Path,
    resolved: &ResolvedRagConfig,
    client: &HttpClient,
    file: &PreparedRagFile,
) -> Result<()> {
    let indexed = index_prepared_file_for_store(vector_store, resolved, client, file).await?;
    persist_indexed_file(vector_store, metadata_path, indexed).await
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
        .load_chunk_vectors_for_file(&file.record.absolute_path, RagChunkState::Active)
        .await?;
    let chunks = build_chunks_for_prepared_file(&file);
    let vectors =
        resolve_chunk_vectors(resolved, client, vector_store, &chunks, &reusable_vectors).await?;
    Ok(IndexedPreparedFile {
        file,
        chunks,
        vectors,
    })
}

async fn persist_indexed_file(
    vector_store: &mut RagVectorStore,
    metadata_path: &Path,
    indexed: IndexedPreparedFile,
) -> Result<()> {
    let IndexedPreparedFile {
        file,
        chunks,
        vectors,
    } = indexed;

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
        let metadata_path = metadata_path.to_path_buf();
        move || {
            let finalized_record = finalize_metadata_record(&file);
            upsert_metadata_records(&metadata_path, &[finalized_record])?;
            replace_lexical_chunks_for_file(&metadata_path, &file)
        }
    })
    .await
    .context("failed to join RAG metadata finalize task")??;
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
        let (vectors, embedding_stats) =
            request_embeddings_with_stats(client, &resolved.provider, inputs).await?;
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
            refresh_metadata,
            clear_staged,
        } => {
            if refresh_metadata {
                Ok(PathUpdatePlan::RefreshMetadata {
                    record,
                    clear_staged,
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

    let file_metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to read file metadata: {}", path.display()))?;
    if file_metadata.len() > MAX_TEXT_FILE_BYTES {
        return Ok(InspectPathOutcome::Skip);
    }

    let size_bytes = i64::try_from(file_metadata.len()).with_context(|| {
        format!(
            "file is too large to track in metadata store: {}",
            path.display()
        )
    })?;
    let modified_at_ms = file_metadata
        .modified()
        .ok()
        .and_then(system_time_to_unix_ms);
    let active_version = stored_record.and_then(|record| record.active.as_ref());
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))?;
    let extracted = extract_document_from_bytes(path, &bytes)?;
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
                    stored_record.refresh_active_metadata(modified_at_ms, size_bytes)
                } else {
                    stored_record.clone()
                },
                refresh_metadata: clear_staged,
                clear_staged,
            });
        }
    }

    let content_md5 = format!("{:x}", md5::compute(&bytes));

    if let Some(stored_record) = stored_record {
        if same_index_target
            && active_version
                .map(|version| version.content_md5 == content_md5)
                .unwrap_or(false)
        {
            let clear_staged = stored_record.has_pending();
            return Ok(InspectPathOutcome::Unchanged {
                record: stored_record.refresh_active_metadata(modified_at_ms, size_bytes),
                refresh_metadata: true,
                clear_staged,
            });
        }
    }

    let chunks =
        split_extracted_document_for_path(path, &extracted, CHUNK_MAX_CHARS, CHUNK_OVERLAP_CHARS)?;
    if chunks.is_empty() {
        return Ok(InspectPathOutcome::Skip);
    }

    let version_id = make_chunk_version_id(path);
    let pending_version = RagIndexedFileVersion {
        version_id: version_id.clone(),
        content_md5,
        modified_at_ms,
        size_bytes,
        chunk_count: i64::try_from(chunks.len()).context("chunk count exceeds i64 range")?,
        indexed_at_ms: now_unix_ms(),
    };

    Ok(InspectPathOutcome::Reindex(PreparedRagFile {
        record: RagIndexedFileRecord {
            source_root: source_root_string,
            absolute_path: normalize_path_string(path),
            relative_path,
            embedding_fingerprint: resolved.embedding_fingerprint.clone(),
            extractor_fingerprint: extracted.extractor_fingerprint.clone(),
            active: stored_record.and_then(|record| record.active.clone()),
            pending: Some(pending_version),
        },
        chunks,
        version_id,
    }))
}

#[cfg(test)]
pub(super) fn collect_chunks_for_path(
    resolved: &ResolvedRagConfig,
    path: &Path,
) -> Result<Vec<RagChunk>> {
    match inspect_path_for_index(resolved, path, None)? {
        InspectPathOutcome::Skip => Ok(Vec::new()),
        InspectPathOutcome::Unchanged { .. } => Ok(Vec::new()),
        InspectPathOutcome::Reindex(file) => Ok(build_chunks_for_prepared_file(&file)),
    }
}

fn build_chunks_for_prepared_file(file: &PreparedRagFile) -> Vec<RagChunk> {
    file.chunks
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

fn make_chunk_version_id(path: &Path) -> String {
    let path_hash = format!("{:x}", md5::compute(normalize_path_string(path)));
    format!("{:x}-{path_hash}", now_unix_ms().max(0))
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
        finished_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    }
}
