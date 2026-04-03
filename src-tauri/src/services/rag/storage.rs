use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use arrow_array::{
    types::Float32Type, Array, FixedSizeListArray, Float32Array, Int32Array, RecordBatch,
    RecordBatchIterator, RecordBatchReader, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::{
    connect,
    index::Index,
    query::{ExecutableQuery, QueryBase, Select},
    table::Table,
    Connection as LanceConnection,
};
use rusqlite::{params, Connection};

use super::{
    config::{now_unix_ms, parse_document_kind, parse_heading_path},
    embedding::text_fingerprint,
    model::{
        PreparedRagFile, RagChunk, RagChunkState, RagIndexedFileRecord, RagIndexedFileVersion,
        RagLexicalSearchHit, ResolvedRagConfig, MAX_DELETE_FILTER_PATHS, MAX_METADATA_BATCH_PATHS,
        MAX_TEXT_FINGERPRINT_FILTERS, RAG_LEXICAL_TABLE_NAME, RAG_TABLE_NAME,
        VECTOR_INDEX_REBUILD_MIN_DIRTY_CHUNKS, VECTOR_INDEX_REBUILD_MIN_DIRTY_DELETES,
    },
};
use crate::services::rag_query;

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
pub(super) struct RagVectorStore {
    db: LanceConnection,
    table: Option<Table>,
    pub(super) created_table: bool,
    pub(super) index_dirty: bool,
    dirty_chunk_count: usize,
    dirty_delete_count: usize,
}

impl RagVectorStore {
    pub(super) async fn open(database_path: &Path) -> Result<Self> {
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

    pub(super) fn mark_index_dirty_for_chunks(&mut self, chunk_count: usize) {
        self.index_dirty = true;
        self.dirty_chunk_count = self.dirty_chunk_count.saturating_add(chunk_count);
    }

    pub(super) fn mark_index_dirty_for_delete(&mut self) {
        self.index_dirty = true;
        self.dirty_delete_count = self.dirty_delete_count.saturating_add(1);
    }

    pub(super) fn should_rebuild_index(&self) -> bool {
        self.created_table
            || self.dirty_chunk_count >= VECTOR_INDEX_REBUILD_MIN_DIRTY_CHUNKS
            || self.dirty_delete_count >= VECTOR_INDEX_REBUILD_MIN_DIRTY_DELETES
    }

    pub(super) async fn add_chunks(
        &mut self,
        chunks: &[RagChunk],
        vectors: &[Vec<f32>],
    ) -> Result<()> {
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

    pub(super) async fn delete_where(&mut self, filter: &str) -> Result<()> {
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

    pub(super) async fn update_where(
        &mut self,
        filter: &str,
        chunk_state: RagChunkState,
    ) -> Result<()> {
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

    pub(super) async fn load_chunk_vectors_for_file(
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

    pub(super) async fn load_cached_vectors_for_texts(
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
                    .map(|fingerprint| format!("'{}'", escape_sql_literal(fingerprint)))
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

    pub(super) async fn ensure_index(&mut self) -> Result<()> {
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

pub(super) fn can_skip_vector_index_build(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        let message = source.to_string();
        message.contains("Not enough rows to train PQ")
            || (message.contains("Requires 256 rows") && message.contains("available"))
    })
}

pub(super) async fn clear_index(database_path: &Path) -> Result<()> {
    rag_query::invalidate_rag_query_db_cache(database_path).await;
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

pub(super) async fn clear_metadata_store(metadata_path: &Path) -> Result<()> {
    tokio::task::spawn_blocking({
        let metadata_path = metadata_path.to_path_buf();
        move || reset_metadata_store(&metadata_path)
    })
    .await
    .context("failed to join RAG metadata cleanup task")??;
    Ok(())
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

fn build_exact_path_filter(paths: &[String]) -> String {
    let escaped = paths
        .iter()
        .map(|path| format!("'{}'", escape_sql_literal(path)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("absolute_path IN ({escaped})")
}

pub(super) async fn load_rag_table_schema(database_path: &Path) -> Result<Option<Arc<Schema>>> {
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

pub(super) fn rag_table_schema_is_compatible(schema: &Schema) -> bool {
    let fields = schema.fields();
    let expected_fields = [
        ("id", DataType::Utf8, false),
        ("source_root", DataType::Utf8, false),
        ("absolute_path", DataType::Utf8, false),
        ("version_id", DataType::Utf8, false),
        ("embedding_fingerprint", DataType::Utf8, false),
        ("document_kind", DataType::Utf8, false),
        ("chunk_state", DataType::Utf8, false),
        ("chunk_index", DataType::Int32, false),
        ("line_start", DataType::Int32, true),
        ("line_end", DataType::Int32, true),
        ("paragraph_line_start", DataType::Int32, true),
        ("page_start", DataType::Int32, true),
        ("page_end", DataType::Int32, true),
        ("heading_path", DataType::Utf8, false),
        ("anchor_label", DataType::Utf8, true),
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
    current_extractor_fingerprint: Option<&str>,
) -> bool {
    !stored_records.is_empty()
        && stored_records.values().any(|record| {
            record.embedding_fingerprint != current_embedding_fingerprint
                || current_extractor_fingerprint
                    .is_some_and(|fingerprint| record.extractor_fingerprint != fingerprint)
        })
}

pub(super) async fn metadata_store_has_active_records(metadata_path: &Path) -> Result<bool> {
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

pub(super) async fn prepare_index_storage(
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
        && rag_storage_requires_fingerprint_reset(
            &stored_records,
            current_embedding_fingerprint,
            None,
        )
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

pub(super) fn build_record_batch_reader(
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
        Field::new("document_kind", DataType::Utf8, false),
        Field::new("chunk_state", DataType::Utf8, false),
        Field::new("chunk_index", DataType::Int32, false),
        Field::new("line_start", DataType::Int32, true),
        Field::new("line_end", DataType::Int32, true),
        Field::new("paragraph_line_start", DataType::Int32, true),
        Field::new("page_start", DataType::Int32, true),
        Field::new("page_end", DataType::Int32, true),
        Field::new("heading_path", DataType::Utf8, false),
        Field::new("anchor_label", DataType::Utf8, true),
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
    let document_kinds = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.document_kind.as_str())
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
    let page_starts = Int32Array::from(
        chunks
            .iter()
            .map(|chunk| chunk.page_start)
            .collect::<Vec<_>>(),
    );
    let page_ends = Int32Array::from(
        chunks
            .iter()
            .map(|chunk| chunk.page_end)
            .collect::<Vec<_>>(),
    );
    let heading_paths = StringArray::from(
        chunks
            .iter()
            .map(|chunk| serde_json::to_string(&chunk.heading_path))
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to serialize heading path metadata")?,
    );
    let anchor_labels = StringArray::from(
        chunks
            .iter()
            .map(|chunk| chunk.anchor_label.clone())
            .collect::<Vec<_>>(),
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
            Arc::new(document_kinds),
            Arc::new(chunk_states),
            Arc::new(chunk_indexes),
            Arc::new(line_starts),
            Arc::new(line_ends),
            Arc::new(paragraph_line_starts),
            Arc::new(page_starts),
            Arc::new(page_ends),
            Arc::new(heading_paths),
            Arc::new(anchor_labels),
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

pub(super) fn replace_lexical_chunks_for_file(
    metadata_path: &Path,
    file: &PreparedRagFile,
) -> Result<()> {
    let mut connection = open_metadata_connection(metadata_path)?;
    let transaction = connection
        .transaction()
        .context("failed to open RAG lexical replace transaction")?;
    transaction
        .execute(
            &format!("DELETE FROM {RAG_LEXICAL_TABLE_NAME} WHERE absolute_path = ?1"),
            [&file.record.absolute_path],
        )
        .with_context(|| {
            format!(
                "failed to clear stale RAG lexical rows for {}",
                file.record.absolute_path
            )
        })?;

    {
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

        for chunk in &file.chunks {
            let heading_path_json = serde_json::to_string(&chunk.heading_path)
                .context("failed to serialize heading path")?;
            statement
                .execute(params![
                    &file.record.source_root,
                    &file.record.absolute_path,
                    &file.record.relative_path,
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
                        file.record.absolute_path, chunk.chunk_index
                    )
                })?;
        }
    }

    transaction
        .commit()
        .context("failed to commit RAG lexical replace transaction")?;
    Ok(())
}

pub(super) fn metadata_store_has_compatible_schema(metadata_path: &Path) -> Result<bool> {
    let connection = open_metadata_connection(metadata_path)?;
    metadata_table_schema_is_compatible(&connection)
}

fn metadata_table_schema_is_compatible(connection: &Connection) -> Result<bool> {
    let expected_columns = [
        ("absolute_path", "TEXT", true),
        ("source_root", "TEXT", true),
        ("relative_path", "TEXT", true),
        ("embedding_fingerprint", "TEXT", true),
        ("extractor_fingerprint", "TEXT", true),
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
            WHERE absolute_path = ?1 OR absolute_path LIKE ?2
            ",
        )
        .context("failed to prepare descendant RAG metadata query")?;
    let mut resolved_paths = BTreeSet::new();

    for prefix in prefixes {
        let like_pattern = format!("{prefix}/%");
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

pub(super) fn finalize_metadata_record(file: &PreparedRagFile) -> RagIndexedFileRecord {
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
            statement
                .execute(params![
                    &record.source_root,
                    &record.absolute_path,
                    &record.relative_path,
                    &record.embedding_fingerprint,
                    &record.extractor_fingerprint,
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
            .prepare("DELETE FROM rag_files WHERE absolute_path = ?1 OR absolute_path LIKE ?2")
            .context("failed to prepare RAG metadata descendant delete statement")?;
        let mut lexical_exact_statement = transaction
            .prepare(&format!(
                "DELETE FROM {RAG_LEXICAL_TABLE_NAME} WHERE absolute_path = ?1"
            ))
            .context("failed to prepare RAG lexical exact delete statement")?;
        let mut lexical_descendant_statement = transaction
            .prepare(&format!(
                "DELETE FROM {RAG_LEXICAL_TABLE_NAME} WHERE absolute_path = ?1 OR absolute_path LIKE ?2"
            ))
            .context("failed to prepare RAG lexical descendant delete statement")?;

        for path in paths {
            if delete_descendants {
                let like_pattern = format!("{path}/%");
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
