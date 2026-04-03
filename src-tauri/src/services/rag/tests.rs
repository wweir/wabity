use std::{
    collections::HashMap,
    io::{Cursor, Write},
    time::Duration,
};

use super::*;
use arrow_array::{
    types::Float32Type, FixedSizeListArray, Int32Array, RecordBatch, RecordBatchIterator,
    StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use lancedb::connect;
use text_splitter::TextSplitter;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
use zip::{write::SimpleFileOptions, ZipWriter};

use crate::services::document_extract::{extract_document_from_bytes, is_supported_document_file};

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

fn test_embedding_provider_with_target(id: &str, base_url: &str, model: &str) -> LlmProviderConfig {
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

fn test_extractor_fingerprint() -> String {
    extract_document_from_bytes(Path::new("/tmp/readme.md"), b"hello")
        .expect("test extractor fingerprint should resolve")
        .extractor_fingerprint
}

fn split_test_text_for_path(
    path: &Path,
    text: &str,
    capacity: usize,
    overlap: usize,
) -> Result<Vec<PreparedRagChunk>> {
    let extracted = extract_document_from_bytes(path, text.as_bytes())?;
    split_extracted_document_for_path(path, &extracted, capacity, overlap)
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
        extractor_fingerprint: test_extractor_fingerprint(),
        active,
        pending,
    }
}

fn test_chunk(path: &str, text: &str) -> RagChunk {
    RagChunk {
        id: "chunk-1".to_string(),
        source_root: "/tmp/docs".to_string(),
        absolute_path: path.to_string(),
        version_id: "active-v1".to_string(),
        embedding_fingerprint: test_embedding_fingerprint(),
        document_kind: DocumentKind::Markdown,
        chunk_state: RagChunkState::Active,
        chunk_index: 0,
        line_start: Some(1),
        line_end: Some(3),
        paragraph_line_start: Some(1),
        page_start: None,
        page_end: None,
        heading_path: vec!["Intro".to_string()],
        anchor_label: None,
        chunk_reuse_key: "reuse-1".to_string(),
        text_fingerprint: text_fingerprint(text),
        text: text.to_string(),
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn read_http_request_body(stream: &mut tokio::net::TcpStream) -> std::io::Result<Vec<u8>> {
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

    let chunks =
        collect_chunks_for_path(&resolved, &file_path).expect("collecting chunks should succeed");

    assert!(chunks.len() > 1);
    assert_eq!(chunks[0].source_root, root.to_string_lossy());
    assert_eq!(chunks[0].absolute_path, file_path.to_string_lossy());
    assert!(chunks[0].line_start.unwrap_or_default() >= 1);
    assert!(chunks[0].line_end.unwrap_or_default() >= chunks[0].line_start.unwrap_or_default());
    assert!(chunks[0].paragraph_line_start.unwrap_or_default() >= 1);
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
    let chunks = split_test_text_for_path(Path::new("/tmp/readme.md"), text, 24, 0)
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

    let metadata = resolve_chunk_metadata(&layout, 2, 6).expect("chunk metadata should resolve");

    assert_eq!(metadata.heading_path, vec!["Intro".to_string()]);
}

#[test]
fn resolve_chunk_metadata_anchors_to_first_non_empty_line() {
    let text = "\n\nAlpha\nBeta";
    let layout = build_text_layout(text, false);

    let metadata = resolve_chunk_metadata(&layout, 0, 3).expect("chunk metadata should resolve");

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

    let chunks = split_test_text_for_path(Path::new("/tmp/readme.txt"), text, 24, 0)
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
    let chunks = split_test_text_for_path(Path::new("/tmp/readme.md"), &text, 24, 0)
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

    let chunks =
        collect_chunks_for_path(&resolved, &file_path).expect("collecting chunks should succeed");

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
    let chunks =
        collect_chunks_for_path(&resolved, &file_path).expect("collect docx chunks should work");

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
        &test_embedding_fingerprint_for("https://other.example.com/v1", "text-embedding-3-small"),
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
    let second =
        test_embedding_fingerprint_for(" https://api.openai.com/v1/ ", "text-embedding-3-small");

    assert_eq!(first, second);
}

#[test]
fn embedding_fingerprint_keeps_generic_compatible_endpoints_separate() {
    let first =
        test_embedding_fingerprint_for("https://proxy-a.example.com/v1", "text-embedding-3-small");
    let second =
        test_embedding_fingerprint_for("https://proxy-b.example.com/v1", "text-embedding-3-small");

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

    let chunk = test_chunk("/tmp/docs/a.md", "current chunk");
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
        extractor_fingerprint: test_extractor_fingerprint(),
        active: Some(test_active_version("md5-a", Some(1), 10, 1, 42)),
        pending: None,
    };
    upsert_metadata_records(&metadata_path, &[indexed_record]).expect("write metadata row");

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
        extractor_fingerprint: test_extractor_fingerprint(),
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
fn metadata_prefix_lookup_returns_exact_and_descendant_paths() {
    let root = temp_test_root("metadata-prefix-lookup");
    let metadata_path = root.join("rag.sqlite3");
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let exact_path = "/tmp/docs/guide.md";
    let descendant_path = "/tmp/docs/nested/child.md";
    let unrelated_path = "/tmp/other/elsewhere.md";
    upsert_metadata_records(
        &metadata_path,
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
    .expect("write metadata rows");

    let resolved = load_metadata_paths_for_prefixes(
        &metadata_path,
        &[exact_path.to_string(), "/tmp/docs/nested".to_string()],
    )
    .expect("lookup descendant metadata paths");

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
    let database_path = root.join("rag-lancedb");
    let metadata_path = root.join("rag.sqlite3");
    std::fs::create_dir_all(&root).expect("create rag temp root");

    let chunk = test_chunk("/tmp/docs/a.md", "current chunk");
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

    execute_path_update_plans(
        None,
        &database_path,
        &metadata_path,
        &test_resolved_config(&root),
        &Arc::new(AsyncRwLock::new(RagRuntimeStatus::default())),
        None,
        vec![(
            root.join(".git/index.lock"),
            PathUpdatePlan::Delete {
                delete_descendants: true,
            },
        )],
    )
    .await
    .expect("untracked missing prefix should not fail");

    let vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("reopen vector store");
    let vectors = vector_store
        .load_chunk_vectors_for_file("/tmp/docs/a.md", RagChunkState::Active)
        .await
        .expect("load surviving vectors");
    assert_eq!(vectors.len(), 1);

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
        extractor_fingerprint: test_extractor_fingerprint(),
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

    let chunk = test_chunk("/tmp/docs/a.md", "current chunk");
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
        extractor_fingerprint: test_extractor_fingerprint(),
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

    let mut chunk = test_chunk("/tmp/docs/a.md", "current chunk");
    chunk.embedding_fingerprint =
        test_embedding_fingerprint_for("https://other.example.com/v1", "text-embedding-3-small");
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
        extractor_fingerprint: test_extractor_fingerprint(),
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

    let chunk = test_chunk("/tmp/docs/a.md", "current chunk");
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
            let response_body =
                serde_json::to_vec(&serde_json::json!({ "data": [{ "embedding": [9.0_f32] }] }))
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

    let mut cached_chunk = test_chunk(&normalize_path_string(&root.join("a.md")), "shared text");
    cached_chunk.source_root = normalize_path_string(&root);
    cached_chunk.absolute_path = normalize_path_string(&root.join("a.md"));
    cached_chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
    cached_chunk.line_end = Some(2);
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
        &[{
            let mut chunk = test_chunk(&normalize_path_string(&root.join("b.md")), "shared text");
            chunk.id = "chunk-2".to_string();
            chunk.source_root = normalize_path_string(&root);
            chunk.absolute_path = normalize_path_string(&root.join("b.md"));
            chunk.version_id = "pending-v1".to_string();
            chunk.embedding_fingerprint = resolved.embedding_fingerprint.clone();
            chunk.chunk_state = RagChunkState::Staged;
            chunk.line_end = Some(2);
            chunk.heading_path = vec!["Elsewhere".to_string()];
            chunk.chunk_reuse_key = "reuse-2".to_string();
            chunk
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
    let chunks = split_test_text_for_path(Path::new("/tmp/readme.md"), text, 48, 0)
        .expect("markdown split should succeed");

    assert!(chunks.iter().all(|chunk| {
        chunk.line_start.unwrap_or_default() >= chunk.paragraph_line_start.unwrap_or_default()
            && chunk.line_end.unwrap_or_default() >= chunk.line_start.unwrap_or_default()
    }));
    assert!(chunks.iter().any(|chunk| {
        chunk.heading_path == vec!["Intro".to_string()] && chunk.line_end.unwrap_or_default() >= 3
    }));
    assert!(chunks.iter().any(|chunk| {
        chunk.heading_path == vec!["Intro".to_string(), "Details".to_string()]
            && chunk.line_start.unwrap_or_default() >= 6
    }));
}

#[test]
fn markdown_list_notes_split_under_same_heading() {
    let bullet =
        "- 甲富而乙贫，并不是因为甲有马，乙却步行，而是因为甲富能备有马车，乙贫不能不步行。\n\n";
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
    let chunks = split_test_text_for_path(Path::new("/tmp/readme.md"), &text, 1_200, 200)
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
        None,
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
        None,
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
