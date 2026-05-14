use super::*;

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
