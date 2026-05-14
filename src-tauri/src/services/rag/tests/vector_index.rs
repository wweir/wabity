use super::*;

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

    assert_eq!(planner.current_size, EMBEDDING_BATCH_SIZE_DEFAULT + 1);
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

    assert_eq!(planner.next_batch_end(&inputs, 0), 1);
    assert_eq!(planner.next_batch_end(&inputs, 1), 2);
    assert_eq!(planner.next_batch_end(&inputs, 2), 3);
    assert_eq!(planner.next_batch_end(&inputs, 3), 4);
}

#[tokio::test]
async fn vector_index_policy_rebuilds_any_dirty_incremental_batch() {
    let root = temp_test_root("vector-index-policy-small");
    std::fs::create_dir_all(&root).expect("create vector policy root");
    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");

    vector_store.mark_index_dirty_for_chunks(1);
    vector_store.mark_index_dirty_for_delete();

    assert!(vector_store.index_dirty);
    assert!(vector_store.should_rebuild_index());

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
async fn vector_store_open_marks_corrupted_index_as_dirty() {
    let root = temp_test_root("vector-index-policy-corrupt-index");
    std::fs::create_dir_all(&root).expect("create vector policy root");
    let chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(&[chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("insert current rag chunk");
    vector_store
        .ensure_index()
        .await
        .expect("build vector index");

    std::fs::write(vector_index_file_path(&root), b"not-a-valid-usearch-index")
        .expect("corrupt vector index");

    let reopened = RagVectorStore::open(&root)
        .await
        .expect("reopen vector store");

    assert!(reopened.index_dirty);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn vector_store_open_backfills_missing_manifest_for_complete_index() {
    let root = temp_test_root("vector-index-policy-backfill-manifest");
    std::fs::create_dir_all(&root).expect("create vector policy root");
    let first_chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let mut second_chunk = test_chunk("/tmp/docs/a.md", "next chunk");
    second_chunk.id.push_str("-second");
    second_chunk.chunk_index = 1;
    second_chunk.chunk_reuse_key.push_str("-second");
    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(
            &[first_chunk, second_chunk],
            &[vec![1.0_f32, 2.0_f32], vec![3.0_f32, 4.0_f32]],
        )
        .await
        .expect("insert current rag chunks");

    std::fs::remove_file(vector_index_manifest_path(&root)).expect("remove old manifest");

    let reopened = RagVectorStore::open(&root)
        .await
        .expect("reopen vector store");

    assert!(!reopened.index_dirty);
    assert_eq!(
        load_vector_index_manifest_for_test(&root).active_vector_count,
        2
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn vector_store_open_upgrades_legacy_manifest_for_complete_index() {
    let root = temp_test_root("vector-index-policy-upgrade-legacy-manifest");
    std::fs::create_dir_all(&root).expect("create vector policy root");
    let first_chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let mut second_chunk = test_chunk("/tmp/docs/a.md", "next chunk");
    second_chunk.id.push_str("-second");
    second_chunk.chunk_index = 1;
    second_chunk.chunk_reuse_key.push_str("-second");
    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(
            &[first_chunk, second_chunk],
            &[vec![1.0_f32, 2.0_f32], vec![3.0_f32, 4.0_f32]],
        )
        .await
        .expect("insert current rag chunks");

    let mut manifest = load_vector_index_manifest_for_test(&root);
    manifest.version = 1;
    manifest.probes.clear();
    manifest.index_md5_hex = Some("legacy-md5".to_string());
    write_vector_index_manifest_for_test(&root, &manifest);

    let reopened = RagVectorStore::open(&root)
        .await
        .expect("reopen vector store");

    assert!(!reopened.index_dirty);
    let upgraded_manifest = load_vector_index_manifest_for_test(&root);
    assert_eq!(upgraded_manifest.version, manifest_version_for_test());
    assert!(!upgraded_manifest.probes.is_empty());
    assert_eq!(
        upgraded_manifest.index_md5_hex,
        Some(index_md5_hex_for_test(&vector_index_file_path(&root))),
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn vector_store_open_marks_legacy_rows_without_hashes_and_blobs_as_dirty() {
    let root = temp_test_root("vector-index-policy-legacy-missing-hash-source");
    std::fs::create_dir_all(&root).expect("create vector policy root");

    let chunk = test_chunk("/tmp/docs/a.md", "legacy current chunk");
    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(std::slice::from_ref(&chunk), &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("insert current rag chunk");

    let chunk_db_path = root.join(RAG_CHUNK_DB_FILE_NAME);
    let connection = rusqlite::Connection::open(&chunk_db_path).expect("open current chunk store");
    let vector_key = connection
        .query_row(
            "SELECT vector_key FROM rag_chunks WHERE id = ?1",
            [&chunk.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("load active vector key");
    drop(connection);
    std::fs::remove_file(&chunk_db_path).expect("remove current chunk database");

    let connection = rusqlite::Connection::open(&chunk_db_path).expect("open legacy chunk store");
    connection
        .execute_batch(
            "
            CREATE TABLE rag_chunks (
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
                vector_dimensions INTEGER NOT NULL
            );
            ",
        )
        .expect("create legacy chunk schema without vector_hash");
    connection
        .execute(
            "
            INSERT INTO rag_chunks (
                vector_key,
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
                ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
            )
            ",
            params![
                vector_key,
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
                serde_json::to_string(&chunk.heading_path).expect("serialize heading path"),
                chunk.anchor_label.as_deref(),
                &chunk.chunk_reuse_key,
                &chunk.text_fingerprint,
                &chunk.text,
                Option::<Vec<u8>>::None,
                2_i64,
            ],
        )
        .expect("insert legacy active chunk without blob");
    std::fs::remove_file(vector_index_manifest_path(&root)).expect("remove current manifest");

    let reopened = RagVectorStore::open(&root)
        .await
        .expect("reopen vector store");

    assert!(reopened.index_dirty);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn vector_store_open_refreshes_stale_manifest_metadata_without_marking_dirty() {
    let root = temp_test_root("vector-index-policy-refresh-stale-metadata");
    std::fs::create_dir_all(&root).expect("create vector policy root");
    let first_chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let mut second_chunk = test_chunk("/tmp/docs/a.md", "next chunk");
    second_chunk.id.push_str("-second");
    second_chunk.chunk_index = 1;
    second_chunk.chunk_reuse_key.push_str("-second");
    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(
            &[first_chunk, second_chunk],
            &[vec![1.0_f32, 2.0_f32], vec![3.0_f32, 4.0_f32]],
        )
        .await
        .expect("insert current rag chunks");

    let mut manifest = load_vector_index_manifest_for_test(&root);
    manifest.index_size_bytes = 0;
    manifest.index_modified_at_ms = 0;
    write_vector_index_manifest_for_test(&root, &manifest);

    let reopened = RagVectorStore::open(&root)
        .await
        .expect("reopen vector store");

    assert!(!reopened.index_dirty);
    let refreshed_manifest = load_vector_index_manifest_for_test(&root);
    assert!(refreshed_manifest.index_size_bytes > 0);
    assert!(refreshed_manifest.index_modified_at_ms > 0);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn vector_store_open_marks_non_probe_mismatch_as_dirty_when_only_digest_detects_it() {
    let root = temp_test_root("vector-index-policy-non-probe-digest-only");
    std::fs::create_dir_all(&root).expect("create vector policy root");
    let mut chunks = Vec::new();
    let mut vectors = Vec::new();
    for index in 0..5 {
        let mut chunk = test_chunk("/tmp/docs/a.md", &format!("chunk {index}"));
        if index > 0 {
            chunk.id.push_str(&format!("-{index}"));
            chunk.chunk_index = index;
            chunk.chunk_reuse_key.push_str(&format!("-{index}"));
        }
        chunks.push(chunk);
        vectors.push(vec![index as f32 + 1.0_f32, index as f32 + 2.0_f32]);
    }

    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(&chunks, &vectors)
        .await
        .expect("insert current rag chunks");

    let mut manifest = load_vector_index_manifest_for_test(&root);
    let original_digest = manifest.index_md5_hex.clone();
    assert!(original_digest.is_some());
    let probe_keys = manifest
        .probes
        .iter()
        .map(|probe| probe.vector_key)
        .collect::<std::collections::HashSet<_>>();
    let connection = open_vector_chunk_connection(&root).expect("open chunk db");
    let mut statement = connection
        .prepare(
            "
            SELECT vector_key
            FROM rag_chunks
            WHERE chunk_state = 'active'
            ORDER BY vector_key
            ",
        )
        .expect("prepare active vector query");
    let non_probe_key = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("query active vector keys")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect active vector keys")
        .into_iter()
        .map(|value| u64::try_from(value).expect("vector key should be positive"))
        .find(|vector_key| !probe_keys.contains(vector_key))
        .expect("expected at least one non-probe vector key");
    rewrite_vector_in_index_for_test(&root, non_probe_key, &[99.0_f32, 100.0_f32]);

    let index_path = vector_index_file_path(&root);
    manifest.index_size_bytes = std::fs::metadata(&index_path)
        .expect("stat rewritten vector index")
        .len();
    manifest.index_modified_at_ms = file_modified_at_ms_for_test(&index_path);
    write_vector_index_manifest_for_test(&root, &manifest);

    let reopened = RagVectorStore::open(&root)
        .await
        .expect("reopen vector store");

    assert!(reopened.index_dirty);

    let refreshed_manifest = load_vector_index_manifest_for_test(&root);
    assert_eq!(refreshed_manifest.index_md5_hex, original_digest);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn vector_store_open_marks_incomplete_index_as_dirty() {
    let root = temp_test_root("vector-index-policy-incomplete-index");
    std::fs::create_dir_all(&root).expect("create vector policy root");
    let first_chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let mut second_chunk = test_chunk("/tmp/docs/a.md", "next chunk");
    second_chunk.id.push_str("-second");
    second_chunk.chunk_index = 1;
    second_chunk.chunk_reuse_key.push_str("-second");
    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(
            &[first_chunk, second_chunk],
            &[vec![1.0_f32, 2.0_f32], vec![3.0_f32, 4.0_f32]],
        )
        .await
        .expect("insert current rag chunks");

    let connection = open_vector_chunk_connection(&root).expect("open chunk db");
    let removed_vector_key = connection
        .query_row(
            "SELECT MIN(vector_key) FROM rag_chunks WHERE chunk_state = 'active'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("load active vector key");
    remove_vector_from_index_for_test(
        &root,
        u64::try_from(removed_vector_key).expect("vector key should be positive"),
    );

    let reopened = RagVectorStore::open(&root)
        .await
        .expect("reopen vector store");

    assert!(reopened.index_dirty);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn vector_store_open_marks_probe_mismatch_as_dirty_even_when_manifest_metadata_matches() {
    let root = temp_test_root("vector-index-policy-probe-mismatch");
    std::fs::create_dir_all(&root).expect("create vector policy root");
    let first_chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let mut second_chunk = test_chunk("/tmp/docs/a.md", "next chunk");
    second_chunk.id.push_str("-second");
    second_chunk.chunk_index = 1;
    second_chunk.chunk_reuse_key.push_str("-second");
    let mut third_chunk = test_chunk("/tmp/docs/a.md", "third chunk");
    third_chunk.id.push_str("-third");
    third_chunk.chunk_index = 2;
    third_chunk.chunk_reuse_key.push_str("-third");
    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(
            &[first_chunk, second_chunk, third_chunk],
            &[
                vec![1.0_f32, 2.0_f32],
                vec![3.0_f32, 4.0_f32],
                vec![5.0_f32, 6.0_f32],
            ],
        )
        .await
        .expect("insert current rag chunks");

    let mut manifest = load_vector_index_manifest_for_test(&root);
    let probe_key = manifest
        .probes
        .first()
        .expect("manifest should include at least one probe")
        .vector_key;
    rewrite_vector_in_index_for_test(&root, probe_key, &[9.0_f32, 9.0_f32]);

    let index_path = vector_index_file_path(&root);
    manifest.index_size_bytes = std::fs::metadata(&index_path)
        .expect("stat rewritten vector index")
        .len();
    manifest.index_modified_at_ms = file_modified_at_ms_for_test(&index_path);
    write_vector_index_manifest_for_test(&root, &manifest);

    let reopened = RagVectorStore::open(&root)
        .await
        .expect("reopen vector store");

    assert!(reopened.index_dirty);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn vector_store_open_marks_non_probe_mismatch_as_dirty_after_index_metadata_drift() {
    let root = temp_test_root("vector-index-policy-non-probe-metadata-drift");
    std::fs::create_dir_all(&root).expect("create vector policy root");
    let mut chunks = Vec::new();
    let mut vectors = Vec::new();
    for index in 0..5 {
        let mut chunk = test_chunk("/tmp/docs/a.md", &format!("chunk {index}"));
        if index > 0 {
            chunk.id.push_str(&format!("-{index}"));
            chunk.chunk_index = index;
            chunk.chunk_reuse_key.push_str(&format!("-{index}"));
        }
        chunks.push(chunk);
        vectors.push(vec![index as f32 + 1.0_f32, index as f32 + 2.0_f32]);
    }

    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(&chunks, &vectors)
        .await
        .expect("insert current rag chunks");

    let mut manifest = load_vector_index_manifest_for_test(&root);
    let probe_keys = manifest
        .probes
        .iter()
        .map(|probe| probe.vector_key)
        .collect::<std::collections::HashSet<_>>();
    let connection = open_vector_chunk_connection(&root).expect("open chunk db");
    let mut statement = connection
        .prepare(
            "
            SELECT vector_key
            FROM rag_chunks
            WHERE chunk_state = 'active'
            ORDER BY vector_key
            ",
        )
        .expect("prepare active vector query");
    let non_probe_key = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("query active vector keys")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect active vector keys")
        .into_iter()
        .map(|value| u64::try_from(value).expect("vector key should be positive"))
        .find(|vector_key| !probe_keys.contains(vector_key))
        .expect("expected at least one non-probe vector key");
    rewrite_vector_in_index_for_test(&root, non_probe_key, &[99.0_f32, 100.0_f32]);

    manifest.index_modified_at_ms = 0;
    write_vector_index_manifest_for_test(&root, &manifest);

    let reopened = RagVectorStore::open(&root)
        .await
        .expect("reopen vector store");

    assert!(reopened.index_dirty);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn vector_store_open_marks_dirty_marker_as_dirty() {
    let root = temp_test_root("vector-index-policy-dirty-marker");
    std::fs::create_dir_all(&root).expect("create vector policy root");
    let chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(&[chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("insert current rag chunk");
    vector_store
        .ensure_index()
        .await
        .expect("build vector index");

    vector_store.mark_index_dirty_for_chunks(1);

    let reopened = RagVectorStore::open(&root)
        .await
        .expect("reopen vector store");

    assert!(reopened.index_dirty);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn prepare_index_storage_clears_incompatible_sqlite_schema_and_vectors() {
    let root = temp_test_root("incompatible-sqlite-reset");
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(&[chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("create current rag vectors");

    let connection = Connection::open(&sqlite_path).expect("open incompatible sqlite database");
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
        .expect("create incompatible sqlite rows");

    prepare_index_storage(
        &database_path,
        &sqlite_path,
        &test_resolved_config(&root),
        true,
    )
    .await
    .expect("prepare index storage should reset incompatible sqlite");

    assert!(RagVectorStore::open(&database_path)
        .await
        .expect("reopen vector store")
        .load_chunk_vectors_for_file(
            "/tmp/docs/a.md",
            RagChunkState::Active,
            &test_embedding_fingerprint()
        )
        .await
        .expect("load surviving vectors after sqlite reset")
        .is_empty());
    assert!(
        rag_sqlite_has_compatible_schema(&sqlite_path).expect("inspect recreated sqlite schema")
    );
    assert!(load_rag_file_records(&sqlite_path)
        .expect("load rag file rows")
        .is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn prepare_index_storage_migrates_legacy_chunk_store_without_resetting_indexed_rows() {
    let root = temp_test_root("legacy-chunk-store-migration");
    let source_root = root.join("docs");
    std::fs::create_dir_all(&source_root).expect("create rag source root");
    let file_path = source_root.join("indexed.md");
    let file_text = "# Indexed\n\nlegacy content\n";
    std::fs::write(&file_path, file_text).expect("write indexed rag source file");

    let resolved = test_resolved_config(&source_root);
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    std::fs::create_dir_all(&database_path).expect("create rag index directory");

    let absolute_path = normalize_path_string(&file_path);
    let mut chunk = test_chunk(&absolute_path, "legacy indexed chunk");
    chunk.source_root = normalize_path_string(&source_root);
    chunk.absolute_path = absolute_path.clone();
    chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    let vector = vec![1.0_f32, 2.0_f32];

    let chunk_connection = rusqlite::Connection::open(database_path.join(RAG_CHUNK_DB_FILE_NAME))
        .expect("open legacy chunk store");
    chunk_connection
        .execute_batch(
            "
            CREATE TABLE rag_chunks (
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
                vector_dimensions INTEGER NOT NULL
            );
            ",
        )
        .expect("create legacy chunk schema without vector_hash");
    chunk_connection
        .execute(
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
                serde_json::to_string(&chunk.heading_path).expect("serialize heading path"),
                chunk.anchor_label.as_deref(),
                &chunk.chunk_reuse_key,
                &chunk.text_fingerprint,
                &chunk.text,
                test_serialize_vector(&vector),
                i64::try_from(vector.len()).expect("vector dimensions fit i64"),
            ],
        )
        .expect("insert legacy active chunk row");

    let file_metadata = std::fs::metadata(&file_path).expect("read indexed file metadata");
    upsert_rag_file_records(
        &sqlite_path,
        &[test_indexed_record(
            &source_root,
            &file_path,
            &resolved.embedding_fingerprint,
            Some(test_active_version(
                &format!("{:x}", md5::compute(file_text.as_bytes())),
                file_metadata
                    .modified()
                    .ok()
                    .and_then(system_time_to_unix_ms),
                i64::try_from(file_metadata.len()).expect("file size fits i64"),
                1,
                1,
            )),
            None,
        )],
    )
    .expect("seed rag file row");

    prepare_index_storage(&database_path, &sqlite_path, &resolved, true)
        .await
        .expect("prepare index storage should migrate legacy chunk store");

    assert!(
        chunk_store_has_compatible_schema(&database_path).expect("inspect migrated chunk schema")
    );
    let reopened = RagVectorStore::open(&database_path)
        .await
        .expect("reopen migrated vector store");
    let surviving_vectors = reopened
        .load_chunk_vectors_for_file(
            &absolute_path,
            RagChunkState::Active,
            &resolved.embedding_fingerprint,
        )
        .await
        .expect("load migrated active vectors");
    assert_eq!(surviving_vectors.get(&chunk.chunk_reuse_key), Some(&vector));
    assert_eq!(
        load_rag_file_records(&sqlite_path)
            .expect("load migrated rag file rows")
            .len(),
        1
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn prepare_index_storage_preserves_blobless_legacy_chunk_store_when_probes_match() {
    let root = temp_test_root("legacy-chunk-store-probe-backed-migration");
    let source_root = root.join("docs");
    std::fs::create_dir_all(&source_root).expect("create rag source root");
    let file_path = source_root.join("indexed.md");
    let file_text = "# Indexed\n\nlegacy content\n";
    std::fs::write(&file_path, file_text).expect("write indexed rag source file");

    let resolved = test_resolved_config(&source_root);
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    let absolute_path = normalize_path_string(&file_path);
    let file_metadata = std::fs::metadata(&file_path).expect("read indexed file metadata");

    let mut chunk = test_chunk(&absolute_path, "legacy indexed chunk");
    chunk.source_root = normalize_path_string(&source_root);
    chunk.absolute_path = absolute_path.clone();
    chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    let vector = vec![1.0_f32, 2.0_f32];

    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open seeded vector store");
    vector_store
        .add_chunks(std::slice::from_ref(&chunk), std::slice::from_ref(&vector))
        .await
        .expect("seed current active vector");

    let mut manifest = load_vector_index_manifest_for_test(&database_path);
    assert!(!manifest.probes.is_empty());
    manifest.index_md5_hex = None;
    write_vector_index_manifest_for_test(&database_path, &manifest);

    let chunk_db_path = database_path.join(RAG_CHUNK_DB_FILE_NAME);
    let connection = rusqlite::Connection::open(&chunk_db_path).expect("open current chunk store");
    let vector_key = connection
        .query_row(
            "SELECT vector_key FROM rag_chunks WHERE id = ?1",
            [&chunk.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("load active vector key");
    drop(connection);
    std::fs::remove_file(&chunk_db_path).expect("remove current chunk database");

    let chunk_connection =
        rusqlite::Connection::open(&chunk_db_path).expect("open legacy chunk store");
    chunk_connection
        .execute_batch(
            "
            CREATE TABLE rag_chunks (
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
                vector_dimensions INTEGER NOT NULL
            );
            ",
        )
        .expect("create legacy chunk schema without vector_hash");
    chunk_connection
        .execute(
            "
            INSERT INTO rag_chunks (
                vector_key,
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
                ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
            )
            ",
            params![
                vector_key,
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
                serde_json::to_string(&chunk.heading_path).expect("serialize heading path"),
                chunk.anchor_label.as_deref(),
                &chunk.chunk_reuse_key,
                &chunk.text_fingerprint,
                &chunk.text,
                Option::<Vec<u8>>::None,
                i64::try_from(vector.len()).expect("vector dimensions fit i64"),
            ],
        )
        .expect("insert legacy active chunk row without blob");

    upsert_rag_file_records(
        &sqlite_path,
        &[test_indexed_record(
            &source_root,
            &file_path,
            &resolved.embedding_fingerprint,
            Some(test_active_version(
                &format!("{:x}", md5::compute(file_text.as_bytes())),
                file_metadata
                    .modified()
                    .ok()
                    .and_then(system_time_to_unix_ms),
                i64::try_from(file_metadata.len()).expect("file size fits i64"),
                1,
                1,
            )),
            None,
        )],
    )
    .expect("seed rag file row");

    prepare_index_storage(&database_path, &sqlite_path, &resolved, true)
        .await
        .expect("prepare index storage should preserve probe-backed legacy chunk store");

    let reopened = RagVectorStore::open(&database_path)
        .await
        .expect("reopen migrated vector store");
    let surviving_vectors = reopened
        .load_chunk_vectors_for_file(
            &absolute_path,
            RagChunkState::Active,
            &resolved.embedding_fingerprint,
        )
        .await
        .expect("load migrated active vectors");
    assert_eq!(surviving_vectors.get(&chunk.chunk_reuse_key), Some(&vector));
    assert_eq!(
        load_vector_index_manifest_for_test(&database_path).index_md5_hex,
        Some(index_md5_hex_for_test(&vector_index_file_path(
            &database_path
        ))),
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn prepare_index_storage_clears_orphaned_vector_index_artifacts() {
    let root = temp_test_root("orphaned-vector-index-artifacts");
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    std::fs::create_dir_all(&database_path).expect("create rag index directory");
    std::fs::write(vector_index_file_path(&database_path), b"stale-index")
        .expect("write orphaned vector index");
    std::fs::write(database_path.join("rag-chunks.dirty"), b"dirty")
        .expect("write orphaned dirty marker");
    std::fs::write(
        vector_index_manifest_path(&database_path),
        br#"{"version":1}"#,
    )
    .expect("write orphaned manifest");

    prepare_index_storage(
        &database_path,
        &sqlite_path,
        &test_resolved_config(&root),
        true,
    )
    .await
    .expect("prepare index storage should clear orphaned vector artifacts");

    assert!(!vector_index_file_path(&database_path).exists());
    assert!(!database_path.join("rag-chunks.dirty").exists());
    assert!(!vector_index_manifest_path(&database_path).exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn prepare_index_storage_clears_orphaned_manifest_without_chunk_store() {
    let root = temp_test_root("orphaned-vector-index-manifest-only");
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    std::fs::create_dir_all(&database_path).expect("create rag index directory");
    std::fs::write(
        vector_index_manifest_path(&database_path),
        br#"{"version":1}"#,
    )
    .expect("write orphaned manifest");

    prepare_index_storage(
        &database_path,
        &sqlite_path,
        &test_resolved_config(&root),
        true,
    )
    .await
    .expect("prepare index storage should clear orphaned manifest");

    assert!(!vector_index_manifest_path(&database_path).exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn prepare_index_storage_clears_active_rag_file_records_when_chunk_store_has_no_active_rows()
{
    let root = temp_test_root("active-rag-file-records-without-active-chunks");
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    std::fs::create_dir_all(&root).expect("create rag temp root");
    std::fs::create_dir_all(&database_path).expect("create rag index dir");

    let chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(&[chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("insert rag chunk");
    delete_vectors_for_exact_paths(&mut vector_store, &["/tmp/docs/a.md".to_string()])
        .await
        .expect("delete active rag chunk");

    let indexed_record = RagIndexedFileRecord {
        source_root: "/tmp/docs".to_string(),
        absolute_path: "/tmp/docs/a.md".to_string(),
        relative_path: "a.md".to_string(),
        embedding_fingerprint: test_embedding_fingerprint(),
        extractor_fingerprint: test_extractor_fingerprint(),
        active: Some(test_active_version("md5-a", Some(1), 10, 1, 42)),
        pending: None,
    };
    upsert_rag_file_records(&sqlite_path, &[indexed_record]).expect("write rag file row");
    std::fs::write(vector_index_file_path(&database_path), b"stale-index")
        .expect("write stale vector index");
    std::fs::write(database_path.join("rag-chunks.dirty"), b"dirty")
        .expect("write stale dirty marker");

    prepare_index_storage(
        &database_path,
        &sqlite_path,
        &test_resolved_config(&root),
        true,
    )
    .await
    .expect("prepare index storage should clear orphaned active rag file records");

    assert!(load_rag_file_records(&sqlite_path)
        .expect("load rag file rows")
        .is_empty());
    assert!(!vector_index_file_path(&database_path).exists());
    assert!(!database_path.join("rag-chunks.dirty").exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn refresh_projection_for_rag_file_records_updates_chunk_and_lexical_paths() {
    let root = temp_test_root("refresh-projection-fields");
    let database_path = root.join("rag-index");
    let sqlite_path = database_path.join(RAG_CHUNK_DB_FILE_NAME);
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let mut chunk = test_chunk("/tmp/docs/guide/a.md", "projection refresh chunk");
    chunk.source_root = "/tmp/docs".to_string();
    let prepared_chunk = PreparedRagChunk {
        document_kind: chunk.document_kind,
        chunk_index: chunk.chunk_index,
        line_start: chunk.line_start,
        line_end: chunk.line_end,
        paragraph_line_start: chunk.paragraph_line_start,
        page_start: chunk.page_start,
        page_end: chunk.page_end,
        heading_path: chunk.heading_path.clone(),
        anchor_label: chunk.anchor_label.clone(),
        chunk_reuse_key: chunk.chunk_reuse_key.clone(),
        text: chunk.text.clone(),
    };
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(&[chunk.clone()], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("insert rag chunk");

    let original_record = RagIndexedFileRecord {
        source_root: "/tmp/docs".to_string(),
        absolute_path: "/tmp/docs/guide/a.md".to_string(),
        relative_path: "guide/a.md".to_string(),
        embedding_fingerprint: test_embedding_fingerprint(),
        extractor_fingerprint: test_extractor_fingerprint(),
        active: Some(test_active_version("md5-a", Some(1), 10, 1, 42)),
        pending: None,
    };
    finalize_rag_file_record_and_replace_lexical_chunks(
        &sqlite_path,
        &original_record,
        1,
        &[prepared_chunk],
    )
    .expect("seed rag file and lexical rows");

    let refreshed_record = RagIndexedFileRecord {
        source_root: "/tmp/docs/guide".to_string(),
        relative_path: "a.md".to_string(),
        ..original_record
    };
    refresh_projection_for_rag_file_records(
        &database_path,
        &sqlite_path,
        std::slice::from_ref(&refreshed_record),
    )
    .expect("refresh projection fields");

    let chunk_connection =
        Connection::open(database_path.join(RAG_CHUNK_DB_FILE_NAME)).expect("open chunk db");
    let chunk_source_root: String = chunk_connection
        .query_row(
            "SELECT source_root FROM rag_chunks WHERE absolute_path = ?1 LIMIT 1",
            [&refreshed_record.absolute_path],
            |row| row.get(0),
        )
        .expect("load refreshed chunk source root");
    assert_eq!(chunk_source_root, refreshed_record.source_root);

    let lexical_hits = search_lexical_chunks(&sqlite_path, "projection", 5)
        .expect("search refreshed lexical rows");
    assert_eq!(
        Connection::open(&sqlite_path)
            .expect("open sqlite db for lexical count")
            .query_row("SELECT COUNT(*) FROM rag_chunk_fts", [], |row| row
                .get::<_, i64>(0))
            .expect("count lexical rows"),
        1
    );
    assert_eq!(lexical_hits.len(), 1);
    assert_eq!(lexical_hits[0].source_root, refreshed_record.source_root);
    assert_eq!(
        lexical_hits[0].absolute_path,
        refreshed_record.absolute_path
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rag_file_records_require_rebuild_when_embedding_target_changes_on_restart() {
    let root = PathBuf::from("/tmp/docs");
    let current_provider =
        test_embedding_provider_with_target("embedding", "http://127.0.0.1:8000/v1", "new-model");
    let resolved = test_resolved_config_with_provider(&root, current_provider);
    let stored_records = HashMap::from([(
        "/tmp/docs/a.md".to_string(),
        RagIndexedFileRecord {
            source_root: "/tmp/docs".to_string(),
            absolute_path: "/tmp/docs/a.md".to_string(),
            relative_path: "a.md".to_string(),
            embedding_fingerprint: test_embedding_fingerprint(),
            extractor_fingerprint: test_extractor_fingerprint(),
            active: Some(test_active_version("md5-a", Some(1), 10, 1, 42)),
            pending: None,
        },
    )]);

    assert!(rag_file_records_require_rebuild(&stored_records, &resolved));
}

#[test]
fn rag_file_records_require_rebuild_when_source_roots_change_on_restart() {
    let current_root = PathBuf::from("/tmp/current");
    let resolved = test_resolved_config(&current_root);
    let stored_records = HashMap::from([(
        "/tmp/legacy/a.md".to_string(),
        RagIndexedFileRecord {
            source_root: "/tmp/legacy".to_string(),
            absolute_path: "/tmp/legacy/a.md".to_string(),
            relative_path: "a.md".to_string(),
            embedding_fingerprint: resolved.embedding_fingerprint.clone(),
            extractor_fingerprint: test_extractor_fingerprint(),
            active: Some(test_active_version("md5-a", Some(1), 10, 1, 42)),
            pending: None,
        },
    )]);

    assert!(rag_file_records_require_rebuild(&stored_records, &resolved));
}

#[test]
fn rag_file_records_require_rebuild_when_extractor_fingerprint_changes_on_restart() {
    let current_root = PathBuf::from("/tmp/current");
    let resolved = test_resolved_config(&current_root);
    let stored_records = HashMap::from([(
        "/tmp/current/a.md".to_string(),
        RagIndexedFileRecord {
            source_root: "/tmp/current".to_string(),
            absolute_path: "/tmp/current/a.md".to_string(),
            relative_path: "a.md".to_string(),
            embedding_fingerprint: resolved.embedding_fingerprint.clone(),
            extractor_fingerprint: "plain-text/v0".to_string(),
            active: Some(test_active_version("md5-a", Some(1), 10, 1, 42)),
            pending: None,
        },
    )]);

    assert!(rag_file_records_require_rebuild(&stored_records, &resolved));
}

#[tokio::test]
async fn prepare_index_storage_resets_mismatched_embedding_rows_for_fresh_rebuild() {
    let root = temp_test_root("preserve-mismatched-embedding");
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(&[chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("insert current rag chunk");
    vector_store
        .ensure_index()
        .await
        .expect("build current vector index");

    let indexed_record = RagIndexedFileRecord {
        source_root: "/tmp/docs".to_string(),
        absolute_path: "/tmp/docs/a.md".to_string(),
        relative_path: "a.md".to_string(),
        embedding_fingerprint: test_embedding_fingerprint(),
        extractor_fingerprint: test_extractor_fingerprint(),
        active: Some(test_active_version("md5-a", Some(1), 10, 1, 42)),
        pending: None,
    };
    upsert_rag_file_records(&sqlite_path, &[indexed_record]).expect("write rag file row");

    let next_resolved = test_resolved_config_with_provider(
        Path::new("/tmp/docs"),
        test_embedding_provider_with_target("embedding", "http://127.0.0.1:8000/v1", "new-model"),
    );
    prepare_index_storage(&database_path, &sqlite_path, &next_resolved, true)
        .await
        .expect("prepare index storage should reset mismatched rows");

    assert_eq!(
        load_rag_file_records(&sqlite_path)
            .expect("load rag file rows")
            .len(),
        0
    );
    assert_eq!(
        RagVectorStore::open(&database_path)
            .await
            .expect("reopen vector store")
            .load_chunk_vectors_for_file(
                "/tmp/docs/a.md",
                RagChunkState::Active,
                &test_embedding_fingerprint()
            )
            .await
            .expect("load rebuilt vectors")
            .len(),
        0
    );

    let _ = std::fs::remove_dir_all(&root);
}
