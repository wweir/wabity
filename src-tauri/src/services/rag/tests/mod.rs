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

mod chunking;
mod embedding;
mod runtime;
mod sqlite;
mod vector_index;
