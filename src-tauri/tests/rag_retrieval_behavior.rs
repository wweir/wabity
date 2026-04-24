use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::{fs, net::TcpListener, sync::Mutex as AsyncMutex, task::JoinHandle};
use wabity_lib::rag_backend::{
    search_chunks, LlmModelType, LlmProviderConfig, LlmProviderProtocol, LlmSettings,
    RagIndexService, RagSettings,
};

fn rag_integration_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

#[derive(Clone, Copy)]
enum EmbeddingScenario {
    PreferSpecificFile,
    DiversifyAcrossFiles,
    GenericBoilerplateWins,
    RewritePrefersFocusedQuery,
    UniformNoise,
}

#[derive(Clone)]
struct MockEmbeddingState {
    requests: Arc<Mutex<Vec<Value>>>,
    scenario: EmbeddingScenario,
}

fn temp_test_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("wabity-rag-retrieval-{label}-{unique}"))
}

fn build_embedding_provider(base_url: String) -> LlmProviderConfig {
    let provider: LlmProviderConfig = serde_json::from_value(json!({
        "id": "embedding-provider",
        "name": "Embedding Provider",
        "baseUrl": base_url,
        "apiKey": "test-key",
        "protocol": "responses",
        "models": [{
            "id": "embedding-provider",
            "modelType": "embedding",
            "model": "mock-embedding-model"
        }]
    }))
    .expect("failed to deserialize embedding provider");
    assert_eq!(provider.models[0].model_type, LlmModelType::Embedding);
    assert_eq!(provider.protocol, LlmProviderProtocol::Responses);
    provider
}

fn build_llm_settings(base_url: String) -> LlmSettings {
    serde_json::from_value(json!({
        "providers": [build_embedding_provider(base_url)]
    }))
    .expect("failed to deserialize embedding llm settings")
}

fn build_rag_settings(source_root: &Path) -> RagSettings {
    RagSettings {
        source_directories: vec![source_root.to_string_lossy().into_owned()],
        ignore_globs: vec![],
        embedding_model_id: Some("embedding-provider".to_string()),
    }
}

async fn spawn_embedding_server(
    scenario: EmbeddingScenario,
) -> (String, Arc<Mutex<Vec<Value>>>, JoinHandle<()>) {
    let state = MockEmbeddingState {
        requests: Arc::new(Mutex::new(Vec::new())),
        scenario,
    };
    let shared_requests = state.requests.clone();
    let app = Router::new()
        .route("/v1/embeddings", post(mock_embeddings))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind mock embedding server");
    let address = listener
        .local_addr()
        .expect("failed to read mock embedding server address");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock embedding server exited unexpectedly");
    });

    (format!("http://{address}/v1"), shared_requests, handle)
}

async fn mock_embeddings(
    State(state): State<MockEmbeddingState>,
    Json(payload): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state
        .requests
        .lock()
        .expect("failed to lock embedding request log")
        .push(payload.clone());

    let inputs = payload
        .get("input")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let data = inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let text = input.as_str().expect("embedding inputs should be strings");
            json!({
                "index": index,
                "embedding": embedding_for(state.scenario, text),
            })
        })
        .collect::<Vec<_>>();

    (StatusCode::OK, Json(json!({ "data": data })))
}

fn embedding_for(scenario: EmbeddingScenario, text: &str) -> Vec<f32> {
    let lower = text.to_ascii_lowercase();
    match scenario {
        EmbeddingScenario::PreferSpecificFile => {
            if lower.contains("alpha") || (lower.contains("payment") && lower.contains("timeout")) {
                vec![1.0, 0.0]
            } else if lower.contains("beta") || lower.contains("export") {
                vec![0.0, 1.0]
            } else {
                vec![0.35, 0.35]
            }
        }
        EmbeddingScenario::DiversifyAcrossFiles => {
            if lower.contains("alpha") {
                vec![1.0, 0.0]
            } else {
                vec![0.0, 1.0]
            }
        }
        EmbeddingScenario::GenericBoilerplateWins => {
            if (lower.contains("alpha")
                && lower.contains("timeout")
                && lower.contains("root cause"))
                || lower.contains("general timeout checklist")
            {
                vec![1.0, 0.0]
            } else if lower.contains("alpha") && lower.contains("timeout") {
                vec![0.6, 0.4]
            } else if lower.contains("timeout") {
                vec![0.8, 0.2]
            } else {
                vec![0.0, 1.0]
            }
        }
        EmbeddingScenario::RewritePrefersFocusedQuery => {
            if lower.contains("where is the alpha timeout root cause documented") {
                vec![0.1, 0.9]
            } else if lower.contains("alpha") && lower.contains("timeout") && lower.contains("root")
            {
                vec![1.0, 0.0]
            } else if lower.contains("timeout") || lower.contains("documented") {
                vec![0.35, 0.65]
            } else {
                vec![0.0, 1.0]
            }
        }
        EmbeddingScenario::UniformNoise => vec![0.5, 0.5],
    }
}

async fn write_docs(root: &Path, files: &[(&str, &str)]) {
    fs::create_dir_all(root)
        .await
        .expect("failed to create rag source root");
    for (name, content) in files {
        fs::write(root.join(name), content)
            .await
            .expect("failed to write rag source fixture");
    }
}

async fn build_index_and_search(
    scenario: EmbeddingScenario,
    docs: &[(&str, &str)],
    query: &str,
    top_k: usize,
) -> (
    wabity_lib::rag_backend::RagSearchResult,
    Arc<Mutex<Vec<Value>>>,
    JoinHandle<()>,
) {
    let _guard = rag_integration_lock().lock().await;
    let root = temp_test_root("workspace");
    let data_dir = root.join("data");
    let source_root = root.join("docs");
    write_docs(&source_root, docs).await;

    let (base_url, requests, server_handle) = spawn_embedding_server(scenario).await;
    let llm_settings = build_llm_settings(base_url);
    let rag_settings = build_rag_settings(&source_root);
    let index_service = RagIndexService::new(data_dir.clone());
    index_service
        .scan_sources(&rag_settings, &llm_settings)
        .await
        .expect("rag indexing should succeed");

    let result = search_chunks(&data_dir, query, &rag_settings, &llm_settings, top_k, 0.0)
        .await
        .expect("rag search should succeed");

    (result, requests, server_handle)
}

#[tokio::test]
async fn rag_retrieval_prefers_specific_document_when_embedding_signal_is_clean() {
    let docs = [
        (
            "alpha.md",
            "Alpha payment timeout is handled in billing service with a dedicated recovery path.",
        ),
        (
            "beta.md",
            "Beta export worker focuses on csv generation retries and does not discuss alpha timeouts.",
        ),
        (
            "generic.md",
            "General platform notes and architecture summary without the specific issue details.",
        ),
    ];
    let (result, requests, server_handle) = build_index_and_search(
        EmbeddingScenario::PreferSpecificFile,
        &docs,
        "alpha payment timeout recovery",
        3,
    )
    .await;

    server_handle.abort();

    assert!(!result.hits.is_empty());
    assert!(result.hits[0].absolute_path.ends_with("/alpha.md"));
    assert!(result.hits[0]
        .text
        .to_ascii_lowercase()
        .contains("alpha payment timeout"));

    let request_log = requests.lock().expect("failed to lock embedding requests");
    assert!(request_log.len() >= 2);
}

#[tokio::test]
async fn rag_retrieval_supports_small_corpora_without_vector_index_training_failure() {
    let docs = [
        (
            "small-a.md",
            "Alpha payment timeout root cause is in the billing lock path.",
        ),
        (
            "small-b.md",
            "Beta export worker retries are unrelated to alpha payment timeouts.",
        ),
    ];
    let (result, _requests, server_handle) = build_index_and_search(
        EmbeddingScenario::PreferSpecificFile,
        &docs,
        "alpha payment timeout root cause",
        2,
    )
    .await;

    server_handle.abort();

    assert!(!result.hits.is_empty());
    assert!(result.hits[0].absolute_path.ends_with("/small-a.md"));
}

#[tokio::test]
async fn rag_retrieval_diversifies_results_across_files_when_secondary_evidence_exists() {
    let dominant_text = format!(
        "# Alpha Overview\n\n{}\n\n## More\n\n{}\n",
        "alpha architecture details ".repeat(90),
        "alpha architecture evidence ".repeat(90)
    );
    let docs = [
        ("dominant.md", dominant_text.as_str()),
        (
            "secondary.md",
            "Alpha architecture fallback notes from a separate file.",
        ),
        (
            "noise.md",
            "beta topic that should stay far away from alpha retrieval.",
        ),
    ];
    let (result, _requests, server_handle) = build_index_and_search(
        EmbeddingScenario::DiversifyAcrossFiles,
        &docs,
        "alpha architecture",
        4,
    )
    .await;

    server_handle.abort();

    assert!(result.hits.len() >= 3);
    assert!(
        result
            .hits
            .iter()
            .any(|hit| hit.absolute_path.ends_with("/secondary.md")),
        "优化后应能从较大的 candidate window 和按文件轮转裁剪中保留下一个次优文件"
    );
    assert!(
        result
            .hits
            .iter()
            .all(|hit| !hit.absolute_path.ends_with("/noise.md")),
        "弱相关噪音文件不应再因为 top_k 充足而被一起返回"
    );
}

#[tokio::test]
async fn rag_retrieval_prefers_exact_root_cause_over_generic_boilerplate_after_rerank() {
    let docs = [
        (
            "exact-root-cause.md",
            "Alpha timeout root cause analysis for the cache lock path and the concrete mitigation.",
        ),
        (
            "generic-checklist.md",
            "General timeout checklist for all services. General timeout checklist. General timeout checklist.",
        ),
    ];
    let (result, _requests, server_handle) = build_index_and_search(
        EmbeddingScenario::GenericBoilerplateWins,
        &docs,
        "alpha timeout root cause",
        2,
    )
    .await;

    server_handle.abort();

    assert!(!result.hits.is_empty());
    assert!(
        result.hits[0]
            .absolute_path
            .ends_with("/exact-root-cause.md"),
        "轻量 lexical rerank 应把包含更完整术语约束的证据文档抬到泛化 checklist 之前"
    );
    assert!(
        result
            .hits
            .iter()
            .all(|hit| !hit.absolute_path.ends_with("/generic-checklist.md")),
        "当前查询只需要高关联证据时，不应把泛化 checklist 一并返回"
    );
}

#[tokio::test]
async fn rag_retrieval_query_rewrite_recovers_focused_evidence_from_question_wrapper() {
    let docs = [
        (
            "exact-root-cause.md",
            "Alpha timeout root cause analysis for the cache lock path and the concrete mitigation.",
        ),
        (
            "question-template.md",
            "Where is the timeout issue documented? General checklist for finding docs.",
        ),
    ];
    let (result, requests, server_handle) = build_index_and_search(
        EmbeddingScenario::RewritePrefersFocusedQuery,
        &docs,
        "where is the alpha timeout root cause documented?",
        2,
    )
    .await;

    server_handle.abort();

    assert!(!result.hits.is_empty());
    assert!(
        result.hits[0]
            .absolute_path
            .ends_with("/exact-root-cause.md"),
        "query rewrite 应把问题包装语剥离为更聚焦的 semantic/lexical 查询"
    );

    let request_log = requests.lock().expect("failed to lock embedding requests");
    let embedded_inputs = request_log
        .iter()
        .flat_map(|request| {
            request
                .get("input")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|input| input.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    assert!(embedded_inputs
        .iter()
        .any(|input| input == "alpha timeout root cause"));
}

#[tokio::test]
async fn rag_retrieval_filters_noise_for_strong_chinese_entity_query() {
    let docs = [
        (
            "国富论.md",
            "# 国富论\n\n亚当·斯密的经济学代表作，讨论分工、市场与财富增长。\n",
        ),
        ("从战争中走来.md", "# 从战争中走来\n"),
        (
            "better_zip.lisense.txt",
            "nHYGNLkAvi8uwX4KzanQJyEk1FTSWDQQnmKj3f0V0goz\nGaGGh8g9TOI2+Uq+PcGLrjYszPVXquCmiOHDgikBwj+F\n",
        ),
        ("测试服务器.md", "# 加拿大\n\nssh ats@207.210.46.24\n"),
    ];
    let (result, _requests, server_handle) = build_index_and_search(
        EmbeddingScenario::UniformNoise,
        &docs,
        "国富论 亚当·斯密 经济学",
        10,
    )
    .await;

    server_handle.abort();

    assert_eq!(result.hit_count, 1);
    assert!(result.hits[0].absolute_path.ends_with("/国富论.md"));
    assert!(result.hits[0].text.contains("亚当·斯密"));
}

#[tokio::test]
async fn rag_retrieval_recovers_exact_path_match_when_embeddings_are_flat_noise() {
    let docs = [
        (
            "billing-timeout-playbook.md",
            "Recovery checklist and escalation notes for incident response.",
        ),
        (
            "operations-runbook.md",
            "Recovery checklist and escalation notes for incident response.",
        ),
        (
            "platform-summary.md",
            "General platform summary without the requested document name.",
        ),
    ];
    let (result, _requests, server_handle) = build_index_and_search(
        EmbeddingScenario::UniformNoise,
        &docs,
        "billing timeout playbook",
        3,
    )
    .await;

    server_handle.abort();

    assert!(!result.hits.is_empty());
    assert!(
        result.hits[0]
            .absolute_path
            .ends_with("/billing-timeout-playbook.md"),
        "混合召回应能在向量信号失效时通过 BM25 路径匹配找回精确文档"
    );
}
