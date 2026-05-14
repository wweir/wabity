use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use super::{
    embedding::text_fingerprint,
    model::{
        RagChunk, RagChunkState, ResolvedRagConfig, MAX_DELETE_FILTER_PATHS,
        MAX_TEXT_FINGERPRINT_FILTERS, RAG_LEXICAL_TABLE_NAME, RAG_SQLITE_DB_FILE_NAME,
    },
};
use crate::services::rag_query;

mod metadata;

#[cfg(test)]
pub(crate) use metadata::load_rag_file_paths_for_prefixes;
pub(crate) use metadata::{
    delete_rag_file_records_for_paths, escape_sql_literal,
    finalize_rag_file_record_and_replace_lexical_chunks, load_rag_file_records,
    load_rag_file_records_for_paths, rag_sqlite_has_compatible_schema, rag_sqlite_has_pending_rows,
    refresh_projection_for_rag_file_records, search_lexical_chunks, upsert_rag_file_records,
};
use metadata::{initialize_rag_sqlite_schema, open_rag_sqlite_connection, reset_sqlite_store};

#[cfg(test)]
use super::config::normalize_path_string;
#[cfg(test)]
use super::model::RagIndexedFileRecord;
#[cfg(test)]
use crate::services::document_extract::extractor_fingerprint_for_path;

pub(super) const RAG_CHUNK_DB_FILE_NAME: &str = RAG_SQLITE_DB_FILE_NAME;
pub(super) const RAG_VECTOR_INDEX_FILE_NAME: &str = "rag-chunks.usearch";
const RAG_VECTOR_INDEX_DIRTY_FILE_NAME: &str = "rag-chunks.dirty";
const RAG_VECTOR_INDEX_MANIFEST_FILE_NAME: &str = "rag-chunks.manifest.json";
const RAG_VECTOR_INDEX_META_TABLE_NAME: &str = "rag_vector_index_meta";
const RAG_VECTOR_INDEX_MANIFEST_VERSION: u32 = 3;
const RAG_VECTOR_INDEX_PROBE_COUNT: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveVectorBlobCoverage {
    None,
    Partial,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct VectorIndexMeta {
    active_vector_count: u64,
    vector_dimensions: usize,
    key_xor: u64,
    key_sum: u64,
    key_hash_xor: u64,
    key_hash_sum: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct VectorIndexProbe {
    pub(crate) vector_key: u64,
    pub(crate) vector_hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct VectorIndexManifest {
    pub(crate) version: u32,
    pub(crate) active_vector_count: u64,
    pub(crate) vector_dimensions: usize,
    pub(crate) key_xor: u64,
    pub(crate) key_sum: u64,
    pub(crate) key_hash_xor: u64,
    pub(crate) key_hash_sum: u64,
    pub(crate) index_size_bytes: u64,
    pub(crate) index_modified_at_ms: u64,
    #[serde(default)]
    pub(crate) probes: Vec<VectorIndexProbe>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) index_md5_hex: Option<String>,
}

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
struct StoredChunkVectorRef {
    vector_key: u64,
    vector_blob: Option<Vec<u8>>,
    vector_dimensions: usize,
}

#[derive(Debug, Clone, Copy)]
struct ActiveVectorRef {
    vector_key: u64,
    vector_dimensions: usize,
    vector_hash: Option<u64>,
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

    #[cfg(test)]
    pub(super) fn mark_index_dirty_for_chunks(&mut self, _chunk_count: usize) {
        if let Err(error) = mark_vector_index_dirty(&self.database_path) {
            tracing::warn!(
                error = format_args!("{:#}", error),
                "failed to persist RAG vector index dirty marker"
            );
        }
        self.index_dirty = true;
    }

    #[cfg(test)]
    pub(super) fn mark_index_dirty_for_delete(&mut self) {
        if let Err(error) = mark_vector_index_dirty(&self.database_path) {
            tracing::warn!(
                error = format_args!("{:#}", error),
                "failed to persist RAG vector index dirty marker"
            );
        }
        self.index_dirty = true;
    }

    #[cfg(test)]
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

        let (was_empty, inserted_active_vectors) = {
            let mut connection = open_chunk_store_connection(&self.database_path)?;
            let was_empty = chunk_store_row_count(&connection)? == 0;
            let transaction = connection
                .transaction()
                .context("failed to open rag chunk insert transaction")?;
            let mut inserted_active_vectors = Vec::new();
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
                            vector_dimensions,
                            vector_hash
                        ) VALUES (
                            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                            ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                            ?21
                        ) RETURNING vector_key
                        ",
                    )
                    .context("failed to prepare rag chunk insert statement")?;

                for (chunk, vector) in chunks.iter().zip(vectors.iter()) {
                    let heading_path_json = serde_json::to_string(&chunk.heading_path)
                        .context("failed to serialize heading path metadata")?;
                    let vector_key_i64 = statement
                        .query_row(
                            params![
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
                                sqlite_u64(stable_vector_value_hash(vector)),
                            ],
                            |row| row.get::<_, i64>(0),
                        )
                        .with_context(|| {
                            format!("failed to insert RAG chunk row: {}", chunk.absolute_path)
                        })?;
                    if chunk.chunk_state == RagChunkState::Active {
                        inserted_active_vectors.push((
                            u64::try_from(vector_key_i64).context("vector_key is negative")?,
                            vector.clone(),
                        ));
                    }
                }
            }
            apply_vector_index_meta_delta_in_transaction(
                &transaction,
                &inserted_active_vectors
                    .iter()
                    .map(|(vector_key, _)| *vector_key)
                    .collect::<Vec<_>>(),
                inserted_active_vectors
                    .first()
                    .map(|(_, vector)| vector.len()),
                &[],
            )?;
            transaction
                .commit()
                .context("failed to commit rag chunk insert transaction")?;
            (was_empty, inserted_active_vectors)
        };
        if !inserted_active_vectors.is_empty() {
            mark_vector_index_dirty(&self.database_path)?;
            save_updated_vector_index(
                &self.database_path,
                inserted_active_vectors.as_slice(),
                &[],
            )?;
            clear_vector_blobs_for_keys(
                &self.database_path,
                &inserted_active_vectors
                    .iter()
                    .map(|(vector_key, _)| *vector_key)
                    .collect::<Vec<_>>(),
            )?;
            clear_vector_index_dirty_marker(&self.database_path)?;
            self.index_dirty = false;
            rag_query::invalidate_rag_query_db_cache(&self.database_path).await;
        }

        if was_empty {
            self.created_table = true;
        }
        Ok(())
    }

    pub(super) async fn delete_where(&mut self, filter: &str) -> Result<()> {
        let (active_keys, active_dimensions, affected_rows) = {
            let mut connection = open_chunk_store_connection(&self.database_path)?;
            let transaction = connection
                .transaction()
                .context("failed to open rag chunk delete transaction")?;
            let active_keys = load_vector_keys_for_filter_in_transaction(
                &transaction,
                filter,
                RagChunkState::Active,
            )?;
            let active_dimensions = load_vector_dimensions_for_filter_in_transaction(
                &transaction,
                filter,
                RagChunkState::Active,
            )?;
            transaction
                .execute(
                    &format!(
                        "DELETE FROM {RAG_LEXICAL_TABLE_NAME} WHERE rowid IN (SELECT vector_key FROM rag_chunks WHERE {filter})"
                    ),
                    [],
                )
                .with_context(|| {
                    format!("failed to delete rag lexical rows with filter: {filter}")
                })?;
            let affected_rows = transaction
                .execute(&format!("DELETE FROM rag_chunks WHERE {filter}"), [])
                .with_context(|| {
                    format!("failed to delete rag chunk rows with filter: {filter}")
                })?;
            apply_vector_index_meta_delta_in_transaction(
                &transaction,
                &[],
                None,
                active_keys.as_slice(),
            )?;
            transaction
                .commit()
                .context("failed to commit rag chunk delete transaction")?;
            (active_keys, active_dimensions, affected_rows)
        };
        if !active_keys.is_empty() {
            mark_vector_index_dirty(&self.database_path)?;
            save_updated_vector_index_with_dimensions(
                &self.database_path,
                &[],
                active_keys.as_slice(),
                active_dimensions.unwrap_or(1),
            )?;
            clear_vector_index_dirty_marker(&self.database_path)?;
            self.index_dirty = false;
            rag_query::invalidate_rag_query_db_cache(&self.database_path).await;
        }
        if affected_rows > 0 && active_keys.is_empty() {
            self.created_table = false;
        }
        Ok(())
    }

    pub(super) async fn update_where(
        &mut self,
        filter: &str,
        chunk_state: RagChunkState,
    ) -> Result<()> {
        if chunk_state == RagChunkState::Active {
            let (staged_vectors, affected_rows) = {
                let mut connection = open_chunk_store_connection(&self.database_path)?;
                let transaction = connection
                    .transaction()
                    .context("failed to open rag chunk update transaction")?;
                let staged_vectors = load_vectors_for_filter_in_transaction(
                    &transaction,
                    filter,
                    RagChunkState::Staged,
                )?;
                let affected_rows = transaction
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
                apply_vector_index_meta_delta_in_transaction(
                    &transaction,
                    &staged_vectors
                        .iter()
                        .map(|row| row.vector_key)
                        .collect::<Vec<_>>(),
                    staged_vectors.first().map(|row| row.vector_dimensions),
                    &[],
                )?;
                transaction
                    .commit()
                    .context("failed to commit rag chunk update transaction")?;
                (staged_vectors, affected_rows)
            };
            if !staged_vectors.is_empty() {
                let active_vectors = staged_vectors
                    .iter()
                    .map(|row| Ok((row.vector_key, deserialize_vector_from_ref(row)?)))
                    .collect::<Result<Vec<_>>>()?;
                if affected_rows > 0 {
                    mark_vector_index_dirty(&self.database_path)?;
                    save_updated_vector_index(&self.database_path, active_vectors.as_slice(), &[])?;
                    clear_vector_blobs_for_keys(
                        &self.database_path,
                        &active_vectors
                            .iter()
                            .map(|(vector_key, _)| *vector_key)
                            .collect::<Vec<_>>(),
                    )?;
                    clear_vector_index_dirty_marker(&self.database_path)?;
                    self.created_table = false;
                    self.index_dirty = false;
                    rag_query::invalidate_rag_query_db_cache(&self.database_path).await;
                }
                return Ok(());
            }
            return Ok(());
        }
        let affected_rows = {
            let mut connection = open_chunk_store_connection(&self.database_path)?;
            let transaction = connection
                .transaction()
                .context("failed to open rag chunk update transaction")?;
            let affected_rows = transaction
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
            transaction
                .commit()
                .context("failed to commit rag chunk update transaction")?;
            affected_rows
        };
        if affected_rows > 0 {
            self.created_table = false;
        }
        Ok(())
    }

    pub(super) async fn load_chunk_vectors_for_file(
        &self,
        absolute_path: &str,
        chunk_state: RagChunkState,
        embedding_fingerprint: &str,
    ) -> Result<HashMap<String, Vec<f32>>> {
        let connection = open_chunk_store_connection(&self.database_path)?;
        let mut statement = connection
            .prepare(
                "
                SELECT chunk_reuse_key, vector_key, vector_blob, vector_dimensions
                FROM rag_chunks
                WHERE absolute_path = ?1 AND chunk_state = ?2 AND embedding_fingerprint = ?3
                ",
            )
            .with_context(|| {
                format!("failed to prepare chunk vector query for path: {absolute_path}")
            })?;
        let mut rows = statement
            .query(params![
                absolute_path,
                chunk_state.as_str(),
                embedding_fingerprint
            ])
            .with_context(|| {
                format!("failed to execute chunk vector query for path: {absolute_path}")
            })?;

        let mut vectors = HashMap::new();
        let mut index_refs = Vec::new();
        while let Some(row) = rows
            .next()
            .context("failed to step rag chunk vector rows")?
        {
            let chunk_reuse_key: String = row.get(0)?;
            let vector_key =
                u64::try_from(row.get::<_, i64>(1)?).context("vector_key is negative")?;
            let vector_blob: Option<Vec<u8>> = row.get(2)?;
            let vector_dimensions = read_vector_dimensions(row, 3)?;
            if let Some(vector_blob) = vector_blob.as_deref() {
                vectors
                    .entry(chunk_reuse_key)
                    .or_insert(deserialize_vector(vector_blob, vector_dimensions)?);
            } else {
                index_refs.push((chunk_reuse_key, vector_key, vector_dimensions));
            }
        }
        if !index_refs.is_empty() {
            let resolved = load_vectors_from_index(
                &self.database_path,
                &index_refs
                    .iter()
                    .map(|(_, key, dimensions)| (*key, *dimensions))
                    .collect::<Vec<_>>(),
            )?;
            for (chunk_reuse_key, vector_key, _) in index_refs {
                let vector = resolved.get(&vector_key).with_context(|| {
                    format!(
                        "missing reusable RAG vector {} in USearch index",
                        vector_key
                    )
                })?;
                vectors.entry(chunk_reuse_key).or_insert(vector.clone());
            }
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
        let mut index_vector_refs = Vec::new();
        let mut index_vector_texts = Vec::new();

        for batch in text_fingerprints.chunks(MAX_TEXT_FINGERPRINT_FILTERS) {
            let placeholders = repeat_sql_placeholders(batch.len(), 2);
            let sql = format!(
                "
                SELECT text_fingerprint, text, vector_key, vector_blob, vector_dimensions
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
                let vector_key =
                    u64::try_from(row.get::<_, i64>(2)?).context("vector_key is negative")?;
                let vector_blob: Option<Vec<u8>> = row.get(3)?;
                let vector_dimensions = read_vector_dimensions(row, 4)?;
                let stored_text_fingerprint: String = row.get(0)?;
                let text: String = row.get(1)?;
                let cached = if let Some(vector_blob) = vector_blob.as_deref() {
                    CachedTextVector {
                        text_fingerprint: stored_text_fingerprint,
                        text,
                        vector: deserialize_vector(vector_blob, vector_dimensions)?,
                    }
                } else {
                    index_vector_refs.push((vector_key, vector_dimensions));
                    index_vector_texts.push((vector_key, stored_text_fingerprint, text));
                    continue;
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

        if !index_vector_refs.is_empty() {
            let resolved = load_vectors_from_index(&self.database_path, &index_vector_refs)?;
            for (vector_key, stored_text_fingerprint, text) in index_vector_texts {
                let Some(vector) = resolved.get(&vector_key) else {
                    continue;
                };
                let cached = CachedTextVector {
                    text_fingerprint: stored_text_fingerprint,
                    text,
                    vector: vector.clone(),
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
        if !chunk_store_has_active_chunks(&self.database_path)? {
            clear_vector_index_artifacts(&self.database_path).await?;
            self.created_table = false;
            self.index_dirty = false;
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

async fn clear_vector_index_artifacts(database_path: &Path) -> Result<()> {
    rag_query::invalidate_rag_query_db_cache(database_path).await;
    for path in [
        vector_index_file_path(database_path),
        vector_index_dirty_marker_path(database_path),
        vector_index_manifest_path(database_path),
    ] {
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to remove RAG vector index artifact: {}",
                        path.display()
                    )
                });
            }
        }
    }
    Ok(())
}

pub(super) async fn clear_sqlite_store(sqlite_path: &Path) -> Result<()> {
    tokio::task::spawn_blocking({
        let sqlite_path = sqlite_path.to_path_buf();
        move || reset_sqlite_store(&sqlite_path)
    })
    .await
    .context("failed to join RAG sqlite cleanup task")??;
    Ok(())
}

fn initialize_empty_storage(database_path: &Path, sqlite_path: &Path) -> Result<()> {
    let _ = open_chunk_store_connection(database_path)?;
    let _ = open_rag_sqlite_connection(sqlite_path)?;
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
pub(super) fn rag_file_records_require_rebuild(
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
    sqlite_path: &Path,
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

    let sqlite_schema_is_compatible = tokio::task::spawn_blocking({
        let sqlite_path = sqlite_path.to_path_buf();
        move || rag_sqlite_has_compatible_schema(&sqlite_path)
    })
    .await
    .context("failed to join RAG sqlite schema task")??;
    let chunk_store_exists = chunk_store_database_file(database_path).exists();
    let vector_index_exists = vector_index_file_path(database_path).exists();
    let vector_index_dirty_marker_exists = vector_index_dirty_marker_path(database_path).exists();
    let vector_index_manifest_exists = vector_index_manifest_path(database_path).exists();
    let vector_schema_is_compatible = chunk_store_has_compatible_schema(database_path)?;
    let vector_table_exists = chunk_store_has_table(database_path)?;
    let vector_artifacts_exist = vector_table_exists
        || chunk_store_exists
        || vector_index_exists
        || vector_index_dirty_marker_exists
        || vector_index_manifest_exists;

    if vector_artifacts_exist && !vector_schema_is_compatible {
        tracing::warn!(
            "resetting RAG storage because vector storage schema is incompatible with current code"
        );
        clear_index(database_path).await?;
        clear_sqlite_store(sqlite_path).await?;
        initialize_empty_storage(database_path, sqlite_path)?;
        return Ok(());
    }

    if !sqlite_schema_is_compatible {
        tracing::warn!(
            "resetting RAG storage because SQLite schema is incompatible with current code"
        );
        if vector_artifacts_exist {
            clear_index(database_path).await?;
        }
        clear_sqlite_store(sqlite_path).await?;
        initialize_empty_storage(database_path, sqlite_path)?;
        return Ok(());
    }

    let stored_records = tokio::task::spawn_blocking({
        let sqlite_path = sqlite_path.to_path_buf();
        move || load_rag_file_records(&sqlite_path)
    })
    .await
    .context("failed to join RAG sqlite state task")??;
    if reset_on_embedding_target_mismatch
        && stored_records
            .values()
            .any(|record| record.embedding_fingerprint != resolved.embedding_fingerprint)
    {
        tracing::warn!(
            current_embedding_fingerprint = %resolved.embedding_fingerprint,
            "resetting RAG storage because indexed rows target a different embedding fingerprint"
        );
        if vector_artifacts_exist {
            clear_index(database_path).await?;
        }
        if !stored_records.is_empty() {
            clear_sqlite_store(sqlite_path).await?;
        }
        initialize_empty_storage(database_path, sqlite_path)?;
        return Ok(());
    }
    let sqlite_has_rows = !stored_records.is_empty();
    let sqlite_has_active_rows = stored_records
        .values()
        .any(|record| record.active.is_some());
    let vector_has_active_chunks = chunk_store_has_active_chunks(database_path)?;
    let chunk_store_artifacts_exist = vector_table_exists || chunk_store_exists;
    let vector_index_usable = if vector_has_active_chunks && vector_index_exists {
        vector_index_is_usable(database_path)?
    } else {
        false
    };
    if !chunk_store_artifacts_exist
        && (vector_index_exists || vector_index_dirty_marker_exists || vector_index_manifest_exists)
    {
        tracing::warn!(
            "resetting RAG storage because vector index artifacts exist without a chunk store"
        );
        clear_index(database_path).await?;
        if sqlite_has_rows {
            clear_sqlite_store(sqlite_path).await?;
        }
        initialize_empty_storage(database_path, sqlite_path)?;
        return Ok(());
    }

    match (vector_table_exists, sqlite_has_rows) {
        (true, false) => clear_index(database_path).await?,
        (false, true) => {
            tracing::warn!(
                "resetting RAG storage because chunk store table is missing \
                 but SQLite has indexed file records"
            );
            clear_sqlite_store(sqlite_path).await?;
            clear_vector_index_artifacts(database_path).await?;
            initialize_empty_storage(database_path, sqlite_path)?;
            return Ok(());
        }
        _ => {}
    }

    match (vector_has_active_chunks, sqlite_has_active_rows) {
        (true, false) => clear_index(database_path).await?,
        (false, true) => {
            tracing::warn!(
                "resetting RAG storage because chunk store has no active chunks \
                 but SQLite has active file records"
            );
            clear_sqlite_store(sqlite_path).await?;
            clear_vector_index_artifacts(database_path).await?;
            initialize_empty_storage(database_path, sqlite_path)?;
            return Ok(());
        }
        _ => {}
    }

    if !vector_has_active_chunks
        && (vector_index_exists || vector_index_dirty_marker_exists || vector_index_manifest_exists)
    {
        clear_vector_index_artifacts(database_path).await?;
    }

    if vector_has_active_chunks && (!vector_index_exists || !vector_index_usable) {
        match active_vector_blob_coverage(database_path)? {
            ActiveVectorBlobCoverage::All => {
                tracing::warn!(
                    "rebuilding RAG vector index because every active vector row still keeps a persisted vector blob"
                );
                let database_path = database_path.to_path_buf();
                let rebuild_path = database_path.clone();
                tokio::task::spawn_blocking(move || rebuild_vector_index(&rebuild_path))
                    .await
                    .context("failed to join startup vector index rebuild task")??;
                rag_query::invalidate_rag_query_db_cache(&database_path).await;
                clear_active_vector_blobs(&database_path)?;
                clear_vector_index_dirty_marker(&database_path)?;
            }
            ActiveVectorBlobCoverage::Partial | ActiveVectorBlobCoverage::None => {
                tracing::warn!(
                    "resetting RAG storage because the USearch index is missing or unusable and active vector blobs are not fully recoverable"
                );
                clear_index(database_path).await?;
                clear_sqlite_store(sqlite_path).await?;
                initialize_empty_storage(database_path, sqlite_path)?;
                return Ok(());
            }
        }
    } else if vector_has_active_chunks && vector_index_dirty_marker_exists {
        tracing::warn!(
            "clearing stale RAG vector index dirty marker after confirming the USearch index is still usable"
        );
        clear_active_vector_blobs(database_path)?;
        clear_vector_index_dirty_marker(database_path)?;
    }

    Ok(())
}

pub(crate) fn vector_index_file_path(database_path: &Path) -> PathBuf {
    database_path.join(RAG_VECTOR_INDEX_FILE_NAME)
}

fn vector_index_dirty_marker_path(database_path: &Path) -> PathBuf {
    database_path.join(RAG_VECTOR_INDEX_DIRTY_FILE_NAME)
}

pub(crate) fn vector_index_manifest_path(database_path: &Path) -> PathBuf {
    database_path.join(RAG_VECTOR_INDEX_MANIFEST_FILE_NAME)
}

fn sqlite_u64(value: u64) -> i64 {
    value as i64
}

fn decode_sqlite_u64(value: i64) -> u64 {
    value as u64
}

fn stable_vector_key_hash(vector_key: u64) -> u64 {
    let mixed = vector_key.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn stable_vector_value_hash(vector: &[f32]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for value in vector {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x1000_0000_01B3);
        }
    }
    hash
}

fn apply_vector_key_add(meta: &mut VectorIndexMeta, vector_key: u64) {
    let key_hash = stable_vector_key_hash(vector_key);
    meta.active_vector_count = meta.active_vector_count.saturating_add(1);
    meta.key_xor ^= vector_key;
    meta.key_sum = meta.key_sum.wrapping_add(vector_key);
    meta.key_hash_xor ^= key_hash;
    meta.key_hash_sum = meta.key_hash_sum.wrapping_add(key_hash);
}

fn apply_vector_key_remove(meta: &mut VectorIndexMeta, vector_key: u64) -> Result<()> {
    if meta.active_vector_count == 0 {
        bail!(
            "cannot remove RAG vector {} from empty index metadata",
            vector_key
        );
    }
    let key_hash = stable_vector_key_hash(vector_key);
    meta.active_vector_count -= 1;
    meta.key_xor ^= vector_key;
    meta.key_sum = meta.key_sum.wrapping_sub(vector_key);
    meta.key_hash_xor ^= key_hash;
    meta.key_hash_sum = meta.key_hash_sum.wrapping_sub(key_hash);
    Ok(())
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

fn vector_index_manifest_matches_meta(
    manifest: &VectorIndexManifest,
    meta: VectorIndexMeta,
) -> bool {
    manifest.version == RAG_VECTOR_INDEX_MANIFEST_VERSION
        && manifest.probes.len() == vector_index_probe_offsets(meta.active_vector_count).len()
        && manifest.active_vector_count == meta.active_vector_count
        && manifest.vector_dimensions == meta.vector_dimensions
        && manifest.key_xor == meta.key_xor
        && manifest.key_sum == meta.key_sum
        && manifest.key_hash_xor == meta.key_hash_xor
        && manifest.key_hash_sum == meta.key_hash_sum
}

pub(crate) fn load_vector_index_manifest(
    database_path: &Path,
) -> Result<Option<VectorIndexManifest>> {
    let manifest_path = vector_index_manifest_path(database_path);
    if !manifest_path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&manifest_path).with_context(|| {
        format!(
            "failed to read RAG vector index manifest: {}",
            manifest_path.display()
        )
    })?;
    let manifest = serde_json::from_slice::<VectorIndexManifest>(&bytes).with_context(|| {
        format!(
            "failed to parse RAG vector index manifest: {}",
            manifest_path.display()
        )
    })?;
    Ok(Some(manifest))
}

fn file_modified_at_ms(path: &Path) -> Result<u64> {
    let modified = std::fs::metadata(path)
        .with_context(|| format!("failed to stat file: {}", path.display()))?
        .modified()
        .with_context(|| format!("failed to read modified time: {}", path.display()))?;
    let modified = modified
        .duration_since(UNIX_EPOCH)
        .with_context(|| format!("modified time is before unix epoch: {}", path.display()))?;
    Ok(u64::try_from(modified.as_millis()).unwrap_or(u64::MAX))
}

fn compute_file_md5_hex(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("failed to open file for md5 digest: {}", path.display()))?;
    let mut context = md5::Context::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read file for md5 digest: {}", path.display()))?;
        if read == 0 {
            break;
        }
        context.consume(&buffer[..read]);
    }
    Ok(format!("{:x}", context.compute()))
}

fn vector_index_probe_offsets(active_vector_count: u64) -> Vec<u64> {
    if active_vector_count == 0 {
        return Vec::new();
    }
    let last = active_vector_count - 1;
    let mut offsets = vec![0, last / 3, (last * 2) / 3, last];
    offsets.sort_unstable();
    offsets.dedup();
    if offsets.len() > RAG_VECTOR_INDEX_PROBE_COUNT {
        offsets.truncate(RAG_VECTOR_INDEX_PROBE_COUNT);
    }
    offsets
}

fn load_probe_key_at_offset(connection: &Connection, offset: u64) -> Result<u64> {
    connection
        .query_row(
            "
            SELECT vector_key
            FROM rag_chunks
            WHERE chunk_state = 'active'
            ORDER BY vector_key
            LIMIT 1 OFFSET ?1
            ",
            [
                i64::try_from(offset)
                    .context("probe key offset does not fit into SQLite INTEGER")?,
            ],
            |row| row.get::<_, i64>(0),
        )
        .with_context(|| format!("failed to load active vector probe key at offset {offset}"))
        .and_then(|value| u64::try_from(value).context("probe vector_key is negative"))
}

fn load_vector_index_probe_keys(
    connection: &Connection,
    active_vector_count: u64,
) -> Result<Vec<u64>> {
    vector_index_probe_offsets(active_vector_count)
        .into_iter()
        .map(|offset| load_probe_key_at_offset(connection, offset))
        .collect()
}

fn export_index_vector(
    index: &Index,
    vector_key: u64,
    expected_dimensions: usize,
) -> Result<Vec<f32>> {
    let mut vector = Vec::with_capacity(expected_dimensions);
    index
        .export::<f32>(vector_key, &mut vector)
        .with_context(|| format!("failed to export vector key {} from USearch", vector_key))?;
    if vector.is_empty() {
        bail!("missing RAG vector {} in USearch index", vector_key);
    }
    if vector.len() != expected_dimensions {
        bail!(
            "USearch vector dimension mismatch for key {}: expected {}, got {}",
            vector_key,
            expected_dimensions,
            vector.len()
        );
    }
    Ok(vector)
}

fn build_vector_index_probes(
    connection: &Connection,
    index: &Index,
    meta: VectorIndexMeta,
) -> Result<Vec<VectorIndexProbe>> {
    let probe_keys = load_vector_index_probe_keys(connection, meta.active_vector_count)?;
    probe_keys
        .into_iter()
        .map(|vector_key| {
            let vector = export_index_vector(index, vector_key, meta.vector_dimensions)?;
            Ok(VectorIndexProbe {
                vector_key,
                vector_hash: stable_vector_value_hash(vector.as_slice()),
            })
        })
        .collect()
}

fn validate_vector_index_probes(
    index: &Index,
    manifest: &VectorIndexManifest,
    expected_dimensions: usize,
) -> Result<()> {
    for probe in &manifest.probes {
        let vector = export_index_vector(index, probe.vector_key, expected_dimensions)?;
        let actual_hash = stable_vector_value_hash(vector.as_slice());
        if actual_hash != probe.vector_hash {
            bail!(
                "USearch probe mismatch for key {}: expected hash {}, got {}",
                probe.vector_key,
                probe.vector_hash,
                actual_hash
            );
        }
    }
    Ok(())
}

fn write_vector_index_manifest(
    connection: &Connection,
    database_path: &Path,
    index: &Index,
    meta: VectorIndexMeta,
    index_md5_hex: Option<&str>,
) -> Result<()> {
    let manifest_path = vector_index_manifest_path(database_path);
    if meta.active_vector_count == 0 {
        match std::fs::remove_file(&manifest_path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to remove empty RAG vector index manifest: {}",
                        manifest_path.display()
                    )
                })
            }
        }
    }

    let index_path = vector_index_file_path(database_path);
    let metadata = std::fs::metadata(&index_path).with_context(|| {
        format!(
            "failed to stat RAG vector index for manifest write: {}",
            index_path.display()
        )
    })?;
    let manifest = VectorIndexManifest {
        version: RAG_VECTOR_INDEX_MANIFEST_VERSION,
        active_vector_count: meta.active_vector_count,
        vector_dimensions: meta.vector_dimensions,
        key_xor: meta.key_xor,
        key_sum: meta.key_sum,
        key_hash_xor: meta.key_hash_xor,
        key_hash_sum: meta.key_hash_sum,
        index_size_bytes: metadata.len(),
        index_modified_at_ms: file_modified_at_ms(&index_path)?,
        probes: build_vector_index_probes(connection, index, meta)?,
        index_md5_hex: index_md5_hex.map(ToOwned::to_owned),
    };
    let bytes =
        serde_json::to_vec_pretty(&manifest).context("failed to serialize RAG index manifest")?;
    let temp_path = manifest_path.with_extension("json.tmp");
    std::fs::write(&temp_path, bytes).with_context(|| {
        format!(
            "failed to write temporary RAG vector index manifest: {}",
            temp_path.display()
        )
    })?;
    std::fs::rename(&temp_path, &manifest_path).with_context(|| {
        format!(
            "failed to replace RAG vector index manifest {} -> {}",
            temp_path.display(),
            manifest_path.display()
        )
    })?;
    Ok(())
}

fn sync_vector_index_manifest_with_meta(database_path: &Path, index: Option<&Index>) -> Result<()> {
    let connection = open_chunk_store_connection(database_path)?;
    let meta = load_vector_index_meta(&connection)?;
    if meta.active_vector_count == 0 {
        let manifest_path = vector_index_manifest_path(database_path);
        return match std::fs::remove_file(&manifest_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to remove empty RAG vector index manifest: {}",
                    manifest_path.display()
                )
            }),
        };
    }
    let index = index.context("missing loaded USearch index while syncing non-empty manifest")?;
    let index_md5_hex = compute_file_md5_hex(&vector_index_file_path(database_path))?;
    write_vector_index_manifest(
        &connection,
        database_path,
        index,
        meta,
        Some(index_md5_hex.as_str()),
    )
}

fn clear_vector_blobs_for_keys(database_path: &Path, vector_keys: &[u64]) -> Result<()> {
    if vector_keys.is_empty() {
        return Ok(());
    }

    let mut connection = open_chunk_store_connection(database_path)?;
    let transaction = connection
        .transaction()
        .context("failed to open transaction for active vector blob cleanup")?;
    for batch in vector_keys.chunks(MAX_DELETE_FILTER_PATHS) {
        let placeholders = repeat_sql_placeholders(batch.len(), 1);
        let sql = format!(
            "UPDATE rag_chunks SET vector_blob = NULL WHERE vector_key IN ({placeholders})"
        );
        let params =
            rusqlite::params_from_iter(batch.iter().map(|key| {
                i64::try_from(*key).expect("vector_key should fit into SQLite INTEGER")
            }));
        transaction
            .execute(&sql, params)
            .context("failed to clear persisted active vector blobs")?;
    }
    transaction
        .commit()
        .context("failed to commit active vector blob cleanup")?;
    Ok(())
}

fn clear_active_vector_blobs(database_path: &Path) -> Result<()> {
    let mut connection = open_chunk_store_connection(database_path)?;
    let transaction = connection
        .transaction()
        .context("failed to open transaction for active vector blob cleanup")?;
    transaction
        .execute(
            "
            UPDATE rag_chunks
            SET vector_blob = NULL
            WHERE chunk_state = 'active' AND vector_blob IS NOT NULL
            ",
            [],
        )
        .context("failed to clear active vector blobs")?;
    transaction
        .commit()
        .context("failed to commit active vector blob cleanup")?;
    Ok(())
}

fn active_vector_blob_coverage(database_path: &Path) -> Result<ActiveVectorBlobCoverage> {
    if !chunk_store_has_compatible_schema(database_path)? {
        return Ok(ActiveVectorBlobCoverage::None);
    }

    let connection = open_chunk_store_connection(database_path)?;
    let (active_chunk_count, active_blob_count): (i64, i64) = connection
        .query_row(
            "
            SELECT
                COUNT(*),
                COUNT(CASE WHEN vector_blob IS NOT NULL THEN 1 END)
            FROM rag_chunks
            WHERE chunk_state = 'active'
            ",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .context("failed to inspect active vector blob coverage")?;
    Ok(match (active_chunk_count, active_blob_count) {
        (0, _) | (_, 0) => ActiveVectorBlobCoverage::None,
        (chunk_count, blob_count) if chunk_count == blob_count => ActiveVectorBlobCoverage::All,
        _ => ActiveVectorBlobCoverage::Partial,
    })
}

fn save_updated_vector_index(
    database_path: &Path,
    added: &[(u64, Vec<f32>)],
    removed: &[u64],
) -> Result<()> {
    let dimensions = added
        .first()
        .map(|(_, vector)| vector.len())
        .or_else(|| load_active_vector_dimensions(database_path).ok().flatten())
        .unwrap_or(1);
    save_updated_vector_index_with_dimensions(database_path, added, removed, dimensions)
}

fn save_updated_vector_index_with_dimensions(
    database_path: &Path,
    added: &[(u64, Vec<f32>)],
    removed: &[u64],
    dimensions: usize,
) -> Result<()> {
    if added.is_empty() && removed.is_empty() {
        return Ok(());
    }

    let index_path = vector_index_file_path(database_path);
    let options = build_usearch_index_options(dimensions);
    let index = Index::new(&options).context("failed to create mutable USearch index")?;
    if index_path.exists() {
        index
            .load(index_path.to_string_lossy().as_ref())
            .with_context(|| format!("failed to load USearch index: {}", index_path.display()))?;
    }
    if index.size() == 0 && added.is_empty() {
        return Ok(());
    }
    if index.size() > 0 && index.dimensions() != dimensions {
        bail!(
            "USearch index dimension mismatch: expected {}, got {}",
            index.dimensions(),
            dimensions
        );
    }
    index
        .reserve(index.size().saturating_add(added.len()))
        .context("failed to reserve USearch index capacity")?;

    for key in removed {
        index
            .remove(*key)
            .with_context(|| format!("failed to remove vector key {} from USearch", key))?;
    }
    for (key, vector) in added {
        index
            .add(*key, vector.as_slice())
            .with_context(|| format!("failed to add vector key {} into USearch", key))?;
    }

    if index.size() == 0 {
        match std::fs::remove_file(&index_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to remove empty USearch index: {}",
                        index_path.display()
                    )
                });
            }
        }
        sync_vector_index_manifest_with_meta(database_path, None)?;
        return Ok(());
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
    sync_vector_index_manifest_with_meta(database_path, Some(&index))?;
    Ok(())
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

fn deserialize_vector_from_ref(row: &StoredChunkVectorRef) -> Result<Vec<f32>> {
    let blob = row
        .vector_blob
        .as_deref()
        .context("missing staged vector blob for vector index update")?;
    deserialize_vector(blob, row.vector_dimensions)
}

fn backfill_vector_hashes(
    connection: &Connection,
    vector_hashes: &[(u64, u64)],
    only_when_missing: bool,
) -> Result<()> {
    if vector_hashes.is_empty() {
        return Ok(());
    }
    let sql = if only_when_missing {
        "UPDATE rag_chunks SET vector_hash = ?1 WHERE vector_key = ?2 AND vector_hash IS NULL"
    } else {
        "UPDATE rag_chunks SET vector_hash = ?1 WHERE vector_key = ?2"
    };
    let mut statement = connection
        .prepare(sql)
        .context("failed to prepare RAG vector hash backfill statement")?;
    for (vector_key, vector_hash) in vector_hashes {
        statement
            .execute(params![sqlite_u64(*vector_hash), sqlite_u64(*vector_key)])
            .with_context(|| format!("failed to persist RAG vector hash for key {}", vector_key))?;
    }
    Ok(())
}

fn backfill_vector_hashes_from_blob_rows(connection: &Connection) -> Result<()> {
    let mut statement = connection
        .prepare(
            "
            SELECT vector_key, vector_blob, vector_dimensions
            FROM rag_chunks
            WHERE vector_hash IS NULL AND vector_blob IS NOT NULL
            ",
        )
        .context("failed to prepare RAG vector hash blob backfill query")?;
    let rows = statement
        .query_map([], |row| {
            Ok(StoredChunkVectorRef {
                vector_key: u64::try_from(row.get::<_, i64>(0)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?,
                vector_blob: row.get(1)?,
                vector_dimensions: usize::try_from(row.get::<_, i64>(2)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?,
            })
        })
        .context("failed to query blob-backed RAG vectors for hash backfill")?;
    let updates = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect blob-backed RAG vectors for hash backfill")?
        .into_iter()
        .map(|row| {
            Ok((
                row.vector_key,
                stable_vector_value_hash(deserialize_vector_from_ref(&row)?.as_slice()),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    backfill_vector_hashes(connection, updates.as_slice(), true)
}

fn load_vectors_from_index(
    database_path: &Path,
    vector_refs: &[(u64, usize)],
) -> Result<HashMap<u64, Vec<f32>>> {
    if vector_refs.is_empty() {
        return Ok(HashMap::new());
    }

    let dimensions = vector_refs[0].1.max(1);
    let options = build_usearch_index_options(dimensions);
    let index = Index::new(&options).context("failed to create USearch reader")?;
    let index_path = vector_index_file_path(database_path);
    index
        .load(index_path.to_string_lossy().as_ref())
        .with_context(|| format!("failed to load USearch index: {}", index_path.display()))?;

    let mut resolved = HashMap::new();
    for (key, expected_dimensions) in vector_refs {
        let mut vector = Vec::new();
        index
            .export::<f32>(*key, &mut vector)
            .with_context(|| format!("failed to export vector key {} from USearch", key))?;
        if vector.is_empty() {
            continue;
        }
        if vector.len() != *expected_dimensions {
            bail!(
                "USearch vector dimension mismatch for key {}: expected {}, got {}",
                key,
                expected_dimensions,
                vector.len()
            );
        }
        resolved.insert(*key, vector);
    }
    Ok(resolved)
}

fn load_vector_keys_for_filter_in_transaction(
    transaction: &Transaction<'_>,
    filter: &str,
    chunk_state: RagChunkState,
) -> Result<Vec<u64>> {
    let sql = format!(
        "SELECT vector_key FROM rag_chunks WHERE {filter} AND chunk_state = '{}'",
        chunk_state.as_str()
    );
    let mut statement = transaction
        .prepare(&sql)
        .context("failed to prepare RAG vector key lookup")?;
    let rows = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .context("failed to query RAG vector keys")?;
    rows.map(|row| {
        row.and_then(|value| {
            u64::try_from(value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Integer,
                    Box::new(error),
                )
            })
        })
    })
    .collect::<rusqlite::Result<Vec<_>>>()
    .context("failed to collect RAG vector keys")
}

fn load_vector_dimensions_for_filter_in_transaction(
    transaction: &Transaction<'_>,
    filter: &str,
    chunk_state: RagChunkState,
) -> Result<Option<usize>> {
    let sql = format!(
        "SELECT vector_dimensions FROM rag_chunks WHERE {filter} AND chunk_state = '{}' LIMIT 1",
        chunk_state.as_str()
    );
    let mut statement = transaction
        .prepare(&sql)
        .context("failed to prepare RAG vector dimension lookup")?;
    let dimensions = statement
        .query_row([], |row| row.get::<_, i64>(0))
        .optional()
        .context("failed to query RAG vector dimensions")?;
    dimensions
        .map(|value| usize::try_from(value).context("vector_dimensions is negative or too large"))
        .transpose()
}

fn load_vectors_for_filter_in_transaction(
    transaction: &Transaction<'_>,
    filter: &str,
    chunk_state: RagChunkState,
) -> Result<Vec<StoredChunkVectorRef>> {
    let sql = format!(
        "
        SELECT vector_key, vector_blob, vector_dimensions
        FROM rag_chunks
        WHERE {filter} AND chunk_state = '{}'
        ",
        chunk_state.as_str()
    );
    let mut statement = transaction
        .prepare(&sql)
        .context("failed to prepare staged vector lookup")?;
    let rows = statement
        .query_map([], |row| {
            let vector_key_i64 = row.get::<_, i64>(0)?;
            let vector_dimensions_i64 = row.get::<_, i64>(2)?;
            Ok(StoredChunkVectorRef {
                vector_key: u64::try_from(vector_key_i64).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?,
                vector_blob: row.get(1)?,
                vector_dimensions: usize::try_from(vector_dimensions_i64).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?,
            })
        })
        .context("failed to query staged vectors")?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect staged vectors")
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
    initialize_rag_sqlite_schema(&connection)?;
    initialize_chunk_store_schema(&connection)?;
    Ok(connection)
}

fn rag_chunk_table_exists(connection: &Connection) -> Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'rag_chunks' LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()
        .context("failed to inspect rag chunk table existence")?
        .is_some())
}

fn load_chunk_table_schema_columns(connection: &Connection) -> Result<Vec<(String, String, bool)>> {
    let mut statement = connection
        .prepare("PRAGMA table_info(rag_chunks)")
        .context("failed to inspect rag chunk schema")?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? != 0,
            ))
        })
        .context("failed to query rag chunk schema")?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect rag chunk schema rows")
}

fn migrate_chunk_table_schema(connection: &Connection) -> Result<()> {
    if !rag_chunk_table_exists(connection)? {
        return Ok(());
    }
    let columns = load_chunk_table_schema_columns(connection)?;
    if columns.iter().any(|(name, _, _)| name == "vector_hash") {
        backfill_vector_hashes_from_blob_rows(connection)?;
        return Ok(());
    }
    connection
        .execute("ALTER TABLE rag_chunks ADD COLUMN vector_hash INTEGER", [])
        .context("failed to add vector_hash column to rag_chunks")?;
    backfill_vector_hashes_from_blob_rows(connection)
}

fn load_vector_index_meta(connection: &Connection) -> Result<VectorIndexMeta> {
    connection
        .query_row(
            &format!(
                "
                SELECT
                    active_vector_count,
                    vector_dimensions,
                    key_xor,
                    key_sum,
                    key_hash_xor,
                    key_hash_sum
                FROM {RAG_VECTOR_INDEX_META_TABLE_NAME}
                WHERE singleton_key = 1
                "
            ),
            [],
            |row| {
                Ok(VectorIndexMeta {
                    active_vector_count: decode_sqlite_u64(row.get::<_, i64>(0)?),
                    vector_dimensions: usize::try_from(row.get::<_, i64>(1)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Integer,
                            Box::new(error),
                        )
                    })?,
                    key_xor: decode_sqlite_u64(row.get::<_, i64>(2)?),
                    key_sum: decode_sqlite_u64(row.get::<_, i64>(3)?),
                    key_hash_xor: decode_sqlite_u64(row.get::<_, i64>(4)?),
                    key_hash_sum: decode_sqlite_u64(row.get::<_, i64>(5)?),
                })
            },
        )
        .optional()
        .context("failed to load RAG vector index metadata")?
        .map(Ok)
        .unwrap_or_else(|| Ok(VectorIndexMeta::default()))
}

fn load_vector_index_meta_in_transaction(transaction: &Transaction<'_>) -> Result<VectorIndexMeta> {
    transaction
        .query_row(
            &format!(
                "
                SELECT
                    active_vector_count,
                    vector_dimensions,
                    key_xor,
                    key_sum,
                    key_hash_xor,
                    key_hash_sum
                FROM {RAG_VECTOR_INDEX_META_TABLE_NAME}
                WHERE singleton_key = 1
                "
            ),
            [],
            |row| {
                Ok(VectorIndexMeta {
                    active_vector_count: decode_sqlite_u64(row.get::<_, i64>(0)?),
                    vector_dimensions: usize::try_from(row.get::<_, i64>(1)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Integer,
                            Box::new(error),
                        )
                    })?,
                    key_xor: decode_sqlite_u64(row.get::<_, i64>(2)?),
                    key_sum: decode_sqlite_u64(row.get::<_, i64>(3)?),
                    key_hash_xor: decode_sqlite_u64(row.get::<_, i64>(4)?),
                    key_hash_sum: decode_sqlite_u64(row.get::<_, i64>(5)?),
                })
            },
        )
        .optional()
        .context("failed to load RAG vector index metadata in transaction")?
        .map(Ok)
        .unwrap_or_else(|| Ok(VectorIndexMeta::default()))
}

fn upsert_vector_index_meta(connection: &Connection, meta: &VectorIndexMeta) -> Result<()> {
    connection
        .execute(
            &format!(
                "
                INSERT INTO {RAG_VECTOR_INDEX_META_TABLE_NAME} (
                    singleton_key,
                    active_vector_count,
                    vector_dimensions,
                    key_xor,
                    key_sum,
                    key_hash_xor,
                    key_hash_sum
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(singleton_key) DO UPDATE SET
                    active_vector_count = excluded.active_vector_count,
                    vector_dimensions = excluded.vector_dimensions,
                    key_xor = excluded.key_xor,
                    key_sum = excluded.key_sum,
                    key_hash_xor = excluded.key_hash_xor,
                    key_hash_sum = excluded.key_hash_sum
                "
            ),
            params![
                1_i64,
                sqlite_u64(meta.active_vector_count),
                i64::try_from(meta.vector_dimensions)
                    .context("vector_dimensions does not fit into SQLite INTEGER")?,
                sqlite_u64(meta.key_xor),
                sqlite_u64(meta.key_sum),
                sqlite_u64(meta.key_hash_xor),
                sqlite_u64(meta.key_hash_sum),
            ],
        )
        .context("failed to upsert RAG vector index metadata")?;
    Ok(())
}

fn upsert_vector_index_meta_in_transaction(
    transaction: &Transaction<'_>,
    meta: &VectorIndexMeta,
) -> Result<()> {
    transaction
        .execute(
            &format!(
                "
                INSERT INTO {RAG_VECTOR_INDEX_META_TABLE_NAME} (
                    singleton_key,
                    active_vector_count,
                    vector_dimensions,
                    key_xor,
                    key_sum,
                    key_hash_xor,
                    key_hash_sum
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(singleton_key) DO UPDATE SET
                    active_vector_count = excluded.active_vector_count,
                    vector_dimensions = excluded.vector_dimensions,
                    key_xor = excluded.key_xor,
                    key_sum = excluded.key_sum,
                    key_hash_xor = excluded.key_hash_xor,
                    key_hash_sum = excluded.key_hash_sum
                "
            ),
            params![
                1_i64,
                sqlite_u64(meta.active_vector_count),
                i64::try_from(meta.vector_dimensions)
                    .context("vector_dimensions does not fit into SQLite INTEGER")?,
                sqlite_u64(meta.key_xor),
                sqlite_u64(meta.key_sum),
                sqlite_u64(meta.key_hash_xor),
                sqlite_u64(meta.key_hash_sum),
            ],
        )
        .context("failed to upsert RAG vector index metadata in transaction")?;
    Ok(())
}

fn rebuild_vector_index_meta(connection: &Connection) -> Result<VectorIndexMeta> {
    let mut statement = connection
        .prepare(
            "
            SELECT vector_key, vector_dimensions
            FROM rag_chunks
            WHERE chunk_state = 'active'
            ORDER BY vector_key
            ",
        )
        .context("failed to prepare active vector metadata rebuild query")?;
    let mut rows = statement
        .query([])
        .context("failed to query active vector metadata for rebuild")?;
    let mut meta = VectorIndexMeta::default();
    while let Some(row) = rows
        .next()
        .context("failed to step active vector metadata rebuild rows")?
    {
        let vector_key = u64::try_from(row.get::<_, i64>(0)?).context("vector_key is negative")?;
        let vector_dimensions = read_vector_dimensions(row, 1)?;
        if meta.active_vector_count == 0 {
            meta.vector_dimensions = vector_dimensions;
        } else if meta.vector_dimensions != vector_dimensions {
            bail!(
                "inconsistent active RAG vector dimensions while rebuilding metadata: expected {}, got {}",
                meta.vector_dimensions,
                vector_dimensions
            );
        }
        apply_vector_key_add(&mut meta, vector_key);
    }
    if meta.active_vector_count == 0 {
        meta.vector_dimensions = 0;
    }
    Ok(meta)
}

fn ensure_vector_index_meta(connection: &Connection) -> Result<()> {
    let row_exists = connection
        .query_row(
            &format!(
                "SELECT 1 FROM {RAG_VECTOR_INDEX_META_TABLE_NAME} WHERE singleton_key = 1 LIMIT 1"
            ),
            [],
            |_| Ok(()),
        )
        .optional()
        .context("failed to inspect RAG vector index metadata row")?
        .is_some();
    if row_exists {
        return Ok(());
    }
    let meta = rebuild_vector_index_meta(connection)?;
    upsert_vector_index_meta(connection, &meta)
}

fn apply_vector_index_meta_delta_in_transaction(
    transaction: &Transaction<'_>,
    added_keys: &[u64],
    added_dimensions: Option<usize>,
    removed_keys: &[u64],
) -> Result<()> {
    let mut meta = load_vector_index_meta_in_transaction(transaction)?;
    if !added_keys.is_empty() {
        let added_dimensions = added_dimensions
            .context("missing vector_dimensions for active vector metadata insert")?;
        if meta.active_vector_count == 0 {
            meta.vector_dimensions = added_dimensions;
        } else if meta.vector_dimensions != added_dimensions {
            bail!(
                "active RAG vector dimension mismatch while updating metadata: expected {}, got {}",
                meta.vector_dimensions,
                added_dimensions
            );
        }
        for key in added_keys {
            apply_vector_key_add(&mut meta, *key);
        }
    }
    for key in removed_keys {
        apply_vector_key_remove(&mut meta, *key)?;
    }
    if meta.active_vector_count == 0 {
        meta = VectorIndexMeta::default();
    }
    upsert_vector_index_meta_in_transaction(transaction, &meta)
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
                vector_blob BLOB,
                vector_dimensions INTEGER NOT NULL,
                vector_hash INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_rag_chunks_path_state
                ON rag_chunks(absolute_path, chunk_state);
            CREATE INDEX IF NOT EXISTS idx_rag_chunks_version_state
                ON rag_chunks(absolute_path, version_id, chunk_state);
            CREATE INDEX IF NOT EXISTS idx_rag_chunks_chunk_reuse
                ON rag_chunks(absolute_path, chunk_state, chunk_reuse_key);
            CREATE INDEX IF NOT EXISTS idx_rag_chunks_embedding_text
                ON rag_chunks(embedding_fingerprint, text_fingerprint);
            CREATE TABLE IF NOT EXISTS rag_vector_index_meta (
                singleton_key INTEGER PRIMARY KEY CHECK(singleton_key = 1),
                active_vector_count INTEGER NOT NULL,
                vector_dimensions INTEGER NOT NULL,
                key_xor INTEGER NOT NULL,
                key_sum INTEGER NOT NULL,
                key_hash_xor INTEGER NOT NULL,
                key_hash_sum INTEGER NOT NULL
            );
            ",
        )
        .context("failed to initialize rag chunk schema")?;
    migrate_chunk_table_schema(connection)?;
    ensure_vector_index_meta(connection)?;
    Ok(())
}

fn chunk_store_has_table(database_path: &Path) -> Result<bool> {
    let db_path = chunk_store_database_file(database_path);
    if !db_path.exists() {
        return Ok(false);
    }
    let connection = Connection::open(&db_path).with_context(|| {
        format!(
            "failed to open RAG chunk database for table inspection: {}",
            db_path.display()
        )
    })?;
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

pub(super) fn chunk_store_has_compatible_schema(database_path: &Path) -> Result<bool> {
    let db_path = chunk_store_database_file(database_path);
    if !db_path.exists() {
        return Ok(false);
    }
    let connection = Connection::open(&db_path).with_context(|| {
        format!(
            "failed to open RAG chunk database for schema inspection: {}",
            db_path.display()
        )
    })?;
    migrate_chunk_table_schema(&connection)?;
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
        ("vector_key", "INTEGER", false),
        ("id", "TEXT", true),
        ("source_root", "TEXT", true),
        ("absolute_path", "TEXT", true),
        ("version_id", "TEXT", true),
        ("embedding_fingerprint", "TEXT", true),
        ("document_kind", "TEXT", true),
        ("chunk_state", "TEXT", true),
        ("chunk_index", "INTEGER", true),
        ("line_start", "INTEGER", false),
        ("line_end", "INTEGER", false),
        ("paragraph_line_start", "INTEGER", false),
        ("page_start", "INTEGER", false),
        ("page_end", "INTEGER", false),
        ("heading_path_json", "TEXT", true),
        ("anchor_label", "TEXT", false),
        ("chunk_reuse_key", "TEXT", true),
        ("text_fingerprint", "TEXT", true),
        ("text", "TEXT", true),
        ("vector_blob", "BLOB", false),
        ("vector_dimensions", "INTEGER", true),
        ("vector_hash", "INTEGER", false),
    ];
    let columns = load_chunk_table_schema_columns(connection)?;

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

fn vector_index_is_usable(database_path: &Path) -> Result<bool> {
    let index_path = vector_index_file_path(database_path);
    if !index_path.exists() {
        return Ok(false);
    }

    let connection = open_chunk_store_connection(database_path)?;
    let meta = load_vector_index_meta(&connection)?;
    let dimensions = meta.vector_dimensions.max(1);
    let options = build_usearch_index_options(dimensions);
    let index = Index::new(&options).context("failed to create USearch index validator")?;
    match index.load(index_path.to_string_lossy().as_ref()) {
        Ok(()) => {
            if meta.active_vector_count > 0 && index.dimensions() != dimensions {
                tracing::warn!(
                    expected_dimensions = dimensions,
                    actual_dimensions = index.dimensions(),
                    path = %index_path.display(),
                    "marking cached USearch index dirty because its dimensions do not match active RAG metadata"
                );
                return Ok(false);
            }
            if index.size() != usize::try_from(meta.active_vector_count).unwrap_or(usize::MAX) {
                tracing::warn!(
                    expected_size = meta.active_vector_count,
                    actual_size = index.size(),
                    path = %index_path.display(),
                    "marking cached USearch index dirty because its size does not match active RAG metadata"
                );
                return Ok(false);
            }
            match load_vector_index_manifest(database_path)? {
                Some(manifest) => {
                    let needs_manifest_backfill = manifest.version
                        != RAG_VECTOR_INDEX_MANIFEST_VERSION
                        || (meta.active_vector_count > 0 && manifest.probes.is_empty())
                        || manifest.index_md5_hex.is_none();
                    if !needs_manifest_backfill
                        && !vector_index_manifest_matches_meta(&manifest, meta)
                    {
                        tracing::warn!(
                            path = %index_path.display(),
                            "marking cached USearch index dirty because its manifest does not match active RAG metadata"
                        );
                        return Ok(false);
                    }
                    if needs_manifest_backfill {
                        let active_vector_refs = load_active_vector_refs(&connection)?;
                        let has_missing_hashes = active_vector_refs
                            .iter()
                            .any(|vector_ref| vector_ref.vector_hash.is_none());
                        if has_missing_hashes && !manifest.probes.is_empty() {
                            let expected_probe_keys = load_vector_index_probe_keys(
                                &connection,
                                meta.active_vector_count,
                            )?;
                            let manifest_probe_keys = manifest
                                .probes
                                .iter()
                                .map(|probe| probe.vector_key)
                                .collect::<Vec<_>>();
                            if manifest_probe_keys != expected_probe_keys {
                                tracing::warn!(
                                    path = %index_path.display(),
                                    "marking cached USearch index dirty because its legacy manifest probe set does not match the active RAG metadata layout"
                                );
                                return Ok(false);
                            }
                            if let Err(error) = validate_vector_index_probes(
                                &index,
                                &manifest,
                                meta.vector_dimensions,
                            ) {
                                tracing::warn!(
                                    error = format_args!("{:#}", error),
                                    path = %index_path.display(),
                                    "marking cached USearch index dirty because its legacy manifest probes do not match the loaded index"
                                );
                                return Ok(false);
                            }
                            backfill_missing_vector_hashes_from_index(
                                &connection,
                                &index,
                                active_vector_refs.as_slice(),
                            )?;
                            let refreshed_vector_refs = load_active_vector_refs(&connection)?;
                            match validate_loaded_vector_index(
                                &index,
                                refreshed_vector_refs.as_slice(),
                            ) {
                                Ok(()) => {
                                    let current_index_md5_hex = compute_file_md5_hex(&index_path)?;
                                    write_vector_index_manifest(
                                        &connection,
                                        database_path,
                                        &index,
                                        meta,
                                        Some(current_index_md5_hex.as_str()),
                                    )?;
                                    return Ok(true);
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        error = format_args!("{:#}", error),
                                        path = %index_path.display(),
                                        "marking cached USearch index dirty because it failed post-backfill vector validation during manifest upgrade"
                                    );
                                    return Ok(false);
                                }
                            }
                        }
                        match validate_loaded_vector_index(&index, active_vector_refs.as_slice()) {
                            Ok(()) => {
                                let current_index_md5_hex = compute_file_md5_hex(&index_path)?;
                                write_vector_index_manifest(
                                    &connection,
                                    database_path,
                                    &index,
                                    meta,
                                    Some(current_index_md5_hex.as_str()),
                                )?;
                                return Ok(true);
                            }
                            Err(error) => {
                                tracing::warn!(
                                    error = format_args!("{:#}", error),
                                    path = %index_path.display(),
                                    "marking cached USearch index dirty because it failed one-time vector coverage validation during manifest upgrade"
                                );
                                return Ok(false);
                            }
                        }
                    }
                    let expected_probe_keys =
                        load_vector_index_probe_keys(&connection, meta.active_vector_count)?;
                    let manifest_probe_keys = manifest
                        .probes
                        .iter()
                        .map(|probe| probe.vector_key)
                        .collect::<Vec<_>>();
                    if manifest_probe_keys != expected_probe_keys {
                        tracing::warn!(
                            path = %index_path.display(),
                            "marking cached USearch index dirty because its manifest probe set does not match the active RAG metadata layout"
                        );
                        return Ok(false);
                    }
                    if let Err(error) =
                        validate_vector_index_probes(&index, &manifest, meta.vector_dimensions)
                    {
                        tracing::warn!(
                            error = format_args!("{:#}", error),
                            path = %index_path.display(),
                            "marking cached USearch index dirty because its manifest probes do not match the loaded index"
                        );
                        return Ok(false);
                    }
                    let metadata = std::fs::metadata(&index_path).with_context(|| {
                        format!(
                            "failed to stat USearch index during validation: {}",
                            index_path.display()
                        )
                    })?;
                    let modified_at_ms = file_modified_at_ms(&index_path)?;
                    if metadata.len() != manifest.index_size_bytes
                        || modified_at_ms != manifest.index_modified_at_ms
                    {
                        let active_vector_refs = load_active_vector_refs(&connection)?;
                        match validate_loaded_vector_index(&index, active_vector_refs.as_slice()) {
                            Ok(()) => {
                                let current_index_md5_hex = compute_file_md5_hex(&index_path)?;
                                write_vector_index_manifest(
                                    &connection,
                                    database_path,
                                    &index,
                                    meta,
                                    Some(current_index_md5_hex.as_str()),
                                )?;
                            }
                            Err(error) => {
                                tracing::warn!(
                                    error = format_args!("{:#}", error),
                                    path = %index_path.display(),
                                    "marking cached USearch index dirty because it failed full vector coverage validation after index metadata drift"
                                );
                                return Ok(false);
                            }
                        }
                    } else {
                        let current_index_md5_hex = compute_file_md5_hex(&index_path)?;
                        if manifest.index_md5_hex.as_deref() != Some(current_index_md5_hex.as_str())
                        {
                            tracing::warn!(
                                path = %index_path.display(),
                                "marking cached USearch index dirty because its manifest digest does not match the loaded index"
                            );
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                None => {
                    let active_vector_refs = load_active_vector_refs(&connection)?;
                    match validate_loaded_vector_index(&index, active_vector_refs.as_slice()) {
                        Ok(()) => {
                            let current_index_md5_hex = compute_file_md5_hex(&index_path)?;
                            write_vector_index_manifest(
                                &connection,
                                database_path,
                                &index,
                                meta,
                                Some(current_index_md5_hex.as_str()),
                            )?;
                            Ok(true)
                        }
                        Err(error) => {
                            tracing::warn!(
                                error = format_args!("{:#}", error),
                                path = %index_path.display(),
                                "marking cached USearch index dirty because it failed one-time vector coverage validation during manifest backfill"
                            );
                            Ok(false)
                        }
                    }
                }
            }
        }
        Err(error) => {
            tracing::warn!(
                error = format_args!("{:#}", error),
                path = %index_path.display(),
                "marking cached USearch index dirty because it failed to load"
            );
            Ok(false)
        }
    }
}

fn load_active_vector_refs(connection: &Connection) -> Result<Vec<ActiveVectorRef>> {
    let mut statement = connection
        .prepare(
            "
            SELECT vector_key, vector_dimensions, vector_hash
            FROM rag_chunks
            WHERE chunk_state = 'active'
            ORDER BY vector_key
            ",
        )
        .context("failed to prepare active vector validation query")?;
    let rows = statement
        .query_map([], |row| {
            let vector_key_i64 = row.get::<_, i64>(0)?;
            let vector_dimensions_i64 = row.get::<_, i64>(1)?;
            let vector_hash_i64 = row.get::<_, Option<i64>>(2)?;
            Ok(ActiveVectorRef {
                vector_key: u64::try_from(vector_key_i64).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?,
                vector_dimensions: usize::try_from(vector_dimensions_i64).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?,
                vector_hash: vector_hash_i64.map(decode_sqlite_u64),
            })
        })
        .context("failed to query active vector refs for validation")?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect active vector refs for validation")
}

fn validate_loaded_vector_index(index: &Index, vector_refs: &[ActiveVectorRef]) -> Result<()> {
    for vector_ref in vector_refs {
        let mut vector = Vec::new();
        index
            .export::<f32>(vector_ref.vector_key, &mut vector)
            .with_context(|| {
                format!(
                    "failed to export vector key {} from USearch during validation",
                    vector_ref.vector_key
                )
            })?;
        if vector.is_empty() {
            bail!(
                "missing active RAG vector {} in USearch index",
                vector_ref.vector_key
            );
        }
        if vector.len() != vector_ref.vector_dimensions {
            bail!(
                "USearch validation dimension mismatch for key {}: expected {}, got {}",
                vector_ref.vector_key,
                vector_ref.vector_dimensions,
                vector.len()
            );
        }
        let actual_hash = stable_vector_value_hash(vector.as_slice());
        if let Some(expected_hash) = vector_ref.vector_hash {
            if actual_hash != expected_hash {
                bail!(
                    "USearch validation hash mismatch for key {}: expected {}, got {}",
                    vector_ref.vector_key,
                    expected_hash,
                    actual_hash
                );
            }
        } else {
            bail!(
                "missing persisted vector hash for active RAG vector {}; \
                 cannot trust legacy USearch contents without a blob-backed hash source",
                vector_ref.vector_key
            );
        }
    }
    Ok(())
}

fn backfill_missing_vector_hashes_from_index(
    connection: &Connection,
    index: &Index,
    vector_refs: &[ActiveVectorRef],
) -> Result<()> {
    let mut missing_hash_updates = Vec::new();
    for vector_ref in vector_refs {
        if vector_ref.vector_hash.is_some() {
            continue;
        }
        let vector =
            export_index_vector(index, vector_ref.vector_key, vector_ref.vector_dimensions)
                .with_context(|| {
                    format!(
                        "failed to backfill missing RAG vector hash from USearch for key {}",
                        vector_ref.vector_key
                    )
                })?;
        missing_hash_updates.push((
            vector_ref.vector_key,
            stable_vector_value_hash(vector.as_slice()),
        ));
    }
    backfill_vector_hashes(connection, missing_hash_updates.as_slice(), true)
}

fn rebuild_vector_index(database_path: &Path) -> Result<()> {
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
        .context("failed to prepare active vector rebuild query")?;
    let mut rows = statement
        .query([])
        .context("failed to query active vectors for rebuild")?;
    let mut active_vectors = Vec::new();
    while let Some(row) = rows
        .next()
        .context("failed to step active vectors for rebuild")?
    {
        let vector_key = u64::try_from(row.get::<_, i64>(0)?).context("vector_key is negative")?;
        let vector_blob: Option<Vec<u8>> = row.get(1)?;
        let vector_dimensions = read_vector_dimensions(row, 2)?;
        let vector = deserialize_vector(
            vector_blob
                .as_deref()
                .context("missing active vector blob for USearch rebuild")?,
            vector_dimensions,
        )?;
        active_vectors.push((vector_key, vector));
    }

    let index_path = vector_index_file_path(database_path);
    if active_vectors.is_empty() {
        match std::fs::remove_file(&index_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to remove empty rebuilt USearch index: {}",
                        index_path.display()
                    )
                });
            }
        }
        return Ok(());
    }

    match std::fs::remove_file(&index_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to remove stale USearch index before rebuild: {}",
                    index_path.display()
                )
            });
        }
    }
    save_updated_vector_index(database_path, active_vectors.as_slice(), &[])?;
    Ok(())
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
