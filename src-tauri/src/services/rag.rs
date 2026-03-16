use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use arrow_array::{
    types::Float32Type, FixedSizeListArray, Int32Array, RecordBatch, RecordBatchIterator,
    RecordBatchReader, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use lancedb::{connect, index::Index};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use text_splitter::{Characters, ChunkConfig, MarkdownSplitter, TextSplitter};
use tokio::{
    sync::{mpsc, RwLock as AsyncRwLock},
    task::JoinHandle,
};

use crate::domain::{
    rag::RagScanResult,
    settings::{LlmProviderConfig, LlmProviderProtocolKind, LlmSettings, RagSettings},
};

const RAG_DB_DIR_NAME: &str = "rag-lancedb";
const RAG_TABLE_NAME: &str = "chunks";
const MAX_TEXT_FILE_BYTES: u64 = 50 * 1024 * 1024;
const CHUNK_MAX_CHARS: usize = 1_200;
const CHUNK_OVERLAP_CHARS: usize = 200;
const EMBEDDING_BATCH_SIZE: usize = 16;
const WATCH_DEBOUNCE_WINDOW: Duration = Duration::from_millis(250);
const SUPPORTED_TEXT_FILE_EXTENSIONS: &[&str] = &["md", "mdx", "txt", "markdown", "rst", "adoc"];
const MARKDOWN_TEXT_FILE_EXTENSIONS: &[&str] = &["md", "mdx", "markdown"];

#[derive(Clone)]
pub struct RagIndexService {
    config_dir: PathBuf,
    runtime: Arc<AsyncRwLock<Option<JoinHandle<()>>>>,
}

#[derive(Debug, Clone)]
struct RagChunk {
    source_root: String,
    absolute_path: String,
    relative_path: String,
    chunk_index: i32,
    text: String,
}

#[derive(Debug, Default)]
struct CollectedRagChunks {
    scanned_file_count: usize,
    indexed_file_count: usize,
    skipped_file_count: usize,
    chunks: Vec<RagChunk>,
}

#[derive(Debug, Clone)]
struct ResolvedRagConfig {
    source_roots: Vec<PathBuf>,
    ignore_globs: Arc<Option<GlobSet>>,
    provider: LlmProviderConfig,
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

impl RagIndexService {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            runtime: Arc::new(AsyncRwLock::new(None)),
        }
    }

    pub async fn apply_settings(&self, settings: RagSettings, llm_settings: LlmSettings) {
        let mut runtime = self.runtime.write().await;
        if let Some(handle) = runtime.take() {
            handle.abort();
        }

        let config_dir = self.config_dir.clone();
        *runtime = Some(tokio::spawn(async move {
            if let Err(error) = run_watch_loop(config_dir, settings, llm_settings).await {
                tracing::error!(?error, "RAG watcher loop exited unexpectedly");
            }
        }));
    }
}

pub async fn scan_rag_sources(
    config_dir: &Path,
    settings: &RagSettings,
    llm_settings: &LlmSettings,
) -> Result<RagScanResult> {
    let resolved = resolve_rag_config(settings, llm_settings)?;
    let database_path = rag_database_path(config_dir);
    rebuild_index(&database_path, &resolved).await
}

async fn run_watch_loop(
    config_dir: PathBuf,
    settings: RagSettings,
    llm_settings: LlmSettings,
) -> Result<()> {
    let database_path = rag_database_path(&config_dir);
    let resolved = match resolve_rag_config(&settings, &llm_settings) {
        Ok(resolved) => resolved,
        Err(error) => {
            clear_index(&database_path).await?;
            if settings.source_directories.is_empty() && settings.embedding_provider_id.is_none() {
                return Ok(());
            }
            return Err(error);
        }
    };

    rebuild_index(&database_path, &resolved).await?;

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

        if let Err(error) = process_event_batch(&database_path, &resolved, events).await {
            tracing::warn!(?error, "failed to process RAG watcher events");
        }
    }
}

fn rag_database_path(config_dir: &Path) -> PathBuf {
    config_dir.join(RAG_DB_DIR_NAME)
}

async fn process_event_batch(
    database_path: &Path,
    resolved: &ResolvedRagConfig,
    events: Vec<notify::Result<Event>>,
) -> Result<()> {
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

        if matches!(event.kind, EventKind::Access(_)) {
            continue;
        }

        if matches!(event.kind, EventKind::Any | EventKind::Other) {
            full_rescan = true;
        }

        for path in event.paths {
            if !path.starts_with_any(&resolved.source_roots) {
                continue;
            }

            if path.exists() && path.is_dir() {
                full_rescan = true;
                continue;
            }

            changed_paths.insert(path);
        }
    }

    if full_rescan {
        rebuild_index(database_path, resolved).await?;
        return Ok(());
    }

    for path in changed_paths {
        update_path_index(database_path, resolved, &path).await?;
    }

    Ok(())
}

async fn rebuild_index(
    database_path: &Path,
    resolved: &ResolvedRagConfig,
) -> Result<RagScanResult> {
    tokio::fs::create_dir_all(database_path)
        .await
        .with_context(|| {
            format!(
                "failed to create RAG database directory: {}",
                database_path.display()
            )
        })?;

    clear_index(database_path).await?;

    let resolved_for_scan = resolved.clone();
    let collected = tokio::task::spawn_blocking(move || collect_chunks(&resolved_for_scan))
        .await
        .context("failed to join RAG scan task")??;

    if collected.chunks.is_empty() {
        return Ok(build_scan_result(database_path, resolved, &collected));
    }

    write_chunks(database_path, resolved, &collected.chunks, false).await?;
    Ok(build_scan_result(database_path, resolved, &collected))
}

async fn update_path_index(
    database_path: &Path,
    resolved: &ResolvedRagConfig,
    path: &Path,
) -> Result<()> {
    delete_vectors_for_path(database_path, path, !path.exists()).await?;

    if !(path.exists() && path.is_file()) {
        return Ok(());
    }

    let resolved_for_file = resolved.clone();
    let path_for_file = path.to_path_buf();
    let chunks = tokio::task::spawn_blocking(move || {
        collect_chunks_for_path(&resolved_for_file, &path_for_file)
    })
    .await
    .context("failed to join RAG file update task")??;

    if chunks.is_empty() {
        return Ok(());
    }

    write_chunks(database_path, resolved, &chunks, true).await
}

async fn write_chunks(
    database_path: &Path,
    resolved: &ResolvedRagConfig,
    chunks: &[RagChunk],
    append: bool,
) -> Result<()> {
    let client = build_embedding_client()?;
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

    let mut table = if table_exists {
        Some(
            db.open_table(RAG_TABLE_NAME)
                .execute()
                .await
                .context("failed to open existing RAG table")?,
        )
    } else {
        None
    };
    let mut created_table = false;

    for batch in chunks.chunks(EMBEDDING_BATCH_SIZE) {
        let inputs = batch
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>();
        let vectors = request_embeddings(&client, &resolved.provider, &inputs).await?;
        let batch_reader = build_record_batch_reader(batch, &vectors)?;

        if let Some(existing_table) = &table {
            existing_table
                .add(batch_reader)
                .execute()
                .await
                .context("failed to append chunks into LanceDB")?;
        } else {
            let created = db
                .create_table(RAG_TABLE_NAME, batch_reader)
                .execute()
                .await
                .context("failed to create RAG LanceDB table")?;
            table = Some(created);
            created_table = true;
        }
    }

    if !append || created_table {
        if let Some(created) = &table {
            created
                .create_index(&["vector"], Index::Auto)
                .execute()
                .await
                .context("failed to create LanceDB vector index")?;
        }
    }

    Ok(())
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

async fn delete_vectors_for_path(
    database_path: &Path,
    path: &Path,
    delete_descendants: bool,
) -> Result<()> {
    let db = connect(database_path.to_string_lossy().as_ref())
        .execute()
        .await
        .context("failed to open LanceDB database for deletion")?;
    let table_exists = db
        .table_names()
        .execute()
        .await
        .context("failed to list LanceDB tables for deletion")?
        .iter()
        .any(|name| name == RAG_TABLE_NAME);
    if !table_exists {
        return Ok(());
    }

    let normalized_path = path.to_string_lossy().replace('\\', "/");
    let escaped = escape_sql_literal(&normalized_path);
    let mut filter = format!("absolute_path = '{escaped}'");
    if delete_descendants {
        filter = format!("{filter} OR absolute_path LIKE '{escaped}/%'");
    }

    db.open_table(RAG_TABLE_NAME)
        .execute()
        .await
        .context("failed to open RAG table for deletion")?
        .delete(&filter)
        .await
        .with_context(|| format!("failed to delete stale vectors for path: {normalized_path}"))?;

    Ok(())
}

fn build_embedding_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("failed to build embedding HTTP client")
}

fn resolve_rag_config(
    settings: &RagSettings,
    llm_settings: &LlmSettings,
) -> Result<ResolvedRagConfig> {
    let provider_id = settings
        .embedding_provider_id
        .as_deref()
        .context("RAG 扫描前必须选择一个 embedding provider")?;
    let provider = llm_settings
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .with_context(|| format!("RAG 选择的 embedding provider 不存在: {provider_id}"))?;
    if provider.protocol != LlmProviderProtocolKind::Embedding {
        anyhow::bail!("RAG 只接受 OpenAI Embedding 协议的 provider");
    }
    if provider.base_url.trim().is_empty() {
        anyhow::bail!("RAG embedding provider base URL 不能为空");
    }
    if provider.model.trim().is_empty() {
        anyhow::bail!("RAG embedding provider model 不能为空");
    }

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
        provider: provider.clone(),
    })
}

fn collect_chunks(resolved: &ResolvedRagConfig) -> Result<CollectedRagChunks> {
    let mut collected = CollectedRagChunks::default();
    let mut visited_paths = HashSet::new();

    for source_root in &resolved.source_roots {
        let mut walker = WalkBuilder::new(source_root);
        walker
            .hidden(false)
            .ignore(true)
            .git_ignore(true)
            .git_global(true);

        for entry in walker.build() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    collected.skipped_file_count = collected.skipped_file_count.saturating_add(1);
                    tracing::warn!(?error, "failed to walk RAG source entry");
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
                    collected.skipped_file_count = collected.skipped_file_count.saturating_add(1);
                    tracing::warn!(?error, path = %path.display(), "failed to canonicalize RAG file path");
                    continue;
                }
            };
            if !visited_paths.insert(canonical_path.clone()) {
                continue;
            }

            collected.scanned_file_count = collected.scanned_file_count.saturating_add(1);
            match collect_chunks_for_path(resolved, &canonical_path) {
                Ok(chunks) if !chunks.is_empty() => {
                    collected.indexed_file_count = collected.indexed_file_count.saturating_add(1);
                    collected.chunks.extend(chunks);
                }
                Ok(_) => {
                    collected.skipped_file_count = collected.skipped_file_count.saturating_add(1);
                }
                Err(error) => {
                    collected.skipped_file_count = collected.skipped_file_count.saturating_add(1);
                    tracing::warn!(?error, path = %canonical_path.display(), "failed to collect RAG chunks for file");
                }
            }
        }
    }

    Ok(collected)
}

fn collect_chunks_for_path(resolved: &ResolvedRagConfig, path: &Path) -> Result<Vec<RagChunk>> {
    let source_root = resolve_source_root_for_path(&resolved.source_roots, path)
        .with_context(|| format!("path is outside configured RAG roots: {}", path.display()))?;

    if should_skip_path(source_root, path, resolved.ignore_globs.as_ref().as_ref()) {
        return Ok(Vec::new());
    }
    if !is_supported_text_file(path) {
        return Ok(Vec::new());
    }

    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to read file metadata: {}", path.display()))?;
    if metadata.len() > MAX_TEXT_FILE_BYTES {
        return Ok(Vec::new());
    }

    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))?;
    if bytes.contains(&0) {
        return Ok(Vec::new());
    }

    let text = String::from_utf8(bytes)
        .with_context(|| format!("file is not valid UTF-8 text: {}", path.display()))?;
    let chunk_texts = split_text_for_path(path, &text, CHUNK_MAX_CHARS, CHUNK_OVERLAP_CHARS)?;
    let relative_path = path
        .strip_prefix(source_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    Ok(chunk_texts
        .into_iter()
        .enumerate()
        .map(|(chunk_index, chunk_text)| RagChunk {
            source_root: source_root.to_string_lossy().into_owned(),
            absolute_path: path.to_string_lossy().into_owned(),
            relative_path: relative_path.clone(),
            chunk_index: chunk_index as i32,
            text: chunk_text,
        })
        .collect())
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
) -> Result<Vec<String>> {
    if is_markdown_text_file(path) {
        return Ok(
            MarkdownSplitter::new(build_chunk_config(capacity, overlap)?)
                .chunks(text)
                .filter(|chunk| !chunk.is_empty())
                .map(str::to_owned)
                .collect(),
        );
    }

    Ok(TextSplitter::new(build_chunk_config(capacity, overlap)?)
        .chunks(text)
        .filter(|chunk| !chunk.is_empty())
        .map(str::to_owned)
        .collect())
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

fn is_supported_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            SUPPORTED_TEXT_FILE_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
        .unwrap_or(false)
}

fn is_markdown_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            MARKDOWN_TEXT_FILE_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
        .unwrap_or(false)
}

async fn request_embeddings(
    client: &Client,
    provider: &LlmProviderConfig,
    inputs: &[String],
) -> Result<Vec<Vec<f32>>> {
    let base_url = normalize_base_url(&provider.base_url)?;
    let mut request = client
        .post(format!("{base_url}/embeddings"))
        .json(&EmbeddingRequest {
            model: provider.model.trim(),
            input: inputs,
        });
    let api_key = provider.api_key.trim();
    if !api_key.is_empty() {
        request = request.bearer_auth(api_key);
    }

    let response = request
        .send()
        .await
        .context("failed to request embeddings from provider")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read embedding response body")?;

    if !status.is_success() {
        anyhow::bail!("embedding provider request failed ({status}): {body}");
    }

    let parsed: EmbeddingResponse =
        serde_json::from_str(&body).context("failed to parse embedding response JSON")?;
    Ok(parsed.data.into_iter().map(|item| item.embedding).collect())
}

fn normalize_base_url(base_url: &str) -> Result<String> {
    let normalized = base_url.trim().trim_end_matches('/');
    if normalized.is_empty() {
        anyhow::bail!("embedding provider base URL 不能为空");
    }

    reqwest::Url::parse(normalized)
        .with_context(|| format!("invalid embedding provider base URL: {normalized}"))?;
    Ok(normalized.to_string())
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
        Field::new("relative_path", DataType::Utf8, false),
        Field::new("chunk_index", DataType::Int32, false),
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
            .map(|chunk| format!("{}#{}", chunk.absolute_path, chunk.chunk_index))
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
    let relative_paths = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.relative_path.clone())
            .collect::<Vec<_>>(),
    );
    let chunk_indexes = Int32Array::from(
        chunks
            .iter()
            .map(|chunk| chunk.chunk_index)
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
            Arc::new(relative_paths),
            Arc::new(chunk_indexes),
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

fn build_scan_result(
    database_path: &Path,
    resolved: &ResolvedRagConfig,
    collected: &CollectedRagChunks,
) -> RagScanResult {
    RagScanResult {
        database_path: database_path.to_string_lossy().into_owned(),
        source_count: resolved.source_roots.len(),
        scanned_file_count: collected.scanned_file_count,
        indexed_file_count: collected.indexed_file_count,
        skipped_file_count: collected.skipped_file_count,
        chunk_count: collected.chunks.len(),
        finished_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    }
}

fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
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
    use super::*;
    use crate::domain::settings::LlmProviderProtocolKind;

    fn temp_test_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("wabity-rag-{label}-{unique}"))
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

        let resolved = ResolvedRagConfig {
            source_roots: vec![root.clone()],
            ignore_globs: Arc::new(None),
            provider: LlmProviderConfig {
                id: "embedding".to_string(),
                name: "Embedding".to_string(),
                protocol: LlmProviderProtocolKind::Embedding,
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                model: "text-embedding-3-small".to_string(),
                supports_multimodal: false,
            },
        };

        let chunks = collect_chunks_for_path(&resolved, &file_path)
            .expect("collecting chunks should succeed");

        assert!(chunks.len() > 1);
        assert_eq!(chunks[0].source_root, root.to_string_lossy());
        assert_eq!(chunks[0].absolute_path, file_path.to_string_lossy());
        assert_eq!(chunks[0].relative_path, "notes.txt");
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
    fn supported_rag_text_extensions_are_case_insensitive() {
        assert!(is_supported_text_file(Path::new("/tmp/README.MD")));
        assert!(is_supported_text_file(Path::new("/tmp/notes.mdx")));
        assert!(is_supported_text_file(Path::new("/tmp/plain.txt")));
        assert!(!is_supported_text_file(Path::new("/tmp/config.toml")));
        assert!(!is_supported_text_file(Path::new("/tmp/README")));
    }

    #[test]
    fn markdown_files_route_to_markdown_splitter() {
        let text = "# Heading\n\nParagraph one.\n\n## Next\n\nParagraph two.";
        let expected = MarkdownSplitter::new(build_chunk_config(24, 0).expect("valid config"))
            .chunks(text)
            .map(str::to_owned)
            .collect::<Vec<_>>();

        let chunks = split_text_for_path(Path::new("/tmp/readme.md"), text, 24, 0)
            .expect("markdown split should succeed");

        assert_eq!(chunks, expected);
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

        assert_eq!(chunks, expected);
    }

    #[test]
    fn collect_chunks_for_path_skips_unsupported_extension() {
        let root = temp_test_root("unsupported-extension");
        let file_path = root.join("notes.toml");
        std::fs::create_dir_all(&root).expect("create rag temp root");
        std::fs::write(&file_path, "title = \"not indexed\"").expect("write rag source file");

        let resolved = ResolvedRagConfig {
            source_roots: vec![root.clone()],
            ignore_globs: Arc::new(None),
            provider: LlmProviderConfig {
                id: "embedding".to_string(),
                name: "Embedding".to_string(),
                protocol: LlmProviderProtocolKind::Embedding,
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                model: "text-embedding-3-small".to_string(),
                supports_multimodal: false,
            },
        };

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

        let resolved = ResolvedRagConfig {
            source_roots: vec![root.clone()],
            ignore_globs: Arc::new(None),
            provider: LlmProviderConfig {
                id: "embedding".to_string(),
                name: "Embedding".to_string(),
                protocol: LlmProviderProtocolKind::Embedding,
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                model: "text-embedding-3-small".to_string(),
                supports_multimodal: false,
            },
        };

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
    fn collect_chunks_for_path_skips_text_files_over_50_mb() {
        let root = temp_test_root("too-large-file");
        let file_path = root.join("too-large.txt");
        std::fs::create_dir_all(&root).expect("create rag temp root");
        let file = std::fs::File::create(&file_path).expect("create oversized rag source file");
        file.set_len(MAX_TEXT_FILE_BYTES + 1)
            .expect("set oversized rag source length");

        let resolved = ResolvedRagConfig {
            source_roots: vec![root.clone()],
            ignore_globs: Arc::new(None),
            provider: LlmProviderConfig {
                id: "embedding".to_string(),
                name: "Embedding".to_string(),
                protocol: LlmProviderProtocolKind::Embedding,
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                model: "text-embedding-3-small".to_string(),
                supports_multimodal: false,
            },
        };

        let chunks = collect_chunks_for_path(&resolved, &file_path)
            .expect("collecting chunks should skip oversized files");

        assert!(chunks.is_empty());

        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir_all(&root);
    }
}
