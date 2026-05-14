use super::*;

#[test]
fn inspect_path_for_index_skips_hashing_when_size_and_mtime_match() {
    let root = temp_test_root("rag-file-record-fast-path");
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
            refresh_rag_file_record,
            clear_staged,
            ..
        } => {
            assert_eq!(record, stored_record);
            assert!(!refresh_rag_file_record);
            assert!(!clear_staged);
        }
        other => panic!("expected unchanged fast path, got {other:?}"),
    }

    let _ = std::fs::remove_file(&file_path);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn inspect_path_for_index_refreshes_rag_file_record_when_md5_matches() {
    let root = temp_test_root("rag-file-record-refresh");
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
            refresh_rag_file_record,
            clear_staged,
            ..
        } => {
            let active = record.active.expect("active version should exist");
            assert!(refresh_rag_file_record);
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
fn inspect_path_for_index_clears_pending_record_when_active_version_matches() {
    let root = temp_test_root("rag-file-record-pending");
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
            refresh_rag_file_record,
            clear_staged,
            ..
        } => {
            assert!(refresh_rag_file_record);
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
fn inspect_path_for_index_refreshes_projection_without_reindex_when_content_is_unchanged() {
    let root = temp_test_root("projection-refresh-without-reindex");
    let nested_root = root.join("nested");
    let file_path = nested_root.join("a.md");
    std::fs::create_dir_all(&nested_root).expect("create nested rag temp root");
    std::fs::write(&file_path, "alpha beta gamma").expect("write rag source file");

    let resolved = test_resolved_config(&nested_root);
    let file_metadata = std::fs::metadata(&file_path).expect("read metadata");
    let modified_at_ms = file_metadata
        .modified()
        .ok()
        .and_then(system_time_to_unix_ms);
    let stored_record = test_indexed_record(
        &root,
        &file_path,
        &resolved.embedding_fingerprint,
        Some(test_active_version(
            &format!("{:x}", md5::compute("alpha beta gamma")),
            modified_at_ms,
            i64::try_from(file_metadata.len()).expect("file size fits i64"),
            1,
            42,
        )),
        None,
    );

    let outcome = inspect_path_for_index(&resolved, &file_path, Some(&stored_record))
        .expect("inspect path should succeed");

    match outcome {
        InspectPathOutcome::Unchanged {
            record,
            refresh_rag_file_record,
            clear_staged,
            refresh_projection,
        } => {
            assert!(refresh_rag_file_record);
            assert!(!clear_staged);
            assert!(refresh_projection);
            assert_eq!(record.source_root, normalize_path_string(&nested_root));
            assert_eq!(record.relative_path, "a.md");
            assert!(record.pending.is_none());
        }
        other => panic!("expected unchanged projection refresh path, got {other:?}"),
    }

    let _ = std::fs::remove_file(&file_path);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn inspect_path_for_index_reindexes_when_embedding_fingerprint_changes() {
    let root = temp_test_root("rag-file-record-model-change");
    let file_path = root.join("notes.txt");
    let content = "alpha beta gamma";
    std::fs::create_dir_all(&root).expect("create rag temp root");
    std::fs::write(&file_path, content).expect("write rag source file");

    let resolved = test_resolved_config(&root);
    let file_metadata = std::fs::metadata(&file_path).expect("read metadata");
    let stored_record = test_indexed_record(
        &root,
        &file_path,
        &test_embedding_fingerprint_for("https://other.example.com/v1", "text-embedding-3-large"),
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
fn inspect_path_for_index_keeps_chunk_snapshot_after_file_change() {
    let root = temp_test_root("prepare-file-snapshot");
    let file_path = root.join("notes.txt");
    std::fs::create_dir_all(&root).expect("create rag temp root");
    std::fs::write(&file_path, "alpha beta gamma").expect("write initial rag source file");

    let resolved = test_resolved_config(&root);
    let file = match inspect_path_for_index(&resolved, &file_path, None)
        .expect("initial inspect should succeed")
    {
        InspectPathOutcome::Reindex(file) => file,
        other => panic!("expected reindex outcome, got {other:?}"),
    };
    let original_version_id = file.version_id.clone();
    let original_chunk_texts = file
        .prepared_chunks
        .iter()
        .map(|chunk| chunk.text.clone())
        .collect::<Vec<_>>();

    std::fs::write(&file_path, "delta epsilon zeta eta").expect("rewrite rag source file");

    let persisted_chunks = build_chunks_for_prepared_file(&file, &file.prepared_chunks);
    assert_eq!(file.version_id, original_version_id);
    assert_eq!(
        file.prepared_chunks
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>(),
        original_chunk_texts
    );
    assert_eq!(persisted_chunks.len(), file.prepared_chunks.len());

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
fn watcher_directory_create_event_collects_nested_supported_files() {
    let root = temp_test_root("watcher-directory-create-collect");
    let nested = root.join("nested");
    let deeper = nested.join("deeper");
    std::fs::create_dir_all(&deeper).expect("create nested directories");
    let markdown_file = nested.join("a.md");
    let text_file = deeper.join("b.txt");
    let unsupported_file = deeper.join("c.png");
    std::fs::write(&markdown_file, "# hello").expect("write markdown file");
    std::fs::write(&text_file, "hello").expect("write text file");
    std::fs::write(&unsupported_file, "nope").expect("write unsupported file");

    let paths = collect_indexable_paths_in_directory(&nested);

    assert!(paths.contains(&markdown_file));
    assert!(paths.contains(&text_file));
    assert!(!paths.contains(&unsupported_file));

    let _ = std::fs::remove_dir_all(&root);
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

#[tokio::test]
async fn initialize_runtime_storage_reconciles_deleted_files_during_reuse_startup() {
    let root = temp_test_root("startup-reuse-reconciles-deletions");
    let source_root = root.join("docs");
    let data_dir = root.join("app-data");
    std::fs::create_dir_all(&source_root).expect("create rag source root");
    let file_path = source_root.join("stale.md");
    std::fs::write(&file_path, "# Indexed\n\ncontent\n").expect("write indexed rag source file");

    let resolved = test_resolved_config(&source_root);
    let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
    let storage_lock = Arc::new(AsyncMutex::new(()));
    let database_path = rag_database_path(&data_dir);
    let sqlite_path = rag_sqlite_database_path(&data_dir);
    let absolute_path = normalize_path_string(&file_path);

    let mut chunk = test_chunk(&absolute_path, "indexed chunk");
    chunk.source_root = normalize_path_string(&source_root);
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
            &source_root,
            &file_path,
            &resolved.embedding_fingerprint,
            Some(test_active_version("seeded-md5", Some(1), 18, 1, 1)),
            None,
        )],
    )
    .expect("seed rag file row");

    assert_eq!(
        load_rag_file_records(&sqlite_path)
            .expect("load seeded rag file rows")
            .len(),
        1
    );

    std::fs::remove_file(&file_path).expect("remove source file while runtime is offline");

    initialize_runtime_storage(
        &data_dir,
        &sqlite_path,
        &resolved,
        None,
        &runtime_status,
        None,
        &storage_lock,
        RagRuntimeStartMode::ReuseIndex,
    )
    .await
    .expect("reuse startup should reconcile deleted files");

    assert!(load_rag_file_records(&sqlite_path)
        .expect("load reconciled rag file rows")
        .is_empty());
    assert!(RagVectorStore::open(&database_path)
        .await
        .expect("open reconciled vector store")
        .load_chunk_vectors_for_file(
            &absolute_path,
            RagChunkState::Active,
            &test_embedding_fingerprint()
        )
        .await
        .expect("load reconciled active vectors")
        .is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn active_vector_rows_release_sqlite_blobs_after_index_save() {
    let root = temp_test_root("active-vectors-release-sqlite-blobs");
    std::fs::create_dir_all(&root).expect("create vector policy root");
    let chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    let expected_vector = vec![1.0_f32, 2.0_f32];
    let mut vector_store = RagVectorStore::open(&root)
        .await
        .expect("open vector store");

    vector_store
        .add_chunks(&[chunk], std::slice::from_ref(&expected_vector))
        .await
        .expect("insert current rag chunk");

    assert_eq!(count_active_vector_blobs(&root), 0);

    let reopened = RagVectorStore::open(&root)
        .await
        .expect("reopen vector store");
    let loaded_vectors = reopened
        .load_chunk_vectors_for_file(
            "/tmp/docs/a.md",
            RagChunkState::Active,
            &test_embedding_fingerprint(),
        )
        .await
        .expect("load vectors from USearch-backed active row");
    assert_eq!(loaded_vectors.len(), 1);
    assert_eq!(
        loaded_vectors.get(&test_chunk("/tmp/docs/a.md", "current chunk").chunk_reuse_key),
        Some(&expected_vector)
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn prepare_index_storage_resets_unusable_vector_index_when_active_blobs_were_reclaimed() {
    let root = temp_test_root("prepare-resets-unusable-vector-index");
    let source_root = root.join("docs");
    std::fs::create_dir_all(&source_root).expect("create rag source root");
    let file_path = source_root.join("indexed.md");
    let file_text = "# Indexed\n\ncontent\n";
    std::fs::write(&file_path, file_text).expect("write indexed rag source file");

    let resolved = test_resolved_config(&source_root);
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    let absolute_path = normalize_path_string(&file_path);
    let file_metadata = std::fs::metadata(&file_path).expect("read indexed file metadata");

    let mut chunk = test_chunk(&absolute_path, "indexed chunk");
    chunk.source_root = normalize_path_string(&source_root);
    chunk.absolute_path = absolute_path.clone();
    chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open seeded vector store");
    vector_store
        .add_chunks(&[chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("seed active vector");
    assert_eq!(count_active_vector_blobs(&database_path), 0);
    assert!(vector_index_file_path(&database_path).exists());
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

    let dirty_marker_path = database_path.join("rag-chunks.dirty");
    std::fs::write(
        vector_index_file_path(&database_path),
        b"not-a-valid-usearch-index",
    )
    .expect("corrupt vector index");
    std::fs::write(&dirty_marker_path, b"dirty").expect("write dirty marker");

    prepare_index_storage(&database_path, &sqlite_path, &resolved, true)
        .await
        .expect("prepare index storage should reset reclaimed vector storage");

    assert!(!vector_index_file_path(&database_path).exists());
    assert!(!dirty_marker_path.exists());
    let rebuilt_records = load_rag_file_records(&sqlite_path).expect("load rebuilt rag file rows");
    assert!(rebuilt_records.is_empty());
    let rebuilt_vectors = RagVectorStore::open(&database_path)
        .await
        .expect("reopen rebuilt vector store")
        .load_chunk_vectors_for_file(
            &absolute_path,
            RagChunkState::Active,
            &test_embedding_fingerprint(),
        )
        .await
        .expect("load rebuilt active vectors");
    assert!(rebuilt_vectors.is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn prepare_index_storage_keeps_usable_index_when_only_dirty_marker_remains() {
    let root = temp_test_root("prepare-keeps-usable-index-with-stale-dirty-marker");
    let source_root = root.join("docs");
    std::fs::create_dir_all(&source_root).expect("create rag source root");
    let file_path = source_root.join("indexed.md");
    let file_text = "# Indexed\n\ncontent\n";
    std::fs::write(&file_path, file_text).expect("write indexed rag source file");

    let resolved = test_resolved_config(&source_root);
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    let absolute_path = normalize_path_string(&file_path);
    let file_metadata = std::fs::metadata(&file_path).expect("read indexed file metadata");

    let mut chunk = test_chunk(&absolute_path, "indexed chunk");
    chunk.source_root = normalize_path_string(&source_root);
    chunk.absolute_path = absolute_path.clone();
    chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    let expected_reuse_key = chunk.chunk_reuse_key.clone();
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open seeded vector store");
    vector_store
        .add_chunks(&[chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("seed active vector");
    assert_eq!(count_active_vector_blobs(&database_path), 0);
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

    let dirty_marker_path = database_path.join("rag-chunks.dirty");
    std::fs::write(&dirty_marker_path, b"dirty").expect("write stale dirty marker");

    prepare_index_storage(&database_path, &sqlite_path, &resolved, true)
        .await
        .expect("prepare index storage should keep usable index");

    assert!(vector_index_file_path(&database_path).exists());
    assert!(!dirty_marker_path.exists());
    assert_eq!(count_active_vector_blobs(&database_path), 0);
    assert_eq!(
        load_rag_file_records(&sqlite_path)
            .expect("load surviving rag file rows")
            .len(),
        1
    );
    let surviving_vectors = RagVectorStore::open(&database_path)
        .await
        .expect("reopen surviving vector store")
        .load_chunk_vectors_for_file(
            &absolute_path,
            RagChunkState::Active,
            &test_embedding_fingerprint(),
        )
        .await
        .expect("load surviving active vectors");
    assert_eq!(
        surviving_vectors.get(&expected_reuse_key),
        Some(&vec![1.0_f32, 2.0_f32])
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn prepare_index_storage_resets_partial_active_blob_recovery_state() {
    let root = temp_test_root("prepare-resets-partial-active-blob-recovery");
    let source_root = root.join("docs");
    std::fs::create_dir_all(&source_root).expect("create rag source root");
    let file_path = source_root.join("indexed.md");
    let file_text = "# Indexed\n\ncontent\n";
    std::fs::write(&file_path, file_text).expect("write indexed rag source file");

    let resolved = test_resolved_config(&source_root);
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    let absolute_path = normalize_path_string(&file_path);
    let file_metadata = std::fs::metadata(&file_path).expect("read indexed file metadata");

    let mut first_chunk = test_chunk(&absolute_path, "indexed chunk one");
    first_chunk.source_root = normalize_path_string(&source_root);
    first_chunk.absolute_path = absolute_path.clone();
    first_chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    let mut second_chunk = test_chunk(&absolute_path, "indexed chunk two");
    second_chunk.id.push_str("-second");
    second_chunk.chunk_index = 1;
    second_chunk.chunk_reuse_key.push_str("-second");
    second_chunk.source_root = normalize_path_string(&source_root);
    second_chunk.absolute_path = absolute_path.clone();
    second_chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    let first_vector = vec![1.0_f32, 2.0_f32];
    let second_vector = vec![3.0_f32, 4.0_f32];

    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open seeded vector store");
    vector_store
        .add_chunks(
            &[first_chunk.clone(), second_chunk],
            &[first_vector.clone(), second_vector],
        )
        .await
        .expect("seed active vectors");
    assert_eq!(count_active_vector_blobs(&database_path), 0);
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
                2,
                1,
            )),
            None,
        )],
    )
    .expect("seed rag file row");

    let connection =
        open_vector_chunk_connection(&database_path).expect("open chunk database for corruption");
    connection
        .execute(
            "UPDATE rag_chunks SET vector_blob = ?1 WHERE chunk_reuse_key = ?2",
            params![
                test_serialize_vector(&first_vector),
                &first_chunk.chunk_reuse_key
            ],
        )
        .expect("restore one active vector blob only");
    assert_eq!(count_active_vector_blobs(&database_path), 1);

    let dirty_marker_path = database_path.join("rag-chunks.dirty");
    std::fs::write(
        vector_index_file_path(&database_path),
        b"not-a-valid-usearch-index",
    )
    .expect("corrupt vector index");
    std::fs::write(&dirty_marker_path, b"dirty").expect("write dirty marker");

    prepare_index_storage(&database_path, &sqlite_path, &resolved, true)
        .await
        .expect("prepare index storage should reset partial recovery state");

    assert!(!vector_index_file_path(&database_path).exists());
    assert!(!dirty_marker_path.exists());
    assert!(load_rag_file_records(&sqlite_path)
        .expect("load rag file rows after partial recovery reset")
        .is_empty());
    assert!(RagVectorStore::open(&database_path)
        .await
        .expect("reopen vector store after partial recovery reset")
        .load_chunk_vectors_for_file(
            &absolute_path,
            RagChunkState::Active,
            &test_embedding_fingerprint()
        )
        .await
        .expect("load vectors after partial recovery reset")
        .is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn prepare_index_storage_rebuilds_incomplete_index_when_active_blobs_are_recoverable() {
    let root = temp_test_root("prepare-rebuilds-incomplete-index");
    let source_root = root.join("docs");
    std::fs::create_dir_all(&source_root).expect("create rag source root");
    let file_path = source_root.join("indexed.md");
    let file_text = "# Indexed\n\ncontent\n";
    std::fs::write(&file_path, file_text).expect("write indexed rag source file");

    let resolved = test_resolved_config(&source_root);
    let database_path = root.join("rag-index");
    let sqlite_path = root.join("rag.sqlite3");
    let absolute_path = normalize_path_string(&file_path);
    let file_metadata = std::fs::metadata(&file_path).expect("read indexed file metadata");

    let mut first_chunk = test_chunk(&absolute_path, "indexed chunk one");
    first_chunk.source_root = normalize_path_string(&source_root);
    first_chunk.absolute_path = absolute_path.clone();
    first_chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    let first_reuse_key = first_chunk.chunk_reuse_key.clone();
    let mut second_chunk = test_chunk(&absolute_path, "indexed chunk two");
    second_chunk.id.push_str("-second");
    second_chunk.chunk_index = 1;
    second_chunk.chunk_reuse_key.push_str("-second");
    second_chunk.source_root = normalize_path_string(&source_root);
    second_chunk.absolute_path = absolute_path.clone();
    second_chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    let second_reuse_key = second_chunk.chunk_reuse_key.clone();
    let first_vector = vec![1.0_f32, 2.0_f32];
    let second_vector = vec![3.0_f32, 4.0_f32];

    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open seeded vector store");
    vector_store
        .add_chunks(
            &[first_chunk.clone(), second_chunk.clone()],
            &[first_vector.clone(), second_vector.clone()],
        )
        .await
        .expect("seed active vectors");
    assert_eq!(count_active_vector_blobs(&database_path), 0);
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
                2,
                1,
            )),
            None,
        )],
    )
    .expect("seed rag file row");

    let connection =
        open_vector_chunk_connection(&database_path).expect("open chunk database for recovery");
    connection
        .execute(
            "UPDATE rag_chunks SET vector_blob = ?1 WHERE chunk_reuse_key = ?2",
            params![test_serialize_vector(&first_vector), &first_reuse_key],
        )
        .expect("restore first active vector blob");
    connection
        .execute(
            "UPDATE rag_chunks SET vector_blob = ?1 WHERE chunk_reuse_key = ?2",
            params![test_serialize_vector(&second_vector), &second_reuse_key],
        )
        .expect("restore second active vector blob");
    assert_eq!(count_active_vector_blobs(&database_path), 2);

    let removed_vector_key = connection
        .query_row(
            "SELECT MIN(vector_key) FROM rag_chunks WHERE chunk_state = 'active'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("load active vector key");
    remove_vector_from_index_for_test(
        &database_path,
        u64::try_from(removed_vector_key).expect("vector key should be positive"),
    );

    prepare_index_storage(&database_path, &sqlite_path, &resolved, true)
        .await
        .expect("prepare index storage should rebuild incomplete vector index");

    assert!(vector_index_file_path(&database_path).exists());
    assert!(!database_path.join("rag-chunks.dirty").exists());
    assert_eq!(count_active_vector_blobs(&database_path), 0);
    assert_eq!(
        load_rag_file_records(&sqlite_path)
            .expect("load surviving rag file rows")
            .len(),
        1
    );
    let surviving_vectors = RagVectorStore::open(&database_path)
        .await
        .expect("reopen vector store")
        .load_chunk_vectors_for_file(
            &absolute_path,
            RagChunkState::Active,
            &resolved.embedding_fingerprint,
        )
        .await
        .expect("load rebuilt active vectors");
    assert_eq!(surviving_vectors.get(&first_reuse_key), Some(&first_vector));
    assert_eq!(
        surviving_vectors.get(&second_reuse_key),
        Some(&second_vector)
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn initialize_runtime_storage_deletes_alias_matched_stale_records_when_file_is_skipped() {
    let root = temp_test_root("startup-reuse-skip-alias-cleanup");
    let source_root = root.join("docs");
    let data_dir = root.join("app-data");
    std::fs::create_dir_all(&source_root).expect("create rag source root");
    let file_path = source_root.join("stale.bin");
    std::fs::write(&file_path, "binary-ish").expect("write unsupported rag source file");

    let resolved = test_resolved_config(&source_root);
    let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
    let storage_lock = Arc::new(AsyncMutex::new(()));
    let database_path = rag_database_path(&data_dir);
    let sqlite_path = rag_sqlite_database_path(&data_dir);
    let canonical_source_root = source_root
        .canonicalize()
        .expect("canonicalize rag source root");
    let canonical_file_path = file_path
        .canonicalize()
        .expect("canonicalize rag file path");
    let canonical_absolute_path = normalize_path_string(&canonical_file_path);

    let mut chunk = test_chunk(&canonical_absolute_path, "indexed chunk");
    chunk.source_root = normalize_path_string(&canonical_source_root);
    chunk.absolute_path = canonical_absolute_path.clone();
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
    .expect("seed canonical rag file row");

    initialize_runtime_storage(
        &data_dir,
        &sqlite_path,
        &resolved,
        None,
        &runtime_status,
        None,
        &storage_lock,
        RagRuntimeStartMode::ReuseIndex,
    )
    .await
    .expect("reuse startup should delete alias-matched stale records");

    assert!(load_rag_file_records(&sqlite_path)
        .expect("load reconciled rag file rows")
        .is_empty());
    assert!(RagVectorStore::open(&database_path)
        .await
        .expect("open reconciled vector store")
        .load_chunk_vectors_for_file(
            &canonical_absolute_path,
            RagChunkState::Active,
            &test_embedding_fingerprint()
        )
        .await
        .expect("load reconciled active vectors")
        .is_empty());

    let _ = std::fs::remove_dir_all(&root);
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
fn embedding_fingerprint_normalizes_base_url_for_same_endpoint() {
    let first =
        test_embedding_fingerprint_for("https://api.openai.com/v1", "text-embedding-3-small");
    let second =
        test_embedding_fingerprint_for(" https://api.openai.com/v1/ ", "text-embedding-3-small");

    assert_eq!(first, second);
}

#[test]
fn embedding_fingerprint_separates_generic_compatible_endpoints() {
    let first =
        test_embedding_fingerprint_for("https://proxy-a.example.com/v1", "text-embedding-3-small");
    let second =
        test_embedding_fingerprint_for("https://proxy-b.example.com/v1", "text-embedding-3-small");

    assert_ne!(first, second);
}

#[test]
fn embedding_fingerprint_normalizes_generic_model_name_case_and_whitespace_per_endpoint() {
    let first =
        test_embedding_fingerprint_for("https://proxy-a.example.com/v1", "Qwen3-Embedding-0.6B");
    let second = test_embedding_fingerprint_for(
        " https://proxy-a.example.com/v1/ ",
        " qwen3-embedding-0.6b ",
    );

    assert_eq!(first, second);
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

    let runtime_status = Arc::new(AsyncRwLock::new(RagRuntimeStatus::default()));
    let runtime_generation = Arc::new(AtomicU64::new(0));
    let runtime_context = RagRuntimeContext {
        app_handle: None,
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
            embedding_model_id: Some("embedding".to_string()),
        },
        LlmSettings {
            providers: vec![LlmProviderConfig {
                id: "embedding".to_string(),
                name: "Embedding".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                models: vec![crate::domain::settings::LlmModelConfig {
                    id: "embedding".to_string(),
                    model_type: crate::domain::settings::LlmModelType::Embedding,
                    model: "text-embedding-3-small".to_string(),
                    supports_multimodal: false,
                    ..crate::domain::settings::LlmModelConfig::default()
                }],
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
    let reopened = RagVectorStore::open(&database_path)
        .await
        .expect("reopen vector store");
    assert_eq!(
        reopened
            .load_chunk_vectors_for_file(
                "/tmp/docs/a.md",
                RagChunkState::Active,
                &test_embedding_fingerprint()
            )
            .await
            .expect("load preserved vectors")
            .len(),
        1
    );
    assert_eq!(
        load_rag_file_records(&sqlite_path)
            .expect("load rag file rows")
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
        embedding_model_id: Some("embedding".to_string()),
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
async fn apply_settings_restarts_active_error_watcher_with_same_inputs() {
    let root = temp_test_root("restart-active-error-watcher");
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let service = RagIndexService::new(root.clone());
    let rag_settings = RagSettings {
        source_directories: vec![root.join("missing").to_string_lossy().into_owned()],
        ignore_globs: Vec::new(),
        embedding_model_id: Some("embedding".to_string()),
    };
    let llm_settings = LlmSettings {
        providers: vec![test_embedding_provider()],
        ..LlmSettings::default()
    };
    let next_inputs = RagRuntimeInputs::from_settings(&rag_settings, &llm_settings);

    service
        .seed_runtime_state_for_test(
            next_inputs,
            RagRuntimePhase::Error,
            tokio::spawn(async {
                std::future::pending::<()>().await;
            }),
        )
        .await;

    service.apply_settings(rag_settings, llm_settings).await;

    assert_eq!(service.runtime_generation.load(Ordering::SeqCst), 1);
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
            embedding_model_id: Some("rag-provider".to_string()),
        },
        &LlmSettings {
            providers: vec![LlmProviderConfig {
                id: "rag-provider".to_string(),
                name: "rag-provider".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                models: vec![crate::domain::settings::LlmModelConfig {
                    id: "rag-provider".to_string(),
                    model_type: crate::domain::settings::LlmModelType::Embedding,
                    model: "model-a".to_string(),
                    supports_multimodal: false,
                    ..crate::domain::settings::LlmModelConfig::default()
                }],
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
            embedding_model_id: Some("rag-provider".to_string()),
        },
        &LlmSettings {
            providers: vec![LlmProviderConfig {
                id: "rag-provider".to_string(),
                name: "rag-provider".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                api_key: String::new(),
                models: vec![crate::domain::settings::LlmModelConfig {
                    id: "rag-provider".to_string(),
                    model_type: crate::domain::settings::LlmModelType::Embedding,
                    model: "model-a".to_string(),
                    supports_multimodal: false,
                    ..crate::domain::settings::LlmModelConfig::default()
                }],
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
