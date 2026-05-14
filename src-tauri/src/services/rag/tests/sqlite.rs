use super::*;

#[test]
fn rag_sqlite_tracks_pending_rows() {
    let root = temp_test_root("rag-sqlite");
    let sqlite_path = root.join("rag.sqlite3");
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let pending_record = RagIndexedFileRecord {
        source_root: "/tmp/source".to_string(),
        absolute_path: "/tmp/source/a.txt".to_string(),
        relative_path: "a.txt".to_string(),
        embedding_fingerprint: test_embedding_fingerprint(),
        extractor_fingerprint: test_extractor_fingerprint(),
        active: None,
        pending: Some(test_pending_version("md5-a", Some(1), 10, 2, 0)),
    };
    upsert_rag_file_records(&sqlite_path, std::slice::from_ref(&pending_record))
        .expect("upsert pending record");
    assert!(rag_sqlite_has_pending_rows(&sqlite_path).expect("query pending records"));

    let indexed_record = RagIndexedFileRecord {
        active: Some(test_active_version("md5-a", Some(1), 10, 2, 99)),
        pending: None,
        ..pending_record.clone()
    };
    upsert_rag_file_records(&sqlite_path, std::slice::from_ref(&indexed_record))
        .expect("upsert indexed record");

    let loaded = load_rag_file_records(&sqlite_path)
        .expect("load rag file records")
        .remove(&indexed_record.absolute_path)
        .expect("rag file record should exist");
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
    assert!(!rag_sqlite_has_pending_rows(&sqlite_path).expect("query pending rows"));

    let _ = std::fs::remove_file(&sqlite_path);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn finalize_rag_file_record_and_lexical_chunks_stays_in_sync() {
    let root = temp_test_root("rag-file-record-lexical-finalize");
    let database_path = root.join("rag-index");
    let sqlite_path = database_path.join(RAG_CHUNK_DB_FILE_NAME);
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let record = RagIndexedFileRecord {
        source_root: "/tmp/source".to_string(),
        absolute_path: "/tmp/source/a.md".to_string(),
        relative_path: "a.md".to_string(),
        embedding_fingerprint: test_embedding_fingerprint(),
        extractor_fingerprint: test_extractor_fingerprint(),
        active: Some(test_active_version("md5-old", Some(1), 10, 1, 10)),
        pending: Some(test_pending_version("md5-new", Some(2), 20, 2, 0)),
    };
    let chunks = vec![
        PreparedRagChunk {
            document_kind: DocumentKind::Markdown,
            chunk_index: 0,
            line_start: Some(1),
            line_end: Some(2),
            paragraph_line_start: Some(1),
            page_start: None,
            page_end: None,
            heading_path: vec!["Intro".to_string()],
            anchor_label: Some("intro".to_string()),
            chunk_reuse_key: "reuse-0".to_string(),
            text: "hello world".to_string(),
        },
        PreparedRagChunk {
            document_kind: DocumentKind::Markdown,
            chunk_index: 1,
            line_start: Some(3),
            line_end: Some(4),
            paragraph_line_start: Some(3),
            page_start: None,
            page_end: None,
            heading_path: vec!["Intro".to_string(), "Next".to_string()],
            anchor_label: None,
            chunk_reuse_key: "reuse-1".to_string(),
            text: "second chunk".to_string(),
        },
    ];
    let active_chunks = chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| RagChunk {
            id: format!("chunk-{index}"),
            source_root: record.source_root.clone(),
            absolute_path: record.absolute_path.clone(),
            version_id: "pending-v1".to_string(),
            embedding_fingerprint: record.embedding_fingerprint.clone(),
            document_kind: chunk.document_kind,
            chunk_state: RagChunkState::Active,
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
        .collect::<Vec<_>>();
    let vectors = vec![vec![1.0_f32, 2.0_f32], vec![3.0_f32, 4.0_f32]];
    let runtime = tokio::runtime::Runtime::new().expect("create runtime");
    runtime.block_on(async {
        let mut vector_store = RagVectorStore::open(&database_path)
            .await
            .expect("open vector store");
        vector_store
            .add_chunks(&active_chunks, &vectors)
            .await
            .expect("insert active rag chunks");
    });

    finalize_rag_file_record_and_replace_lexical_chunks(
        &sqlite_path,
        &record,
        chunks.len(),
        &chunks,
    )
    .expect("finalize rag file record and lexical chunks");

    let stored = load_rag_file_records(&sqlite_path)
        .expect("load rag file records")
        .remove(&record.absolute_path)
        .expect("rag file row should exist");
    let active = stored.active.expect("active version should exist");
    assert_eq!(active.version_id, "pending-v1");
    assert_eq!(active.content_md5, "md5-new");
    assert_eq!(active.chunk_count, 2);
    assert!(stored.pending.is_none());

    let connection =
        rusqlite::Connection::open(&sqlite_path).expect("open sqlite database for lexical rows");
    let lexical_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM rag_chunk_fts", [], |row| row.get(0))
        .expect("count lexical rows");
    assert_eq!(lexical_count, 2);

    let _ = std::fs::remove_file(&sqlite_path);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rag_file_path_lookup_returns_exact_and_descendant_paths() {
    let root = temp_test_root("rag-file-path-lookup");
    let sqlite_path = root.join("rag.sqlite3");
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let exact_path = "/tmp/docs/guide.md";
    let descendant_path = "/tmp/docs/nested/child.md";
    let unrelated_path = "/tmp/other/elsewhere.md";
    upsert_rag_file_records(
        &sqlite_path,
        &[
            RagIndexedFileRecord {
                source_root: "/tmp/docs".to_string(),
                absolute_path: exact_path.to_string(),
                relative_path: "guide.md".to_string(),
                embedding_fingerprint: test_embedding_fingerprint(),
                extractor_fingerprint: test_extractor_fingerprint(),
                active: Some(test_active_version("md5-guide", Some(1), 10, 1, 1)),
                pending: None,
            },
            RagIndexedFileRecord {
                source_root: "/tmp/docs".to_string(),
                absolute_path: descendant_path.to_string(),
                relative_path: "nested/child.md".to_string(),
                embedding_fingerprint: test_embedding_fingerprint(),
                extractor_fingerprint: test_extractor_fingerprint(),
                active: Some(test_active_version("md5-child", Some(2), 20, 2, 2)),
                pending: None,
            },
            RagIndexedFileRecord {
                source_root: "/tmp/other".to_string(),
                absolute_path: unrelated_path.to_string(),
                relative_path: "elsewhere.md".to_string(),
                embedding_fingerprint: test_embedding_fingerprint(),
                extractor_fingerprint: test_extractor_fingerprint(),
                active: Some(test_active_version("md5-other", Some(3), 30, 3, 3)),
                pending: None,
            },
        ],
    )
    .expect("write rag file rows");

    let resolved = load_rag_file_paths_for_prefixes(
        &sqlite_path,
        &[exact_path.to_string(), "/tmp/docs/nested".to_string()],
    )
    .expect("lookup descendant rag file paths");

    assert_eq!(
        resolved,
        vec![exact_path.to_string(), descendant_path.to_string()]
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stream_rebuild_scan_defers_stale_cleanup_until_after_reindex_candidates_are_emitted() {
    let root = temp_test_root("stream-rebuild-scan");
    std::fs::create_dir_all(&root).expect("create rebuild scan root");
    let canonical_root = root.canonicalize().expect("canonicalize rebuild scan root");

    let changed_path = root.join("changed.md");
    std::fs::write(&changed_path, "# Title\n\nfresh content\n").expect("write changed RAG file");
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

#[tokio::test]
async fn execute_path_update_plans_skips_untracked_missing_prefixes() {
    let root = temp_test_root("missing-prefix-delete");
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(&[chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("insert current rag chunk");

    execute_path_update_plans(
        PathUpdateRuntimeContext {
            app_handle: None,
            runtime_status: &runtime_status,
            runtime_guard: None,
        },
        &database_path,
        &sqlite_path,
        &test_resolved_config(&root),
        vec![(
            root.join(".git/index.lock"),
            PathUpdatePlan::Delete {
                delete_descendants: true,
            },
        )],
        RuntimeStatusUpdate::default(),
    )
    .await
    .expect("untracked missing prefix should not fail");

    let vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("reopen vector store");
    let vectors = vector_store
        .load_chunk_vectors_for_file(
            "/tmp/docs/a.md",
            RagChunkState::Active,
            &test_embedding_fingerprint(),
        )
        .await
        .expect("load surviving vectors");
    assert_eq!(vectors.len(), 1);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn execute_path_update_plans_deletes_prefix_chunks_without_rag_file_rows() {
    let root = temp_test_root("prefix-delete-without-rag-file-records");
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let mut descendant_chunk = test_chunk("/tmp/docs/nested/a.md", "descendant chunk");
    descendant_chunk.id = "chunk-descendant".to_string();
    descendant_chunk.absolute_path = "/tmp/docs/nested/a.md".to_string();
    descendant_chunk.source_root = "/tmp/docs".to_string();
    let mut sibling_chunk = test_chunk("/tmp/other/b.md", "sibling chunk");
    sibling_chunk.id = "chunk-sibling".to_string();
    sibling_chunk.absolute_path = "/tmp/other/b.md".to_string();
    sibling_chunk.source_root = "/tmp/other".to_string();
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(
            &[descendant_chunk, sibling_chunk],
            &[vec![1.0_f32, 2.0_f32], vec![3.0_f32, 4.0_f32]],
        )
        .await
        .expect("insert rag chunks");

    execute_path_update_plans(
        PathUpdateRuntimeContext {
            app_handle: None,
            runtime_status: &runtime_status,
            runtime_guard: None,
        },
        &database_path,
        &sqlite_path,
        &test_resolved_config(&root),
        vec![(
            PathBuf::from("/tmp/docs"),
            PathUpdatePlan::Delete {
                delete_descendants: true,
            },
        )],
        RuntimeStatusUpdate::default(),
    )
    .await
    .expect("prefix delete should remove descendant chunks without rag file rows");

    let vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("reopen vector store");
    assert!(vector_store
        .load_chunk_vectors_for_file(
            "/tmp/docs/nested/a.md",
            RagChunkState::Active,
            &test_embedding_fingerprint()
        )
        .await
        .expect("load deleted descendant vectors")
        .is_empty());
    assert_eq!(
        vector_store
            .load_chunk_vectors_for_file(
                "/tmp/other/b.md",
                RagChunkState::Active,
                &test_embedding_fingerprint()
            )
            .await
            .expect("load surviving sibling vectors")
            .len(),
        1
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn execute_path_update_plans_treats_percent_in_prefix_as_literal_path_text() {
    let root = temp_test_root("prefix-delete-percent-literal");
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let mut literal_percent_chunk = test_chunk("/tmp/docs%/nested/a.md", "literal percent chunk");
    literal_percent_chunk.id = "chunk-percent".to_string();
    literal_percent_chunk.absolute_path = "/tmp/docs%/nested/a.md".to_string();
    literal_percent_chunk.source_root = "/tmp/docs%".to_string();
    let mut similarly_prefixed_chunk = test_chunk("/tmp/docsX/nested/b.md", "sibling chunk");
    similarly_prefixed_chunk.id = "chunk-sibling".to_string();
    similarly_prefixed_chunk.absolute_path = "/tmp/docsX/nested/b.md".to_string();
    similarly_prefixed_chunk.source_root = "/tmp/docsX".to_string();
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(
            &[literal_percent_chunk, similarly_prefixed_chunk],
            &[vec![1.0_f32, 2.0_f32], vec![3.0_f32, 4.0_f32]],
        )
        .await
        .expect("insert rag chunks");

    execute_path_update_plans(
        PathUpdateRuntimeContext {
            app_handle: None,
            runtime_status: &runtime_status,
            runtime_guard: None,
        },
        &database_path,
        &sqlite_path,
        &test_resolved_config(&root),
        vec![(
            PathBuf::from("/tmp/docs%"),
            PathUpdatePlan::Delete {
                delete_descendants: true,
            },
        )],
        RuntimeStatusUpdate::default(),
    )
    .await
    .expect("prefix delete should treat percent as literal path text");

    let vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("reopen vector store");
    assert!(vector_store
        .load_chunk_vectors_for_file(
            "/tmp/docs%/nested/a.md",
            RagChunkState::Active,
            &test_embedding_fingerprint()
        )
        .await
        .expect("load deleted percent-prefix vectors")
        .is_empty());
    assert_eq!(
        vector_store
            .load_chunk_vectors_for_file(
                "/tmp/docsX/nested/b.md",
                RagChunkState::Active,
                &test_embedding_fingerprint()
            )
            .await
            .expect("load surviving non-matching vectors")
            .len(),
        1
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rag_sqlite_schema_rejects_incompatible_layout() {
    let root = temp_test_root("incompatible-sqlite-schema");
    let sqlite_path = root.join("rag.sqlite3");
    std::fs::create_dir_all(&root).expect("create rag temp root");

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
                ",
        )
        .expect("create incompatible sqlite schema");

    assert!(!rag_sqlite_has_compatible_schema(&sqlite_path)
        .expect("inspect sqlite schema compatibility"));

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

#[cfg(unix)]
#[test]
fn stream_rebuild_scan_keeps_existing_record_when_file_is_temporarily_unreadable() {
    let root = temp_test_root("stream-rebuild-unreadable");
    std::fs::create_dir_all(&root).expect("create unreadable scan root");
    let file_path = root.join("locked.md");
    std::fs::write(&file_path, "# Locked\n\ncontent\n").expect("write unreadable test file");
    let canonical_root = root
        .canonicalize()
        .expect("canonicalize unreadable scan root");
    let canonical_path = file_path
        .canonicalize()
        .expect("canonicalize unreadable test file");
    let resolved = test_resolved_config(&canonical_root);
    let normalized_path = normalize_path_string(&canonical_path);
    let stored_records = HashMap::from([(
        normalized_path.clone(),
        test_indexed_record(
            &canonical_root,
            &canonical_path,
            &resolved.embedding_fingerprint,
            Some(test_active_version("locked-md5", Some(1), 18, 1, 1)),
            None,
        ),
    )]);

    let original_permissions = std::fs::metadata(&canonical_path)
        .expect("read original permissions")
        .permissions();
    let mut unreadable_permissions = original_permissions.clone();
    unreadable_permissions.set_mode(0o000);
    std::fs::set_permissions(&canonical_path, unreadable_permissions)
        .expect("make test file unreadable");

    let (scan_tx, mut scan_rx) = mpsc::channel(8);
    let scan_result = stream_rebuild_scan(&resolved, &stored_records, scan_tx);

    std::fs::set_permissions(&canonical_path, original_permissions)
        .expect("restore test file permissions");

    scan_result.expect("streaming rebuild scan should survive unreadable files");

    let events = std::iter::from_fn(|| scan_rx.blocking_recv()).collect::<Vec<_>>();
    assert!(events.iter().any(|event| event.skipped_file_count == 1));
    assert!(events
        .iter()
        .any(|event| event.warning_count == 1 && !event.recent_warnings.is_empty()));
    assert!(events.iter().all(|event| !event
        .stale_paths
        .iter()
        .any(|path| path == &normalized_path)));

    let _ = std::fs::remove_file(&file_path);
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[tokio::test]
async fn process_event_batch_surfaces_planning_failures_without_failing_the_watcher() {
    let root = temp_test_root("watcher-planning-failure");
    let source_root = root.join("docs");
    let data_dir = root.join("app-data");
    std::fs::create_dir_all(&source_root).expect("create watcher source root");
    let file_path = source_root.join("broken.md");
    std::fs::write(&file_path, b"# Broken\0content\n")
        .expect("write invalid watcher file contents");

    let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
    let storage_lock = Arc::new(AsyncMutex::new(()));
    let canonical_source_root = source_root
        .canonicalize()
        .expect("canonicalize watcher source root");
    let resolved = test_resolved_config(&canonical_source_root);

    let result = process_event_batch(
        None,
        &data_dir,
        &resolved,
        &runtime_status,
        None,
        &storage_lock,
        vec![Ok(Event {
            kind: EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            paths: vec![file_path.clone()],
            attrs: Default::default(),
        })],
    )
    .await;

    result.expect("watcher batch should ignore planning failures");
    let status = runtime_status.read().await.clone();
    assert_eq!(status.phase, RagRuntimePhase::Idle);
    assert_eq!(status.warning_count, 1);
    assert_eq!(status.recent_warnings.len(), 1);
    assert!(status.recent_warnings[0].contains("broken.md"));
    assert!(status.recent_warnings[0].contains("cannot be indexed as text"));

    let _ = std::fs::remove_file(&file_path);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn process_event_batch_normalizes_deleted_file_paths_before_cleanup() {
    let root = temp_test_root("watcher-normalizes-deleted-path");
    let source_root = root.join("docs");
    let nested_dir = source_root.join("nested");
    let data_dir = root.join("app-data");
    std::fs::create_dir_all(&nested_dir).expect("create nested watcher source root");
    let file_path = source_root.join("a.md");
    std::fs::write(&file_path, "# Indexed\n\ncontent\n").expect("write indexed watcher file");
    let canonical_source_root = source_root
        .canonicalize()
        .expect("canonicalize watcher source root");

    let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
    let storage_lock = Arc::new(AsyncMutex::new(()));
    let resolved = test_resolved_config(&canonical_source_root);
    let database_path = rag_database_path(&data_dir);
    let sqlite_path = rag_sqlite_database_path(&data_dir);
    let canonical_file_path = file_path
        .canonicalize()
        .expect("canonicalize watcher indexed file");
    let absolute_path = normalize_path_string(&canonical_file_path);

    let mut chunk = test_chunk(&absolute_path, "indexed chunk");
    chunk.source_root = normalize_path_string(&canonical_source_root);
    chunk.absolute_path = absolute_path.clone();
    chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open seeded vector store");
    vector_store
        .add_chunks(&[chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("seed active vector");
    upsert_rag_file_records(
        &sqlite_path,
        &[test_indexed_record(
            &canonical_source_root,
            &canonical_file_path,
            &resolved.embedding_fingerprint,
            Some(test_active_version("seeded-md5", Some(1), 18, 1, 1)),
            None,
        )],
    )
    .expect("seed watcher rag file row");

    std::fs::remove_file(&file_path).expect("remove watcher file");
    let deleted_event_path = canonical_source_root.join("nested/../a.md");

    process_event_batch(
        None,
        &data_dir,
        &resolved,
        &runtime_status,
        None,
        &storage_lock,
        vec![Ok(Event {
            kind: EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![deleted_event_path],
            attrs: Default::default(),
        })],
    )
    .await
    .expect("watcher batch should normalize deleted file paths");

    assert!(load_rag_file_records(&sqlite_path)
        .expect("load rag file rows after normalized delete")
        .is_empty());
    assert!(RagVectorStore::open(&database_path)
        .await
        .expect("open vector store after normalized delete")
        .load_chunk_vectors_for_file(
            &absolute_path,
            RagChunkState::Active,
            &test_embedding_fingerprint()
        )
        .await
        .expect("load vectors after normalized delete")
        .is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn process_event_batch_lexically_normalizes_deleted_directory_paths_before_cleanup() {
    let root = temp_test_root("watcher-normalizes-deleted-directory-path");
    let source_root = root.join("docs");
    let nested_dir = source_root.join("nested");
    let data_dir = root.join("app-data");
    std::fs::create_dir_all(&nested_dir).expect("create nested watcher source root");
    let file_path = nested_dir.join("a.md");
    std::fs::write(&file_path, "# Indexed\n\ncontent\n").expect("write indexed watcher file");
    let canonical_source_root = source_root
        .canonicalize()
        .expect("canonicalize watcher source root");

    let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
    let storage_lock = Arc::new(AsyncMutex::new(()));
    let resolved = test_resolved_config(&canonical_source_root);
    let database_path = rag_database_path(&data_dir);
    let sqlite_path = rag_sqlite_database_path(&data_dir);
    let canonical_file_path = file_path
        .canonicalize()
        .expect("canonicalize watcher indexed file");
    let absolute_path = normalize_path_string(&canonical_file_path);

    let mut chunk = test_chunk(&absolute_path, "indexed chunk");
    chunk.source_root = normalize_path_string(&canonical_source_root);
    chunk.absolute_path = absolute_path.clone();
    chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open seeded vector store");
    vector_store
        .add_chunks(&[chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("seed active vector");
    upsert_rag_file_records(
        &sqlite_path,
        &[test_indexed_record(
            &canonical_source_root,
            &canonical_file_path,
            &resolved.embedding_fingerprint,
            Some(test_active_version("seeded-md5", Some(1), 18, 1, 1)),
            None,
        )],
    )
    .expect("seed watcher rag file row");

    std::fs::remove_dir_all(&nested_dir).expect("remove watcher nested directory");
    let deleted_event_path = canonical_source_root.join("missing-parent/../nested");

    process_event_batch(
        None,
        &data_dir,
        &resolved,
        &runtime_status,
        None,
        &storage_lock,
        vec![Ok(Event {
            kind: EventKind::Remove(notify::event::RemoveKind::Folder),
            paths: vec![deleted_event_path],
            attrs: Default::default(),
        })],
    )
    .await
    .expect("watcher batch should lexically normalize deleted directory paths");

    assert!(load_rag_file_records(&sqlite_path)
        .expect("load rag file rows after normalized directory delete")
        .is_empty());
    assert!(RagVectorStore::open(&database_path)
        .await
        .expect("open vector store after normalized directory delete")
        .load_chunk_vectors_for_file(
            &absolute_path,
            RagChunkState::Active,
            &test_embedding_fingerprint()
        )
        .await
        .expect("load vectors after normalized directory delete")
        .is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn process_event_batch_deletes_alias_matched_stale_records_when_file_becomes_unsupported() {
    let root = temp_test_root("watcher-alias-skip-cleanup");
    let source_root = root.join("docs");
    let data_dir = root.join("app-data");
    std::fs::create_dir_all(&source_root).expect("create watcher source root");
    let file_path = source_root.join("stale.bin");
    std::fs::write(&file_path, "binary-ish").expect("write unsupported watcher file");

    let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
    let storage_lock = Arc::new(AsyncMutex::new(()));
    let canonical_source_root = source_root
        .canonicalize()
        .expect("canonicalize watcher source root");
    let resolved = test_resolved_config(&canonical_source_root);
    let database_path = rag_database_path(&data_dir);
    let sqlite_path = rag_sqlite_database_path(&data_dir);
    let original_absolute_path = normalize_path_string(&file_path);

    let mut chunk = test_chunk(&original_absolute_path, "indexed chunk");
    chunk.source_root = normalize_path_string(&source_root);
    chunk.absolute_path = original_absolute_path.clone();
    chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open seeded vector store");
    vector_store
        .add_chunks(&[chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("seed active vector");
    upsert_rag_file_records(
        &sqlite_path,
        &[test_indexed_record(
            &source_root,
            &file_path,
            &resolved.embedding_fingerprint,
            Some(test_active_version("seeded-md5", Some(1), 18, 1, 1)),
            None,
        )],
    )
    .expect("seed watcher rag file row using original path alias");

    process_event_batch(
        None,
        &data_dir,
        &resolved,
        &runtime_status,
        None,
        &storage_lock,
        vec![Ok(Event {
            kind: EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            paths: vec![file_path.clone()],
            attrs: Default::default(),
        })],
    )
    .await
    .expect("watcher batch should delete alias-matched stale records");

    assert!(load_rag_file_records(&sqlite_path)
        .expect("load watcher rag file rows after alias cleanup")
        .is_empty());
    assert!(RagVectorStore::open(&database_path)
        .await
        .expect("open vector store after alias cleanup")
        .load_chunk_vectors_for_file(
            &original_absolute_path,
            RagChunkState::Active,
            &test_embedding_fingerprint()
        )
        .await
        .expect("load vectors after alias cleanup")
        .is_empty());

    let _ = std::fs::remove_dir_all(&root);
}
