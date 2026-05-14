use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::{descendant_like_pattern, initialize_chunk_store_schema, open_chunk_store_connection};
use crate::services::rag::{
    config::{now_unix_ms, parse_document_kind, parse_heading_path},
    model::{
        PreparedRagChunk, RagChunkState, RagIndexedFileRecord, RagIndexedFileVersion,
        RagLexicalSearchHit, MAX_METADATA_BATCH_PATHS, RAG_LEXICAL_TABLE_NAME,
    },
};

pub(super) fn open_rag_sqlite_connection(sqlite_path: &Path) -> Result<Connection> {
    if let Some(parent) = sqlite_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create RAG sqlite directory: {}",
                parent.display()
            )
        })?;
    }

    let connection = Connection::open(sqlite_path).with_context(|| {
        format!(
            "failed to open RAG sqlite database: {}",
            sqlite_path.display()
        )
    })?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .context("failed to configure RAG sqlite busy timeout")?;
    initialize_chunk_store_schema(&connection)?;
    initialize_rag_sqlite_schema(&connection)?;
    Ok(connection)
}

pub(super) fn initialize_rag_sqlite_schema(connection: &Connection) -> Result<()> {
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
                heading_text,
                anchor_label,
                text,
                tokenize = \"unicode61 remove_diacritics 2 tokenchars '-_./#'\",
                content = '',
                contentless_delete = 1
            );
            ",
        )
        .context("failed to initialize RAG sqlite schema")?;
    Ok(())
}

pub(crate) fn finalize_rag_file_record_and_replace_lexical_chunks(
    sqlite_path: &Path,
    record: &RagIndexedFileRecord,
    chunk_count: usize,
    chunks: &[PreparedRagChunk],
) -> Result<()> {
    let mut connection = open_rag_sqlite_connection(sqlite_path)?;
    let transaction = connection
        .transaction()
        .context("failed to open RAG file record finalize transaction")?;
    let finalized_record = finalize_rag_file_record(record, chunk_count);
    upsert_rag_file_records_in_transaction(&transaction, &[finalized_record])?;
    replace_lexical_chunks_in_transaction(&transaction, record, chunks)?;
    transaction
        .commit()
        .context("failed to commit RAG file record finalize transaction")?;
    Ok(())
}

pub(crate) fn rag_sqlite_has_compatible_schema(sqlite_path: &Path) -> Result<bool> {
    if !sqlite_path.exists() {
        return Ok(false);
    }
    let connection = Connection::open(sqlite_path).with_context(|| {
        format!(
            "failed to open RAG sqlite database for schema inspection: {}",
            sqlite_path.display()
        )
    })?;
    Ok(rag_file_table_schema_is_compatible(&connection)?
        && rag_lexical_table_schema_is_compatible(&connection)?)
}

fn rag_file_table_schema_is_compatible(connection: &Connection) -> Result<bool> {
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
        .context("failed to inspect RAG sqlite rag_files schema")?;
    let columns = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .context("failed to query RAG sqlite rag_files schema")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect RAG sqlite rag_files schema rows")?;

    if columns.len() != expected_columns.len() {
        return Ok(false);
    }

    Ok(columns.iter().zip(expected_columns.iter()).all(
        |((name, data_type), (expected_name, expected_type))| {
            name == expected_name && data_type.eq_ignore_ascii_case(expected_type)
        },
    ))
}

fn rag_lexical_table_schema_is_compatible(connection: &Connection) -> Result<bool> {
    let sql: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [RAG_LEXICAL_TABLE_NAME],
            |row| row.get(0),
        )
        .optional()
        .context("failed to inspect RAG lexical table schema")?;

    let Some(sql) = sql else {
        return Ok(false);
    };

    let normalized = sql.to_ascii_lowercase();
    Ok(normalized.contains("create virtual table")
        && normalized.contains("using fts5")
        && normalized.contains("heading_text")
        && normalized.contains("anchor_label")
        && normalized.contains("text")
        && normalized.contains("content = ''")
        && normalized.contains("contentless_delete = 1"))
}

pub(crate) fn load_rag_file_records(
    sqlite_path: &Path,
) -> Result<HashMap<String, RagIndexedFileRecord>> {
    let connection = open_rag_sqlite_connection(sqlite_path)?;
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
        .context("failed to prepare RAG file records query")?;
    let mut rows = statement
        .query([])
        .context("failed to read RAG file records")?;
    let mut records = HashMap::new();
    while let Some(row) = rows.next().context("failed to step RAG file records")? {
        let record = read_rag_file_record(row)?;
        records.insert(record.absolute_path.clone(), record);
    }
    Ok(records)
}

pub(crate) fn load_rag_file_records_for_paths(
    sqlite_path: &Path,
    absolute_paths: &[String],
) -> Result<HashMap<String, RagIndexedFileRecord>> {
    if absolute_paths.is_empty() {
        return Ok(HashMap::new());
    }

    let connection = open_rag_sqlite_connection(sqlite_path)?;
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
            .context("failed to prepare batched RAG file records query")?;
        let params = rusqlite::params_from_iter(batch.iter());
        let mut rows = statement
            .query(params)
            .context("failed to read batched RAG file records")?;
        while let Some(row) = rows
            .next()
            .context("failed to step batched RAG file records")?
        {
            let record = read_rag_file_record(row)?;
            records.insert(record.absolute_path.clone(), record);
        }
    }

    Ok(records)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn load_rag_file_paths_for_prefixes(
    sqlite_path: &Path,
    prefixes: &[String],
) -> Result<Vec<String>> {
    if prefixes.is_empty() {
        return Ok(Vec::new());
    }

    let connection = open_rag_sqlite_connection(sqlite_path)?;
    let mut statement = connection
        .prepare(
            "
            SELECT absolute_path
            FROM rag_files
            WHERE absolute_path = ?1 OR absolute_path LIKE ?2 ESCAPE '\\'
            ",
        )
        .context("failed to prepare descendant RAG file records query")?;
    let mut resolved_paths = BTreeSet::new();

    for prefix in prefixes {
        let like_pattern = descendant_like_pattern(prefix);
        let rows = statement
            .query_map(params![prefix, like_pattern], |row| row.get::<_, String>(0))
            .with_context(|| format!("failed to query descendant RAG file records: {prefix}"))?;

        for row in rows {
            resolved_paths.insert(
                row.with_context(|| {
                    format!("failed to read descendant RAG file record: {prefix}")
                })?,
            );
        }
    }

    Ok(resolved_paths.into_iter().collect())
}

fn read_rag_file_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RagIndexedFileRecord> {
    Ok(RagIndexedFileRecord {
        source_root: row.get(0)?,
        absolute_path: row.get(1)?,
        relative_path: row.get(2)?,
        embedding_fingerprint: row.get(3)?,
        extractor_fingerprint: row.get(4)?,
        active: read_rag_file_version(
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
            row.get(10)?,
        )?,
        pending: read_rag_file_version(
            row.get(11)?,
            row.get(12)?,
            row.get(13)?,
            row.get(14)?,
            row.get(15)?,
            row.get(16)?,
        )?,
    })
}

fn missing_rag_file_value<T>(
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
                format!("missing {field_name} for indexed rag file version"),
            )),
        )
    })
}

fn read_rag_file_version(
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
        content_md5: missing_rag_file_value(
            "content_md5",
            rusqlite::types::Type::Text,
            content_md5,
        )?,
        modified_at_ms,
        size_bytes: missing_rag_file_value(
            "size_bytes",
            rusqlite::types::Type::Integer,
            size_bytes,
        )?,
        chunk_count: missing_rag_file_value(
            "chunk_count",
            rusqlite::types::Type::Integer,
            chunk_count,
        )?,
        indexed_at_ms: indexed_at_ms.unwrap_or_default(),
    }))
}

pub(super) fn finalize_rag_file_record(
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

pub(crate) fn rag_sqlite_has_pending_rows(sqlite_path: &Path) -> Result<bool> {
    let connection = open_rag_sqlite_connection(sqlite_path)?;
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM rag_files WHERE pending_version_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .context("failed to count pending RAG file records")?;
    Ok(count > 0)
}

pub(super) fn reset_sqlite_store(sqlite_path: &Path) -> Result<()> {
    for path in sqlite_file_paths(sqlite_path) {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to remove RAG sqlite file: {}", path.display())
                });
            }
        }
    }

    Ok(())
}

fn sqlite_file_paths(sqlite_path: &Path) -> [PathBuf; 3] {
    let base = sqlite_path.to_path_buf();
    let wal = PathBuf::from(format!("{}-wal", sqlite_path.to_string_lossy()));
    let shm = PathBuf::from(format!("{}-shm", sqlite_path.to_string_lossy()));
    [base, wal, shm]
}

struct RagFileVersionFields<'a> {
    version_id: Option<&'a str>,
    content_md5: Option<&'a str>,
    modified_at_ms: Option<i64>,
    size_bytes: Option<i64>,
    chunk_count: Option<i64>,
    indexed_at_ms: Option<i64>,
}

fn rag_file_version_fields(version: Option<&RagIndexedFileVersion>) -> RagFileVersionFields<'_> {
    RagFileVersionFields {
        version_id: version.map(|value| value.version_id.as_str()),
        content_md5: version.map(|value| value.content_md5.as_str()),
        modified_at_ms: version.and_then(|value| value.modified_at_ms),
        size_bytes: version.map(|value| value.size_bytes),
        chunk_count: version.map(|value| value.chunk_count),
        indexed_at_ms: version.map(|value| value.indexed_at_ms),
    }
}

pub(crate) fn upsert_rag_file_records(
    sqlite_path: &Path,
    records: &[RagIndexedFileRecord],
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }

    let mut connection = open_rag_sqlite_connection(sqlite_path)?;
    let transaction = connection
        .transaction()
        .context("failed to open RAG file record transaction")?;
    upsert_rag_file_records_in_transaction(&transaction, records)?;
    transaction
        .commit()
        .context("failed to commit RAG file record transaction")?;
    Ok(())
}

fn upsert_rag_file_records_in_transaction(
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
            .context("failed to prepare RAG file records upsert statement")?;

        for record in records {
            let active = rag_file_version_fields(record.active.as_ref());
            let pending = rag_file_version_fields(record.pending.as_ref());
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
                    format!("failed to upsert RAG file record: {}", record.absolute_path)
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
    delete_lexical_rows_for_filter_in_transaction(
        transaction,
        "absolute_path = ?1",
        params![&record.absolute_path],
        || {
            format!(
                "failed to clear stale RAG lexical rows for {}",
                record.absolute_path
            )
        },
    )?;

    if chunks.is_empty() {
        return Ok(());
    }

    let version_id = record
        .pending
        .as_ref()
        .or(record.active.as_ref())
        .map(|version| version.version_id.as_str())
        .context("missing indexed RAG file version for lexical row replacement")?;
    let lexical_rowids = load_chunk_rowids_for_version_in_transaction(
        transaction,
        &record.absolute_path,
        version_id,
        RagChunkState::Active,
    )?;
    let chunk_by_index = chunks
        .iter()
        .map(|chunk| (chunk.chunk_index, chunk))
        .collect::<HashMap<_, _>>();

    let mut statement = transaction
        .prepare(&format!(
            "
            INSERT INTO {RAG_LEXICAL_TABLE_NAME} (
                rowid,
                heading_text,
                anchor_label,
                text
            ) VALUES (?1, ?2, ?3, ?4)
            "
        ))
        .context("failed to prepare RAG lexical chunk insert statement")?;

    for (vector_key, chunk_index) in lexical_rowids {
        let chunk = chunk_by_index.get(&chunk_index).with_context(|| {
            format!(
                "missing prepared chunk for lexical row {}#{}",
                record.absolute_path, chunk_index
            )
        })?;
        statement
            .execute(params![
                vector_key,
                chunk.heading_path.join(" "),
                chunk.anchor_label.as_deref(),
                &chunk.text,
            ])
            .with_context(|| {
                format!(
                    "failed to insert RAG lexical chunk {}#{}",
                    record.absolute_path, chunk_index
                )
            })?;
    }

    Ok(())
}

fn load_chunk_rowids_for_version_in_transaction(
    transaction: &Transaction<'_>,
    absolute_path: &str,
    version_id: &str,
    chunk_state: RagChunkState,
) -> Result<Vec<(i64, i32)>> {
    let mut statement = transaction
        .prepare(
            "
            SELECT vector_key, chunk_index
            FROM rag_chunks
            WHERE absolute_path = ?1
              AND version_id = ?2
              AND chunk_state = ?3
            ORDER BY chunk_index
            ",
        )
        .context("failed to prepare RAG lexical rowid query")?;
    let rows = statement
        .query_map(
            params![absolute_path, version_id, chunk_state.as_str()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i32>(1)?)),
        )
        .with_context(|| format!("failed to query lexical rowids for {absolute_path}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to collect lexical rowids")
}

fn delete_lexical_rows_for_filter_in_transaction<P>(
    transaction: &Transaction<'_>,
    chunk_filter: &str,
    params: P,
    error_context: impl FnOnce() -> String,
) -> Result<()>
where
    P: rusqlite::Params,
{
    transaction
        .execute(
            &format!(
                "DELETE FROM {RAG_LEXICAL_TABLE_NAME} WHERE rowid IN (SELECT vector_key FROM rag_chunks WHERE {chunk_filter})"
            ),
            params,
        )
        .with_context(error_context)?;
    Ok(())
}

pub(crate) fn delete_rag_file_records_for_paths(
    sqlite_path: &Path,
    paths: &[String],
    delete_descendants: bool,
) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }

    let mut connection = open_rag_sqlite_connection(sqlite_path)?;
    let transaction = connection
        .transaction()
        .context("failed to open RAG file records delete transaction")?;
    {
        let mut exact_statement = transaction
            .prepare("DELETE FROM rag_files WHERE absolute_path = ?1")
            .context("failed to prepare RAG file records exact delete statement")?;
        let mut descendant_statement = transaction
            .prepare(
                "DELETE FROM rag_files WHERE absolute_path = ?1 OR absolute_path LIKE ?2 ESCAPE '\\'",
            )
            .context("failed to prepare RAG file records descendant delete statement")?;
        for path in paths {
            if delete_descendants {
                let like_pattern = descendant_like_pattern(path);
                delete_lexical_rows_for_filter_in_transaction(
                    &transaction,
                    "absolute_path = ?1 OR absolute_path LIKE ?2 ESCAPE '\\'",
                    params![path, like_pattern],
                    || format!("failed to delete RAG lexical rows: {path}"),
                )?;
                descendant_statement
                    .execute(params![path, like_pattern])
                    .with_context(|| format!("failed to delete RAG file records: {path}"))?;
            } else {
                delete_lexical_rows_for_filter_in_transaction(
                    &transaction,
                    "absolute_path = ?1",
                    [path],
                    || format!("failed to delete RAG lexical row: {path}"),
                )?;
                exact_statement
                    .execute([path])
                    .with_context(|| format!("failed to delete RAG file record: {path}"))?;
            }
        }
    }
    transaction
        .commit()
        .context("failed to commit RAG file records delete transaction")?;
    Ok(())
}

pub(crate) fn refresh_projection_for_rag_file_records(
    database_path: &Path,
    _sqlite_path: &Path,
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
                        "failed to refresh RAG chunk projection fields: {}",
                        record.absolute_path
                    )
                })?;
        }
    }
    chunk_transaction
        .commit()
        .context("failed to commit RAG chunk projection refresh transaction")?;

    Ok(())
}

pub(crate) fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

pub(crate) fn search_lexical_chunks(
    sqlite_path: &Path,
    match_query: &str,
    top_k: usize,
) -> Result<Vec<RagLexicalSearchHit>> {
    if match_query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let connection = open_rag_sqlite_connection(sqlite_path)?;
    let mut statement = connection
        .prepare(&format!(
            "
            SELECT
                rag_chunks.source_root,
                rag_chunks.absolute_path,
                rag_chunks.document_kind,
                rag_chunks.chunk_index,
                rag_chunks.line_start,
                rag_chunks.line_end,
                rag_chunks.paragraph_line_start,
                rag_chunks.page_start,
                rag_chunks.page_end,
                rag_chunks.heading_path_json,
                rag_chunks.anchor_label,
                rag_chunks.text,
                bm25({RAG_LEXICAL_TABLE_NAME}, 4.0, 2.5, 1.5, 1.0) AS bm25_rank
            FROM {RAG_LEXICAL_TABLE_NAME}
            JOIN rag_chunks ON rag_chunks.vector_key = {RAG_LEXICAL_TABLE_NAME}.rowid
            WHERE {RAG_LEXICAL_TABLE_NAME} MATCH ?1
              AND rag_chunks.chunk_state = 'active'
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
        let document_kind_raw: String = row.get(2)?;
        let heading_path_raw: String = row.get(9)?;
        hits.push(RagLexicalSearchHit {
            source_root: row.get(0)?,
            absolute_path: row.get(1)?,
            document_kind: parse_document_kind(&document_kind_raw)?,
            chunk_index: row.get(3)?,
            line_start: row.get(4)?,
            line_end: row.get(5)?,
            paragraph_line_start: row.get(6)?,
            page_start: row.get(7)?,
            page_end: row.get(8)?,
            heading_path: parse_heading_path(&heading_path_raw)?,
            anchor_label: row.get(10)?,
            text: row.get(11)?,
            bm25_rank: row.get::<_, f64>(12)? as f32,
        });
    }

    Ok(hits)
}
