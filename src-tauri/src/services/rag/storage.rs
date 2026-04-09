use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use super::{
    config::{now_unix_ms, parse_document_kind, parse_heading_path},
    embedding::text_fingerprint,
    model::{
        PreparedRagChunk, RagChunk, RagChunkState, RagIndexedFileRecord, RagIndexedFileVersion,
        RagLexicalSearchHit, ResolvedRagConfig, MAX_DELETE_FILTER_PATHS, MAX_METADATA_BATCH_PATHS,
        MAX_TEXT_FINGERPRINT_FILTERS, RAG_LEXICAL_TABLE_NAME,
    },
};
use crate::services::rag_query;

#[cfg(test)]
use super::config::normalize_path_string;
#[cfg(test)]
use crate::services::document_extract::extractor_fingerprint_for_path;

pub(super) const RAG_CHUNK_DB_FILE_NAME: &str = "rag-chunks.sqlite3";
pub(super) const RAG_VECTOR_INDEX_FILE_NAME: &str = "rag-chunks.usearch";
const RAG_VECTOR_INDEX_DIRTY_FILE_NAME: &str = "rag-chunks.dirty";

pub(crate) fn build_usearch_index_options(dimensions: usize) -> IndexOptions {
    IndexOptions {
        dimensions: dimensions.max(1),
        metric: MetricKind::Cos,
        quantization: ScalarKind::F32,
        ..Default::default()
    }
}

#[derive(Debug, Clone)]
struct CachedTextVector {
    text_fingerprint: String,
    text: String,
    vector: Vec<f32>,
}

#[derive(Debug, Clone)]
struct ActiveVectorRow {
    vector_key: u64,
    vector: Vec<f32>,
}

#[derive(Clone)]
pub(super) struct RagVectorStore {
    database_path: PathBuf,
    pub(super) created_table: bool,
    pub(super) index_dirty: bool,
}

impl RagVectorStore {
    pub(super) async fn open(database_path: &Path) -> Result<Self> {
        tokio::fs::create_dir_all(database_path)
            .await
            .with_context(|| {
                format!(
                    "failed to create RAG database directory: {}",
                    database_path.display()
                )
            })?;

        let has_active_chunks = chunk_store_has_active_chunks(database_path)?;
        let index_dirty = has_active_chunks
            && (vector_index_is_marked_dirty(database_path)
                || !vector_index_is_usable(database_path)?);
        Ok(Self {
            database_path: database_path.to_path_buf(),
            created_table: false,
            index_dirty,
        })
    }

    pub(super) fn mark_index_dirty_for_chunks(&mut self, _chunk_count: usize) {
        if let Err(error) = mark_vector_index_dirty(&self.database_path) {
            tracing::warn!(?error, "failed to persist RAG vector index dirty marker");
        }
        self.index_dirty = true;
    }

    pub(super) fn mark_index_dirty_for_delete(&mut self) {
        if let Err(error) = mark_vector_index_dirty(&self.database_path) {
            tracing::warn!(?error, "failed to persist RAG vector index dirty marker");
        }
        self.index_dirty = true;
    }

    pub(super) fn should_rebuild_index(&self) -> bool {
        self.index_dirty
    }

    pub(super) async fn add_chunks(
        &mut self,
        chunks: &[RagChunk],
        vectors: &[Vec<f32>],
    ) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        if chunks.len() != vectors.len() {
            bail!(
                "chunk/vector length mismatch: {} chunks vs {} vectors",
                chunks.len(),
                vectors.len()
            );
        }

        let mut connection = open_chunk_store_connection(&self.database_path)?;
        let was_empty = chunk_store_row_count(&connection)? == 0;
        let transaction = connection
            .transaction()
            .context("failed to open rag chunk insert transaction")?;
        {
            let mut statement = transaction
                .prepare(
                    "
                    INSERT INTO rag_chunks (
                        id,
                        source_root,
                        absolute_path,
                        version_id,
                        embedding_fingerprint,
                        document_kind,
                        chunk_state,
                        chunk_index,
                        line_start,
                        line_end,
                        paragraph_line_start,
                        page_start,
                        page_end,
                        heading_path_json,
                        anchor_label,
                        chunk_reuse_key,
                        text_fingerprint,
                        text,
                        vector_blob,
                        vector_dimensions
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                        ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
                    )
                    ",
                )
                .context("failed to prepare rag chunk insert statement")?;

            for (chunk, vector) in chunks.iter().zip(vectors.iter()) {
                let heading_path_json = serde_json::to_string(&chunk.heading_path)
                    .context("failed to serialize heading path metadata")?;
                statement
                    .execute(params![
                        &chunk.id,
                        &chunk.source_root,
                        &chunk.absolute_path,
                        &chunk.version_id,
                        &chunk.embedding_fingerprint,
                        chunk.document_kind.as_str(),
                        chunk.chunk_state.as_str(),
                        chunk.chunk_index,
                        chunk.line_start,
                        chunk.line_end,
                        chunk.paragraph_line_start,
                        chunk.page_start,
                        chunk.page_end,
                        heading_path_json,
                        chunk.anchor_label.as_deref(),
                        &chunk.chunk_reuse_key,
                        &chunk.text_fingerprint,
                        &chunk.text,
                        serialize_vector(vector),
                        i64::try_from(vector.len()).unwrap_or(i64::MAX),
                    ])
                    .with_context(|| {
                        format!("failed to insert RAG chunk row: {}", chunk.absolute_path)
                    })?;
            }
        }
        transaction
            .commit()
            .context("failed to commit rag chunk insert transaction")?;

        if was_empty {
            self.created_table = true;
        }
        self.mark_index_dirty_for_chunks(chunks.len());
        Ok(())
    }

    pub(super) async fn delete_where(&mut self, filter: &str) -> Result<()> {
        let connection = open_chunk_store_connection(&self.database_path)?;
        let affected_rows = connection
            .execute(&format!("DELETE FROM rag_chunks WHERE {filter}"), [])
            .with_context(|| format!("failed to delete rag chunk rows with filter: {filter}"))?;
        if affected_rows > 0 {
            self.mark_index_dirty_for_delete();
        }
        Ok(())
    }

    pub(super) async fn update_where(
        &mut self,
        filter: &str,
        chunk_state: RagChunkState,
    ) -> Result<()> {
        let connection = open_chunk_store_connection(&self.database_path)?;
        let affected_rows = connection
            .execute(
                &format!(
                    "UPDATE rag_chunks SET chunk_state = '{}' WHERE {filter}",
                    chunk_state.as_str()
                ),
                [],
            )
            .with_context(|| {
                format!(
                    "failed to update rag chunk_state to {} with filter: {filter}",
                    chunk_state.as_str()
                )
            })?;
        if affected_rows > 0 {
            self.mark_index_dirty_for_delete();
        }
        Ok(())
    }

    pub(super) async fn load_chunk_vectors_for_file(
        &self,
        absolute_path: &str,
        chunk_state: RagChunkState,
    ) -> Result<HashMap<String, Vec<f32>>> {
        let connection = open_chunk_store_connection(&self.database_path)?;
        let mut statement = connection
            .prepare(
                "
                SELECT chunk_reuse_key, vector_blob, vector_dimensions
                FROM rag_chunks
                WHERE absolute_path = ?1 AND chunk_state = ?2
                ",
            )
            .with_context(|| {
                format!("failed to prepare chunk vector query for path: {absolute_path}")
            })?;
        let mut rows = statement
            .query(params![absolute_path, chunk_state.as_str()])
            .with_context(|| {
                format!("failed to execute chunk vector query for path: {absolute_path}")
            })?;

        let mut vectors = HashMap::new();
        while let Some(row) = rows
            .next()
            .context("failed to step rag chunk vector rows")?
        {
            let chunk_reuse_key: String = row.get(0)?;
            let vector_blob: Vec<u8> = row.get(1)?;
            let vector_dimensions = read_vector_dimensions(row, 2)?;
            vectors
                .entry(chunk_reuse_key)
                .or_insert(deserialize_vector(&vector_blob, vector_dimensions)?);
        }
        Ok(vectors)
    }

    pub(super) async fn load_cached_vectors_for_texts(
        &self,
        embedding_fingerprint: &str,
        texts: &[String],
    ) -> Result<HashMap<String, Vec<f32>>> {
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
        let connection = open_chunk_store_connection(&self.database_path)?;
        let mut cached_vectors = HashMap::new();

        for batch in text_fingerprints.chunks(MAX_TEXT_FINGERPRINT_FILTERS) {
            let placeholders = repeat_sql_placeholders(batch.len(), 2);
            let sql = format!(
                "
                SELECT text_fingerprint, text, vector_blob, vector_dimensions
                FROM rag_chunks
                WHERE embedding_fingerprint = ?1
                  AND text_fingerprint IN ({placeholders})
                "
            );
            let mut statement = connection
                .prepare(&sql)
                .context("failed to prepare cached rag vector query")?;
            let params = rusqlite::params_from_iter(
                std::iter::once(embedding_fingerprint).chain(batch.iter().map(String::as_str)),
            );
            let mut rows = statement
                .query(params)
                .with_context(|| {
                    format!(
                        "failed to execute cached rag vector query for embedding fingerprint: {embedding_fingerprint}"
                    )
                })?;

            while let Some(row) = rows
                .next()
                .context("failed to step cached rag vector rows")?
            {
                let cached = CachedTextVector {
                    text_fingerprint: row.get(0)?,
                    text: row.get(1)?,
                    vector: deserialize_vector(
                        &row.get::<_, Vec<u8>>(2)?,
                        read_vector_dimensions(row, 3)?,
                    )?,
                };
                if text_fingerprint(&cached.text) != cached.text_fingerprint {
                    continue;
                }
                if !requested_texts.contains(&cached.text) {
                    continue;
                }
                cached_vectors.entry(cached.text).or_insert(cached.vector);
            }
        }

        Ok(cached_vectors)
    }

    pub(super) async fn ensure_index(&mut self) -> Result<()> {
        if !self.index_dirty {
            return Ok(());
        }
        if !self.should_rebuild_index() {
            return Ok(());
        }

        let database_path = self.database_path.clone();
        tokio::task::spawn_blocking(move || rebuild_vector_index(&database_path))
            .await
            .context("failed to join vector index rebuild task")??;
        rag_query::invalidate_rag_query_db_cache(&self.database_path).await;
        clear_vector_index_dirty_marker(&self.database_path)?;
        self.created_table = false;
        self.index_dirty = false;
        Ok(())
    }
}

pub(super) async fn clear_index(database_path: &Path) -> Result<()> {
    rag_query::invalidate_rag_query_db_cache(database_path).await;
    match tokio::fs::remove_dir_all(database_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to remove RAG database directory: {}",
                    database_path.display()
                )
            });
        }
    }
    tokio::fs::create_dir_all(database_path)
        .await
        .with_context(|| {
            format!(
                "failed to recreate RAG database directory: {}",
                database_path.display()
            )
        })?;
    Ok(())
}

pub(super) async fn clear_metadata_store(metadata_path: &Path) -> Result<()> {
    tokio::task::spawn_blocking({
        let metadata_path = metadata_path.to_path_buf();
        move || reset_metadata_store(&metadata_path)
    })
    .await
    .context("failed to join RAG metadata cleanup task")??;
    Ok(())
}

pub(super) async fn delete_vectors_with_filter(
    vector_store: &mut RagVectorStore,
    filter: &str,
) -> Result<()> {
    vector_store.delete_where(filter).await
}

pub(super) async fn delete_vectors_for_exact_paths(
    vector_store: &mut RagVectorStore,
    paths: &[String],
) -> Result<()> {
    for chunk in paths.chunks(MAX_DELETE_FILTER_PATHS) {
        let filter = build_exact_path_filter(chunk);
        delete_vectors_with_filter(vector_store, &filter).await?;
    }
    Ok(())
}

pub(super) async fn delete_vectors_for_exact_paths_in_state(
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

pub(super) async fn delete_vectors_for_prefix_paths(
    vector_store: &mut RagVectorStore,
    prefixes: &[String],
) -> Result<()> {
    for chunk in prefixes.chunks(MAX_DELETE_FILTER_PATHS) {
        let filter = build_prefix_path_filter(chunk);
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

fn build_prefix_path_filter(prefixes: &[String]) -> String {
    prefixes
        .iter()
        .map(|prefix| {
            let escaped = escape_sql_literal(prefix);
            let like_pattern = escape_sql_literal(&descendant_like_pattern(prefix));
            format!(
                "(absolute_path = '{escaped}' OR absolute_path LIKE '{like_pattern}' ESCAPE '\\')"
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn descendant_like_pattern(prefix: &str) -> String {
    let mut escaped = String::with_capacity(prefix.len() + 2);
    for character in prefix.chars() {
        match character {
            '\\' | '%' | '_' => {
                escaped.push('\\');
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    escaped.push('/');
    escaped.push('%');
    escaped
}

#[cfg(test)]
pub(super) fn metadata_records_require_rebuild(
    stored_records: &HashMap<String, RagIndexedFileRecord>,
    resolved: &ResolvedRagConfig,
) -> bool {
    if stored_records.is_empty() {
        return false;
    }

    let normalized_source_roots = resolved
        .source_roots
        .iter()
        .map(|path| normalize_path_string(path))
        .collect::<HashSet<_>>();

    stored_records.values().any(|record| {
        record.embedding_fingerprint != resolved.embedding_fingerprint
            || extractor_fingerprint_for_path(Path::new(&record.absolute_path))
                .is_some_and(|fingerprint| fingerprint != record.extractor_fingerprint)
            || !normalized_source_roots.contains(&record.source_root)
    })
}

pub(super) async fn prepare_index_storage(
    database_path: &Path,
    metadata_path: &Path,
    _resolved: &ResolvedRagConfig,
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

    let metadata_schema_is_compatible = tokio::task::spawn_blocking({
        let metadata_path = metadata_path.to_path_buf();
        move || metadata_store_has_compatible_schema(&metadata_path)
    })
    .await
    .context("failed to join RAG metadata schema task")??;
    let chunk_store_exists = chunk_store_database_file(database_path).exists();
    let vector_index_exists = vector_index_file_path(database_path).exists();
    let vector_index_dirty_marker_exists = vector_index_dirty_marker_path(database_path).exists();
    let vector_schema_is_compatible = chunk_store_has_compatible_schema(database_path)?;
    let vector_table_exists = chunk_store_has_table(database_path)?;
    let vector_artifacts_exist = vector_table_exists
        || chunk_store_exists
        || vector_index_exists
        || vector_index_dirty_marker_exists;

    if vector_artifacts_exist && !vector_schema_is_compatible {
        tracing::warn!(
            "resetting RAG storage because vector storage schema is incompatible with current code"
        );
        clear_index(database_path).await?;
        clear_metadata_store(metadata_path).await?;
        return Ok(());
    }

    if !metadata_schema_is_compatible {
        tracing::warn!(
            "resetting RAG storage because SQLite metadata schema is incompatible with current code"
        );
        if vector_artifacts_exist {
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
    let metadata_has_active_rows = stored_records
        .values()
        .any(|record| record.active.is_some());
    let vector_has_active_chunks = chunk_store_has_active_chunks(database_path)?;
    let chunk_store_artifacts_exist = vector_table_exists || chunk_store_exists;
    let _ = reset_on_embedding_target_mismatch;

    if !chunk_store_artifacts_exist && (vector_index_exists || vector_index_dirty_marker_exists) {
        tracing::warn!(
            "resetting RAG storage because vector index artifacts exist without a chunk store"
        );
        clear_index(database_path).await?;
        if metadata_has_rows {
            clear_metadata_store(metadata_path).await?;
        }
        return Ok(());
    }

    match (vector_table_exists, metadata_has_rows) {
        (true, false) => clear_index(database_path).await?,
        (false, true) => clear_metadata_store(metadata_path).await?,
        _ => {}
    }

    match (vector_has_active_chunks, metadata_has_active_rows) {
        (true, false) => clear_index(database_path).await?,
        (false, true) => clear_metadata_store(metadata_path).await?,
        _ => {}
    }

    Ok(())
}

pub(crate) fn vector_index_file_path(database_path: &Path) -> PathBuf {
    database_path.join(RAG_VECTOR_INDEX_FILE_NAME)
}

fn vector_index_dirty_marker_path(database_path: &Path) -> PathBuf {
    database_path.join(RAG_VECTOR_INDEX_DIRTY_FILE_NAME)
}

pub(crate) fn open_vector_chunk_connection(database_path: &Path) -> Result<Connection> {
    open_chunk_store_connection(database_path)
}

fn chunk_store_database_file(database_path: &Path) -> PathBuf {
    database_path.join(RAG_CHUNK_DB_FILE_NAME)
}

fn vector_index_is_marked_dirty(database_path: &Path) -> bool {
    vector_index_dirty_marker_path(database_path).exists()
}

fn mark_vector_index_dirty(database_path: &Path) -> Result<()> {
    std::fs::create_dir_all(database_path).with_context(|| {
        format!(
            "failed to create RAG database directory for dirty marker: {}",
            database_path.display()
        )
    })?;
    std::fs::write(vector_index_dirty_marker_path(database_path), b"dirty")
        .context("failed to write RAG vector index dirty marker")?;
    Ok(())
}

fn clear_vector_index_dirty_marker(database_path: &Path) -> Result<()> {
    let marker_path = vector_index_dirty_marker_path(database_path);
    match std::fs::remove_file(&marker_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to clear RAG vector index dirty marker: {}",
                marker_path.display()
            )
        }),
    }
}

fn repeat_sql_placeholders(count: usize, start_index: usize) -> String {
    (0..count)
        .map(|offset| format!("?{}", start_index + offset))
        .collect::<Vec<_>>()
        .join(", ")
}

fn serialize_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len().saturating_mul(std::mem::size_of::<f32>()));
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn deserialize_vector(bytes: &[u8], dimensions: usize) -> Result<Vec<f32>> {
    let expected_len = dimensions.saturating_mul(std::mem::size_of::<f32>());
    if bytes.len() != expected_len {
        bail!(
            "vector blob length mismatch: expected {} bytes, got {}",
            expected_len,
            bytes.len()
        );
    }

    Ok(bytes
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|chunk| {
            let mut raw = [0_u8; std::mem::size_of::<f32>()];
            raw.copy_from_slice(chunk);
            f32::from_le_bytes(raw)
        })
        .collect())
}

fn read_vector_dimensions(row: &rusqlite::Row<'_>, index: usize) -> Result<usize> {
    let value = row.get::<_, i64>(index)?;
    usize::try_from(value).context("vector_dimensions is negative or too large")
}

fn open_chunk_store_connection(database_path: &Path) -> Result<Connection> {
    if let Some(parent) = database_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create RAG database parent directory: {}",
                parent.display()
            )
        })?;
    }
    std::fs::create_dir_all(database_path).with_context(|| {
        format!(
            "failed to create RAG database directory: {}",
            database_path.display()
        )
    })?;

    let db_path = chunk_store_database_file(database_path);
    let connection = Connection::open(&db_path)
        .with_context(|| format!("failed to open RAG chunk database: {}", db_path.display()))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .context("failed to configure RAG chunk busy timeout")?;
    initialize_chunk_store_schema(&connection)?;
    Ok(connection)
}

fn initialize_chunk_store_schema(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            CREATE TABLE IF NOT EXISTS rag_chunks (
                vector_key INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT NOT NULL UNIQUE,
                source_root TEXT NOT NULL,
                absolute_path TEXT NOT NULL,
                version_id TEXT NOT NULL,
                embedding_fingerprint TEXT NOT NULL,
                document_kind TEXT NOT NULL,
                chunk_state TEXT NOT NULL,
                chunk_index INTEGER NOT NULL,
                line_start INTEGER,
                line_end INTEGER,
                paragraph_line_start INTEGER,
                page_start INTEGER,
                page_end INTEGER,
                heading_path_json TEXT NOT NULL,
                anchor_label TEXT,
                chunk_reuse_key TEXT NOT NULL,
                text_fingerprint TEXT NOT NULL,
                text TEXT NOT NULL,
                vector_blob BLOB NOT NULL,
                vector_dimensions INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_rag_chunks_path_state
                ON rag_chunks(absolute_path, chunk_state);
            CREATE INDEX IF NOT EXISTS idx_rag_chunks_version_state
                ON rag_chunks(absolute_path, version_id, chunk_state);
            CREATE INDEX IF NOT EXISTS idx_rag_chunks_chunk_reuse
                ON rag_chunks(absolute_path, chunk_state, chunk_reuse_key);
            CREATE INDEX IF NOT EXISTS idx_rag_chunks_embedding_text
                ON rag_chunks(embedding_fingerprint, text_fingerprint);
            ",
        )
        .context("failed to initialize rag chunk schema")?;
    Ok(())
}

fn chunk_store_has_table(database_path: &Path) -> Result<bool> {
    let db_path = chunk_store_database_file(database_path);
    if !db_path.exists() {
        return Ok(false);
    }
    let connection = Connection::open(&db_path)
        .with_context(|| format!("failed to open RAG chunk database: {}", db_path.display()))?;
    let exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'rag_chunks' LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()
        .context("failed to inspect rag chunk table existence")?
        .is_some();
    Ok(exists)
}

fn chunk_store_has_compatible_schema(database_path: &Path) -> Result<bool> {
    let db_path = chunk_store_database_file(database_path);
    if !db_path.exists() {
        return Ok(false);
    }
    let connection = Connection::open(&db_path)
        .with_context(|| format!("failed to open RAG chunk database: {}", db_path.display()))?;
    chunk_table_schema_is_compatible(&connection)
}

fn chunk_store_has_active_chunks(database_path: &Path) -> Result<bool> {
    if !chunk_store_has_compatible_schema(database_path)? {
        return Ok(false);
    }
    let connection = open_chunk_store_connection(database_path)?;
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM rag_chunks WHERE chunk_state = 'active'",
            [],
            |row| row.get(0),
        )
        .context("failed to count active rag chunks")?;
    Ok(count > 0)
}

fn chunk_store_row_count(connection: &Connection) -> Result<i64> {
    connection
        .query_row("SELECT COUNT(*) FROM rag_chunks", [], |row| row.get(0))
        .context("failed to count rag chunk rows")
}

fn chunk_table_schema_is_compatible(connection: &Connection) -> Result<bool> {
    let expected_columns = [
        ("vector_key", "INTEGER"),
        ("id", "TEXT"),
        ("source_root", "TEXT"),
        ("absolute_path", "TEXT"),
        ("version_id", "TEXT"),
        ("embedding_fingerprint", "TEXT"),
        ("document_kind", "TEXT"),
        ("chunk_state", "TEXT"),
        ("chunk_index", "INTEGER"),
        ("line_start", "INTEGER"),
        ("line_end", "INTEGER"),
        ("paragraph_line_start", "INTEGER"),
        ("page_start", "INTEGER"),
        ("page_end", "INTEGER"),
        ("heading_path_json", "TEXT"),
        ("anchor_label", "TEXT"),
        ("chunk_reuse_key", "TEXT"),
        ("text_fingerprint", "TEXT"),
        ("text", "TEXT"),
        ("vector_blob", "BLOB"),
        ("vector_dimensions", "INTEGER"),
    ];
    let mut statement = connection
        .prepare("PRAGMA table_info(rag_chunks)")
        .context("failed to inspect rag chunk schema")?;
    let columns = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .context("failed to query rag chunk schema")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect rag chunk schema rows")?;

    if columns.len() != expected_columns.len() {
        return Ok(false);
    }

    Ok(columns.iter().zip(expected_columns.iter()).all(
        |((name, data_type), (expected_name, expected_type))| {
            name == expected_name && data_type.eq_ignore_ascii_case(expected_type)
        },
    ))
}

fn rebuild_vector_index(database_path: &Path) -> Result<()> {
    let rows = load_active_vectors(database_path)?;
    let index_path = vector_index_file_path(database_path);
    if rows.is_empty() {
        match std::fs::remove_file(&index_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to remove empty vector index file: {}",
                        index_path.display()
                    )
                });
            }
        }
        return Ok(());
    }

    let dimensions = rows
        .first()
        .map(|row| row.vector.len())
        .context("missing dimensions for active vector index rebuild")?;
    if rows.iter().any(|row| row.vector.len() != dimensions) {
        bail!("active rag chunks contain inconsistent vector dimensions");
    }

    let options = build_usearch_index_options(dimensions);
    let index = Index::new(&options).context("failed to create USearch index")?;
    index
        .reserve(rows.len())
        .context("failed to reserve USearch index capacity")?;
    for row in &rows {
        index
            .add(row.vector_key, row.vector.as_slice())
            .with_context(|| format!("failed to add vector key {} into USearch", row.vector_key))?;
    }

    std::fs::create_dir_all(database_path).with_context(|| {
        format!(
            "failed to create RAG database directory for index save: {}",
            database_path.display()
        )
    })?;
    let temp_path = index_path.with_extension("usearch.tmp");
    index
        .save(temp_path.to_string_lossy().as_ref())
        .with_context(|| format!("failed to save USearch index: {}", temp_path.display()))?;
    std::fs::rename(&temp_path, &index_path).with_context(|| {
        format!(
            "failed to replace USearch index {} -> {}",
            temp_path.display(),
            index_path.display()
        )
    })?;
    Ok(())
}

fn vector_index_is_usable(database_path: &Path) -> Result<bool> {
    let index_path = vector_index_file_path(database_path);
    if !index_path.exists() {
        return Ok(false);
    }

    let dimensions = load_active_vector_dimensions(database_path)?.unwrap_or(1);
    let options = build_usearch_index_options(dimensions);
    let index = Index::new(&options).context("failed to create USearch index validator")?;
    match index.load(index_path.to_string_lossy().as_ref()) {
        Ok(()) => Ok(true),
        Err(error) => {
            tracing::warn!(
                ?error,
                path = %index_path.display(),
                "marking cached USearch index dirty because it failed to load"
            );
            Ok(false)
        }
    }
}

fn load_active_vectors(database_path: &Path) -> Result<Vec<ActiveVectorRow>> {
    let connection = open_chunk_store_connection(database_path)?;
    let mut statement = connection
        .prepare(
            "
            SELECT vector_key, vector_blob, vector_dimensions
            FROM rag_chunks
            WHERE chunk_state = 'active'
            ORDER BY vector_key
            ",
        )
        .context("failed to prepare active rag vector query")?;
    let mut rows = statement
        .query([])
        .context("failed to execute active rag vector query")?;
    let mut vectors = Vec::new();
    while let Some(row) = rows
        .next()
        .context("failed to step active rag vector rows")?
    {
        let vector_key_i64: i64 = row.get(0)?;
        let vector_key = u64::try_from(vector_key_i64).context("vector_key is negative")?;
        let vector_blob: Vec<u8> = row.get(1)?;
        let vector_dimensions = read_vector_dimensions(row, 2)?;
        vectors.push(ActiveVectorRow {
            vector_key,
            vector: deserialize_vector(&vector_blob, vector_dimensions)?,
        });
    }
    Ok(vectors)
}

pub(crate) fn load_active_vector_dimensions(database_path: &Path) -> Result<Option<usize>> {
    let connection = open_chunk_store_connection(database_path)?;
    let dimensions = connection
        .query_row(
            "
            SELECT vector_dimensions
            FROM rag_chunks
            WHERE chunk_state = 'active'
            LIMIT 1
            ",
            [],
            |row: &rusqlite::Row<'_>| row.get::<_, i64>(0),
        )
        .optional()
        .context("failed to query active vector dimensions")?;
    dimensions
        .map(|value| usize::try_from(value).context("vector_dimensions is negative or too large"))
        .transpose()
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
                extractor_fingerprint TEXT NOT NULL DEFAULT '',
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
            CREATE VIRTUAL TABLE IF NOT EXISTS rag_chunk_fts USING fts5(
                source_root UNINDEXED,
                absolute_path UNINDEXED,
                path,
                document_kind UNINDEXED,
                chunk_index UNINDEXED,
                line_start UNINDEXED,
                line_end UNINDEXED,
                paragraph_line_start UNINDEXED,
                page_start UNINDEXED,
                page_end UNINDEXED,
                heading_path_json UNINDEXED,
                heading_text,
                anchor_label,
                text,
                tokenize = \"unicode61 remove_diacritics 2 tokenchars '-_./#'\"
            );
            ",
        )
        .context("failed to initialize RAG metadata schema")?;
    Ok(connection)
}

pub(super) fn finalize_metadata_and_replace_lexical_chunks(
    metadata_path: &Path,
    record: &RagIndexedFileRecord,
    chunk_count: usize,
    chunks: &[PreparedRagChunk],
) -> Result<()> {
    let mut connection = open_metadata_connection(metadata_path)?;
    let transaction = connection
        .transaction()
        .context("failed to open RAG metadata finalize transaction")?;
    let finalized_record = finalize_metadata_record(record, chunk_count);
    upsert_metadata_records_in_transaction(&transaction, &[finalized_record])?;
    replace_lexical_chunks_in_transaction(&transaction, record, chunks)?;
    transaction
        .commit()
        .context("failed to commit RAG metadata finalize transaction")?;
    Ok(())
}

pub(super) fn metadata_store_has_compatible_schema(metadata_path: &Path) -> Result<bool> {
    let connection = open_metadata_connection(metadata_path)?;
    metadata_table_schema_is_compatible(&connection)
}

fn metadata_table_schema_is_compatible(connection: &Connection) -> Result<bool> {
    let expected_columns = [
        ("absolute_path", "TEXT"),
        ("source_root", "TEXT"),
        ("relative_path", "TEXT"),
        ("embedding_fingerprint", "TEXT"),
        ("extractor_fingerprint", "TEXT"),
        ("active_version_id", "TEXT"),
        ("active_content_md5", "TEXT"),
        ("active_modified_at_ms", "INTEGER"),
        ("active_size_bytes", "INTEGER"),
        ("active_chunk_count", "INTEGER"),
        ("active_indexed_at_ms", "INTEGER"),
        ("pending_version_id", "TEXT"),
        ("pending_content_md5", "TEXT"),
        ("pending_modified_at_ms", "INTEGER"),
        ("pending_size_bytes", "INTEGER"),
        ("pending_chunk_count", "INTEGER"),
        ("pending_started_at_ms", "INTEGER"),
    ];
    let mut statement = connection
        .prepare("PRAGMA table_info(rag_files)")
        .context("failed to inspect RAG metadata schema")?;
    let columns = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .context("failed to query RAG metadata schema")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect RAG metadata schema rows")?;

    if columns.len() != expected_columns.len() {
        return Ok(false);
    }

    Ok(columns.iter().zip(expected_columns.iter()).all(
        |((name, data_type), (expected_name, expected_type))| {
            name == expected_name && data_type.eq_ignore_ascii_case(expected_type)
        },
    ))
}

pub(super) fn load_metadata_records(
    metadata_path: &Path,
) -> Result<HashMap<String, RagIndexedFileRecord>> {
    let connection = open_metadata_connection(metadata_path)?;
    let mut statement = connection
        .prepare(
            "
            SELECT
                source_root,
                absolute_path,
                relative_path,
                embedding_fingerprint,
                extractor_fingerprint,
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

pub(super) fn load_metadata_records_for_paths(
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
                extractor_fingerprint,
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

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn load_metadata_paths_for_prefixes(
    metadata_path: &Path,
    prefixes: &[String],
) -> Result<Vec<String>> {
    if prefixes.is_empty() {
        return Ok(Vec::new());
    }

    let connection = open_metadata_connection(metadata_path)?;
    let mut statement = connection
        .prepare(
            "
            SELECT absolute_path
            FROM rag_files
            WHERE absolute_path = ?1 OR absolute_path LIKE ?2 ESCAPE '\\'
            ",
        )
        .context("failed to prepare descendant RAG metadata query")?;
    let mut resolved_paths = BTreeSet::new();

    for prefix in prefixes {
        let like_pattern = descendant_like_pattern(prefix);
        let rows = statement
            .query_map(params![prefix, like_pattern], |row| row.get::<_, String>(0))
            .with_context(|| format!("failed to query descendant RAG metadata rows: {prefix}"))?;

        for row in rows {
            resolved_paths.insert(row.with_context(|| {
                format!("failed to read descendant RAG metadata row: {prefix}")
            })?);
        }
    }

    Ok(resolved_paths.into_iter().collect())
}

fn read_metadata_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RagIndexedFileRecord> {
    Ok(RagIndexedFileRecord {
        source_root: row.get(0)?,
        absolute_path: row.get(1)?,
        relative_path: row.get(2)?,
        embedding_fingerprint: row.get(3)?,
        extractor_fingerprint: row.get(4)?,
        active: read_metadata_version(
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
            row.get(10)?,
        )?,
        pending: read_metadata_version(
            row.get(11)?,
            row.get(12)?,
            row.get(13)?,
            row.get(14)?,
            row.get(15)?,
            row.get(16)?,
        )?,
    })
}

fn missing_metadata_value<T>(
    field_name: &'static str,
    sql_type: rusqlite::types::Type,
    value: Option<T>,
) -> rusqlite::Result<T> {
    value.ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            sql_type,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("missing {field_name} for indexed metadata version"),
            )),
        )
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
        content_md5: missing_metadata_value(
            "content_md5",
            rusqlite::types::Type::Text,
            content_md5,
        )?,
        modified_at_ms,
        size_bytes: missing_metadata_value(
            "size_bytes",
            rusqlite::types::Type::Integer,
            size_bytes,
        )?,
        chunk_count: missing_metadata_value(
            "chunk_count",
            rusqlite::types::Type::Integer,
            chunk_count,
        )?,
        indexed_at_ms: indexed_at_ms.unwrap_or_default(),
    }))
}

pub(super) fn finalize_metadata_record(
    record: &RagIndexedFileRecord,
    chunk_count: usize,
) -> RagIndexedFileRecord {
    let mut finalized = record.clone();
    finalized.active = finalized
        .pending
        .as_ref()
        .map(|pending| RagIndexedFileVersion {
            version_id: pending.version_id.clone(),
            content_md5: pending.content_md5.clone(),
            modified_at_ms: pending.modified_at_ms,
            size_bytes: pending.size_bytes,
            chunk_count: i64::try_from(chunk_count).unwrap_or(i64::MAX),
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

struct MetadataVersionFields<'a> {
    version_id: Option<&'a str>,
    content_md5: Option<&'a str>,
    modified_at_ms: Option<i64>,
    size_bytes: Option<i64>,
    chunk_count: Option<i64>,
    indexed_at_ms: Option<i64>,
}

fn metadata_version_fields(version: Option<&RagIndexedFileVersion>) -> MetadataVersionFields<'_> {
    MetadataVersionFields {
        version_id: version.map(|value| value.version_id.as_str()),
        content_md5: version.map(|value| value.content_md5.as_str()),
        modified_at_ms: version.and_then(|value| value.modified_at_ms),
        size_bytes: version.map(|value| value.size_bytes),
        chunk_count: version.map(|value| value.chunk_count),
        indexed_at_ms: version.map(|value| value.indexed_at_ms),
    }
}

pub(super) fn upsert_metadata_records(
    metadata_path: &Path,
    records: &[RagIndexedFileRecord],
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }

    let mut connection = open_metadata_connection(metadata_path)?;
    let transaction = connection
        .transaction()
        .context("failed to open RAG metadata transaction")?;
    upsert_metadata_records_in_transaction(&transaction, records)?;
    transaction
        .commit()
        .context("failed to commit RAG metadata transaction")?;
    Ok(())
}

fn upsert_metadata_records_in_transaction(
    transaction: &Transaction<'_>,
    records: &[RagIndexedFileRecord],
) -> Result<()> {
    {
        let mut statement = transaction
            .prepare(
                "
                INSERT INTO rag_files (
                    source_root,
                    absolute_path,
                    relative_path,
                    embedding_fingerprint,
                    extractor_fingerprint,
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
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                ON CONFLICT(absolute_path) DO UPDATE SET
                    source_root = excluded.source_root,
                    relative_path = excluded.relative_path,
                    embedding_fingerprint = excluded.embedding_fingerprint,
                    extractor_fingerprint = excluded.extractor_fingerprint,
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
            let active = metadata_version_fields(record.active.as_ref());
            let pending = metadata_version_fields(record.pending.as_ref());
            statement
                .execute(params![
                    &record.source_root,
                    &record.absolute_path,
                    &record.relative_path,
                    &record.embedding_fingerprint,
                    &record.extractor_fingerprint,
                    active.version_id,
                    active.content_md5,
                    active.modified_at_ms,
                    active.size_bytes,
                    active.chunk_count,
                    active.indexed_at_ms,
                    pending.version_id,
                    pending.content_md5,
                    pending.modified_at_ms,
                    pending.size_bytes,
                    pending.chunk_count,
                    pending.indexed_at_ms,
                ])
                .with_context(|| {
                    format!(
                        "failed to upsert RAG metadata row: {}",
                        record.absolute_path
                    )
                })?;
        }
    }
    Ok(())
}

fn replace_lexical_chunks_in_transaction(
    transaction: &Transaction<'_>,
    record: &RagIndexedFileRecord,
    chunks: &[PreparedRagChunk],
) -> Result<()> {
    transaction
        .execute(
            &format!("DELETE FROM {RAG_LEXICAL_TABLE_NAME} WHERE absolute_path = ?1"),
            [&record.absolute_path],
        )
        .with_context(|| {
            format!(
                "failed to clear stale RAG lexical rows for {}",
                record.absolute_path
            )
        })?;

    let mut statement = transaction
        .prepare(&format!(
            "
            INSERT INTO {RAG_LEXICAL_TABLE_NAME} (
                source_root,
                absolute_path,
                path,
                document_kind,
                chunk_index,
                line_start,
                line_end,
                paragraph_line_start,
                page_start,
                page_end,
                heading_path_json,
                heading_text,
                anchor_label,
                text
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "
        ))
        .context("failed to prepare RAG lexical chunk insert statement")?;

    for chunk in chunks {
        let heading_path_json = serde_json::to_string(&chunk.heading_path)
            .context("failed to serialize heading path")?;
        statement
            .execute(params![
                &record.source_root,
                &record.absolute_path,
                &record.relative_path,
                chunk.document_kind.as_str(),
                chunk.chunk_index,
                chunk.line_start,
                chunk.line_end,
                chunk.paragraph_line_start,
                chunk.page_start,
                chunk.page_end,
                heading_path_json,
                chunk.heading_path.join(" "),
                chunk.anchor_label.as_deref(),
                &chunk.text,
            ])
            .with_context(|| {
                format!(
                    "failed to insert RAG lexical chunk {}#{}",
                    record.absolute_path, chunk.chunk_index
                )
            })?;
    }

    Ok(())
}

pub(super) fn delete_metadata_for_paths(
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
            .prepare(
                "DELETE FROM rag_files WHERE absolute_path = ?1 OR absolute_path LIKE ?2 ESCAPE '\\'",
            )
            .context("failed to prepare RAG metadata descendant delete statement")?;
        let mut lexical_exact_statement = transaction
            .prepare(&format!(
                "DELETE FROM {RAG_LEXICAL_TABLE_NAME} WHERE absolute_path = ?1"
            ))
            .context("failed to prepare RAG lexical exact delete statement")?;
        let mut lexical_descendant_statement = transaction
            .prepare(&format!(
                "DELETE FROM {RAG_LEXICAL_TABLE_NAME} WHERE absolute_path = ?1 OR absolute_path LIKE ?2 ESCAPE '\\'"
            ))
            .context("failed to prepare RAG lexical descendant delete statement")?;

        for path in paths {
            if delete_descendants {
                let like_pattern = descendant_like_pattern(path);
                descendant_statement
                    .execute(params![path, like_pattern])
                    .with_context(|| format!("failed to delete RAG metadata rows: {path}"))?;
                lexical_descendant_statement
                    .execute(params![path, like_pattern])
                    .with_context(|| format!("failed to delete RAG lexical rows: {path}"))?;
            } else {
                exact_statement
                    .execute([path])
                    .with_context(|| format!("failed to delete RAG metadata row: {path}"))?;
                lexical_exact_statement
                    .execute([path])
                    .with_context(|| format!("failed to delete RAG lexical row: {path}"))?;
            }
        }
    }
    transaction
        .commit()
        .context("failed to commit RAG metadata delete transaction")?;
    Ok(())
}

pub(super) fn refresh_projection_metadata_for_records(
    database_path: &Path,
    metadata_path: &Path,
    records: &[RagIndexedFileRecord],
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }

    let mut chunk_connection = open_chunk_store_connection(database_path)?;
    let chunk_transaction = chunk_connection
        .transaction()
        .context("failed to open RAG chunk projection refresh transaction")?;
    {
        let mut chunk_statement = chunk_transaction
            .prepare("UPDATE rag_chunks SET source_root = ?1 WHERE absolute_path = ?2")
            .context("failed to prepare RAG chunk projection refresh statement")?;
        for record in records {
            chunk_statement
                .execute(params![&record.source_root, &record.absolute_path])
                .with_context(|| {
                    format!(
                        "failed to refresh RAG chunk projection metadata: {}",
                        record.absolute_path
                    )
                })?;
        }
    }
    chunk_transaction
        .commit()
        .context("failed to commit RAG chunk projection refresh transaction")?;

    let mut metadata_connection = open_metadata_connection(metadata_path)?;
    let metadata_transaction = metadata_connection
        .transaction()
        .context("failed to open RAG lexical projection refresh transaction")?;
    {
        let mut lexical_statement = metadata_transaction
            .prepare(&format!(
                "UPDATE {RAG_LEXICAL_TABLE_NAME} SET source_root = ?1, path = ?2 WHERE absolute_path = ?3"
            ))
            .context("failed to prepare RAG lexical projection refresh statement")?;
        for record in records {
            lexical_statement
                .execute(params![
                    &record.source_root,
                    &record.relative_path,
                    &record.absolute_path,
                ])
                .with_context(|| {
                    format!(
                        "failed to refresh RAG lexical projection metadata: {}",
                        record.absolute_path
                    )
                })?;
        }
    }
    metadata_transaction
        .commit()
        .context("failed to commit RAG lexical projection refresh transaction")?;
    Ok(())
}

pub(super) fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

pub(crate) fn search_lexical_chunks(
    metadata_path: &Path,
    match_query: &str,
    top_k: usize,
) -> Result<Vec<RagLexicalSearchHit>> {
    if match_query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let connection = open_metadata_connection(metadata_path)?;
    let mut statement = connection
        .prepare(&format!(
            "
            SELECT
                source_root,
                absolute_path,
                path,
                document_kind,
                chunk_index,
                line_start,
                line_end,
                paragraph_line_start,
                page_start,
                page_end,
                heading_path_json,
                anchor_label,
                text,
                bm25({RAG_LEXICAL_TABLE_NAME}, 4.0, 2.5, 1.5, 1.0) AS bm25_rank
            FROM {RAG_LEXICAL_TABLE_NAME}
            WHERE {RAG_LEXICAL_TABLE_NAME} MATCH ?1
            ORDER BY bm25_rank
            LIMIT ?2
            "
        ))
        .context("failed to prepare RAG lexical search query")?;
    let mut rows = statement
        .query(params![
            match_query,
            i64::try_from(top_k.max(1)).unwrap_or(i64::MAX)
        ])
        .context("failed to execute RAG lexical search query")?;
    let mut hits = Vec::new();
    while let Some(row) = rows
        .next()
        .context("failed to step RAG lexical search rows")?
    {
        let document_kind_raw: String = row.get(3)?;
        let heading_path_raw: String = row.get(10)?;
        hits.push(RagLexicalSearchHit {
            source_root: row.get(0)?,
            absolute_path: row.get(1)?,
            document_kind: parse_document_kind(&document_kind_raw)?,
            chunk_index: row.get(4)?,
            line_start: row.get(5)?,
            line_end: row.get(6)?,
            paragraph_line_start: row.get(7)?,
            page_start: row.get(8)?,
            page_end: row.get(9)?,
            heading_path: parse_heading_path(&heading_path_raw)?,
            anchor_label: row.get(11)?,
            text: row.get(12)?,
            bm25_rank: row.get::<_, f64>(13)? as f32,
        });
    }

    Ok(hits)
}
