use std::{
    collections::HashMap,
    io::{Cursor, Write},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use rusqlite::params;
use usearch::Index;

use super::*;
use crate::services::document_extract::{extract_document_from_bytes, is_supported_document_file};
use text_splitter::TextSplitter;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
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
        models: vec![crate::domain::settings::LlmModelConfig {
            id: id.to_string(),
            model_type: crate::domain::settings::LlmModelType::Embedding,
            model: model.to_string(),
            model_identity_hint: model_identity_hint.map(ToOwned::to_owned),
            supports_multimodal: false,
            ..crate::domain::settings::LlmModelConfig::default()
        }],
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

fn test_resolved_config_with_provider(
    root: &Path,
    provider: LlmProviderConfig,
) -> ResolvedRagConfig {
    let embedding_fingerprint =
        embedding_fingerprint(&provider).expect("test embedding fingerprint should resolve");
    ResolvedRagConfig {
        source_roots: vec![root.to_path_buf()],
        ignore_globs: Arc::new(None),
        embedding_fingerprint,
        provider,
    }
}

fn test_runtime_inputs(
    embedding_model_id: Option<&str>,
    providers: &[(&str, &str)],
) -> RagRuntimeInputs {
    test_runtime_inputs_with_directories(
        vec!["/tmp/docs".to_string()],
        embedding_model_id,
        providers,
    )
}

fn count_active_vector_blobs(database_path: &Path) -> i64 {
    let connection =
        open_vector_chunk_connection(database_path).expect("open rag chunk database for test");
    connection
        .query_row(
            "SELECT COUNT(*) FROM rag_chunks WHERE chunk_state = 'active' AND vector_blob IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("count active vector blobs")
}

fn test_serialize_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len().saturating_mul(std::mem::size_of::<f32>()));
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn remove_vector_from_index_for_test(database_path: &Path, vector_key: u64) {
    let dimensions = load_active_vector_dimensions(database_path)
        .expect("load active vector dimensions")
        .unwrap_or(1);
    let options = build_usearch_index_options(dimensions);
    let index = Index::new(&options).expect("create mutable usearch index");
    let index_path = vector_index_file_path(database_path);
    index
        .load(index_path.to_string_lossy().as_ref())
        .expect("load usearch index for test mutation");
    index
        .remove(vector_key)
        .expect("remove vector from usearch index");
    index
        .save(index_path.to_string_lossy().as_ref())
        .expect("save mutated usearch index");
}

fn rewrite_vector_in_index_for_test(database_path: &Path, vector_key: u64, vector: &[f32]) {
    let dimensions = load_active_vector_dimensions(database_path)
        .expect("load active vector dimensions")
        .unwrap_or(1);
    let options = build_usearch_index_options(dimensions);
    let index = Index::new(&options).expect("create mutable usearch index");
    let index_path = vector_index_file_path(database_path);
    index
        .load(index_path.to_string_lossy().as_ref())
        .expect("load usearch index for test mutation");
    index
        .remove(vector_key)
        .expect("remove vector before test rewrite");
    index
        .add(vector_key, vector)
        .expect("add rewritten vector into usearch index");
    index
        .save(index_path.to_string_lossy().as_ref())
        .expect("save rewritten usearch index");
}

fn load_vector_index_manifest_for_test(database_path: &Path) -> VectorIndexManifest {
    load_vector_index_manifest(database_path)
        .expect("load vector index manifest")
        .expect("vector index manifest should exist")
}

fn write_vector_index_manifest_for_test(database_path: &Path, manifest: &VectorIndexManifest) {
    let path = vector_index_manifest_path(database_path);
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(manifest).expect("serialize vector index manifest"),
    )
    .expect("write vector index manifest");
}

fn manifest_version_for_test() -> u32 {
    3
}

fn index_md5_hex_for_test(path: &Path) -> String {
    format!(
        "{:x}",
        md5::compute(std::fs::read(path).expect("read vector index for test digest"))
    )
}

fn file_modified_at_ms_for_test(path: &Path) -> u64 {
    let modified = std::fs::metadata(path)
        .expect("stat file for test mtime")
        .modified()
        .expect("read file mtime for test")
        .duration_since(UNIX_EPOCH)
        .expect("test file mtime should be after unix epoch");
    u64::try_from(modified.as_millis()).expect("test mtime should fit into u64")
}

fn test_runtime_inputs_with_directories(
    source_directories: Vec<String>,
    embedding_model_id: Option<&str>,
    providers: &[(&str, &str)],
) -> RagRuntimeInputs {
    test_runtime_inputs_with_provider_targets(
        source_directories,
        embedding_model_id,
        &providers
            .iter()
            .map(|(id, model)| (*id, "https://api.example.com/v1", *model, None))
            .collect::<Vec<_>>(),
    )
}

fn test_runtime_inputs_with_provider_targets(
    source_directories: Vec<String>,
    embedding_model_id: Option<&str>,
    providers: &[(&str, &str, &str, Option<&str>)],
) -> RagRuntimeInputs {
    RagRuntimeInputs::from_settings(
        &RagSettings {
            source_directories,
            ignore_globs: vec![],
            embedding_model_id: embedding_model_id.map(ToOwned::to_owned),
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
fn collect_chunks_for_path_supports_text_files_under_plain_text_limit() {
    let root = temp_test_root("large-file");
    let file_path = root.join("large.txt");
    let content = "alpha beta gamma delta epsilon zeta eta theta iota kappa\n".repeat(20_000);
    std::fs::create_dir_all(&root).expect("create rag temp root");
    std::fs::write(&file_path, &content).expect("write rag source file");

    let resolved = test_resolved_config(&root);

    let file_size = std::fs::metadata(&file_path).expect("read metadata").len();
    assert!(file_size > 1_000_000);
    assert!(file_size < MAX_TEXT_FILE_BYTES_PLAIN_TEXT);

    let chunks = collect_chunks_for_path(&resolved, &file_path)
        .expect("collecting chunks should succeed for large files");

    assert!(!chunks.is_empty());
    assert!(chunks.len() > 1);

    let _ = std::fs::remove_file(&file_path);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn collect_chunks_for_path_supports_docx_files() {
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
    let root = temp_test_root("docx-file");
    let file_path = root.join("notes.docx");
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
fn collect_chunks_for_path_skips_text_files_over_plain_text_limit() {
    let root = temp_test_root("too-large-file");
    let file_path = root.join("too-large.txt");
    std::fs::create_dir_all(&root).expect("create rag temp root");
    let file = std::fs::File::create(&file_path).expect("create oversized rag source file");
    file.set_len(MAX_TEXT_FILE_BYTES_PLAIN_TEXT + 1)
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

#[tokio::test]
async fn resolve_chunk_vectors_reuses_cached_vectors_before_requesting_embeddings() {
    let root = temp_test_root("cached-vectors");
    let database_path = root.join("rag-index");
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
    let mut vector_store = RagVectorStore::open(&database_path)
        .await
        .expect("open vector store");
    vector_store
        .add_chunks(&[cached_chunk], &[vec![1.0_f32, 2.0_f32]])
        .await
        .expect("create cache rag table");
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
        models: vec![crate::domain::settings::LlmModelConfig {
            id: "embedding".to_string(),
            model_type: crate::domain::settings::LlmModelType::Embedding,
            model: "test-embedding".to_string(),
            supports_multimodal: false,
            ..crate::domain::settings::LlmModelConfig::default()
        }],
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

#[tokio::test]
async fn request_embedding_inputs_sends_multimodal_payload() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test multimodal embedding listener");
    let base_url = format!("http://{}", listener.local_addr().expect("listener addr"));
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept request");
        let body = read_http_request_body(&mut stream)
            .await
            .expect("read request body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("parse request body");
        let inputs = payload["input"]
            .as_array()
            .expect("embedding input should be an array");

        assert_eq!(payload["model"], "multimodal-embedding-1");
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0][0]["type"], "input_text");
        assert_eq!(inputs[0][0]["text"], "describe this image");
        assert_eq!(inputs[0][1]["type"], "input_image");
        assert_eq!(inputs[0][1]["image_url"], "data:image/png;base64,abc123");

        let response_body = serde_json::to_vec(&serde_json::json!({
            "data": [{ "embedding": [1.0, 2.0] }]
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

    let client = HttpClient::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .expect("build embedding client");
    let provider = LlmProviderConfig {
        id: "embedding".to_string(),
        name: "Embedding".to_string(),
        base_url,
        api_key: String::new(),
        models: vec![crate::domain::settings::LlmModelConfig {
            id: "embedding".to_string(),
            model_type: crate::domain::settings::LlmModelType::Embedding,
            model: "multimodal-embedding-1".to_string(),
            supports_multimodal: true,
            ..crate::domain::settings::LlmModelConfig::default()
        }],
        ..LlmProviderConfig::default()
    };
    let inputs = vec![EmbeddingInput::Multi(vec![
        EmbeddingContentPart::InputText {
            text: "describe this image".to_string(),
        },
        EmbeddingContentPart::InputImage {
            image_url: "data:image/png;base64,abc123".to_string(),
        },
    ])];

    let embeddings = request_embedding_inputs(&client, &provider, &inputs)
        .await
        .expect("multimodal embedding request should succeed");

    assert_eq!(embeddings, vec![vec![1.0, 2.0]]);
    server.await.expect("server task should complete");
}

#[tokio::test]
async fn request_embeddings_with_stats_wraps_text_for_multimodal_embedding_provider() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test multimodal text embedding listener");
    let base_url = format!("http://{}", listener.local_addr().expect("listener addr"));
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept request");
        let body = read_http_request_body(&mut stream)
            .await
            .expect("read request body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("parse request body");
        let inputs = payload["input"]
            .as_array()
            .expect("embedding input should be an array");

        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0][0]["type"], "input_text");
        assert_eq!(inputs[0][0]["text"], "alpha");

        let response_body = serde_json::to_vec(&serde_json::json!({
            "data": [{ "embedding": [5.0] }]
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

    let client = HttpClient::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .expect("build embedding client");
    let provider = LlmProviderConfig {
        id: "embedding".to_string(),
        name: "Embedding".to_string(),
        base_url,
        api_key: String::new(),
        models: vec![crate::domain::settings::LlmModelConfig {
            id: "embedding".to_string(),
            model_type: crate::domain::settings::LlmModelType::Embedding,
            model: "multimodal-embedding-1".to_string(),
            supports_multimodal: true,
            ..crate::domain::settings::LlmModelConfig::default()
        }],
        ..LlmProviderConfig::default()
    };
    let inputs = vec!["alpha".to_string()];

    let (embeddings, stats) = request_embeddings_with_stats(&client, &provider, &inputs)
        .await
        .expect("multimodal embedding request should succeed");

    assert_eq!(embeddings, vec![vec![5.0]]);
    assert_eq!(stats.largest_successful_batch_size, 1);
    server.await.expect("server task should complete");
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
        RuntimeStatusUpdate::default(),
    )
    .await;
    set_runtime_status_for_generation(
        None,
        &runtime_status,
        &runtime_generation,
        1,
        RagRuntimePhase::Error,
        RuntimeProgress::default(),
        RuntimeStatusUpdate {
            last_error: Some("stale error".to_string()),
            ..RuntimeStatusUpdate::default()
        },
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
