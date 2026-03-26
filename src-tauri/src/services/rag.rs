use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use arrow_array::{
    types::Float32Type, Array, FixedSizeListArray, Float32Array, Int32Array, RecordBatch,
    RecordBatchIterator, RecordBatchReader, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use lancedb::{
    connect,
    index::Index,
    query::{ExecutableQuery, QueryBase, Select},
    table::Table,
    Connection as LanceConnection,
};
use notify::{event::ModifyKind, Event, EventKind, RecursiveMode, Watcher};
use reqwest::Client as HttpClient;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use text_splitter::{Characters, ChunkCharIndex, ChunkConfig, MarkdownSplitter, TextSplitter};
use tokio::{
    sync::{mpsc, Mutex as AsyncMutex, RwLock as AsyncRwLock, Semaphore},
    task::{JoinHandle, JoinSet},
};

use crate::domain::{
    rag::{RagRuntimePhase, RagRuntimeStatus, RagScanResult},
    settings::{LlmProviderConfig, LlmSettings, RagSettings},
};
use crate::infrastructure::openai_compatible::{normalize_base_url, OpenAiCompatibleClient};
use crate::services::document_extract::{
    extract_document_text_from_bytes, is_supported_document_file, uses_markdown_chunking,
};

pub(crate) const RAG_DB_DIR_NAME: &str = "rag-lancedb";
const RAG_METADATA_DB_FILE_NAME: &str = "rag-metadata.sqlite3";
pub(crate) const RAG_TABLE_NAME: &str = "chunks";
const MAX_TEXT_FILE_BYTES: u64 = 50 * 1024 * 1024;
const CHUNK_MAX_CHARS: usize = 1_200;
const CHUNK_OVERLAP_CHARS: usize = 200;
const MARKDOWN_CHUNK_TARGET_CHARS: usize = 350;
const MARKDOWN_CHUNK_HARD_MAX_CHARS: usize = 550;
const MARKDOWN_CHUNK_OVERLAP_CHARS: usize = 80;
const EMBEDDING_BATCH_SIZE_MIN: usize = 1;
const EMBEDDING_BATCH_SIZE_DEFAULT: usize = 8;
const EMBEDDING_BATCH_SIZE_MAX: usize = 128;
const EMBEDDING_BATCH_CHAR_BUDGET: usize = CHUNK_MAX_CHARS * EMBEDDING_BATCH_SIZE_DEFAULT;
const EMBEDDING_BATCH_GROWTH_SUCCESS_STREAK: usize = 3;
const EMBEDDING_BATCH_GROWTH_DIVISOR: usize = 4;
const EMBEDDING_BATCH_COOLDOWN_ROUNDS: usize = 2;
const EMBEDDING_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const WATCH_DEBOUNCE_WINDOW: Duration = Duration::from_millis(250);
const MAX_DELETE_FILTER_PATHS: usize = 128;
const MAX_METADATA_BATCH_PATHS: usize = 256;
const MAX_TEXT_FINGERPRINT_FILTERS: usize = 256;
const MAX_STREAMING_REINDEX_CONCURRENCY: usize = 4;
const VECTOR_INDEX_REBUILD_MIN_DIRTY_CHUNKS: usize = 256;
const VECTOR_INDEX_REBUILD_MIN_DIRTY_DELETES: usize = 8;

#[derive(Clone)]
pub struct RagIndexService {
    data_dir: PathBuf,
    runtime: Arc<AsyncRwLock<Option<JoinHandle<()>>>>,
    runtime_inputs: Arc<AsyncRwLock<Option<RagRuntimeInputs>>>,
    runtime_status: Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_generation: Arc<AtomicU64>,
    storage_lock: Arc<AsyncMutex<()>>,
}

#[derive(Debug, Clone)]
struct RagChunk {
    id: String,
    source_root: String,
    absolute_path: String,
    version_id: String,
    embedding_fingerprint: String,
    chunk_state: RagChunkState,
    chunk_index: i32,
    line_start: i32,
    line_end: i32,
    paragraph_line_start: i32,
    heading_path: Vec<String>,
    chunk_reuse_key: String,
    text_fingerprint: String,
    text: String,
}

#[derive(Debug, Clone)]
struct ResolvedRagConfig {
    source_roots: Vec<PathBuf>,
    ignore_globs: Arc<Option<GlobSet>>,
    embedding_fingerprint: String,
    provider: LlmProviderConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RagRuntimeInputs {
    settings: RagSettings,
    embedding_provider: Option<LlmProviderConfig>,
}

#[derive(Clone)]
struct RagRuntimeContext {
    runtime_status: Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_generation: Arc<AtomicU64>,
    storage_lock: Arc<AsyncMutex<()>>,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RagRuntimeStartMode {
    ReuseIndex,
    RebuildIndex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EmbeddingTargetIdentity {
    StableModel {
        namespace: &'static str,
        model_identity: String,
    },
    EndpointBound {
        normalized_base_url: String,
        model_identity: String,
    },
}

impl RagRuntimeInputs {
    fn from_settings(settings: &RagSettings, llm_settings: &LlmSettings) -> Self {
        let embedding_provider =
            settings
                .embedding_provider_id
                .as_deref()
                .and_then(|provider_id| {
                    llm_settings
                        .providers
                        .iter()
                        .find(|provider| provider.id == provider_id)
                        .cloned()
                });

        Self {
            settings: settings.clone(),
            embedding_provider,
        }
    }

    fn source_directory_set(&self) -> BTreeSet<String> {
        self.settings
            .source_directories
            .iter()
            .map(|directory| normalize_runtime_source_directory(directory))
            .filter(|directory| !directory.is_empty())
            .collect()
    }

    fn ignore_glob_set(&self) -> BTreeSet<String> {
        self.settings
            .ignore_globs
            .iter()
            .map(|pattern| pattern.trim().to_string())
            .filter(|pattern| !pattern.is_empty())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RagIndexedFileRecord {
    source_root: String,
    absolute_path: String,
    relative_path: String,
    embedding_fingerprint: String,
    active: Option<RagIndexedFileVersion>,
    pending: Option<RagIndexedFileVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RagIndexedFileVersion {
    version_id: String,
    content_md5: String,
    modified_at_ms: Option<i64>,
    size_bytes: i64,
    chunk_count: i64,
    indexed_at_ms: i64,
}

#[derive(Debug, Clone)]
struct PreparedRagFile {
    record: RagIndexedFileRecord,
    chunks: Vec<PreparedRagChunk>,
    version_id: String,
}

#[derive(Debug, Clone)]
struct PreparedRagChunk {
    chunk_index: i32,
    line_start: i32,
    line_end: i32,
    paragraph_line_start: i32,
    heading_path: Vec<String>,
    chunk_reuse_key: String,
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RagChunkState {
    Staged,
    Active,
}

#[derive(Debug, Clone)]
struct StoredChunkVector {
    chunk_reuse_key: String,
    vector: Vec<f32>,
}

#[derive(Debug, Clone)]
struct CachedTextVector {
    text_fingerprint: String,
    text: String,
    vector: Vec<f32>,
}

#[derive(Clone)]
struct RagVectorStore {
    db: LanceConnection,
    table: Option<Table>,
    created_table: bool,
    index_dirty: bool,
    dirty_chunk_count: usize,
    dirty_delete_count: usize,
}

#[derive(Debug)]
struct TextLine {
    start_byte: usize,
    end_byte: usize,
    content: String,
}

#[derive(Debug)]
struct TextLayout {
    lines: Vec<TextLine>,
    paragraph_start_lines: Vec<usize>,
    heading_path_by_line: Vec<Vec<String>>,
}

#[derive(Debug)]
struct ChunkMetadata {
    paragraph_start_line_index: usize,
    heading_path: Vec<String>,
}

#[derive(Debug, Clone)]
struct ChunkByteRange {
    start_byte: usize,
    end_byte: usize,
}

#[derive(Debug, Clone)]
struct SemanticBlock {
    start_byte: usize,
    end_byte: usize,
    char_count: usize,
    heading_path: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MarkdownFence {
    marker: char,
    length: usize,
}

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
struct RebuildScanEvent {
    scanned_file_count: usize,
    indexed_file_count: usize,
    skipped_file_count: usize,
    chunk_count: usize,
    staged_cleanup_paths: Vec<String>,
    stale_paths: Vec<String>,
    metadata_refresh: Option<RagIndexedFileRecord>,
    file_to_index: Option<PreparedRagFile>,
}

#[derive(Debug)]
struct IndexedPreparedFile {
    file: PreparedRagFile,
    chunks: Vec<RagChunk>,
    vectors: Vec<Vec<f32>>,
}

#[derive(Debug)]
enum InspectPathOutcome {
    Skip,
    Unchanged {
        record: RagIndexedFileRecord,
        refresh_metadata: bool,
        clear_staged: bool,
    },
    Reindex(PreparedRagFile),
}

#[derive(Debug)]
enum PathUpdatePlan {
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

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingItem {
    embedding: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmbeddingRequestStats {
    largest_successful_batch_size: usize,
    split_retry_count: usize,
}

impl EmbeddingRequestStats {
    fn record_success(&mut self, batch_size: usize) {
        self.largest_successful_batch_size = self.largest_successful_batch_size.max(batch_size);
    }

    fn record_split_retry(&mut self) {
        self.split_retry_count += 1;
    }

    fn had_to_split(self) -> bool {
        self.split_retry_count > 0
    }
}

#[derive(Debug, Clone, Copy)]
struct EmbeddingBatchPlanner {
    current_size: usize,
    clean_success_streak: usize,
    cooldown_rounds: usize,
}

impl Default for EmbeddingBatchPlanner {
    fn default() -> Self {
        Self {
            current_size: EMBEDDING_BATCH_SIZE_DEFAULT,
            clean_success_streak: 0,
            cooldown_rounds: 0,
        }
    }
}

impl EmbeddingBatchPlanner {
    fn next_batch_end(self, inputs: &[String], start: usize) -> usize {
        let remaining = inputs.len().saturating_sub(start);
        if remaining == 0 {
            return start;
        }

        let item_limit = remaining.min(
            self.current_size
                .clamp(EMBEDDING_BATCH_SIZE_MIN, EMBEDDING_BATCH_SIZE_MAX),
        );
        let mut total_chars = 0usize;
        let mut end = start;

        while end < inputs.len() && end - start < item_limit {
            let input_chars = inputs[end].chars().count().max(1);
            if end > start && total_chars.saturating_add(input_chars) > EMBEDDING_BATCH_CHAR_BUDGET
            {
                break;
            }

            total_chars = total_chars.saturating_add(input_chars);
            end += 1;
            if total_chars >= EMBEDDING_BATCH_CHAR_BUDGET {
                break;
            }
        }

        end
    }

    fn record_success(&mut self, requested_batch_size: usize, stats: EmbeddingRequestStats) {
        if stats.had_to_split() {
            self.current_size = stats
                .largest_successful_batch_size
                .max(EMBEDDING_BATCH_SIZE_MIN)
                .clamp(EMBEDDING_BATCH_SIZE_MIN, EMBEDDING_BATCH_SIZE_MAX);
            self.clean_success_streak = 0;
            self.cooldown_rounds = EMBEDDING_BATCH_COOLDOWN_ROUNDS;
            return;
        }

        if self.cooldown_rounds > 0 {
            self.cooldown_rounds -= 1;
            self.clean_success_streak = 0;
            return;
        }

        if requested_batch_size
            < self
                .current_size
                .clamp(EMBEDDING_BATCH_SIZE_MIN, EMBEDDING_BATCH_SIZE_MAX)
        {
            return;
        }

        self.clean_success_streak += 1;
        if self.clean_success_streak < EMBEDDING_BATCH_GROWTH_SUCCESS_STREAK {
            return;
        }

        self.clean_success_streak = 0;
        let growth = (self.current_size / EMBEDDING_BATCH_GROWTH_DIVISOR).max(1);
        self.current_size = self
            .current_size
            .saturating_add(growth)
            .clamp(EMBEDDING_BATCH_SIZE_MIN, EMBEDDING_BATCH_SIZE_MAX);
    }
}

impl RagIndexedFileRecord {
    fn current_chunk_count(&self) -> usize {
        self.active
            .as_ref()
            .map(|version| version.chunk_count.max(0) as usize)
            .unwrap_or_default()
    }

    fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn refresh_active_metadata(&self, modified_at_ms: Option<i64>, size_bytes: i64) -> Self {
        let mut refreshed = self.clone();
        if let Some(active) = refreshed.active.as_mut() {
            active.modified_at_ms = modified_at_ms;
            active.size_bytes = size_bytes;
        }
        refreshed.pending = None;
        refreshed
    }
}

impl RagChunkState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Active => "active",
        }
    }
}

impl RagVectorStore {
    async fn open(database_path: &Path) -> Result<Self> {
        let db = connect(database_path.to_string_lossy().as_ref())
            .execute()
            .await
            .context("failed to open LanceDB database")?;
        let table = open_existing_rag_table(&db).await?;
        Ok(Self {
            db,
            table,
            created_table: false,
            index_dirty: false,
            dirty_chunk_count: 0,
            dirty_delete_count: 0,
        })
    }

    fn mark_index_dirty_for_chunks(&mut self, chunk_count: usize) {
        self.index_dirty = true;
        self.dirty_chunk_count = self.dirty_chunk_count.saturating_add(chunk_count);
    }

    fn mark_index_dirty_for_delete(&mut self) {
        self.index_dirty = true;
        self.dirty_delete_count = self.dirty_delete_count.saturating_add(1);
    }

    fn should_rebuild_index(&self) -> bool {
        self.created_table
            || self.dirty_chunk_count >= VECTOR_INDEX_REBUILD_MIN_DIRTY_CHUNKS
            || self.dirty_delete_count >= VECTOR_INDEX_REBUILD_MIN_DIRTY_DELETES
    }

    async fn add_chunks(&mut self, chunks: &[RagChunk], vectors: &[Vec<f32>]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        let batch_reader = build_record_batch_reader(chunks, vectors)?;
        if let Some(table) = &self.table {
            table.add(batch_reader).execute().await.with_context(|| {
                format!("failed to append {} chunk(s) into LanceDB", chunks.len())
            })?;
        } else {
            let created = self
                .db
                .create_table(RAG_TABLE_NAME, batch_reader)
                .execute()
                .await
                .context("failed to create RAG LanceDB table")?;
            self.table = Some(created);
            self.created_table = true;
        }
        self.mark_index_dirty_for_chunks(chunks.len());
        Ok(())
    }

    async fn delete_where(&mut self, filter: &str) -> Result<()> {
        let Some(table) = &self.table else {
            return Ok(());
        };
        table
            .delete(filter)
            .await
            .with_context(|| format!("failed to delete LanceDB rows with filter: {filter}"))?;
        self.mark_index_dirty_for_delete();
        Ok(())
    }

    async fn update_where(&mut self, filter: &str, chunk_state: RagChunkState) -> Result<()> {
        let Some(table) = &self.table else {
            return Ok(());
        };
        table
            .update()
            .only_if(filter)
            .column("chunk_state", format!("'{}'", chunk_state.as_str()))
            .execute()
            .await
            .with_context(|| {
                format!(
                    "failed to update LanceDB chunk_state to {} with filter: {filter}",
                    chunk_state.as_str()
                )
            })?;
        Ok(())
    }

    async fn load_chunk_vectors_for_file(
        &self,
        absolute_path: &str,
        chunk_state: RagChunkState,
    ) -> Result<HashMap<String, Vec<f32>>> {
        let Some(table) = &self.table else {
            return Ok(HashMap::new());
        };

        let filter = format!(
            "absolute_path = '{}' AND chunk_state = '{}'",
            escape_sql_literal(absolute_path),
            chunk_state.as_str()
        );
        let stream = table
            .query()
            .only_if(filter.as_str())
            .select(Select::columns(&["chunk_reuse_key", "vector"]))
            .execute()
            .await
            .with_context(|| {
                format!("failed to load existing chunk vectors for path: {absolute_path}")
            })?;
        let batches = stream
            .try_collect::<Vec<_>>()
            .await
            .context("failed to collect chunk reuse batches")?;

        let mut vectors = HashMap::new();
        for batch in batches {
            for chunk in parse_chunk_vector_batch(&batch)? {
                vectors.entry(chunk.chunk_reuse_key).or_insert(chunk.vector);
            }
        }
        Ok(vectors)
    }

    async fn load_cached_vectors_for_texts(
        &self,
        embedding_fingerprint: &str,
        texts: &[String],
    ) -> Result<HashMap<String, Vec<f32>>> {
        let Some(table) = &self.table else {
            return Ok(HashMap::new());
        };
        if texts.is_empty() {
            return Ok(HashMap::new());
        }

        let requested_texts = texts.iter().cloned().collect::<HashSet<_>>();
        let text_fingerprints = texts
            .iter()
            .map(|text| text_fingerprint(text))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut cached_vectors = HashMap::new();

        for text_fingerprint_batch in text_fingerprints.chunks(MAX_TEXT_FINGERPRINT_FILTERS) {
            let filter = format!(
                "embedding_fingerprint = '{}' AND text_fingerprint IN ({})",
                escape_sql_literal(embedding_fingerprint),
                text_fingerprint_batch
                    .iter()
                    .map(|text_fingerprint| format!("'{}'", escape_sql_literal(text_fingerprint)))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let stream = table
                .query()
                .only_if(filter.as_str())
                .select(Select::columns(&["text_fingerprint", "text", "vector"]))
                .execute()
                .await
                .with_context(|| {
                    format!(
                        "failed to load cached RAG vectors for embedding fingerprint: {embedding_fingerprint}"
                    )
                })?;
            let batches = stream
                .try_collect::<Vec<_>>()
                .await
                .context("failed to collect cached RAG vector batches")?;

            for batch in batches {
                for cached in parse_text_vector_batch(&batch)? {
                    if text_fingerprint(&cached.text) != cached.text_fingerprint {
                        continue;
                    }
                    if !requested_texts.contains(&cached.text) {
                        continue;
                    }
                    cached_vectors.entry(cached.text).or_insert(cached.vector);
                }
            }
        }

        Ok(cached_vectors)
    }

    async fn ensure_index(&mut self) -> Result<()> {
        if !self.index_dirty {
            return Ok(());
        }
        if !self.should_rebuild_index() {
            return Ok(());
        }
        if let Some(table) = &self.table {
            if let Err(error) = table.create_index(&["vector"], Index::Auto).execute().await {
                let error =
                    anyhow::Error::new(error).context("failed to create LanceDB vector index");
                if can_skip_vector_index_build(&error) {
                    tracing::info!(
                        ?error,
                        "skipping LanceDB vector index build because current corpus is too small"
                    );
                } else {
                    return Err(error);
                }
            }
        }
        self.created_table = false;
        self.index_dirty = false;
        self.dirty_chunk_count = 0;
        self.dirty_delete_count = 0;
        Ok(())
    }
}

fn can_skip_vector_index_build(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        let message = source.to_string();
        message.contains("Not enough rows to train PQ")
            || (message.contains("Requires 256 rows") && message.contains("available"))
    })
}

impl RagIndexService {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
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
        let runtime_context = RagRuntimeContext {
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
                    &runtime_status,
                    &runtime_generation,
                    generation,
                    RagRuntimePhase::Error,
                    RuntimeProgress::default(),
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
        rebuild_index_locked(&self.data_dir, &resolved, &runtime_status, None).await
    }
}

async fn run_watch_loop(
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
                    &runtime_context.runtime_status,
                    &runtime_context.runtime_generation,
                    runtime_context.generation,
                    RagRuntimePhase::Idle,
                    RuntimeProgress::default(),
                    None,
                )
                .await;
                return Ok(());
            }
            set_runtime_status_for_generation(
                &runtime_context.runtime_status,
                &runtime_context.runtime_generation,
                runtime_context.generation,
                RagRuntimePhase::Error,
                RuntimeProgress::default(),
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
                &runtime_context.runtime_status,
                &runtime_context.runtime_generation,
                runtime_context.generation,
                RagRuntimePhase::Error,
                RuntimeProgress::default(),
                Some(error.to_string()),
            )
            .await;
            tracing::warn!(?error, "failed to process RAG watcher events");
        }
    }
}

pub(crate) fn rag_database_path(config_dir: &Path) -> PathBuf {
    config_dir.join(RAG_DB_DIR_NAME)
}

pub(crate) fn rag_metadata_database_path(data_dir: &Path) -> PathBuf {
    data_dir.join(RAG_METADATA_DB_FILE_NAME)
}

async fn process_event_batch(
    data_dir: &Path,
    resolved: &ResolvedRagConfig,
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
        rebuild_index_locked(data_dir, resolved, runtime_status, runtime_guard).await?;
        return Ok(());
    }

    if !changed_paths.is_empty() {
        set_runtime_status(
            runtime_status,
            runtime_guard,
            RagRuntimePhase::Scanning,
            RuntimeProgress {
                scanned_file_count: changed_paths.len(),
                total_file_count: changed_paths.len(),
                ..RuntimeProgress::default()
            },
            None,
        )
        .await;
    }

    if changed_paths.is_empty() {
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

    let changed_paths = changed_paths.into_iter().collect::<Vec<_>>();
    let stored_records = tokio::task::spawn_blocking({
        let metadata_path = metadata_path.clone();
        let absolute_paths = changed_paths
            .iter()
            .map(|path| normalize_path_string(path))
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
                let normalized_path = normalize_path_string(&path);
                let stored_record = stored_records.get(&normalized_path).cloned();
                build_path_update_plan(&resolved_for_batch, &path, stored_record)
                    .map(|plan| (path, plan))
            })
            .collect::<Result<Vec<_>>>()
    })
    .await
    .context("failed to join RAG path batch planning task")??;

    execute_path_update_plans(
        &database_path,
        &metadata_path,
        resolved,
        runtime_status,
        runtime_guard,
        plans,
    )
    .await?;

    set_runtime_status(
        runtime_status,
        runtime_guard,
        RagRuntimePhase::Idle,
        RuntimeProgress::default(),
        None,
    )
    .await;
    Ok(())
}

fn should_force_full_rescan_for_event(event: &Event) -> bool {
    matches!(event.kind, EventKind::Any)
        || (matches!(event.kind, EventKind::Other) && event.paths.is_empty())
}

fn should_ignore_event_for_indexing(event: &Event) -> bool {
    matches!(event.kind, EventKind::Access(_))
        || matches!(event.kind, EventKind::Modify(ModifyKind::Metadata(_)))
}

fn should_force_full_rescan_for_existing_directory(kind: EventKind) -> bool {
    matches!(kind, EventKind::Modify(ModifyKind::Name(_)))
}

fn classify_rag_runtime_start(
    previous: Option<&RagRuntimeInputs>,
    next: &RagRuntimeInputs,
) -> RagRuntimeStartMode {
    let Some(previous) = previous else {
        return RagRuntimeStartMode::ReuseIndex;
    };

    let source_directories_changed = previous.source_directory_set() != next.source_directory_set();
    let ignore_globs_changed = previous.ignore_glob_set() != next.ignore_glob_set();
    let embedding_target_changed = rag_embedding_target_changed(previous, next);

    if source_directories_changed || ignore_globs_changed || embedding_target_changed {
        RagRuntimeStartMode::RebuildIndex
    } else {
        RagRuntimeStartMode::ReuseIndex
    }
}

fn rag_embedding_target_changed(previous: &RagRuntimeInputs, next: &RagRuntimeInputs) -> bool {
    effective_embedding_target(previous) != effective_embedding_target(next)
}

fn rag_settings_disabled(settings: &RagSettings) -> bool {
    settings
        .source_directories
        .iter()
        .all(|directory| directory.trim().is_empty())
        && settings
            .embedding_provider_id
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
}

fn effective_embedding_target(inputs: &RagRuntimeInputs) -> Option<EmbeddingTargetIdentity> {
    let provider = inputs.embedding_provider.as_ref()?;
    Some(infer_embedding_target_identity(
        provider.base_url.trim().trim_end_matches('/'),
        provider.model_name(),
        provider.model_identity_hint.as_deref(),
    ))
}

fn infer_embedding_target_identity(
    normalized_base_url: &str,
    model_name: &str,
    model_identity_hint: Option<&str>,
) -> EmbeddingTargetIdentity {
    if let Some(identity_hint) = model_identity_hint.and_then(parse_stable_model_identity_hint) {
        return identity_hint;
    }

    if let Some(digest) = extract_stable_model_digest(model_name) {
        return EmbeddingTargetIdentity::StableModel {
            namespace: "digest",
            model_identity: digest,
        };
    }

    if let Some(namespace) = managed_embedding_model_namespace(normalized_base_url) {
        return EmbeddingTargetIdentity::StableModel {
            namespace,
            model_identity: model_name.to_string(),
        };
    }

    EmbeddingTargetIdentity::EndpointBound {
        normalized_base_url: normalized_base_url.to_string(),
        model_identity: model_name.to_string(),
    }
}

fn parse_stable_model_identity_hint(identity_hint: &str) -> Option<EmbeddingTargetIdentity> {
    let trimmed = identity_hint.trim();
    let digest = trimmed.strip_prefix("digest:")?;
    (!digest.is_empty()).then_some(EmbeddingTargetIdentity::StableModel {
        namespace: "digest",
        model_identity: digest.to_ascii_lowercase(),
    })
}

fn extract_stable_model_digest(model_name: &str) -> Option<String> {
    let normalized_model = model_name.trim().to_ascii_lowercase();
    let marker = "sha256:";
    let start = normalized_model.find(marker)? + marker.len();
    let digest = normalized_model[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_hexdigit())
        .collect::<String>();
    (digest.len() == 64).then_some(format!("{marker}{digest}"))
}

fn managed_embedding_model_namespace(normalized_base_url: &str) -> Option<&'static str> {
    let parsed = reqwest::Url::parse(normalized_base_url).ok()?;
    if parsed.query().is_some() {
        return None;
    }

    let host = parsed.host_str()?.to_ascii_lowercase();
    let path = parsed.path().trim_end_matches('/');
    if host == "api.openai.com" && path == "/v1" {
        Some("openai")
    } else {
        None
    }
}

fn normalize_runtime_source_directory(directory: &str) -> String {
    let trimmed = directory.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Ok(canonical_path) = std::fs::canonicalize(trimmed) {
        return normalize_path_string(&canonical_path);
    }

    trim_trailing_path_separators(trimmed)
}

fn trim_trailing_path_separators(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let normalized = trimmed.replace('\\', "/");
    let without_trailing = normalized.trim_end_matches('/');
    if without_trailing.is_empty() {
        normalized
    } else {
        without_trailing.to_string()
    }
}

async fn initialize_runtime_storage(
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

async fn rebuild_index_locked(
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

fn stream_rebuild_scan(
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

async fn execute_path_update_plans(
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
    let mut delete_descendants_paths = Vec::new();
    let mut staged_cleanup_paths = Vec::new();
    let mut metadata_refreshes = Vec::new();
    let mut files_to_index = Vec::new();

    for (path, plan) in plans {
        match plan {
            PathUpdatePlan::Noop => {}
            PathUpdatePlan::Delete { delete_descendants } => {
                if delete_descendants {
                    delete_descendants_paths.push(normalize_path_string(&path));
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

    for path in &delete_descendants_paths {
        delete_vectors_with_filter(
            &mut vector_store,
            &format!(
                "absolute_path = '{}' OR absolute_path LIKE '{}/%'",
                escape_sql_literal(path),
                escape_sql_literal(path)
            ),
        )
        .await?;
    }
    if !delete_descendants_paths.is_empty() {
        tokio::task::spawn_blocking({
            let metadata_path = metadata_path.to_path_buf();
            let delete_descendants_paths = delete_descendants_paths.clone();
            move || delete_metadata_for_paths(&metadata_path, &delete_descendants_paths, true)
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

    let finalized_record = finalize_metadata_record(&file);
    tokio::task::spawn_blocking({
        let metadata_path = metadata_path.to_path_buf();
        move || upsert_metadata_records(&metadata_path, &[finalized_record])
    })
    .await
    .context("failed to join RAG metadata finalize task")??;
    Ok(())
}

async fn resolve_chunk_vectors(
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

async fn clear_index(database_path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(database_path)
        .await
        .with_context(|| {
            format!(
                "failed to create RAG database directory: {}",
                database_path.display()
            )
        })?;
    let db = connect(database_path.to_string_lossy().as_ref())
        .execute()
        .await
        .context("failed to open LanceDB database for cleanup")?;
    if db
        .table_names()
        .execute()
        .await
        .context("failed to list LanceDB tables for cleanup")?
        .iter()
        .any(|name| name == RAG_TABLE_NAME)
    {
        db.drop_table(RAG_TABLE_NAME, &[])
            .await
            .context("failed to drop existing RAG table")?;
    }

    Ok(())
}

async fn clear_metadata_store(metadata_path: &Path) -> Result<()> {
    tokio::task::spawn_blocking({
        let metadata_path = metadata_path.to_path_buf();
        move || reset_metadata_store(&metadata_path)
    })
    .await
    .context("failed to join RAG metadata cleanup task")??;
    Ok(())
}

pub(crate) fn build_embedding_client() -> Result<HttpClient> {
    HttpClient::builder()
        .timeout(EMBEDDING_REQUEST_TIMEOUT)
        .build()
        .context("failed to build embedding HTTP client")
}

fn resolve_rag_config(
    settings: &RagSettings,
    llm_settings: &LlmSettings,
) -> Result<ResolvedRagConfig> {
    let provider = resolve_embedding_provider(settings, llm_settings)?;
    let embedding_fingerprint = embedding_fingerprint(provider)?;
    if settings.source_directories.is_empty() {
        anyhow::bail!("RAG 至少需要一个扫描目录");
    }

    let mut source_roots = Vec::new();
    let mut seen_roots = HashSet::new();
    for source_directory in &settings.source_directories {
        let root = std::fs::canonicalize(source_directory).with_context(|| {
            format!("failed to resolve RAG source directory: {source_directory}")
        })?;
        if !root.is_dir() {
            anyhow::bail!("RAG source path is not a directory: {}", root.display());
        }
        if seen_roots.insert(root.clone()) {
            source_roots.push(root);
        }
    }

    Ok(ResolvedRagConfig {
        source_roots,
        ignore_globs: Arc::new(build_ignore_glob_set(&settings.ignore_globs)?),
        embedding_fingerprint,
        provider: provider.clone(),
    })
}

pub(crate) fn resolve_embedding_provider<'a>(
    settings: &RagSettings,
    llm_settings: &'a LlmSettings,
) -> Result<&'a LlmProviderConfig> {
    let provider_id = settings
        .embedding_provider_id
        .as_deref()
        .context("RAG 扫描前必须选择一个 embedding provider")?;
    let provider = llm_settings
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .with_context(|| format!("RAG 选择的 embedding provider 不存在: {provider_id}"))?;
    if !provider.has_embedding_model() {
        anyhow::bail!("RAG 只接受启用了 embedding 能力的 provider");
    }
    if provider.base_url.trim().is_empty() {
        anyhow::bail!("RAG embedding provider base URL 不能为空");
    }
    if provider.embedding_model_name().is_none() {
        anyhow::bail!("RAG embedding provider model 不能为空");
    }

    Ok(provider)
}

fn build_path_update_plan(
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

fn inspect_path_for_index(
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
    let same_index_target = stored_record
        .map(|record| {
            record.source_root == source_root_string
                && record.relative_path == relative_path
                && record.embedding_fingerprint == resolved.embedding_fingerprint
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

    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))?;
    let content_md5 = format!("{:x}", md5::compute(&bytes));

    let text = extract_document_text_from_bytes(path, &bytes)?;

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

    let chunks = split_text_for_path(path, &text, CHUNK_MAX_CHARS, CHUNK_OVERLAP_CHARS)?;
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
            active: stored_record.and_then(|record| record.active.clone()),
            pending: Some(pending_version),
        },
        chunks,
        version_id,
    }))
}

#[cfg(test)]
fn collect_chunks_for_path(resolved: &ResolvedRagConfig, path: &Path) -> Result<Vec<RagChunk>> {
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
                "{}#{}:{}:{}:{}",
                file.record.absolute_path,
                file.version_id,
                chunk.chunk_index,
                chunk.line_start,
                chunk.line_end
            ),
            source_root: file.record.source_root.clone(),
            absolute_path: file.record.absolute_path.clone(),
            version_id: file.version_id.clone(),
            embedding_fingerprint: file.record.embedding_fingerprint.clone(),
            chunk_state: RagChunkState::Staged,
            chunk_index: chunk.chunk_index,
            line_start: chunk.line_start,
            line_end: chunk.line_end,
            paragraph_line_start: chunk.paragraph_line_start,
            heading_path: chunk.heading_path.clone(),
            chunk_reuse_key: chunk.chunk_reuse_key.clone(),
            text_fingerprint: text_fingerprint(&chunk.text),
            text: chunk.text.clone(),
        })
        .collect()
}

fn build_chunk_config(capacity: usize, overlap: usize) -> Result<ChunkConfig<Characters>> {
    ChunkConfig::new(capacity)
        .with_overlap(overlap)
        .context("invalid text splitter overlap configuration")
}

fn split_text_for_path(
    path: &Path,
    text: &str,
    capacity: usize,
    overlap: usize,
) -> Result<Vec<PreparedRagChunk>> {
    let layout = build_text_layout(text, uses_markdown_chunking(path));
    if uses_markdown_chunking(path) {
        return split_markdown_text(
            text,
            &layout,
            MARKDOWN_CHUNK_TARGET_CHARS,
            MARKDOWN_CHUNK_HARD_MAX_CHARS,
            MARKDOWN_CHUNK_OVERLAP_CHARS,
        );
    }

    build_chunks_from_offsets(
        TextSplitter::new(build_chunk_config(capacity, overlap)?).chunk_char_indices(text),
        &layout,
    )
}

fn build_chunks_from_offsets<'text>(
    chunks: impl Iterator<Item = ChunkCharIndex<'text>>,
    layout: &TextLayout,
) -> Result<Vec<PreparedRagChunk>> {
    chunks
        .enumerate()
        .filter(|(_, chunk)| !chunk.chunk.is_empty())
        .map(|(chunk_index, chunk)| {
            build_chunk_from_byte_range(
                layout,
                chunk_index,
                chunk.byte_offset,
                chunk.byte_offset.saturating_add(chunk.chunk.len()),
                chunk.chunk,
            )
        })
        .collect()
}

fn build_chunk_from_byte_range(
    layout: &TextLayout,
    chunk_index: usize,
    start_byte: usize,
    end_byte: usize,
    text: &str,
) -> Result<PreparedRagChunk> {
    let start_line_index = line_index_for_offset(layout, start_byte);
    let end_offset = end_byte.saturating_sub(1);
    let end_line_index = line_index_for_offset(layout, end_offset);
    let chunk_metadata = resolve_chunk_metadata(layout, start_line_index, end_line_index)?;

    Ok(PreparedRagChunk {
        chunk_index: i32::try_from(chunk_index).context("chunk index exceeds i32 range")?,
        line_start: i32::try_from(start_line_index + 1)
            .context("chunk start line exceeds i32 range")?,
        line_end: i32::try_from(end_line_index + 1).context("chunk end line exceeds i32 range")?,
        paragraph_line_start: i32::try_from(chunk_metadata.paragraph_start_line_index + 1)
            .context("paragraph start line exceeds i32 range")?,
        chunk_reuse_key: chunk_reuse_key(text, &chunk_metadata.heading_path),
        heading_path: chunk_metadata.heading_path,
        text: text.to_string(),
    })
}

fn split_markdown_text(
    text: &str,
    layout: &TextLayout,
    target_chars: usize,
    hard_max_chars: usize,
    overlap_chars: usize,
) -> Result<Vec<PreparedRagChunk>> {
    let semantic_blocks = collect_markdown_semantic_blocks(layout)?;
    if semantic_blocks.is_empty() {
        return Ok(Vec::new());
    }

    let ranges = pack_markdown_blocks(
        text,
        &semantic_blocks,
        target_chars,
        hard_max_chars,
        overlap_chars,
    )?;

    ranges
        .into_iter()
        .enumerate()
        .map(|(chunk_index, range)| {
            let chunk_text = &text[range.start_byte..range.end_byte];
            build_chunk_from_byte_range(
                layout,
                chunk_index,
                range.start_byte,
                range.end_byte,
                chunk_text,
            )
        })
        .collect()
}

fn collect_markdown_semantic_blocks(layout: &TextLayout) -> Result<Vec<SemanticBlock>> {
    let mut blocks = Vec::new();
    let mut line_index = 0usize;

    while line_index < layout.lines.len() {
        if layout.lines[line_index].content.trim().is_empty() {
            line_index += 1;
            continue;
        }

        let trimmed = layout.lines[line_index].content.trim();
        let end_line_index = if let Some(fence) = parse_markdown_fence(trimmed) {
            find_markdown_fence_end(layout, line_index, fence)
        } else if parse_atx_heading(trimmed).is_some() {
            line_index
        } else if parse_setext_heading(&layout.lines, line_index).is_some() {
            line_index
                .saturating_add(1)
                .min(layout.lines.len().saturating_sub(1))
        } else if parse_markdown_list_item(trimmed).is_some() {
            find_list_item_end(layout, line_index)
        } else {
            find_paragraph_end(layout, line_index)
        };

        blocks.push(build_semantic_block(layout, line_index, end_line_index)?);
        line_index = end_line_index.saturating_add(1);
    }

    Ok(blocks)
}

fn build_semantic_block(
    layout: &TextLayout,
    start_line_index: usize,
    end_line_index: usize,
) -> Result<SemanticBlock> {
    let metadata = resolve_chunk_metadata(layout, start_line_index, end_line_index)?;
    let start_byte = layout
        .lines
        .get(start_line_index)
        .map(|line| line.start_byte)
        .context("missing semantic block start line")?;
    let end_byte = layout
        .lines
        .get(end_line_index)
        .map(|line| line.end_byte)
        .context("missing semantic block end line")?;

    Ok(SemanticBlock {
        start_byte,
        end_byte,
        char_count: semantic_block_char_count(layout, start_line_index, end_line_index),
        heading_path: metadata.heading_path,
    })
}

fn semantic_block_char_count(
    layout: &TextLayout,
    start_line_index: usize,
    end_line_index: usize,
) -> usize {
    layout.lines[start_line_index..=end_line_index]
        .iter()
        .map(|line| {
            line.content.chars().count().saturating_add(
                line.end_byte
                    .saturating_sub(line.start_byte)
                    .saturating_sub(line.content.len()),
            )
        })
        .sum()
}

fn pack_markdown_blocks(
    text: &str,
    blocks: &[SemanticBlock],
    target_chars: usize,
    hard_max_chars: usize,
    overlap_chars: usize,
) -> Result<Vec<ChunkByteRange>> {
    let mut ranges = Vec::new();
    let mut start_index = 0usize;

    while start_index < blocks.len() {
        let block = &blocks[start_index];
        if block.char_count > hard_max_chars {
            ranges.extend(split_oversized_markdown_block(
                text,
                block,
                hard_max_chars,
                overlap_chars,
            )?);
            start_index += 1;
            continue;
        }

        let mut end_index = start_index;
        let heading_path = &block.heading_path;
        while end_index + 1 < blocks.len() {
            let next_block = &blocks[end_index + 1];
            if next_block.char_count > hard_max_chars || next_block.heading_path != *heading_path {
                break;
            }

            let candidate_chars =
                markdown_range_char_count(text, blocks, start_index, end_index + 1);
            if candidate_chars > hard_max_chars {
                break;
            }

            end_index += 1;
            if candidate_chars >= target_chars {
                break;
            }
        }

        ranges.push(ChunkByteRange {
            start_byte: blocks[start_index].start_byte,
            end_byte: blocks[end_index].end_byte,
        });

        if end_index + 1 >= blocks.len() {
            break;
        }

        let next_index = end_index + 1;
        if blocks[next_index].heading_path == *heading_path {
            start_index =
                markdown_overlap_start_index(text, blocks, start_index, end_index, overlap_chars);
        } else {
            start_index = next_index;
        }
    }

    Ok(ranges)
}

fn split_oversized_markdown_block(
    text: &str,
    block: &SemanticBlock,
    hard_max_chars: usize,
    overlap_chars: usize,
) -> Result<Vec<ChunkByteRange>> {
    let block_text = &text[block.start_byte..block.end_byte];
    let splitter = MarkdownSplitter::new(build_chunk_config(hard_max_chars, overlap_chars)?);

    Ok(splitter
        .chunk_char_indices(block_text)
        .filter(|chunk| !chunk.chunk.is_empty())
        .map(|chunk| ChunkByteRange {
            start_byte: block.start_byte.saturating_add(chunk.byte_offset),
            end_byte: block
                .start_byte
                .saturating_add(chunk.byte_offset)
                .saturating_add(chunk.chunk.len()),
        })
        .collect())
}

fn markdown_overlap_start_index(
    text: &str,
    blocks: &[SemanticBlock],
    current_start: usize,
    current_end: usize,
    overlap_chars: usize,
) -> usize {
    if overlap_chars == 0 {
        return current_end.saturating_add(1);
    }

    let mut overlap_start = current_end;
    while overlap_start > current_start
        && blocks[overlap_start - 1].heading_path == blocks[current_end].heading_path
    {
        let overlap_char_count = markdown_byte_range_char_count(
            text,
            blocks[overlap_start - 1].start_byte,
            blocks[current_end].end_byte,
        );
        overlap_start -= 1;
        if overlap_char_count >= overlap_chars {
            break;
        }
    }

    overlap_start.max(current_start.saturating_add(1))
}

fn markdown_range_char_count(
    text: &str,
    blocks: &[SemanticBlock],
    start_index: usize,
    end_index: usize,
) -> usize {
    markdown_byte_range_char_count(
        text,
        blocks[start_index].start_byte,
        blocks[end_index].end_byte,
    )
}

fn markdown_byte_range_char_count(text: &str, start_byte: usize, end_byte: usize) -> usize {
    text[start_byte..end_byte].chars().count()
}

fn find_markdown_fence_end(
    layout: &TextLayout,
    start_line_index: usize,
    fence: MarkdownFence,
) -> usize {
    for line_index in start_line_index.saturating_add(1)..layout.lines.len() {
        if is_markdown_fence_close(layout.lines[line_index].content.trim(), fence) {
            return line_index;
        }
    }

    layout.lines.len().saturating_sub(1)
}

fn find_list_item_end(layout: &TextLayout, start_line_index: usize) -> usize {
    let mut end_line_index = start_line_index;

    for line_index in start_line_index.saturating_add(1)..layout.lines.len() {
        let trimmed = layout.lines[line_index].content.trim();
        if trimmed.is_empty()
            || parse_atx_heading(trimmed).is_some()
            || parse_setext_heading(&layout.lines, line_index).is_some()
            || parse_markdown_fence(trimmed).is_some()
            || parse_markdown_list_item(trimmed).is_some()
        {
            break;
        }
        end_line_index = line_index;
    }

    end_line_index
}

fn find_paragraph_end(layout: &TextLayout, start_line_index: usize) -> usize {
    let mut end_line_index = start_line_index;

    for line_index in start_line_index.saturating_add(1)..layout.lines.len() {
        let trimmed = layout.lines[line_index].content.trim();
        if trimmed.is_empty()
            || parse_atx_heading(trimmed).is_some()
            || parse_setext_heading(&layout.lines, line_index).is_some()
            || parse_markdown_fence(trimmed).is_some()
            || parse_markdown_list_item(trimmed).is_some()
        {
            break;
        }
        end_line_index = line_index;
    }

    end_line_index
}

fn parse_markdown_list_item(trimmed_line: &str) -> Option<()> {
    if ["- ", "* ", "+ "]
        .iter()
        .any(|marker| trimmed_line.starts_with(marker))
    {
        return Some(());
    }

    let digit_count = trimmed_line
        .chars()
        .take_while(|char| char.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }

    let rest = &trimmed_line[digit_count..];
    (rest.starts_with(". ") || rest.starts_with(") ")).then_some(())
}

fn resolve_chunk_metadata(
    layout: &TextLayout,
    start_line_index: usize,
    end_line_index: usize,
) -> Result<ChunkMetadata> {
    let metadata_anchor_line_index =
        first_non_empty_line_index(layout, start_line_index, end_line_index)
            .unwrap_or(start_line_index);
    let paragraph_start_line_index = *layout
        .paragraph_start_lines
        .get(metadata_anchor_line_index)
        .context("missing paragraph start line for chunk")?;

    Ok(ChunkMetadata {
        paragraph_start_line_index,
        heading_path: common_heading_path_for_range(
            layout,
            metadata_anchor_line_index,
            end_line_index,
        )
        .context("missing heading path metadata for chunk")?,
    })
}

fn build_text_layout(text: &str, is_markdown: bool) -> TextLayout {
    let lines = collect_text_lines(text);
    let paragraph_start_lines = build_paragraph_start_lines(&lines);
    let heading_path_by_line = if is_markdown {
        build_markdown_heading_paths(&lines)
    } else {
        vec![Vec::new(); lines.len()]
    };

    TextLayout {
        lines,
        paragraph_start_lines,
        heading_path_by_line,
    }
}

fn collect_text_lines(text: &str) -> Vec<TextLine> {
    if text.is_empty() {
        return vec![TextLine {
            start_byte: 0,
            end_byte: 0,
            content: String::new(),
        }];
    }

    let mut lines = Vec::new();
    let mut start_byte = 0usize;
    for segment in text.split_inclusive('\n') {
        let end_byte = start_byte + segment.len();
        lines.push(TextLine {
            start_byte,
            end_byte,
            content: segment.trim_end_matches(['\r', '\n']).to_string(),
        });
        start_byte = end_byte;
    }

    if !text.ends_with('\n') {
        return lines;
    }

    lines
}

fn build_paragraph_start_lines(lines: &[TextLine]) -> Vec<usize> {
    let mut paragraph_start_lines = Vec::with_capacity(lines.len());
    let mut current_start = 0usize;
    let mut in_paragraph = false;

    for (index, line) in lines.iter().enumerate() {
        if line.content.trim().is_empty() {
            paragraph_start_lines.push(index);
            in_paragraph = false;
            current_start = index.saturating_add(1);
            continue;
        }

        if !in_paragraph {
            current_start = index;
            in_paragraph = true;
        }
        paragraph_start_lines.push(current_start);
    }

    paragraph_start_lines
}

fn build_markdown_heading_paths(lines: &[TextLine]) -> Vec<Vec<String>> {
    let mut heading_path_by_line = Vec::with_capacity(lines.len());
    let mut heading_stack: Vec<String> = Vec::new();
    let mut active_fence: Option<MarkdownFence> = None;

    for index in 0..lines.len() {
        let trimmed = lines[index].content.trim();
        let previous_fence = active_fence;
        if let Some(fence) = parse_markdown_fence(trimmed) {
            if let Some(current_fence) = active_fence {
                if is_markdown_fence_close(trimmed, current_fence) {
                    active_fence = None;
                    heading_path_by_line.push(heading_stack.clone());
                    continue;
                }
            } else {
                active_fence = Some(fence);
                heading_path_by_line.push(heading_stack.clone());
                continue;
            }
        }

        if previous_fence.is_some() {
            heading_path_by_line.push(heading_stack.clone());
            continue;
        }

        let heading = parse_atx_heading(trimmed).or_else(|| parse_setext_heading(lines, index));
        if let Some((level, title)) = heading {
            update_heading_stack(&mut heading_stack, level, title);
        }

        heading_path_by_line.push(heading_stack.clone());
    }

    heading_path_by_line
}

fn parse_atx_heading(trimmed_line: &str) -> Option<(usize, String)> {
    let hashes = trimmed_line.chars().take_while(|char| *char == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }

    let rest = trimmed_line[hashes..].trim();
    if rest.is_empty() {
        return None;
    }

    Some((hashes, rest.trim_end_matches('#').trim().to_string()))
}

fn parse_setext_heading(lines: &[TextLine], index: usize) -> Option<(usize, String)> {
    let title = lines.get(index)?.content.trim();
    if title.is_empty() {
        return None;
    }

    let underline = lines.get(index + 1)?.content.trim();
    if underline.len() < 3 {
        return None;
    }

    if underline.chars().all(|char| char == '=') {
        return Some((1, title.to_string()));
    }
    if underline.chars().all(|char| char == '-') {
        return Some((2, title.to_string()));
    }

    None
}

fn update_heading_stack(stack: &mut Vec<String>, level: usize, title: String) {
    let keep = level.saturating_sub(1);
    stack.truncate(keep);
    stack.push(title);
}

fn parse_markdown_fence(trimmed_line: &str) -> Option<MarkdownFence> {
    let marker = trimmed_line.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }

    let length = trimmed_line
        .chars()
        .take_while(|char| *char == marker)
        .count();
    (length >= 3).then_some(MarkdownFence { marker, length })
}

fn is_markdown_fence_close(trimmed_line: &str, active_fence: MarkdownFence) -> bool {
    let Some(fence) = parse_markdown_fence(trimmed_line) else {
        return false;
    };
    if fence.marker != active_fence.marker || fence.length < active_fence.length {
        return false;
    }

    trimmed_line[fence.length..].trim().is_empty()
}

fn first_non_empty_line_index(
    layout: &TextLayout,
    start_line_index: usize,
    end_line_index: usize,
) -> Option<usize> {
    (start_line_index..=end_line_index).find(|index| {
        layout
            .lines
            .get(*index)
            .map(|line| !line.content.trim().is_empty())
            .unwrap_or(false)
    })
}

fn common_heading_path_for_range(
    layout: &TextLayout,
    start_line_index: usize,
    end_line_index: usize,
) -> Option<Vec<String>> {
    let mut heading_paths = (start_line_index..=end_line_index).filter_map(|index| {
        let line = layout.lines.get(index)?;
        if line.content.trim().is_empty() {
            return None;
        }
        layout.heading_path_by_line.get(index).cloned()
    });
    let mut prefix = heading_paths.next()?;

    for path in heading_paths {
        let shared_len = prefix
            .iter()
            .zip(path.iter())
            .take_while(|(left, right)| left == right)
            .count();
        prefix.truncate(shared_len);
        if prefix.is_empty() {
            break;
        }
    }

    Some(prefix)
}

fn line_index_for_offset(layout: &TextLayout, offset: usize) -> usize {
    let clamped_offset = offset.min(
        layout
            .lines
            .last()
            .map(|line| line.end_byte.saturating_sub(1))
            .unwrap_or_default(),
    );
    let insertion_index = layout
        .lines
        .partition_point(|line| line.start_byte <= clamped_offset);
    insertion_index.saturating_sub(1)
}

fn resolve_source_root_for_path<'a>(roots: &'a [PathBuf], path: &Path) -> Option<&'a PathBuf> {
    roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.as_os_str().len())
}

fn build_ignore_glob_set(patterns: &[String]) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            continue;
        }
        let glob = Glob::new(trimmed)
            .with_context(|| format!("invalid RAG ignore glob pattern: {trimmed}"))?;
        builder.add(glob);
    }

    Ok(Some(
        builder
            .build()
            .context("failed to compile RAG ignore glob set")?,
    ))
}

fn should_skip_path(root: &Path, path: &Path, ignore_matcher: Option<&GlobSet>) -> bool {
    let Some(ignore_matcher) = ignore_matcher else {
        return false;
    };

    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let absolute = path.to_string_lossy().replace('\\', "/");

    ignore_matcher.is_match(&relative) || ignore_matcher.is_match(&absolute)
}

fn make_chunk_version_id(path: &Path) -> String {
    let path_hash = format!("{:x}", md5::compute(normalize_path_string(path)));
    format!("{:x}-{path_hash}", now_unix_ms().max(0))
}

fn chunk_reuse_key(text: &str, heading_path: &[String]) -> String {
    let heading_key = heading_path.join("\u{1f}");
    format!("{:x}", md5::compute(format!("{heading_key}\u{0}{text}")))
}

fn text_fingerprint(text: &str) -> String {
    format!("{:x}", md5::compute(text.as_bytes()))
}

pub(crate) async fn request_embeddings(
    client: &HttpClient,
    provider: &LlmProviderConfig,
    inputs: &[String],
) -> Result<Vec<Vec<f32>>> {
    let (vectors, _) = request_embeddings_with_stats(client, provider, inputs).await?;
    Ok(vectors)
}

async fn request_embeddings_with_stats(
    client: &HttpClient,
    provider: &LlmProviderConfig,
    inputs: &[String],
) -> Result<(Vec<Vec<f32>>, EmbeddingRequestStats)> {
    if inputs.is_empty() {
        return Ok((
            Vec::new(),
            EmbeddingRequestStats {
                largest_successful_batch_size: 0,
                split_retry_count: 0,
            },
        ));
    }

    let mut pending_batches = vec![(0usize, inputs.len())];
    let mut resolved_vectors = vec![None; inputs.len()];
    let mut stats = EmbeddingRequestStats {
        largest_successful_batch_size: 0,
        split_retry_count: 0,
    };
    while let Some((start, end)) = pending_batches.pop() {
        let batch_inputs = &inputs[start..end];
        match request_embeddings_batch(client, provider, batch_inputs).await {
            Ok(vectors) => {
                if vectors.len() != batch_inputs.len() {
                    anyhow::bail!(
                        "embedding provider returned {} vectors for {} inputs",
                        vectors.len(),
                        batch_inputs.len()
                    );
                }
                stats.record_success(batch_inputs.len());
                for (offset, vector) in vectors.into_iter().enumerate() {
                    resolved_vectors[start + offset] = Some(vector);
                }
            }
            Err(error) if is_embedding_batch_overloaded(&error) && batch_inputs.len() > 1 => {
                let midpoint = start + (batch_inputs.len() / 2);
                stats.record_split_retry();
                tracing::warn!(
                    batch_size = batch_inputs.len(),
                    retry_left = midpoint - start,
                    retry_right = end - midpoint,
                    "embedding batch overloaded; retrying with smaller batches"
                );
                pending_batches.push((midpoint, end));
                pending_batches.push((start, midpoint));
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to embed batch with {} input(s)", batch_inputs.len())
                });
            }
        }
    }

    let vectors = resolved_vectors
        .into_iter()
        .map(|vector| vector.context("embedding batch completed without a vector"))
        .collect::<Result<Vec<_>>>()?;
    Ok((vectors, stats))
}

async fn request_embeddings_batch(
    client: &HttpClient,
    provider: &LlmProviderConfig,
    inputs: &[String],
) -> Result<Vec<Vec<f32>>> {
    let client = OpenAiCompatibleClient::new_async(
        client,
        &provider.base_url,
        &provider.api_key,
        "embedding provider base URL",
    )?;
    let parsed: EmbeddingResponse = client
        .post_json(
            "/embeddings",
            &EmbeddingRequest {
                model: provider.model_name(),
                input: inputs,
            },
            "embeddings from provider",
            crate::infrastructure::openai_compatible::OpenAiCompatibleResponseFormat::Json,
        )
        .await?;
    Ok(parsed.data.into_iter().map(|item| item.embedding).collect())
}

fn is_embedding_timeout(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|source| source.downcast_ref::<reqwest::Error>())
        .any(reqwest::Error::is_timeout)
}

fn is_embedding_batch_overloaded(error: &anyhow::Error) -> bool {
    if is_embedding_timeout(error) {
        return true;
    }

    let message = error.to_string().to_ascii_lowercase();
    [
        "out of memory",
        "cuda out of memory",
        "resource exhausted",
        "payload too large",
        "request entity too large",
        "413 payload too large",
    ]
    .iter()
    .any(|pattern| message.contains(pattern))
}

fn embedding_fingerprint(provider: &LlmProviderConfig) -> Result<String> {
    let base_url = normalize_base_url(&provider.base_url, "embedding provider base URL")?;
    let target_identity = infer_embedding_target_identity(
        &base_url,
        provider.model_name(),
        provider.model_identity_hint.as_deref(),
    );
    let fingerprint_source = match target_identity {
        EmbeddingTargetIdentity::StableModel {
            namespace,
            model_identity,
        } => format!("v2\u{0}stable\u{0}{namespace}\u{0}{model_identity}"),
        EmbeddingTargetIdentity::EndpointBound {
            normalized_base_url,
            model_identity,
        } => {
            format!("v2\u{0}endpoint\u{0}{normalized_base_url}\u{0}{model_identity}")
        }
    };
    Ok(format!("{:x}", md5::compute(fingerprint_source)))
}

async fn open_existing_rag_table(db: &LanceConnection) -> Result<Option<Table>> {
    let table_exists = db
        .table_names()
        .execute()
        .await
        .context("failed to list LanceDB tables")?
        .iter()
        .any(|name| name == RAG_TABLE_NAME);
    if !table_exists {
        return Ok(None);
    }
    Ok(Some(
        db.open_table(RAG_TABLE_NAME)
            .execute()
            .await
            .context("failed to open existing RAG table")?,
    ))
}

async fn delete_vectors_with_filter(vector_store: &mut RagVectorStore, filter: &str) -> Result<()> {
    vector_store.delete_where(filter).await
}

async fn delete_vectors_for_exact_paths(
    vector_store: &mut RagVectorStore,
    paths: &[String],
) -> Result<()> {
    for chunk in paths.chunks(MAX_DELETE_FILTER_PATHS) {
        let filter = build_exact_path_filter(chunk);
        delete_vectors_with_filter(vector_store, &filter).await?;
    }
    Ok(())
}

async fn delete_vectors_for_exact_paths_in_state(
    vector_store: &mut RagVectorStore,
    paths: &[String],
    chunk_state: RagChunkState,
) -> Result<()> {
    for chunk in paths.chunks(MAX_DELETE_FILTER_PATHS) {
        let filter = format!(
            "{} AND chunk_state = '{}'",
            build_exact_path_filter(chunk),
            chunk_state.as_str()
        );
        delete_vectors_with_filter(vector_store, &filter).await?;
    }
    Ok(())
}

fn build_exact_path_filter(paths: &[String]) -> String {
    let escaped = paths
        .iter()
        .map(|path| format!("'{}'", escape_sql_literal(path)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("absolute_path IN ({escaped})")
}

async fn load_rag_table_schema(database_path: &Path) -> Result<Option<Arc<Schema>>> {
    let db = connect(database_path.to_string_lossy().as_ref())
        .execute()
        .await
        .context("failed to open LanceDB database")?;
    let table_exists = db
        .table_names()
        .execute()
        .await
        .context("failed to list LanceDB tables")?
        .iter()
        .any(|name| name == RAG_TABLE_NAME);
    if !table_exists {
        return Ok(None);
    }

    let schema = db
        .open_table(RAG_TABLE_NAME)
        .execute()
        .await
        .context("failed to open RAG table for schema inspection")?
        .schema()
        .await
        .context("failed to read RAG table schema")?;
    Ok(Some(schema))
}

fn rag_table_schema_is_compatible(schema: &Schema) -> bool {
    let fields = schema.fields();
    let expected_fields = [
        ("id", DataType::Utf8, false),
        ("source_root", DataType::Utf8, false),
        ("absolute_path", DataType::Utf8, false),
        ("version_id", DataType::Utf8, false),
        ("embedding_fingerprint", DataType::Utf8, false),
        ("chunk_state", DataType::Utf8, false),
        ("chunk_index", DataType::Int32, false),
        ("line_start", DataType::Int32, false),
        ("line_end", DataType::Int32, false),
        ("paragraph_line_start", DataType::Int32, false),
        ("heading_path", DataType::Utf8, false),
        ("chunk_reuse_key", DataType::Utf8, false),
        ("text_fingerprint", DataType::Utf8, false),
        ("text", DataType::Utf8, false),
    ];

    if fields.len() != expected_fields.len() + 1 {
        return false;
    }

    for (field, (name, data_type, nullable)) in fields.iter().zip(expected_fields.iter()) {
        if field.name() != *name
            || field.data_type() != data_type
            || field.is_nullable() != *nullable
        {
            return false;
        }
    }

    let vector_field = &fields[expected_fields.len()];
    if vector_field.name() != "vector" || !vector_field.is_nullable() {
        return false;
    }

    match vector_field.data_type() {
        DataType::FixedSizeList(item, dimension) => {
            *dimension > 0
                && item.name() == "item"
                && item.data_type() == &DataType::Float32
                && item.is_nullable()
        }
        _ => false,
    }
}

fn rag_storage_requires_fingerprint_reset(
    stored_records: &HashMap<String, RagIndexedFileRecord>,
    current_embedding_fingerprint: &str,
) -> bool {
    !stored_records.is_empty()
        && stored_records
            .values()
            .any(|record| record.embedding_fingerprint != current_embedding_fingerprint)
}

async fn metadata_store_has_active_records(metadata_path: &Path) -> Result<bool> {
    let stored_records = tokio::task::spawn_blocking({
        let metadata_path = metadata_path.to_path_buf();
        move || load_metadata_records(&metadata_path)
    })
    .await
    .context("failed to join RAG metadata state task")??;

    Ok(stored_records
        .values()
        .any(|record| record.active.is_some()))
}

async fn prepare_index_storage(
    database_path: &Path,
    metadata_path: &Path,
    resolved: &ResolvedRagConfig,
    reset_on_embedding_target_mismatch: bool,
) -> Result<()> {
    tokio::fs::create_dir_all(database_path)
        .await
        .with_context(|| {
            format!(
                "failed to create RAG database directory: {}",
                database_path.display()
            )
        })?;
    let table_schema = load_rag_table_schema(database_path).await?;
    let metadata_schema_is_compatible = tokio::task::spawn_blocking({
        let metadata_path = metadata_path.to_path_buf();
        move || metadata_store_has_compatible_schema(&metadata_path)
    })
    .await
    .context("failed to join RAG metadata schema task")??;
    let vector_table_exists = table_schema.is_some();

    if let Some(schema) = table_schema.as_deref() {
        if !rag_table_schema_is_compatible(schema) {
            tracing::warn!(
                field_count = schema.fields().len(),
                "resetting RAG storage because LanceDB schema is incompatible with current code"
            );
            clear_index(database_path).await?;
            clear_metadata_store(metadata_path).await?;
            return Ok(());
        }
    }

    if !metadata_schema_is_compatible {
        tracing::warn!(
            "resetting RAG storage because SQLite metadata schema is incompatible with current code"
        );
        if vector_table_exists {
            clear_index(database_path).await?;
        }
        clear_metadata_store(metadata_path).await?;
        return Ok(());
    }

    let stored_records = tokio::task::spawn_blocking({
        let metadata_path = metadata_path.to_path_buf();
        move || load_metadata_records(&metadata_path)
    })
    .await
    .context("failed to join RAG metadata state task")??;
    let metadata_has_rows = !stored_records.is_empty();
    let current_embedding_fingerprint = resolved.embedding_fingerprint.as_str();

    if reset_on_embedding_target_mismatch
        && rag_storage_requires_fingerprint_reset(&stored_records, current_embedding_fingerprint)
    {
        tracing::info!(
            embedding_fingerprint = current_embedding_fingerprint,
            "resetting RAG storage because indexed embedding target changed"
        );
        if vector_table_exists {
            clear_index(database_path).await?;
        }
        clear_metadata_store(metadata_path).await?;
        return Ok(());
    }

    match (vector_table_exists, metadata_has_rows) {
        (true, false) => clear_index(database_path).await?,
        (false, true) => clear_metadata_store(metadata_path).await?,
        _ => {}
    }

    Ok(())
}

fn build_record_batch_reader(
    chunks: &[RagChunk],
    vectors: &[Vec<f32>],
) -> Result<Box<dyn RecordBatchReader + Send>> {
    let dimension = vectors
        .first()
        .map(|vector| vector.len())
        .context("cannot build LanceDB batch without vectors")?;

    if vectors.iter().any(|vector| vector.len() != dimension) {
        anyhow::bail!("embedding provider returned inconsistent vector dimensions");
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("source_root", DataType::Utf8, false),
        Field::new("absolute_path", DataType::Utf8, false),
        Field::new("version_id", DataType::Utf8, false),
        Field::new("embedding_fingerprint", DataType::Utf8, false),
        Field::new("chunk_state", DataType::Utf8, false),
        Field::new("chunk_index", DataType::Int32, false),
        Field::new("line_start", DataType::Int32, false),
        Field::new("line_end", DataType::Int32, false),
        Field::new("paragraph_line_start", DataType::Int32, false),
        Field::new("heading_path", DataType::Utf8, false),
        Field::new("chunk_reuse_key", DataType::Utf8, false),
        Field::new("text_fingerprint", DataType::Utf8, false),
        Field::new("text", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dimension as i32,
            ),
            true,
        ),
    ]));

    let ids = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.id.clone())
            .collect::<Vec<_>>(),
    );
    let source_roots = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.source_root.clone())
            .collect::<Vec<_>>(),
    );
    let absolute_paths = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.absolute_path.clone())
            .collect::<Vec<_>>(),
    );
    let version_ids = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.version_id.clone())
            .collect::<Vec<_>>(),
    );
    let embedding_fingerprints = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.embedding_fingerprint.clone())
            .collect::<Vec<_>>(),
    );
    let chunk_states = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.chunk_state.as_str())
            .collect::<Vec<_>>(),
    );
    let chunk_indexes = Int32Array::from(
        chunks
            .iter()
            .map(|chunk| chunk.chunk_index)
            .collect::<Vec<_>>(),
    );
    let line_starts = Int32Array::from(
        chunks
            .iter()
            .map(|chunk| chunk.line_start)
            .collect::<Vec<_>>(),
    );
    let line_ends = Int32Array::from(
        chunks
            .iter()
            .map(|chunk| chunk.line_end)
            .collect::<Vec<_>>(),
    );
    let paragraph_line_starts = Int32Array::from(
        chunks
            .iter()
            .map(|chunk| chunk.paragraph_line_start)
            .collect::<Vec<_>>(),
    );
    let heading_paths = StringArray::from(
        chunks
            .iter()
            .map(|chunk| serde_json::to_string(&chunk.heading_path))
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to serialize heading path metadata")?,
    );
    let chunk_reuse_keys = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.chunk_reuse_key.clone())
            .collect::<Vec<_>>(),
    );
    let text_fingerprints = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.text_fingerprint.clone())
            .collect::<Vec<_>>(),
    );
    let texts = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>(),
    );
    let vector_array = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        vectors
            .iter()
            .map(|vector| Some(vector.iter().copied().map(Some))),
        dimension as i32,
    );

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(ids),
            Arc::new(source_roots),
            Arc::new(absolute_paths),
            Arc::new(version_ids),
            Arc::new(embedding_fingerprints),
            Arc::new(chunk_states),
            Arc::new(chunk_indexes),
            Arc::new(line_starts),
            Arc::new(line_ends),
            Arc::new(paragraph_line_starts),
            Arc::new(heading_paths),
            Arc::new(chunk_reuse_keys),
            Arc::new(text_fingerprints),
            Arc::new(texts),
            Arc::new(vector_array),
        ],
    )
    .context("failed to build LanceDB record batch")?;

    Ok(Box::new(RecordBatchIterator::new(
        vec![Ok(batch)].into_iter(),
        schema,
    )))
}

fn parse_chunk_vector_batch(batch: &RecordBatch) -> Result<Vec<StoredChunkVector>> {
    if batch.num_rows() == 0 {
        return Ok(Vec::new());
    }

    let chunk_reuse_keys = batch
        .column(
            batch
                .schema()
                .index_of("chunk_reuse_key")
                .context("chunk_reuse_key column missing from LanceDB batch")?,
        )
        .as_any()
        .downcast_ref::<StringArray>()
        .context("chunk_reuse_key column is not a StringArray")?;
    let vectors = batch
        .column(
            batch
                .schema()
                .index_of("vector")
                .context("vector column missing from LanceDB batch")?,
        )
        .as_any()
        .downcast_ref::<FixedSizeListArray>()
        .context("vector column is not a FixedSizeListArray")?;
    let values = vectors
        .values()
        .as_any()
        .downcast_ref::<Float32Array>()
        .context("vector values are not Float32Array")?;
    let dimension = usize::try_from(vectors.value_length()).unwrap_or_default();

    let mut stored = Vec::with_capacity(batch.num_rows());
    for row_index in 0..batch.num_rows() {
        let start = row_index.saturating_mul(dimension);
        let end = start.saturating_add(dimension);
        stored.push(StoredChunkVector {
            chunk_reuse_key: chunk_reuse_keys.value(row_index).to_string(),
            vector: (start..end).map(|offset| values.value(offset)).collect(),
        });
    }
    Ok(stored)
}

fn parse_text_vector_batch(batch: &RecordBatch) -> Result<Vec<CachedTextVector>> {
    if batch.num_rows() == 0 {
        return Ok(Vec::new());
    }

    let text_fingerprints = batch
        .column(
            batch
                .schema()
                .index_of("text_fingerprint")
                .context("text_fingerprint column missing from LanceDB batch")?,
        )
        .as_any()
        .downcast_ref::<StringArray>()
        .context("text_fingerprint column is not a StringArray")?;
    let texts = batch
        .column(
            batch
                .schema()
                .index_of("text")
                .context("text column missing from LanceDB batch")?,
        )
        .as_any()
        .downcast_ref::<StringArray>()
        .context("text column is not a StringArray")?;
    let vectors = batch
        .column(
            batch
                .schema()
                .index_of("vector")
                .context("vector column missing from LanceDB batch")?,
        )
        .as_any()
        .downcast_ref::<FixedSizeListArray>()
        .context("vector column is not a FixedSizeListArray")?;
    let values = vectors
        .values()
        .as_any()
        .downcast_ref::<Float32Array>()
        .context("vector values are not Float32Array")?;
    let dimension = usize::try_from(vectors.value_length()).unwrap_or_default();

    let mut stored = Vec::with_capacity(batch.num_rows());
    for row_index in 0..batch.num_rows() {
        let start = row_index.saturating_mul(dimension);
        let end = start.saturating_add(dimension);
        stored.push(CachedTextVector {
            text_fingerprint: text_fingerprints.value(row_index).to_string(),
            text: texts.value(row_index).to_string(),
            vector: (start..end).map(|offset| values.value(offset)).collect(),
        });
    }
    Ok(stored)
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

fn open_metadata_connection(metadata_path: &Path) -> Result<Connection> {
    if let Some(parent) = metadata_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create RAG metadata directory: {}",
                parent.display()
            )
        })?;
    }

    let connection = Connection::open(metadata_path).with_context(|| {
        format!(
            "failed to open RAG metadata database: {}",
            metadata_path.display()
        )
    })?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .context("failed to configure RAG metadata busy timeout")?;
    connection
        .execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            CREATE TABLE IF NOT EXISTS rag_files (
                absolute_path TEXT PRIMARY KEY NOT NULL,
                source_root TEXT NOT NULL,
                relative_path TEXT NOT NULL,
                embedding_fingerprint TEXT NOT NULL DEFAULT '',
                active_version_id TEXT,
                active_content_md5 TEXT,
                active_modified_at_ms INTEGER,
                active_size_bytes INTEGER,
                active_chunk_count INTEGER,
                active_indexed_at_ms INTEGER,
                pending_version_id TEXT,
                pending_content_md5 TEXT,
                pending_modified_at_ms INTEGER,
                pending_size_bytes INTEGER,
                pending_chunk_count INTEGER,
                pending_started_at_ms INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_rag_files_source_root ON rag_files(source_root);
            ",
        )
        .context("failed to initialize RAG metadata schema")?;
    Ok(connection)
}

fn metadata_store_has_compatible_schema(metadata_path: &Path) -> Result<bool> {
    let connection = open_metadata_connection(metadata_path)?;
    metadata_table_schema_is_compatible(&connection)
}

fn metadata_table_schema_is_compatible(connection: &Connection) -> Result<bool> {
    let expected_columns = [
        ("absolute_path", "TEXT", true),
        ("source_root", "TEXT", true),
        ("relative_path", "TEXT", true),
        ("embedding_fingerprint", "TEXT", true),
        ("active_version_id", "TEXT", false),
        ("active_content_md5", "TEXT", false),
        ("active_modified_at_ms", "INTEGER", false),
        ("active_size_bytes", "INTEGER", false),
        ("active_chunk_count", "INTEGER", false),
        ("active_indexed_at_ms", "INTEGER", false),
        ("pending_version_id", "TEXT", false),
        ("pending_content_md5", "TEXT", false),
        ("pending_modified_at_ms", "INTEGER", false),
        ("pending_size_bytes", "INTEGER", false),
        ("pending_chunk_count", "INTEGER", false),
        ("pending_started_at_ms", "INTEGER", false),
    ];
    let mut statement = connection
        .prepare("PRAGMA table_info(rag_files)")
        .context("failed to inspect RAG metadata schema")?;
    let columns = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? != 0,
            ))
        })
        .context("failed to query RAG metadata schema")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect RAG metadata schema rows")?;

    if columns.len() != expected_columns.len() {
        return Ok(false);
    }

    Ok(columns.iter().zip(expected_columns.iter()).all(
        |((name, data_type, not_null), (expected_name, expected_type, expected_not_null))| {
            name == expected_name
                && data_type.eq_ignore_ascii_case(expected_type)
                && not_null == expected_not_null
        },
    ))
}

fn load_metadata_records(metadata_path: &Path) -> Result<HashMap<String, RagIndexedFileRecord>> {
    let connection = open_metadata_connection(metadata_path)?;
    let mut statement = connection
        .prepare(
            "
            SELECT
                source_root,
                absolute_path,
                relative_path,
                embedding_fingerprint,
                active_version_id,
                active_content_md5,
                active_modified_at_ms,
                active_size_bytes,
                active_chunk_count,
                active_indexed_at_ms,
                pending_version_id,
                pending_content_md5,
                pending_modified_at_ms,
                pending_size_bytes,
                pending_chunk_count,
                pending_started_at_ms
            FROM rag_files
            ",
        )
        .context("failed to prepare RAG metadata query")?;
    let mut rows = statement
        .query([])
        .context("failed to read RAG metadata rows")?;
    let mut records = HashMap::new();
    while let Some(row) = rows.next().context("failed to step RAG metadata rows")? {
        let record = read_metadata_record(row)?;
        records.insert(record.absolute_path.clone(), record);
    }
    Ok(records)
}

fn load_metadata_records_for_paths(
    metadata_path: &Path,
    absolute_paths: &[String],
) -> Result<HashMap<String, RagIndexedFileRecord>> {
    if absolute_paths.is_empty() {
        return Ok(HashMap::new());
    }

    let connection = open_metadata_connection(metadata_path)?;
    let mut records = HashMap::new();
    for batch in absolute_paths.chunks(MAX_METADATA_BATCH_PATHS) {
        let placeholders = (0..batch.len())
            .map(|index| format!("?{}", index + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "
            SELECT
                source_root,
                absolute_path,
                relative_path,
                embedding_fingerprint,
                active_version_id,
                active_content_md5,
                active_modified_at_ms,
                active_size_bytes,
                active_chunk_count,
                active_indexed_at_ms,
                pending_version_id,
                pending_content_md5,
                pending_modified_at_ms,
                pending_size_bytes,
                pending_chunk_count,
                pending_started_at_ms
            FROM rag_files
            WHERE absolute_path IN ({placeholders})
            "
        );
        let mut statement = connection
            .prepare(&sql)
            .context("failed to prepare batched RAG metadata query")?;
        let params = rusqlite::params_from_iter(batch.iter());
        let mut rows = statement
            .query(params)
            .context("failed to read batched RAG metadata rows")?;
        while let Some(row) = rows
            .next()
            .context("failed to step batched metadata rows")?
        {
            let record = read_metadata_record(row)?;
            records.insert(record.absolute_path.clone(), record);
        }
    }

    Ok(records)
}

fn read_metadata_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RagIndexedFileRecord> {
    Ok(RagIndexedFileRecord {
        source_root: row.get(0)?,
        absolute_path: row.get(1)?,
        relative_path: row.get(2)?,
        embedding_fingerprint: row.get(3)?,
        active: read_metadata_version(
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
        )?,
        pending: read_metadata_version(
            row.get(10)?,
            row.get(11)?,
            row.get(12)?,
            row.get(13)?,
            row.get(14)?,
            row.get(15)?,
        )?,
    })
}

fn read_metadata_version(
    version_id: Option<String>,
    content_md5: Option<String>,
    modified_at_ms: Option<i64>,
    size_bytes: Option<i64>,
    chunk_count: Option<i64>,
    indexed_at_ms: Option<i64>,
) -> rusqlite::Result<Option<RagIndexedFileVersion>> {
    let Some(version_id) = version_id else {
        return Ok(None);
    };

    Ok(Some(RagIndexedFileVersion {
        version_id,
        content_md5: content_md5.ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing content_md5 for indexed metadata version",
                )),
            )
        })?,
        modified_at_ms,
        size_bytes: size_bytes.ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing size_bytes for indexed metadata version",
                )),
            )
        })?,
        chunk_count: chunk_count.ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing chunk_count for indexed metadata version",
                )),
            )
        })?,
        indexed_at_ms: indexed_at_ms.unwrap_or_default(),
    }))
}

fn finalize_metadata_record(file: &PreparedRagFile) -> RagIndexedFileRecord {
    let mut finalized = file.record.clone();
    finalized.active = finalized
        .pending
        .as_ref()
        .map(|pending| RagIndexedFileVersion {
            version_id: pending.version_id.clone(),
            content_md5: pending.content_md5.clone(),
            modified_at_ms: pending.modified_at_ms,
            size_bytes: pending.size_bytes,
            chunk_count: pending.chunk_count,
            indexed_at_ms: now_unix_ms(),
        });
    finalized.pending = None;
    finalized
}

pub(crate) fn metadata_store_has_pending_rows(metadata_path: &Path) -> Result<bool> {
    let connection = open_metadata_connection(metadata_path)?;
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM rag_files WHERE pending_version_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .context("failed to count pending RAG metadata rows")?;
    Ok(count > 0)
}

fn reset_metadata_store(metadata_path: &Path) -> Result<()> {
    for path in metadata_store_file_paths(metadata_path) {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to remove RAG metadata file: {}", path.display())
                });
            }
        }
    }

    Ok(())
}

fn metadata_store_file_paths(metadata_path: &Path) -> [PathBuf; 3] {
    let base = metadata_path.to_path_buf();
    let wal = PathBuf::from(format!("{}-wal", metadata_path.to_string_lossy()));
    let shm = PathBuf::from(format!("{}-shm", metadata_path.to_string_lossy()));
    [base, wal, shm]
}

fn upsert_metadata_records(metadata_path: &Path, records: &[RagIndexedFileRecord]) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }

    let mut connection = open_metadata_connection(metadata_path)?;
    let transaction = connection
        .transaction()
        .context("failed to open RAG metadata transaction")?;
    {
        let mut statement = transaction
            .prepare(
                "
                INSERT INTO rag_files (
                    source_root,
                    absolute_path,
                    relative_path,
                    embedding_fingerprint,
                    active_version_id,
                    active_content_md5,
                    active_modified_at_ms,
                    active_size_bytes,
                    active_chunk_count,
                    active_indexed_at_ms,
                    pending_version_id,
                    pending_content_md5,
                    pending_modified_at_ms,
                    pending_size_bytes,
                    pending_chunk_count,
                    pending_started_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                ON CONFLICT(absolute_path) DO UPDATE SET
                    source_root = excluded.source_root,
                    relative_path = excluded.relative_path,
                    embedding_fingerprint = excluded.embedding_fingerprint,
                    active_version_id = excluded.active_version_id,
                    active_content_md5 = excluded.active_content_md5,
                    active_modified_at_ms = excluded.active_modified_at_ms,
                    active_size_bytes = excluded.active_size_bytes,
                    active_chunk_count = excluded.active_chunk_count,
                    active_indexed_at_ms = excluded.active_indexed_at_ms,
                    pending_version_id = excluded.pending_version_id,
                    pending_content_md5 = excluded.pending_content_md5,
                    pending_modified_at_ms = excluded.pending_modified_at_ms,
                    pending_size_bytes = excluded.pending_size_bytes,
                    pending_chunk_count = excluded.pending_chunk_count,
                    pending_started_at_ms = excluded.pending_started_at_ms
                ",
            )
            .context("failed to prepare RAG metadata upsert statement")?;

        for record in records {
            statement
                .execute(params![
                    &record.source_root,
                    &record.absolute_path,
                    &record.relative_path,
                    &record.embedding_fingerprint,
                    record
                        .active
                        .as_ref()
                        .map(|version| version.version_id.as_str()),
                    record
                        .active
                        .as_ref()
                        .map(|version| version.content_md5.as_str()),
                    record
                        .active
                        .as_ref()
                        .and_then(|version| version.modified_at_ms),
                    record.active.as_ref().map(|version| version.size_bytes),
                    record.active.as_ref().map(|version| version.chunk_count),
                    record.active.as_ref().map(|version| version.indexed_at_ms),
                    record
                        .pending
                        .as_ref()
                        .map(|version| version.version_id.as_str()),
                    record
                        .pending
                        .as_ref()
                        .map(|version| version.content_md5.as_str()),
                    record
                        .pending
                        .as_ref()
                        .and_then(|version| version.modified_at_ms),
                    record.pending.as_ref().map(|version| version.size_bytes),
                    record.pending.as_ref().map(|version| version.chunk_count),
                    record.pending.as_ref().map(|version| version.indexed_at_ms),
                ])
                .with_context(|| {
                    format!(
                        "failed to upsert RAG metadata row: {}",
                        record.absolute_path
                    )
                })?;
        }
    }
    transaction
        .commit()
        .context("failed to commit RAG metadata transaction")?;
    Ok(())
}

fn delete_metadata_for_paths(
    metadata_path: &Path,
    paths: &[String],
    delete_descendants: bool,
) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }

    let mut connection = open_metadata_connection(metadata_path)?;
    let transaction = connection
        .transaction()
        .context("failed to open RAG metadata delete transaction")?;
    {
        let mut exact_statement = transaction
            .prepare("DELETE FROM rag_files WHERE absolute_path = ?1")
            .context("failed to prepare RAG metadata exact delete statement")?;
        let mut descendant_statement = transaction
            .prepare("DELETE FROM rag_files WHERE absolute_path = ?1 OR absolute_path LIKE ?2")
            .context("failed to prepare RAG metadata descendant delete statement")?;

        for path in paths {
            if delete_descendants {
                let like_pattern = format!("{path}/%");
                descendant_statement
                    .execute(params![path, like_pattern])
                    .with_context(|| format!("failed to delete RAG metadata rows: {path}"))?;
            } else {
                exact_statement
                    .execute([path])
                    .with_context(|| format!("failed to delete RAG metadata row: {path}"))?;
            }
        }
    }
    transaction
        .commit()
        .context("failed to commit RAG metadata delete transaction")?;
    Ok(())
}

fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn normalize_path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn collect_document_access_roots(
    workspace_root: &Path,
    rag_settings: &RagSettings,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    push_document_access_root(&mut roots, &mut seen, workspace_root);
    for directory in &rag_settings.source_directories {
        let trimmed = directory.trim();
        if trimmed.is_empty() {
            continue;
        }
        push_document_access_root(&mut roots, &mut seen, Path::new(trimmed));
    }

    roots
}

fn push_document_access_root(roots: &mut Vec<PathBuf>, seen: &mut HashSet<String>, path: &Path) {
    let Ok(canonical_root) = path.canonicalize() else {
        return;
    };
    if !canonical_root.is_dir() {
        return;
    }

    let normalized_root = normalize_path_string(&canonical_root);
    if seen.insert(normalized_root) {
        roots.push(canonical_root);
    }
}

pub(crate) fn path_is_within_roots(path: &Path, roots: &[PathBuf]) -> bool {
    path.starts_with_any(roots)
}

pub(crate) fn display_path_for_prompt(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let Some(home_dir) = dirs::home_dir() else {
        return normalized;
    };
    let home = normalize_path_string(&home_dir);
    if normalized == home {
        return "~".to_string();
    }
    if let Some(stripped) = normalized.strip_prefix(&(home.clone() + "/")) {
        return format!("~/{stripped}");
    }
    normalized
}

pub(crate) fn parse_heading_path(raw: &str) -> Result<Vec<String>> {
    serde_json::from_str(raw).context("failed to parse heading path metadata")
}

fn system_time_to_unix_ms(value: SystemTime) -> Option<i64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

fn now_unix_ms() -> i64 {
    system_time_to_unix_ms(SystemTime::now()).unwrap_or_default()
}

#[derive(Debug, Clone, Copy, Default)]
struct RuntimeProgress {
    scanned_file_count: usize,
    completed_file_count: usize,
    total_file_count: usize,
    pending_file_count: usize,
}

async fn set_runtime_status(
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_guard: Option<(&Arc<AtomicU64>, u64)>,
    phase: RagRuntimePhase,
    progress: RuntimeProgress,
    last_error: Option<String>,
) {
    if let Some((runtime_generation, generation)) = runtime_guard {
        if runtime_generation.load(Ordering::SeqCst) != generation {
            return;
        }
    }

    let mut status = runtime_status.write().await;
    if let Some((runtime_generation, generation)) = runtime_guard {
        if runtime_generation.load(Ordering::SeqCst) != generation {
            return;
        }
    }

    *status = RagRuntimeStatus {
        phase,
        scanned_file_count: progress.scanned_file_count,
        completed_file_count: progress.completed_file_count,
        total_file_count: progress.total_file_count,
        pending_file_count: progress.pending_file_count,
        last_error,
        updated_at_ms: u64::try_from(now_unix_ms()).unwrap_or_default(),
    };
}

async fn set_runtime_status_for_generation(
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_generation: &Arc<AtomicU64>,
    generation: u64,
    phase: RagRuntimePhase,
    progress: RuntimeProgress,
    last_error: Option<String>,
) {
    set_runtime_status(
        runtime_status,
        Some((runtime_generation, generation)),
        phase,
        progress,
        last_error,
    )
    .await;
}

trait PathStartsWithAny {
    fn starts_with_any(&self, prefixes: &[PathBuf]) -> bool;
}

impl PathStartsWithAny for Path {
    fn starts_with_any(&self, prefixes: &[PathBuf]) -> bool {
        prefixes.iter().any(|prefix| self.starts_with(prefix))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn temp_test_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("wabity-rag-{label}-{unique}"))
    }

    fn build_test_docx(document_xml: &str, styles_xml: Option<&str>) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default();

        writer
            .start_file("word/document.xml", options)
            .expect("start document.xml");
        writer
            .write_all(document_xml.as_bytes())
            .expect("write document.xml");

        if let Some(styles_xml) = styles_xml {
            writer
                .start_file("word/styles.xml", options)
                .expect("start styles.xml");
            writer
                .write_all(styles_xml.as_bytes())
                .expect("write styles.xml");
        }

        writer.finish().expect("finish docx writer").into_inner()
    }

    fn test_embedding_provider() -> LlmProviderConfig {
        test_embedding_provider_with_target(
            "embedding",
            "https://api.example.com/v1",
            "text-embedding-3-small",
        )
    }

    fn test_embedding_provider_with_target(
        id: &str,
        base_url: &str,
        model: &str,
    ) -> LlmProviderConfig {
        test_embedding_provider_with_hint(id, base_url, model, None)
    }

    fn test_embedding_provider_with_hint(
        id: &str,
        base_url: &str,
        model: &str,
        model_identity_hint: Option<&str>,
    ) -> LlmProviderConfig {
        LlmProviderConfig {
            id: id.to_string(),
            name: id.to_string(),
            base_url: base_url.to_string(),
            api_key: String::new(),
            model_type: crate::domain::settings::LlmModelType::Embedding,
            model: model.to_string(),
            model_identity_hint: model_identity_hint.map(ToOwned::to_owned),
            supports_multimodal: false,
            ..LlmProviderConfig::default()
        }
    }

    fn test_embedding_fingerprint() -> String {
        embedding_fingerprint(&test_embedding_provider())
            .expect("test embedding fingerprint should resolve")
    }

    fn test_embedding_fingerprint_for(base_url: &str, model: &str) -> String {
        embedding_fingerprint(&test_embedding_provider_with_target(
            "embedding",
            base_url,
            model,
        ))
        .expect("test embedding fingerprint should resolve")
    }

    fn test_resolved_config(root: &Path) -> ResolvedRagConfig {
        let provider = test_embedding_provider();
        ResolvedRagConfig {
            source_roots: vec![root.to_path_buf()],
            ignore_globs: Arc::new(None),
            embedding_fingerprint: test_embedding_fingerprint(),
            provider,
        }
    }

    fn test_runtime_inputs(
        embedding_provider_id: Option<&str>,
        providers: &[(&str, &str)],
    ) -> RagRuntimeInputs {
        test_runtime_inputs_with_directories(
            vec!["/tmp/docs".to_string()],
            embedding_provider_id,
            providers,
        )
    }

    fn test_runtime_inputs_with_directories(
        source_directories: Vec<String>,
        embedding_provider_id: Option<&str>,
        providers: &[(&str, &str)],
    ) -> RagRuntimeInputs {
        test_runtime_inputs_with_provider_targets(
            source_directories,
            embedding_provider_id,
            &providers
                .iter()
                .map(|(id, model)| (*id, "https://api.example.com/v1", *model, None))
                .collect::<Vec<_>>(),
        )
    }

    fn test_runtime_inputs_with_provider_targets(
        source_directories: Vec<String>,
        embedding_provider_id: Option<&str>,
        providers: &[(&str, &str, &str, Option<&str>)],
    ) -> RagRuntimeInputs {
        RagRuntimeInputs::from_settings(
            &RagSettings {
                source_directories,
                ignore_globs: vec![],
                embedding_provider_id: embedding_provider_id.map(ToOwned::to_owned),
            },
            &LlmSettings {
                providers: providers
                    .iter()
                    .map(|(id, base_url, model, model_identity_hint)| {
                        test_embedding_provider_with_hint(id, base_url, model, *model_identity_hint)
                    })
                    .collect(),
                ..LlmSettings::default()
            },
        )
    }

    async fn wait_for_runtime_exit(service: &RagIndexService) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let runtime_finished = service
                    .runtime
                    .read()
                    .await
                    .as_ref()
                    .is_some_and(|handle| handle.is_finished());
                if runtime_finished {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("watcher task should exit");
    }

    fn test_active_version(
        content_md5: &str,
        modified_at_ms: Option<i64>,
        size_bytes: i64,
        chunk_count: i64,
        indexed_at_ms: i64,
    ) -> RagIndexedFileVersion {
        RagIndexedFileVersion {
            version_id: "active-v1".to_string(),
            content_md5: content_md5.to_string(),
            modified_at_ms,
            size_bytes,
            chunk_count,
            indexed_at_ms,
        }
    }

    fn test_pending_version(
        content_md5: &str,
        modified_at_ms: Option<i64>,
        size_bytes: i64,
        chunk_count: i64,
        indexed_at_ms: i64,
    ) -> RagIndexedFileVersion {
        RagIndexedFileVersion {
            version_id: "pending-v1".to_string(),
            content_md5: content_md5.to_string(),
            modified_at_ms,
            size_bytes,
            chunk_count,
            indexed_at_ms,
        }
    }

    fn test_indexed_record(
        root: &Path,
        file_path: &Path,
        embedding_fingerprint: &str,
        active: Option<RagIndexedFileVersion>,
        pending: Option<RagIndexedFileVersion>,
    ) -> RagIndexedFileRecord {
        RagIndexedFileRecord {
            source_root: normalize_path_string(root),
            absolute_path: normalize_path_string(file_path),
            relative_path: file_path
                .strip_prefix(root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .replace('\\', "/"),
            embedding_fingerprint: embedding_fingerprint.to_string(),
            active,
            pending,
        }
    }

    fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    async fn read_http_request_body(
        stream: &mut tokio::net::TcpStream,
    ) -> std::io::Result<Vec<u8>> {
        let mut request = Vec::new();
        let header_end = loop {
            let mut chunk = [0u8; 1024];
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                return Ok(Vec::new());
            }
            request.extend_from_slice(&chunk[..read]);
            if let Some(position) = find_subsequence(&request, b"\r\n\r\n") {
                break position + 4;
            }
        };

        let headers = std::str::from_utf8(&request[..header_end])
            .expect("http request headers should be valid utf-8");
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);

        while request.len() < header_end + content_length {
            let mut chunk = [0u8; 1024];
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
        }

        Ok(request[header_end..header_end + content_length].to_vec())
    }

    #[test]
    fn ignore_glob_matches_relative_path() {
        let matcher = build_ignore_glob_set(&["**/*.lock".to_string()])
            .expect("glob compilation should succeed")
            .expect("glob set should exist");
        let root = PathBuf::from("/tmp/workspace");
        let path = root.join("Cargo.lock");

        assert!(should_skip_path(&root, &path, Some(&matcher)));
    }

    #[test]
    fn source_root_prefers_deepest_match() {
        let roots = vec![
            PathBuf::from("/tmp/workspace"),
            PathBuf::from("/tmp/workspace/nested"),
        ];
        let path = PathBuf::from("/tmp/workspace/nested/file.txt");

        assert_eq!(
            resolve_source_root_for_path(&roots, &path),
            Some(&PathBuf::from("/tmp/workspace/nested"))
        );
    }

    #[test]
    fn collect_chunks_for_path_splits_text_and_preserves_metadata() {
        let root = temp_test_root("split");
        let file_path = root.join("notes.txt");
        std::fs::create_dir_all(&root).expect("create rag temp root");
        std::fs::write(&file_path, "alpha beta gamma ".repeat(120)).expect("write rag source file");

        let resolved = test_resolved_config(&root);

        let chunks = collect_chunks_for_path(&resolved, &file_path)
            .expect("collecting chunks should succeed");

        assert!(chunks.len() > 1);
        assert_eq!(chunks[0].source_root, root.to_string_lossy());
        assert_eq!(chunks[0].absolute_path, file_path.to_string_lossy());
        assert!(chunks[0].line_start >= 1);
        assert!(chunks[0].line_end >= chunks[0].line_start);
        assert!(chunks[0].paragraph_line_start >= 1);
        assert!(chunks.iter().all(|chunk| !chunk.text.trim().is_empty()));
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.chunk_index)
                .collect::<Vec<_>>(),
            (0..chunks.len() as i32).collect::<Vec<_>>()
        );

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn supported_rag_document_extensions_are_case_insensitive() {
        assert!(is_supported_document_file(Path::new("/tmp/README.MD")));
        assert!(is_supported_document_file(Path::new("/tmp/notes.mdx")));
        assert!(is_supported_document_file(Path::new("/tmp/plain.txt")));
        assert!(is_supported_document_file(Path::new("/tmp/spec.DOCX")));
        assert!(!is_supported_document_file(Path::new("/tmp/config.toml")));
        assert!(!is_supported_document_file(Path::new("/tmp/README")));
    }

    #[test]
    fn markdown_files_are_packed_by_semantic_blocks() {
        let text = "# Heading\n\n- First note.\n\n- Second note.\n\n- Third note.\n\n## Next\n\nParagraph two.";
        let chunks = split_text_for_path(Path::new("/tmp/readme.md"), text, 24, 0)
            .expect("markdown split should succeed");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].heading_path, vec!["Heading".to_string()]);
        assert!(chunks[0].text.contains("First note."));
        assert!(chunks[0].text.contains("Second note."));
        assert!(chunks[0].text.contains("Third note."));
        assert_eq!(
            chunks[1].heading_path,
            vec!["Heading".to_string(), "Next".to_string()]
        );
        assert!(chunks[1].text.contains("Paragraph two."));
    }

    #[test]
    fn markdown_heading_paths_ignore_fenced_code_with_info_string() {
        let text = "# Intro\n\n```rust\n# not a heading\n```\n\n## Details\n";
        let layout = build_text_layout(text, true);

        assert_eq!(layout.heading_path_by_line[2], vec!["Intro".to_string()]);
        assert_eq!(layout.heading_path_by_line[3], vec!["Intro".to_string()]);
        assert_eq!(
            layout.heading_path_by_line[6],
            vec!["Intro".to_string(), "Details".to_string()]
        );
    }

    #[test]
    fn resolve_chunk_metadata_uses_common_heading_prefix_across_subsections() {
        let text = "# Intro\n\nAlpha\n\n## Details\n\nBeta";
        let layout = build_text_layout(text, true);

        let metadata =
            resolve_chunk_metadata(&layout, 2, 6).expect("chunk metadata should resolve");

        assert_eq!(metadata.heading_path, vec!["Intro".to_string()]);
    }

    #[test]
    fn resolve_chunk_metadata_anchors_to_first_non_empty_line() {
        let text = "\n\nAlpha\nBeta";
        let layout = build_text_layout(text, false);

        let metadata =
            resolve_chunk_metadata(&layout, 0, 3).expect("chunk metadata should resolve");

        assert_eq!(metadata.paragraph_start_line_index, 2);
        assert!(metadata.heading_path.is_empty());
    }

    #[test]
    fn plain_text_files_route_to_text_splitter() {
        let text = "# Heading\n\nParagraph one.\n\n## Next\n\nParagraph two.";
        let expected = TextSplitter::new(build_chunk_config(24, 0).expect("valid config"))
            .chunks(text)
            .map(str::to_owned)
            .collect::<Vec<_>>();

        let chunks = split_text_for_path(Path::new("/tmp/readme.txt"), text, 24, 0)
            .expect("plain text split should succeed");

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.text.clone())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(chunks.iter().all(|chunk| chunk.heading_path.is_empty()));
    }

    #[test]
    fn oversized_markdown_blocks_fall_back_to_markdown_splitter() {
        let text = format!("# Heading\n\n- {}\n", "alpha ".repeat(180));
        let chunks = split_text_for_path(Path::new("/tmp/readme.md"), &text, 24, 0)
            .expect("markdown split should succeed");

        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| {
            chunk.heading_path == vec!["Heading".to_string()]
                && chunk.text.chars().count() <= MARKDOWN_CHUNK_HARD_MAX_CHARS
        }));
    }

    #[test]
    fn collect_chunks_for_path_skips_unsupported_extension() {
        let root = temp_test_root("unsupported-extension");
        let file_path = root.join("notes.toml");
        std::fs::create_dir_all(&root).expect("create rag temp root");
        std::fs::write(&file_path, "title = \"not indexed\"").expect("write rag source file");

        let resolved = test_resolved_config(&root);

        let chunks = collect_chunks_for_path(&resolved, &file_path)
            .expect("collecting chunks should succeed");

        assert!(chunks.is_empty());

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_chunks_for_path_supports_text_files_under_50_mb() {
        let root = temp_test_root("large-file");
        let file_path = root.join("large.txt");
        let content = "alpha beta gamma delta epsilon zeta eta theta iota kappa\n".repeat(20_000);
        std::fs::create_dir_all(&root).expect("create rag temp root");
        std::fs::write(&file_path, &content).expect("write rag source file");

        let resolved = test_resolved_config(&root);

        let file_size = std::fs::metadata(&file_path).expect("read metadata").len();
        assert!(file_size > 1_000_000);
        assert!(file_size < MAX_TEXT_FILE_BYTES);

        let chunks = collect_chunks_for_path(&resolved, &file_path)
            .expect("collecting chunks should succeed for large files");

        assert!(!chunks.is_empty());
        assert!(chunks.len() > 1);

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_chunks_for_path_supports_docx_files() {
        let root = temp_test_root("docx-file");
        let file_path = root.join("notes.docx");
        let document_xml = r#"
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:body>
                <w:p>
                  <w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
                  <w:r><w:t>Architecture</w:t></w:r>
                </w:p>
                <w:p>
                  <w:r><w:t>Alpha paragraph.</w:t></w:r>
                </w:p>
              </w:body>
            </w:document>
        "#;
        std::fs::create_dir_all(&root).expect("create rag temp root");
        std::fs::write(&file_path, build_test_docx(document_xml, None)).expect("write docx");

        let resolved = test_resolved_config(&root);
        let chunks = collect_chunks_for_path(&resolved, &file_path)
            .expect("collect docx chunks should work");

        assert!(!chunks.is_empty());
        assert!(chunks
            .iter()
            .any(|chunk| chunk.heading_path == vec!["Architecture".to_string()]));
        assert!(chunks
            .iter()
            .any(|chunk| chunk.text.contains("Alpha paragraph.")));

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_chunks_for_path_skips_text_files_over_50_mb() {
        let root = temp_test_root("too-large-file");
        let file_path = root.join("too-large.txt");
        std::fs::create_dir_all(&root).expect("create rag temp root");
        let file = std::fs::File::create(&file_path).expect("create oversized rag source file");
        file.set_len(MAX_TEXT_FILE_BYTES + 1)
            .expect("set oversized rag source length");

        let resolved = test_resolved_config(&root);

        let chunks = collect_chunks_for_path(&resolved, &file_path)
            .expect("collecting chunks should skip oversized files");

        assert!(chunks.is_empty());

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn inspect_path_for_index_skips_hashing_when_size_and_mtime_match() {
        let root = temp_test_root("metadata-fast-path");
        let file_path = root.join("notes.txt");
        std::fs::create_dir_all(&root).expect("create rag temp root");
        std::fs::write(&file_path, "alpha beta gamma").expect("write rag source file");

        let resolved = test_resolved_config(&root);
        let file_metadata = std::fs::metadata(&file_path).expect("read metadata");
        let stored_record = test_indexed_record(
            &root,
            &file_path,
            &resolved.embedding_fingerprint,
            Some(test_active_version(
                "unused-fast-path",
                file_metadata
                    .modified()
                    .ok()
                    .and_then(system_time_to_unix_ms),
                i64::try_from(file_metadata.len()).expect("file size fits i64"),
                3,
                now_unix_ms(),
            )),
            None,
        );

        let outcome = inspect_path_for_index(&resolved, &file_path, Some(&stored_record))
            .expect("inspect path should succeed");

        match outcome {
            InspectPathOutcome::Unchanged {
                record,
                refresh_metadata,
                clear_staged,
            } => {
                assert_eq!(record, stored_record);
                assert!(!refresh_metadata);
                assert!(!clear_staged);
            }
            other => panic!("expected unchanged fast path, got {other:?}"),
        }

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn inspect_path_for_index_refreshes_metadata_when_md5_matches() {
        let root = temp_test_root("metadata-refresh");
        let file_path = root.join("notes.txt");
        let content = "alpha beta gamma";
        std::fs::create_dir_all(&root).expect("create rag temp root");
        std::fs::write(&file_path, content).expect("write rag source file");

        let resolved = test_resolved_config(&root);
        let file_metadata = std::fs::metadata(&file_path).expect("read metadata");
        let stored_record = test_indexed_record(
            &root,
            &file_path,
            &resolved.embedding_fingerprint,
            Some(test_active_version(
                &format!("{:x}", md5::compute(content.as_bytes())),
                None,
                i64::try_from(file_metadata.len()).expect("file size fits i64"),
                3,
                1,
            )),
            None,
        );

        let outcome = inspect_path_for_index(&resolved, &file_path, Some(&stored_record))
            .expect("inspect path should succeed");

        match outcome {
            InspectPathOutcome::Unchanged {
                record,
                refresh_metadata,
                clear_staged,
            } => {
                let active = record.active.expect("active version should exist");
                assert!(refresh_metadata);
                assert!(!clear_staged);
                assert_eq!(
                    active.content_md5,
                    stored_record
                        .active
                        .as_ref()
                        .expect("stored active version")
                        .content_md5
                );
                assert_eq!(
                    active.size_bytes,
                    stored_record
                        .active
                        .as_ref()
                        .expect("stored active version")
                        .size_bytes
                );
                assert_eq!(
                    active.modified_at_ms,
                    file_metadata
                        .modified()
                        .ok()
                        .and_then(system_time_to_unix_ms)
                );
                assert!(record.pending.is_none());
            }
            other => panic!("expected unchanged refresh path, got {other:?}"),
        }

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn inspect_path_for_index_clears_pending_record_when_active_metadata_matches() {
        let root = temp_test_root("metadata-pending");
        let file_path = root.join("notes.txt");
        let content = "alpha beta gamma";
        std::fs::create_dir_all(&root).expect("create rag temp root");
        std::fs::write(&file_path, content).expect("write rag source file");

        let resolved = test_resolved_config(&root);
        let file_metadata = std::fs::metadata(&file_path).expect("read metadata");
        let content_md5 = format!("{:x}", md5::compute(content.as_bytes()));
        let stored_record = test_indexed_record(
            &root,
            &file_path,
            &resolved.embedding_fingerprint,
            Some(test_active_version(
                &content_md5,
                file_metadata
                    .modified()
                    .ok()
                    .and_then(system_time_to_unix_ms),
                i64::try_from(file_metadata.len()).expect("file size fits i64"),
                3,
                7,
            )),
            Some(test_pending_version(
                &content_md5,
                file_metadata
                    .modified()
                    .ok()
                    .and_then(system_time_to_unix_ms),
                i64::try_from(file_metadata.len()).expect("file size fits i64"),
                3,
                7,
            )),
        );

        let outcome = inspect_path_for_index(&resolved, &file_path, Some(&stored_record))
            .expect("inspect path should succeed");

        match outcome {
            InspectPathOutcome::Unchanged {
                record,
                refresh_metadata,
                clear_staged,
            } => {
                assert!(refresh_metadata);
                assert!(clear_staged);
                assert!(record.active.is_some());
                assert!(record.pending.is_none());
            }
            other => panic!("expected pending record to be cleared, got {other:?}"),
        }

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn inspect_path_for_index_reindexes_when_embedding_fingerprint_changes() {
        let root = temp_test_root("metadata-model-change");
        let file_path = root.join("notes.txt");
        let content = "alpha beta gamma";
        std::fs::create_dir_all(&root).expect("create rag temp root");
        std::fs::write(&file_path, content).expect("write rag source file");

        let resolved = test_resolved_config(&root);
        let file_metadata = std::fs::metadata(&file_path).expect("read metadata");
        let stored_record = test_indexed_record(
            &root,
            &file_path,
            &test_embedding_fingerprint_for(
                "https://other.example.com/v1",
                "text-embedding-3-small",
            ),
            Some(test_active_version(
                &format!("{:x}", md5::compute(content.as_bytes())),
                file_metadata
                    .modified()
                    .ok()
                    .and_then(system_time_to_unix_ms),
                i64::try_from(file_metadata.len()).expect("file size fits i64"),
                3,
                9,
            )),
            None,
        );

        let outcome = inspect_path_for_index(&resolved, &file_path, Some(&stored_record))
            .expect("inspect path should succeed");

        match outcome {
            InspectPathOutcome::Reindex(file) => {
                assert_eq!(
                    file.record.embedding_fingerprint,
                    resolved.embedding_fingerprint
                );
                assert!(file.record.pending.is_some());
            }
            other => panic!("expected fingerprint change to trigger reindex, got {other:?}"),
        }

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn watcher_directory_metadata_event_does_not_force_full_rescan() {
        assert!(!should_force_full_rescan_for_existing_directory(
            EventKind::Modify(ModifyKind::Metadata(notify::event::MetadataKind::WriteTime))
        ));
    }

    #[test]
    fn watcher_directory_create_event_does_not_force_full_rescan() {
        assert!(!should_force_full_rescan_for_existing_directory(
            EventKind::Create(notify::event::CreateKind::Folder,)
        ));
    }

    #[test]
    fn watcher_directory_rename_event_forces_full_rescan() {
        assert!(should_force_full_rescan_for_existing_directory(
            EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::Both))
        ));
    }

    #[test]
    fn watcher_other_event_only_forces_full_rescan_without_paths() {
        let with_paths = Event {
            kind: EventKind::Other,
            paths: vec![PathBuf::from("/tmp/docs/a.md")],
            attrs: Default::default(),
        };
        let without_paths = Event {
            kind: EventKind::Other,
            paths: Vec::new(),
            attrs: Default::default(),
        };

        assert!(!should_force_full_rescan_for_event(&with_paths));
        assert!(should_force_full_rescan_for_event(&without_paths));
    }

    #[test]
    fn watcher_metadata_event_is_ignored_for_indexing() {
        let metadata_event = Event {
            kind: EventKind::Modify(ModifyKind::Metadata(notify::event::MetadataKind::WriteTime)),
            paths: vec![PathBuf::from("/tmp/docs/a.md")],
            attrs: Default::default(),
        };
        let data_event = Event {
            kind: EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            paths: vec![PathBuf::from("/tmp/docs/a.md")],
            attrs: Default::default(),
        };

        assert!(should_ignore_event_for_indexing(&metadata_event));
        assert!(!should_ignore_event_for_indexing(&data_event));
    }

    #[test]
    fn rag_runtime_start_reuses_index_when_effective_inputs_do_not_change() {
        let previous = test_runtime_inputs(Some("rag-provider"), &[("rag-provider", "model-a")]);
        let same_model_different_unrelated_provider = test_runtime_inputs(
            Some("rag-provider"),
            &[("rag-provider", "model-a"), ("other-provider", "model-b")],
        );

        assert_eq!(
            classify_rag_runtime_start(None, &previous),
            RagRuntimeStartMode::ReuseIndex
        );
        assert_eq!(
            classify_rag_runtime_start(Some(&previous), &same_model_different_unrelated_provider),
            RagRuntimeStartMode::ReuseIndex
        );
    }

    #[test]
    fn rag_runtime_start_rebuilds_when_embedding_target_changes() {
        let previous = test_runtime_inputs(Some("rag-provider"), &[("rag-provider", "model-a")]);
        let changed_rag_provider_model = test_runtime_inputs(
            Some("rag-provider"),
            &[("rag-provider", "model-c"), ("other-provider", "model-b")],
        );
        let switched_rag_provider_with_same_target = test_runtime_inputs(
            Some("other-provider"),
            &[("rag-provider", "model-a"), ("other-provider", "model-a")],
        );
        let switched_rag_provider_with_different_target = test_runtime_inputs(
            Some("other-provider"),
            &[("rag-provider", "model-a"), ("other-provider", "model-b")],
        );

        assert_eq!(
            classify_rag_runtime_start(Some(&previous), &changed_rag_provider_model),
            RagRuntimeStartMode::RebuildIndex
        );
        assert_eq!(
            classify_rag_runtime_start(Some(&previous), &switched_rag_provider_with_same_target),
            RagRuntimeStartMode::ReuseIndex
        );
        assert_eq!(
            classify_rag_runtime_start(
                Some(&previous),
                &switched_rag_provider_with_different_target
            ),
            RagRuntimeStartMode::RebuildIndex
        );
    }

    #[test]
    fn embedding_fingerprint_reuses_stable_digest_across_base_urls() {
        let digest_model = concat!(
            "mxbai-embed-large@sha256:",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        let first = test_embedding_fingerprint_for("http://127.0.0.1:11434/v1", digest_model);
        let second = test_embedding_fingerprint_for("http://192.168.1.10:11434/v1", digest_model);

        assert_eq!(first, second);
    }

    #[test]
    fn embedding_fingerprint_uses_global_identity_for_official_openai_models() {
        let first =
            test_embedding_fingerprint_for("https://api.openai.com/v1", "text-embedding-3-small");
        let second = test_embedding_fingerprint_for(
            " https://api.openai.com/v1/ ",
            "text-embedding-3-small",
        );

        assert_eq!(first, second);
    }

    #[test]
    fn embedding_fingerprint_keeps_generic_compatible_endpoints_separate() {
        let first = test_embedding_fingerprint_for(
            "https://proxy-a.example.com/v1",
            "text-embedding-3-small",
        );
        let second = test_embedding_fingerprint_for(
            "https://proxy-b.example.com/v1",
            "text-embedding-3-small",
        );

        assert_ne!(first, second);
    }

    #[test]
    fn embedding_fingerprint_reuses_generic_endpoint_when_identity_hint_matches() {
        let first = embedding_fingerprint(&test_embedding_provider_with_hint(
            "embedding",
            "https://proxy-a.example.com/v1",
            "text-embedding-3-small",
            Some("digest:sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        ))
        .expect("fingerprint should resolve");
        let second = embedding_fingerprint(&test_embedding_provider_with_hint(
            "embedding",
            "https://proxy-b.example.com/v1",
            "text-embedding-3-small",
            Some("digest:sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        ))
        .expect("fingerprint should resolve");

        assert_eq!(first, second);
    }

    #[test]
    fn rag_runtime_start_reuses_index_when_digest_model_moves_endpoints() {
        let digest_model = concat!(
            "mxbai-embed-large@sha256:",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        let previous = test_runtime_inputs_with_provider_targets(
            vec!["/tmp/docs".to_string()],
            Some("rag-provider"),
            &[(
                "rag-provider",
                "http://127.0.0.1:11434/v1",
                digest_model,
                None,
            )],
        );
        let next = test_runtime_inputs_with_provider_targets(
            vec!["/tmp/docs".to_string()],
            Some("rag-provider"),
            &[(
                "rag-provider",
                "http://192.168.1.10:11434/v1",
                digest_model,
                None,
            )],
        );

        assert_eq!(
            classify_rag_runtime_start(Some(&previous), &next),
            RagRuntimeStartMode::ReuseIndex
        );
    }

    #[test]
    fn rag_runtime_start_reuses_index_when_generic_endpoint_hint_matches() {
        let identity_hint =
            Some("digest:sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
        let previous = test_runtime_inputs_with_provider_targets(
            vec!["/tmp/docs".to_string()],
            Some("rag-provider"),
            &[(
                "rag-provider",
                "https://proxy-a.example.com/v1",
                "text-embedding-3-small",
                identity_hint,
            )],
        );
        let next = test_runtime_inputs_with_provider_targets(
            vec!["/tmp/docs".to_string()],
            Some("rag-provider"),
            &[(
                "rag-provider",
                "https://proxy-b.example.com/v1",
                "text-embedding-3-small",
                identity_hint,
            )],
        );

        assert_eq!(
            classify_rag_runtime_start(Some(&previous), &next),
            RagRuntimeStartMode::ReuseIndex
        );
    }

    #[test]
    fn rag_runtime_start_reuses_index_for_equivalent_source_directory_paths() {
        let root = temp_test_root("runtime-input-path-normalization");
        std::fs::create_dir_all(&root).expect("create rag temp root");
        let root_display = root.to_string_lossy().into_owned();
        let trailing_root_display = format!("{root_display}/");
        let previous = test_runtime_inputs_with_directories(
            vec![root_display],
            Some("rag-provider"),
            &[("rag-provider", "model-a")],
        );
        let next = test_runtime_inputs_with_directories(
            vec![trailing_root_display],
            Some("rag-provider"),
            &[("rag-provider", "model-a")],
        );

        assert_eq!(
            classify_rag_runtime_start(Some(&previous), &next),
            RagRuntimeStartMode::ReuseIndex
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn run_watch_loop_preserves_existing_storage_when_config_is_invalid() {
        let root = temp_test_root("invalid-config-preserves-storage");
        let database_path = root.join("rag-lancedb");
        let metadata_path = root.join("rag.sqlite3");
        std::fs::create_dir_all(&root).expect("create rag temp root");

        let chunk = RagChunk {
            id: "chunk-1".to_string(),
            source_root: "/tmp/docs".to_string(),
            absolute_path: "/tmp/docs/a.md".to_string(),
            version_id: "active-v1".to_string(),
            embedding_fingerprint: test_embedding_fingerprint(),
            chunk_state: RagChunkState::Active,
            chunk_index: 0,
            line_start: 1,
            line_end: 3,
            paragraph_line_start: 1,
            heading_path: vec!["Intro".to_string()],
            chunk_reuse_key: "reuse-1".to_string(),
            text_fingerprint: text_fingerprint("current chunk"),
            text: "current chunk".to_string(),
        };
        let batch_reader =
            build_record_batch_reader(&[chunk], &[vec![1.0_f32, 2.0_f32]]).expect("build batch");
        let db = connect(database_path.to_string_lossy().as_ref())
            .execute()
            .await
            .expect("open lancedb");
        db.create_table(RAG_TABLE_NAME, batch_reader)
            .execute()
            .await
            .expect("create current rag table");

        let indexed_record = RagIndexedFileRecord {
            source_root: "/tmp/docs".to_string(),
            absolute_path: "/tmp/docs/a.md".to_string(),
            relative_path: "a.md".to_string(),
            embedding_fingerprint: test_embedding_fingerprint(),
            active: Some(test_active_version("md5-a", Some(1), 10, 1, 42)),
            pending: None,
        };
        upsert_metadata_records(&metadata_path, &[indexed_record]).expect("write metadata row");

        let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
        let runtime_generation = Arc::new(AtomicU64::new(0));
        let runtime_context = RagRuntimeContext {
            runtime_status: runtime_status.clone(),
            runtime_generation: runtime_generation.clone(),
            storage_lock: Arc::new(AsyncMutex::new(())),
            generation: 1,
        };
        let error = run_watch_loop(
            root.clone(),
            RagSettings {
                source_directories: vec![root.join("missing").to_string_lossy().into_owned()],
                ignore_globs: Vec::new(),
                embedding_provider_id: Some("embedding".to_string()),
            },
            LlmSettings {
                providers: vec![LlmProviderConfig {
                    id: "embedding".to_string(),
                    name: "Embedding".to_string(),
                    base_url: "https://api.example.com/v1".to_string(),
                    api_key: String::new(),
                    model_type: crate::domain::settings::LlmModelType::Embedding,
                    model: "text-embedding-3-small".to_string(),
                    supports_multimodal: false,
                    ..LlmProviderConfig::default()
                }],
                ..LlmSettings::default()
            },
            runtime_context,
            RagRuntimeStartMode::ReuseIndex,
        )
        .await
        .expect_err("invalid config should stop watcher loop");

        assert!(error
            .to_string()
            .contains("failed to resolve RAG source directory"));
        assert!(load_rag_table_schema(&database_path)
            .await
            .expect("query rag table state")
            .is_some());
        assert_eq!(
            load_metadata_records(&metadata_path)
                .expect("load metadata rows")
                .len(),
            1
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn apply_settings_restarts_exited_error_watcher_with_same_inputs() {
        let root = temp_test_root("restart-exited-error-watcher");
        std::fs::create_dir_all(&root).expect("create rag temp root");

        let service = RagIndexService::new(root.clone());
        let rag_settings = RagSettings {
            source_directories: vec![root.join("missing").to_string_lossy().into_owned()],
            ignore_globs: Vec::new(),
            embedding_provider_id: Some("embedding".to_string()),
        };
        let llm_settings = LlmSettings {
            providers: vec![test_embedding_provider()],
            ..LlmSettings::default()
        };

        service
            .apply_settings(rag_settings.clone(), llm_settings.clone())
            .await;
        wait_for_runtime_exit(&service).await;

        assert_eq!(service.runtime_generation.load(Ordering::SeqCst), 1);
        assert_eq!(service.runtime_status().await.phase, RagRuntimePhase::Error);

        service.apply_settings(rag_settings, llm_settings).await;

        assert_eq!(service.runtime_generation.load(Ordering::SeqCst), 2);

        wait_for_runtime_exit(&service).await;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn apply_settings_skips_restart_for_finished_non_error_runtime_with_same_inputs() {
        let root = temp_test_root("skip-finished-non-error-runtime");
        std::fs::create_dir_all(&root).expect("create rag temp root");

        let service = RagIndexService::new(root.clone());
        let rag_settings = RagSettings::default();
        let llm_settings = LlmSettings::default();

        service
            .apply_settings(rag_settings.clone(), llm_settings.clone())
            .await;
        wait_for_runtime_exit(&service).await;

        assert_eq!(service.runtime_generation.load(Ordering::SeqCst), 1);
        assert_eq!(service.runtime_status().await.phase, RagRuntimePhase::Idle);

        service.apply_settings(rag_settings, llm_settings).await;

        assert_eq!(service.runtime_generation.load(Ordering::SeqCst), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rag_runtime_start_rebuilds_when_source_directories_change() {
        let previous = test_runtime_inputs(Some("rag-provider"), &[("rag-provider", "model-a")]);
        let next = RagRuntimeInputs::from_settings(
            &RagSettings {
                source_directories: vec!["/tmp/other-docs".to_string()],
                ignore_globs: vec![],
                embedding_provider_id: Some("rag-provider".to_string()),
            },
            &LlmSettings {
                providers: vec![LlmProviderConfig {
                    id: "rag-provider".to_string(),
                    name: "rag-provider".to_string(),
                    base_url: "https://api.example.com/v1".to_string(),
                    api_key: String::new(),
                    model_type: crate::domain::settings::LlmModelType::Embedding,
                    model: "model-a".to_string(),
                    supports_multimodal: false,
                    ..LlmProviderConfig::default()
                }],
                ..LlmSettings::default()
            },
        );

        assert_eq!(
            classify_rag_runtime_start(Some(&previous), &next),
            RagRuntimeStartMode::RebuildIndex
        );
    }

    #[test]
    fn rag_runtime_start_rebuilds_when_ignore_globs_change() {
        let previous = test_runtime_inputs(Some("rag-provider"), &[("rag-provider", "model-a")]);
        let next = RagRuntimeInputs::from_settings(
            &RagSettings {
                source_directories: vec!["/tmp/docs".to_string()],
                ignore_globs: vec!["**/node_modules/**".to_string()],
                embedding_provider_id: Some("rag-provider".to_string()),
            },
            &LlmSettings {
                providers: vec![LlmProviderConfig {
                    id: "rag-provider".to_string(),
                    name: "rag-provider".to_string(),
                    base_url: "https://api.example.com/v1".to_string(),
                    api_key: String::new(),
                    model_type: crate::domain::settings::LlmModelType::Embedding,
                    model: "model-a".to_string(),
                    supports_multimodal: false,
                    ..LlmProviderConfig::default()
                }],
                ..LlmSettings::default()
            },
        );

        assert_eq!(
            classify_rag_runtime_start(Some(&previous), &next),
            RagRuntimeStartMode::RebuildIndex
        );
    }

    #[test]
    fn metadata_store_tracks_pending_status() {
        let root = temp_test_root("metadata-store");
        let metadata_path = root.join("rag.sqlite3");
        std::fs::create_dir_all(&root).expect("create rag temp root");

        let pending_record = RagIndexedFileRecord {
            source_root: "/tmp/source".to_string(),
            absolute_path: "/tmp/source/a.txt".to_string(),
            relative_path: "a.txt".to_string(),
            embedding_fingerprint: test_embedding_fingerprint(),
            active: None,
            pending: Some(test_pending_version("md5-a", Some(1), 10, 2, 0)),
        };
        upsert_metadata_records(&metadata_path, std::slice::from_ref(&pending_record))
            .expect("upsert pending record");
        assert!(metadata_store_has_pending_rows(&metadata_path).expect("query pending records"));

        let indexed_record = RagIndexedFileRecord {
            active: Some(test_active_version("md5-a", Some(1), 10, 2, 99)),
            pending: None,
            ..pending_record.clone()
        };
        upsert_metadata_records(&metadata_path, std::slice::from_ref(&indexed_record))
            .expect("upsert indexed record");

        let loaded = load_metadata_records(&metadata_path)
            .expect("load metadata records")
            .remove(&indexed_record.absolute_path)
            .expect("metadata record should exist");
        assert_eq!(
            loaded
                .active
                .expect("active version should exist")
                .indexed_at_ms,
            99
        );
        assert_eq!(
            loaded.embedding_fingerprint,
            indexed_record.embedding_fingerprint
        );
        assert!(loaded.pending.is_none());
        assert!(!metadata_store_has_pending_rows(&metadata_path).expect("query pending rows"));

        let _ = std::fs::remove_file(&metadata_path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stream_rebuild_scan_defers_stale_cleanup_until_after_reindex_candidates_are_emitted() {
        let root = temp_test_root("stream-rebuild-scan");
        std::fs::create_dir_all(&root).expect("create rebuild scan root");
        let canonical_root = root.canonicalize().expect("canonicalize rebuild scan root");

        let changed_path = root.join("changed.md");
        std::fs::write(&changed_path, "# Title\n\nfresh content\n")
            .expect("write changed RAG file");
        let changed_path = changed_path
            .canonicalize()
            .expect("canonicalize changed RAG file");
        let removed_path = canonical_root.join("removed.md");
        let resolved = test_resolved_config(&canonical_root);
        let mut stored_records = HashMap::new();
        stored_records.insert(
            normalize_path_string(&changed_path),
            test_indexed_record(
                &canonical_root,
                &changed_path,
                &resolved.embedding_fingerprint,
                Some(test_active_version("old-md5", Some(0), 1, 1, 0)),
                None,
            ),
        );
        stored_records.insert(
            normalize_path_string(&removed_path),
            test_indexed_record(
                &canonical_root,
                &removed_path,
                &resolved.embedding_fingerprint,
                Some(test_active_version("removed-md5", Some(0), 1, 1, 0)),
                None,
            ),
        );

        let (scan_tx, mut scan_rx) = mpsc::channel(8);
        stream_rebuild_scan(&resolved, &stored_records, scan_tx)
            .expect("streaming rebuild scan should succeed");

        let mut events = Vec::new();
        while let Some(event) = scan_rx.blocking_recv() {
            events.push(event);
        }

        let changed_path = normalize_path_string(&changed_path);
        let removed_path = normalize_path_string(&removed_path);
        let reindex_event_index = events
            .iter()
            .position(|event| {
                event
                    .file_to_index
                    .as_ref()
                    .map(|file| file.record.absolute_path == changed_path)
                    .unwrap_or(false)
            })
            .expect("changed file should be emitted for reindex");
        let stale_event_index = events
            .iter()
            .position(|event| event.stale_paths.iter().any(|path| path == &removed_path))
            .expect("removed file should be emitted as stale");

        assert!(reindex_event_index < stale_event_index);

        let _ = std::fs::remove_file(root.join("changed.md"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rag_table_schema_rejects_legacy_layout() {
        let legacy_schema = Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("absolute_path", DataType::Utf8, false),
            Field::new("path", DataType::Utf8, false),
            Field::new("chunk_index", DataType::Int32, false),
            Field::new("line_start", DataType::Int32, false),
            Field::new("line_end", DataType::Int32, false),
            Field::new("text", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 2),
                true,
            ),
        ]);

        assert!(!rag_table_schema_is_compatible(&legacy_schema));
    }

    #[test]
    fn metadata_table_schema_rejects_legacy_layout() {
        let root = temp_test_root("legacy-metadata-schema");
        let metadata_path = root.join("rag.sqlite3");
        std::fs::create_dir_all(&root).expect("create rag temp root");

        let connection = Connection::open(&metadata_path).expect("open legacy metadata database");
        connection
            .execute_batch(
                "
                CREATE TABLE rag_files (
                    absolute_path TEXT PRIMARY KEY NOT NULL,
                    source_root TEXT NOT NULL,
                    relative_path TEXT NOT NULL,
                    content_md5 TEXT NOT NULL,
                    modified_at_ms INTEGER,
                    size_bytes INTEGER NOT NULL,
                    chunk_count INTEGER NOT NULL,
                    indexed_at_ms INTEGER NOT NULL,
                    indexing_status TEXT NOT NULL DEFAULT 'indexed',
                    embedding_fingerprint TEXT NOT NULL DEFAULT ''
                );
                ",
            )
            .expect("create legacy metadata schema");

        assert!(!metadata_store_has_compatible_schema(&metadata_path)
            .expect("inspect metadata schema compatibility"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stream_rebuild_scan_does_not_apply_gitignore_as_implicit_filter() {
        let root = temp_test_root("stream-rebuild-gitignore");
        std::fs::create_dir_all(&root).expect("create gitignore scan root");
        std::fs::write(root.join(".gitignore"), "ignored.md\n").expect("write gitignore");
        std::fs::write(root.join("ignored.md"), "# Visible\n\nstill indexed\n")
            .expect("write ignored-looking file");
        let canonical_root = root
            .canonicalize()
            .expect("canonicalize gitignore scan root");
        let resolved = test_resolved_config(&canonical_root);

        let (scan_tx, mut scan_rx) = mpsc::channel(8);
        stream_rebuild_scan(&resolved, &HashMap::new(), scan_tx)
            .expect("streaming rebuild scan should succeed");

        let events = std::iter::from_fn(|| scan_rx.blocking_recv()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| {
            event
                .file_to_index
                .as_ref()
                .map(|file| file.record.relative_path == "ignored.md")
                .unwrap_or(false)
        }));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn embedding_batch_planner_grows_slowly_after_three_clean_full_batches() {
        let mut planner = EmbeddingBatchPlanner::default();

        for _ in 0..2 {
            planner.record_success(
                EMBEDDING_BATCH_SIZE_DEFAULT,
                EmbeddingRequestStats {
                    largest_successful_batch_size: EMBEDDING_BATCH_SIZE_DEFAULT,
                    split_retry_count: 0,
                },
            );
        }

        assert_eq!(planner.current_size, EMBEDDING_BATCH_SIZE_DEFAULT);

        planner.record_success(
            EMBEDDING_BATCH_SIZE_DEFAULT,
            EmbeddingRequestStats {
                largest_successful_batch_size: EMBEDDING_BATCH_SIZE_DEFAULT,
                split_retry_count: 0,
            },
        );

        assert_eq!(planner.current_size, EMBEDDING_BATCH_SIZE_DEFAULT + 2);
    }

    #[test]
    fn embedding_batch_planner_shrinks_to_stable_size_after_split_retry() {
        let mut planner = EmbeddingBatchPlanner {
            current_size: 64,
            clean_success_streak: 2,
            cooldown_rounds: 0,
        };

        planner.record_success(
            64,
            EmbeddingRequestStats {
                largest_successful_batch_size: 4,
                split_retry_count: 2,
            },
        );

        assert_eq!(planner.current_size, 4);
        assert_eq!(planner.clean_success_streak, 0);
        assert_eq!(planner.cooldown_rounds, EMBEDDING_BATCH_COOLDOWN_ROUNDS);
    }

    #[test]
    fn embedding_batch_planner_limits_batch_by_total_chars() {
        let planner = EmbeddingBatchPlanner::default();
        let inputs = vec![
            "a".repeat(4_500),
            "b".repeat(4_500),
            "c".repeat(4_500),
            "d".repeat(12_000),
        ];

        assert_eq!(planner.next_batch_end(&inputs, 0), 2);
        assert_eq!(planner.next_batch_end(&inputs, 2), 3);
        assert_eq!(planner.next_batch_end(&inputs, 3), 4);
    }

    #[tokio::test]
    async fn vector_index_policy_skips_small_incremental_batches() {
        let root = temp_test_root("vector-index-policy-small");
        std::fs::create_dir_all(&root).expect("create vector policy root");
        let mut vector_store = RagVectorStore::open(&root)
            .await
            .expect("open vector store");

        vector_store.mark_index_dirty_for_chunks(VECTOR_INDEX_REBUILD_MIN_DIRTY_CHUNKS - 1);
        vector_store.mark_index_dirty_for_delete();

        assert!(vector_store.index_dirty);
        assert!(!vector_store.should_rebuild_index());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn vector_index_policy_rebuilds_new_table_immediately() {
        let root = temp_test_root("vector-index-policy-created-table");
        std::fs::create_dir_all(&root).expect("create vector policy root");
        let mut vector_store = RagVectorStore::open(&root)
            .await
            .expect("open vector store");

        vector_store.created_table = true;
        vector_store.mark_index_dirty_for_chunks(1);

        assert!(vector_store.should_rebuild_index());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn prepare_index_storage_clears_legacy_lancedb_table_and_metadata() {
        let root = temp_test_root("legacy-lancedb-reset");
        let database_path = root.join("rag-lancedb");
        let metadata_path = root.join("rag.sqlite3");
        std::fs::create_dir_all(&root).expect("create rag temp root");

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("absolute_path", DataType::Utf8, false),
            Field::new("path", DataType::Utf8, false),
            Field::new("chunk_index", DataType::Int32, false),
            Field::new("line_start", DataType::Int32, false),
            Field::new("line_end", DataType::Int32, false),
            Field::new("text", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 2),
                true,
            ),
        ]));
        let vector_array = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
            vec![Some([Some(1.0_f32), Some(2.0_f32)].into_iter())],
            2,
        );
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["chunk-1"])),
                Arc::new(StringArray::from(vec!["/tmp/docs/a.md"])),
                Arc::new(StringArray::from(vec!["/tmp/docs/a.md"])),
                Arc::new(Int32Array::from(vec![0])),
                Arc::new(Int32Array::from(vec![1])),
                Arc::new(Int32Array::from(vec![3])),
                Arc::new(StringArray::from(vec!["legacy chunk"])),
                Arc::new(vector_array),
            ],
        )
        .expect("build legacy lance record batch");
        let db = connect(database_path.to_string_lossy().as_ref())
            .execute()
            .await
            .expect("open legacy lancedb");
        db.create_table(
            RAG_TABLE_NAME,
            RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema),
        )
        .execute()
        .await
        .expect("create legacy rag table");

        let indexed_record = RagIndexedFileRecord {
            source_root: "/tmp/docs".to_string(),
            absolute_path: "/tmp/docs/a.md".to_string(),
            relative_path: "a.md".to_string(),
            embedding_fingerprint: test_embedding_fingerprint(),
            active: Some(test_active_version("md5-a", Some(1), 10, 1, 42)),
            pending: None,
        };
        upsert_metadata_records(&metadata_path, &[indexed_record]).expect("write metadata row");

        prepare_index_storage(
            &database_path,
            &metadata_path,
            &test_resolved_config(&root),
            true,
        )
        .await
        .expect("prepare index storage should reset incompatible storage");

        assert!(load_rag_table_schema(&database_path)
            .await
            .expect("query rag table state")
            .is_none());
        assert!(load_metadata_records(&metadata_path)
            .expect("load metadata rows")
            .is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn prepare_index_storage_keeps_existing_rows_when_fingerprint_reset_is_disabled() {
        let root = temp_test_root("fingerprint-reset-disabled");
        let database_path = root.join("rag-lancedb");
        let metadata_path = root.join("rag.sqlite3");
        std::fs::create_dir_all(&root).expect("create rag temp root");

        let chunk = RagChunk {
            id: "chunk-1".to_string(),
            source_root: "/tmp/docs".to_string(),
            absolute_path: "/tmp/docs/a.md".to_string(),
            version_id: "active-v1".to_string(),
            embedding_fingerprint: test_embedding_fingerprint(),
            chunk_state: RagChunkState::Active,
            chunk_index: 0,
            line_start: 1,
            line_end: 3,
            paragraph_line_start: 1,
            heading_path: vec!["Intro".to_string()],
            chunk_reuse_key: "reuse-1".to_string(),
            text_fingerprint: text_fingerprint("current chunk"),
            text: "current chunk".to_string(),
        };
        let batch_reader =
            build_record_batch_reader(&[chunk], &[vec![1.0_f32, 2.0_f32]]).expect("build batch");
        let db = connect(database_path.to_string_lossy().as_ref())
            .execute()
            .await
            .expect("open lancedb");
        db.create_table(RAG_TABLE_NAME, batch_reader)
            .execute()
            .await
            .expect("create current rag table");

        let indexed_record = RagIndexedFileRecord {
            source_root: "/tmp/docs".to_string(),
            absolute_path: "/tmp/docs/a.md".to_string(),
            relative_path: "a.md".to_string(),
            embedding_fingerprint: test_embedding_fingerprint_for(
                "https://other.example.com/v1",
                "text-embedding-3-small",
            ),
            active: Some(test_active_version("md5-a", Some(1), 10, 1, 42)),
            pending: None,
        };
        upsert_metadata_records(&metadata_path, &[indexed_record]).expect("write metadata row");

        prepare_index_storage(
            &database_path,
            &metadata_path,
            &test_resolved_config(&root),
            false,
        )
        .await
        .expect("prepare index storage should preserve mismatched fingerprint rows");

        assert!(load_rag_table_schema(&database_path)
            .await
            .expect("query rag table state")
            .is_some());
        assert_eq!(
            load_metadata_records(&metadata_path)
                .expect("load metadata rows")
                .len(),
            1
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn prepare_index_storage_resets_when_embedding_fingerprint_changes() {
        let root = temp_test_root("fingerprint-reset-enabled");
        let database_path = root.join("rag-lancedb");
        let metadata_path = root.join("rag.sqlite3");
        std::fs::create_dir_all(&root).expect("create rag temp root");

        let chunk = RagChunk {
            id: "chunk-1".to_string(),
            source_root: "/tmp/docs".to_string(),
            absolute_path: "/tmp/docs/a.md".to_string(),
            version_id: "active-v1".to_string(),
            embedding_fingerprint: test_embedding_fingerprint_for(
                "https://other.example.com/v1",
                "text-embedding-3-small",
            ),
            chunk_state: RagChunkState::Active,
            chunk_index: 0,
            line_start: 1,
            line_end: 3,
            paragraph_line_start: 1,
            heading_path: vec!["Intro".to_string()],
            chunk_reuse_key: "reuse-1".to_string(),
            text_fingerprint: text_fingerprint("current chunk"),
            text: "current chunk".to_string(),
        };
        let batch_reader =
            build_record_batch_reader(&[chunk], &[vec![1.0_f32, 2.0_f32]]).expect("build batch");
        let db = connect(database_path.to_string_lossy().as_ref())
            .execute()
            .await
            .expect("open lancedb");
        db.create_table(RAG_TABLE_NAME, batch_reader)
            .execute()
            .await
            .expect("create current rag table");

        let indexed_record = RagIndexedFileRecord {
            source_root: "/tmp/docs".to_string(),
            absolute_path: "/tmp/docs/a.md".to_string(),
            relative_path: "a.md".to_string(),
            embedding_fingerprint: test_embedding_fingerprint_for(
                "https://other.example.com/v1",
                "text-embedding-3-small",
            ),
            active: Some(test_active_version("md5-a", Some(1), 10, 1, 42)),
            pending: None,
        };
        upsert_metadata_records(&metadata_path, &[indexed_record]).expect("write metadata row");

        prepare_index_storage(
            &database_path,
            &metadata_path,
            &test_resolved_config(&root),
            true,
        )
        .await
        .expect("prepare index storage should reset mismatched fingerprint rows");

        assert!(load_rag_table_schema(&database_path)
            .await
            .expect("query rag table state")
            .is_none());
        assert!(load_metadata_records(&metadata_path)
            .expect("load metadata rows")
            .is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn prepare_index_storage_clears_legacy_metadata_schema_and_vectors() {
        let root = temp_test_root("legacy-metadata-reset");
        let database_path = root.join("rag-lancedb");
        let metadata_path = root.join("rag.sqlite3");
        std::fs::create_dir_all(&root).expect("create rag temp root");

        let chunk = RagChunk {
            id: "chunk-1".to_string(),
            source_root: "/tmp/docs".to_string(),
            absolute_path: "/tmp/docs/a.md".to_string(),
            version_id: "active-v1".to_string(),
            embedding_fingerprint: test_embedding_fingerprint(),
            chunk_state: RagChunkState::Active,
            chunk_index: 0,
            line_start: 1,
            line_end: 3,
            paragraph_line_start: 1,
            heading_path: vec!["Intro".to_string()],
            chunk_reuse_key: "reuse-1".to_string(),
            text_fingerprint: text_fingerprint("current chunk"),
            text: "current chunk".to_string(),
        };
        let batch_reader =
            build_record_batch_reader(&[chunk], &[vec![1.0_f32, 2.0_f32]]).expect("build batch");
        let db = connect(database_path.to_string_lossy().as_ref())
            .execute()
            .await
            .expect("open lancedb");
        db.create_table(RAG_TABLE_NAME, batch_reader)
            .execute()
            .await
            .expect("create current rag table");

        let connection = Connection::open(&metadata_path).expect("open legacy metadata database");
        connection
            .execute_batch(
                "
                CREATE TABLE rag_files (
                    absolute_path TEXT PRIMARY KEY NOT NULL,
                    source_root TEXT NOT NULL,
                    relative_path TEXT NOT NULL,
                    content_md5 TEXT NOT NULL,
                    modified_at_ms INTEGER,
                    size_bytes INTEGER NOT NULL,
                    chunk_count INTEGER NOT NULL,
                    indexed_at_ms INTEGER NOT NULL,
                    indexing_status TEXT NOT NULL DEFAULT 'indexed',
                    embedding_fingerprint TEXT NOT NULL DEFAULT ''
                );
                INSERT INTO rag_files (
                    absolute_path,
                    source_root,
                    relative_path,
                    content_md5,
                    modified_at_ms,
                    size_bytes,
                    chunk_count,
                    indexed_at_ms,
                    indexing_status,
                    embedding_fingerprint
                ) VALUES (
                    '/tmp/docs/a.md',
                    '/tmp/docs',
                    'a.md',
                    'md5-a',
                    1,
                    10,
                    1,
                    42,
                    'indexed',
                    'legacy-fingerprint'
                );
                ",
            )
            .expect("create legacy metadata rows");

        prepare_index_storage(
            &database_path,
            &metadata_path,
            &test_resolved_config(&root),
            true,
        )
        .await
        .expect("prepare index storage should reset incompatible metadata");

        assert!(load_rag_table_schema(&database_path)
            .await
            .expect("query rag table state")
            .is_none());
        assert!(metadata_store_has_compatible_schema(&metadata_path)
            .expect("inspect recreated metadata schema"));
        assert!(load_metadata_records(&metadata_path)
            .expect("load metadata rows")
            .is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn resolve_chunk_vectors_reuses_cached_vectors_before_requesting_embeddings() {
        let root = temp_test_root("cached-vectors");
        let database_path = root.join("rag-lancedb");
        std::fs::create_dir_all(&root).expect("create rag temp root");

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test embedding listener");
        let base_url = format!("http://{}", listener.local_addr().expect("listener addr"));
        let request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server_request_count = request_count.clone();
        let server = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                server_request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let _ = read_http_request_body(&mut stream).await;
                let response_body = serde_json::to_vec(
                    &serde_json::json!({ "data": [{ "embedding": [9.0_f32] }] }),
                )
                .expect("serialize fallback embedding response");
                let response_head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    response_body.len()
                );
                let _ = stream.write_all(response_head.as_bytes()).await;
                let _ = stream.write_all(&response_body).await;
            }
        });

        let mut provider = test_embedding_provider();
        provider.base_url = base_url;
        let resolved = ResolvedRagConfig {
            source_roots: vec![root.clone()],
            ignore_globs: Arc::new(None),
            embedding_fingerprint: embedding_fingerprint(&provider)
                .expect("cache test embedding fingerprint should resolve"),
            provider,
        };

        let cached_chunk = RagChunk {
            id: "chunk-1".to_string(),
            source_root: normalize_path_string(&root),
            absolute_path: normalize_path_string(&root.join("a.md")),
            version_id: "active-v1".to_string(),
            embedding_fingerprint: resolved.embedding_fingerprint.clone(),
            chunk_state: RagChunkState::Active,
            chunk_index: 0,
            line_start: 1,
            line_end: 2,
            paragraph_line_start: 1,
            heading_path: vec!["Intro".to_string()],
            chunk_reuse_key: "reuse-1".to_string(),
            text_fingerprint: text_fingerprint("shared text"),
            text: "shared text".to_string(),
        };
        let batch_reader = build_record_batch_reader(&[cached_chunk], &[vec![1.0_f32, 2.0_f32]])
            .expect("build cache batch");
        let db = connect(database_path.to_string_lossy().as_ref())
            .execute()
            .await
            .expect("open lancedb");
        db.create_table(RAG_TABLE_NAME, batch_reader)
            .execute()
            .await
            .expect("create cache rag table");

        let vector_store = RagVectorStore::open(&database_path)
            .await
            .expect("open vector store");
        let vectors = resolve_chunk_vectors(
            &resolved,
            &build_embedding_client().expect("build embedding client"),
            &vector_store,
            &[RagChunk {
                id: "chunk-2".to_string(),
                source_root: normalize_path_string(&root),
                absolute_path: normalize_path_string(&root.join("b.md")),
                version_id: "pending-v1".to_string(),
                embedding_fingerprint: resolved.embedding_fingerprint.clone(),
                chunk_state: RagChunkState::Staged,
                chunk_index: 0,
                line_start: 1,
                line_end: 2,
                paragraph_line_start: 1,
                heading_path: vec!["Elsewhere".to_string()],
                chunk_reuse_key: "reuse-2".to_string(),
                text_fingerprint: text_fingerprint("shared text"),
                text: "shared text".to_string(),
            }],
            &HashMap::new(),
        )
        .await
        .expect("resolve chunk vectors should reuse cached vector");

        assert_eq!(vectors, vec![vec![1.0_f32, 2.0_f32]]);
        assert_eq!(request_count.load(std::sync::atomic::Ordering::SeqCst), 0);

        server.abort();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn display_path_for_prompt_uses_home_relative_format() {
        let home = dirs::home_dir().expect("home directory should exist");
        let path = home.join("docs/readme.md");

        assert_eq!(
            display_path_for_prompt(&normalize_path_string(&path)),
            "~/docs/readme.md"
        );
    }

    #[test]
    fn markdown_chunks_capture_heading_and_line_metadata() {
        let text = "# Intro\n\nFirst line.\nSecond line.\n\n## Details\n\nThird line.";
        let chunks = split_text_for_path(Path::new("/tmp/readme.md"), text, 48, 0)
            .expect("markdown split should succeed");

        assert!(chunks.iter().all(|chunk| {
            chunk.line_start >= chunk.paragraph_line_start && chunk.line_end >= chunk.line_start
        }));
        assert!(chunks.iter().any(|chunk| {
            chunk.heading_path == vec!["Intro".to_string()] && chunk.line_end >= 3
        }));
        assert!(chunks.iter().any(|chunk| {
            chunk.heading_path == vec!["Intro".to_string(), "Details".to_string()]
                && chunk.line_start >= 6
        }));
    }

    #[test]
    fn markdown_list_notes_split_under_same_heading() {
        let bullet = "- 甲富而乙贫，并不是因为甲有马，乙却步行，而是因为甲富能备有马车，乙贫不能不步行。\n\n";
        let text = format!(
            "# 国富论\n\n{}{}{}{}{}{}{}{}{}{}{}{}",
            bullet,
            bullet,
            bullet,
            bullet,
            bullet,
            bullet,
            bullet,
            bullet,
            bullet,
            bullet,
            bullet,
            bullet
        );
        let chunks = split_text_for_path(Path::new("/tmp/readme.md"), &text, 1_200, 200)
            .expect("markdown split should succeed");

        assert!(chunks.len() > 1);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.heading_path == vec!["国富论".to_string()]));
        assert!(chunks
            .iter()
            .all(|chunk| chunk.text.chars().count() <= MARKDOWN_CHUNK_HARD_MAX_CHARS));
    }

    #[tokio::test]
    async fn request_embeddings_retries_timed_out_batch_with_smaller_inputs() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test embedding listener");
        let base_url = format!("http://{}", listener.local_addr().expect("listener addr"));
        let server = tokio::spawn(async move {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().await.expect("accept request");
                tokio::spawn(async move {
                    let body = read_http_request_body(&mut stream)
                        .await
                        .expect("read request body");
                    let payload: serde_json::Value =
                        serde_json::from_slice(&body).expect("parse request body");
                    let inputs = payload["input"]
                        .as_array()
                        .expect("embedding input should be an array");

                    if inputs.len() > 1 {
                        tokio::time::sleep(Duration::from_millis(120)).await;
                    }

                    let response_body = serde_json::to_vec(&serde_json::json!({
                        "data": inputs
                            .iter()
                            .map(|value| {
                                let text = value.as_str().expect("embedding input should be text");
                                serde_json::json!({ "embedding": [text.len() as f32] })
                            })
                            .collect::<Vec<_>>()
                    }))
                    .expect("serialize embedding response");
                    let response_head = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        response_body.len()
                    );
                    stream
                        .write_all(response_head.as_bytes())
                        .await
                        .expect("write response head");
                    stream
                        .write_all(&response_body)
                        .await
                        .expect("write response body");
                });
            }
        });

        let client = HttpClient::builder()
            .timeout(Duration::from_millis(40))
            .build()
            .expect("build short-timeout client");
        let provider = LlmProviderConfig {
            id: "embedding".to_string(),
            name: "Embedding".to_string(),
            base_url,
            api_key: String::new(),
            model_type: crate::domain::settings::LlmModelType::Embedding,
            model: "test-embedding".to_string(),
            supports_multimodal: false,
            ..LlmProviderConfig::default()
        };
        let inputs = vec!["alpha".to_string(), "be".to_string()];

        let (embeddings, stats) = request_embeddings_with_stats(&client, &provider, &inputs)
            .await
            .expect("adaptive embedding request should succeed");

        assert_eq!(embeddings, vec![vec![5.0], vec![2.0]]);
        assert_eq!(stats.largest_successful_batch_size, 1);
        assert!(stats.had_to_split());
        server.await.expect("server task should complete");
    }

    #[test]
    fn small_corpus_vector_index_failure_is_treated_as_skippable() {
        let error = anyhow::anyhow!(
            "failed to create LanceDB vector index: Not enough rows to train PQ. Requires 256 rows but only 2 available"
        );

        assert!(can_skip_vector_index_build(&error));
    }

    #[tokio::test]
    async fn stale_runtime_generation_cannot_override_current_status() {
        let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
        let runtime_generation = Arc::new(AtomicU64::new(2));

        set_runtime_status_for_generation(
            &runtime_status,
            &runtime_generation,
            2,
            RagRuntimePhase::Indexing,
            RuntimeProgress {
                scanned_file_count: 12,
                completed_file_count: 9,
                total_file_count: 12,
                pending_file_count: 3,
            },
            None,
        )
        .await;
        set_runtime_status_for_generation(
            &runtime_status,
            &runtime_generation,
            1,
            RagRuntimePhase::Error,
            RuntimeProgress::default(),
            Some("stale error".to_string()),
        )
        .await;

        let status = runtime_status.read().await.clone();
        assert_eq!(status.phase, RagRuntimePhase::Indexing);
        assert_eq!(status.scanned_file_count, 12);
        assert_eq!(status.completed_file_count, 9);
        assert_eq!(status.total_file_count, 12);
        assert_eq!(status.pending_file_count, 3);
        assert_eq!(status.last_error, None);
    }
}
