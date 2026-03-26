use std::{fmt::Write as _, path::PathBuf, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{
        header::{self, HeaderMap, HeaderName, HeaderValue},
        Response, StatusCode,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock as AsyncRwLock;

use crate::{
    domain::{
        acp::{AcpMcpServerConfig, AcpMcpServerHttpConfig},
        rag::BuiltinRagMcpServerStatus,
    },
    infrastructure::config::ConfigStore,
    services::rag_query::{self, RagSearchResult},
};

const JSON_RPC_VERSION: &str = "2.0";
const CURRENT_PROTOCOL_VERSION: &str = "2025-06-18";
const LEGACY_PROTOCOL_VERSION: &str = "2025-03-26";
const MCP_PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
const INTERNAL_HTTP_HOST: &str = "127.0.0.1";
const INTERNAL_HTTP_PORT: u16 = 43189;
const RAG_MCP_PATH: &str = "/internal/mcp/rag";
pub const RAG_MCP_SERVER_NAME: &str = "Wabity RAG Query";
const RAG_MCP_TOOL_NAME: &str = "wabity.rag.search";
const MAX_HTTP_BODY_BYTES: usize = 512 * 1024;
const DEFAULT_TOOL_TOP_K: usize = 8;
const DEFAULT_TOOL_MIN_SCORE: f32 = 0.35;
const MAX_TOOL_TOP_K: usize = 20;

type HttpResponseResult<T> = std::result::Result<T, Box<Response<Body>>>;

#[derive(Clone)]
pub struct RagMcpServerService {
    inner: Arc<RagMcpServerInner>,
}

struct RagMcpServerInner {
    data_dir: PathBuf,
    config_store: Arc<AsyncRwLock<ConfigStore>>,
    status: Arc<AsyncRwLock<BuiltinRagMcpServerStatus>>,
}

#[derive(Clone)]
struct RagMcpRequestState {
    data_dir: PathBuf,
    config_store: Arc<AsyncRwLock<ConfigStore>>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializeParams {
    protocol_version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolsListParams {
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CallToolParams {
    name: String,
    #[serde(default)]
    arguments: Option<Value>,
}

#[derive(Debug)]
struct RagSearchToolInput {
    query: String,
    top_k: usize,
    min_score: f32,
}

impl RagMcpServerService {
    pub fn new(data_dir: PathBuf, config_store: Arc<AsyncRwLock<ConfigStore>>) -> Self {
        Self {
            inner: Arc::new(RagMcpServerInner {
                data_dir,
                config_store,
                status: Arc::new(AsyncRwLock::new(BuiltinRagMcpServerStatus {
                    server: builtin_server_config(),
                    running: false,
                    last_error: None,
                })),
            }),
        }
    }

    pub async fn start(&self) {
        let listener =
            match tokio::net::TcpListener::bind((INTERNAL_HTTP_HOST, INTERNAL_HTTP_PORT)).await {
                Ok(listener) => listener,
                Err(error) => {
                    self.set_error(format!(
                    "failed to bind internal HTTP server on {}:{} for built-in RAG MCP: {error}",
                    INTERNAL_HTTP_HOST, INTERNAL_HTTP_PORT
                ))
                    .await;
                    tracing::warn!(?error, "failed to start built-in RAG MCP server");
                    return;
                }
            };

        self.update_status(true, None).await;

        let router = Router::new()
            .route(RAG_MCP_PATH, get(handle_get).post(handle_post))
            .with_state(Arc::new(RagMcpRequestState {
                data_dir: self.inner.data_dir.clone(),
                config_store: self.inner.config_store.clone(),
            }));
        let status = self.inner.status.clone();

        tauri::async_runtime::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                tracing::warn!(?error, "built-in RAG MCP server exited unexpectedly");
                let mut guard = status.write().await;
                guard.running = false;
                guard.last_error = Some(format!(
                    "built-in RAG MCP server exited unexpectedly: {error}"
                ));
            }
        });
    }

    pub async fn status(&self) -> BuiltinRagMcpServerStatus {
        self.inner.status.read().await.clone()
    }

    async fn set_error(&self, error: String) {
        self.update_status(false, Some(error)).await;
    }

    async fn update_status(&self, running: bool, last_error: Option<String>) {
        let mut status = self.inner.status.write().await;
        status.running = running;
        status.last_error = last_error;
    }
}

pub fn builtin_server_config() -> AcpMcpServerConfig {
    AcpMcpServerConfig::Http(AcpMcpServerHttpConfig {
        name: RAG_MCP_SERVER_NAME.to_string(),
        url: builtin_server_url(),
        headers: Vec::new(),
    })
}

pub fn builtin_server_url() -> String {
    format!(
        "http://{}:{}{}",
        INTERNAL_HTTP_HOST, INTERNAL_HTTP_PORT, RAG_MCP_PATH
    )
}

async fn handle_get(headers: HeaderMap) -> Response<Body> {
    if let Err(response) = validate_local_http_request(&headers) {
        return *response;
    }

    text_response(
        StatusCode::METHOD_NOT_ALLOWED,
        "This MCP endpoint only supports POST JSON-RPC requests. SSE streaming is not enabled.",
    )
}

async fn handle_post(
    State(state): State<Arc<RagMcpRequestState>>,
    headers: HeaderMap,
    body: Body,
) -> Response<Body> {
    if let Err(response) = validate_local_http_request(&headers) {
        return *response;
    }

    let body = match to_bytes(body, MAX_HTTP_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => {
            return text_response(
                StatusCode::BAD_REQUEST,
                &format!("invalid request body: {error}"),
            );
        }
    };

    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(payload) => payload,
        Err(error) => {
            return json_rpc_error_response(
                StatusCode::BAD_REQUEST,
                Value::Null,
                -32700,
                format!("invalid JSON payload: {error}"),
                None,
                CURRENT_PROTOCOL_VERSION,
            );
        }
    };

    if payload.is_array() {
        return json_rpc_error_response(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32600,
            "JSON-RPC batch requests are not supported",
            None,
            CURRENT_PROTOCOL_VERSION,
        );
    }

    let request = match serde_json::from_value::<JsonRpcRequest>(payload) {
        Ok(request) => request,
        Err(error) => {
            return json_rpc_error_response(
                StatusCode::BAD_REQUEST,
                Value::Null,
                -32600,
                format!("invalid JSON-RPC request: {error}"),
                None,
                CURRENT_PROTOCOL_VERSION,
            );
        }
    };

    if request.jsonrpc != JSON_RPC_VERSION {
        return json_rpc_error_response(
            StatusCode::BAD_REQUEST,
            request.id.unwrap_or(Value::Null),
            -32600,
            "jsonrpc must equal \"2.0\"",
            None,
            CURRENT_PROTOCOL_VERSION,
        );
    }

    let protocol_version = match negotiated_protocol_version(&request, &headers) {
        Ok(version) => version,
        Err(response) => return *response,
    };

    let Some(request_id) = request.id.clone() else {
        return handle_notification(request, protocol_version);
    };

    match request.method.as_str() {
        "initialize" => {
            let params = match parse_initialize_params(request.params) {
                Ok(params) => params,
                Err(response) => return *response,
            };
            let negotiated = match select_initialize_protocol(&params.protocol_version) {
                Ok(version) => version,
                Err(response) => return *response,
            };

            json_rpc_success_response(
                request_id,
                json!({
                    "protocolVersion": negotiated,
                    "capabilities": {
                        "tools": {
                            "listChanged": false
                        }
                    },
                    "serverInfo": {
                        "name": "wabity-rag-mcp",
                        "title": RAG_MCP_SERVER_NAME,
                        "version": env!("WABITY_APP_VERSION"),
                    },
                    "instructions": "Use wabity.rag.search to query Wabity's local LanceDB vector index. It returns indexed chunks with path, score, and chunk text. If the index is still building or unconfigured, the tool returns an explicit error payload instead of guessing."
                }),
                negotiated,
            )
        }
        "ping" => json_rpc_success_response(request_id, json!({}), protocol_version),
        "tools/list" => {
            let params = match parse_tools_list_params(request.params) {
                Ok(params) => params,
                Err(response) => return *response,
            };
            if params
                .cursor
                .as_deref()
                .is_some_and(|cursor| !cursor.trim().is_empty())
            {
                return json_rpc_error_response(
                    StatusCode::BAD_REQUEST,
                    request_id,
                    -32602,
                    "tools/list does not support cursor pagination",
                    None,
                    protocol_version,
                );
            }

            json_rpc_success_response(
                request_id,
                json!({
                    "tools": [
                        {
                            "name": RAG_MCP_TOOL_NAME,
                            "title": "Wabity RAG Search",
                            "description": "Search Wabity's local LanceDB vector index and return the nearest indexed chunks with path, score, and chunk text.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "query": {
                                        "type": "string",
                                        "description": "Natural-language query used to search the local vector index."
                                    },
                                    "topK": {
                                        "type": "integer",
                                        "minimum": 1,
                                        "maximum": MAX_TOOL_TOP_K,
                                        "default": DEFAULT_TOOL_TOP_K,
                                        "description": "Maximum number of chunks to return."
                                    },
                                    "minScore": {
                                        "type": "number",
                                        "minimum": 0.0,
                                        "maximum": 1.0,
                                        "default": DEFAULT_TOOL_MIN_SCORE,
                                        "description": "Discard hits whose normalized similarity score is below this threshold. The default keeps only relatively high-confidence matches."
                                    }
                                },
                                "required": ["query"],
                                "additionalProperties": false
                            },
                            "outputSchema": {
                                "type": "object",
                                "properties": {
                                    "query": { "type": "string" },
                                    "hitCount": { "type": "integer" },
                                    "pendingIndexing": { "type": "boolean" },
                                    "hits": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "sourceRoot": { "type": "string" },
                                                "absolutePath": { "type": "string" },
                                                "path": { "type": "string" },
                                                "chunkIndex": { "type": "integer" },
                                                "lineStart": { "type": "integer" },
                                                "lineEnd": { "type": "integer" },
                                                "paragraphLineStart": { "type": "integer" },
                                                "headingPath": {
                                                    "type": "array",
                                                    "items": { "type": "string" }
                                                },
                                                "text": { "type": "string" },
                                                "distance": { "type": "number" },
                                                "score": { "type": "number" }
                                            },
                                            "required": [
                                                "sourceRoot",
                                                "absolutePath",
                                                "path",
                                                "chunkIndex",
                                                "lineStart",
                                                "lineEnd",
                                                "paragraphLineStart",
                                                "headingPath",
                                                "text",
                                                "distance",
                                                "score"
                                            ],
                                            "additionalProperties": false
                                        }
                                    }
                                },
                                "required": ["query", "hitCount", "pendingIndexing", "hits"],
                                "additionalProperties": false
                            }
                        }
                    ]
                }),
                protocol_version,
            )
        }
        "tools/call" => {
            let params = match parse_call_tool_params(request.params) {
                Ok(params) => params,
                Err(response) => return *response,
            };

            if params.name != RAG_MCP_TOOL_NAME {
                return json_rpc_error_response(
                    StatusCode::BAD_REQUEST,
                    request_id,
                    -32602,
                    format!("unknown tool: {}", params.name),
                    None,
                    protocol_version,
                );
            }

            let tool_input = match parse_rag_search_tool_input(params.arguments) {
                Ok(tool_input) => tool_input,
                Err(message) => {
                    return json_rpc_success_response(
                        request_id,
                        build_tool_error_result(message),
                        protocol_version,
                    );
                }
            };

            let config = match state.config_store.read().await.load().await {
                Ok(config) => config,
                Err(error) => {
                    return json_rpc_success_response(
                        request_id,
                        build_tool_error_result(format!("failed to load Wabity config: {error}")),
                        protocol_version,
                    );
                }
            };

            match rag_query::search_chunks(
                &state.data_dir,
                &tool_input.query,
                &config.rag,
                &config.llm,
                tool_input.top_k,
                tool_input.min_score,
            )
            .await
            {
                Ok(result) => {
                    if result.pending_indexing {
                        json_rpc_success_response(
                            request_id,
                            build_tool_pending_result(result),
                            protocol_version,
                        )
                    } else {
                        json_rpc_success_response(
                            request_id,
                            build_tool_success_result(result),
                            protocol_version,
                        )
                    }
                }
                Err(error) => json_rpc_success_response(
                    request_id,
                    build_tool_error_result(error.to_string()),
                    protocol_version,
                ),
            }
        }
        _ => json_rpc_error_response(
            StatusCode::NOT_FOUND,
            request_id,
            -32601,
            format!("unsupported MCP method: {}", request.method),
            None,
            protocol_version,
        ),
    }
}

fn handle_notification(request: JsonRpcRequest, protocol_version: &'static str) -> Response<Body> {
    match request.method.as_str() {
        "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
        _ => {
            let _ = protocol_version;
            StatusCode::ACCEPTED.into_response()
        }
    }
}

fn parse_initialize_params(params: Option<Value>) -> HttpResponseResult<InitializeParams> {
    let Some(params) = params else {
        return Err(Box::new(json_rpc_error_response(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32602,
            "initialize params are required",
            None,
            CURRENT_PROTOCOL_VERSION,
        )));
    };

    serde_json::from_value(params).map_err(|error| {
        Box::new(json_rpc_error_response(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32602,
            format!("invalid initialize params: {error}"),
            None,
            CURRENT_PROTOCOL_VERSION,
        ))
    })
}

fn parse_tools_list_params(params: Option<Value>) -> HttpResponseResult<ToolsListParams> {
    match params {
        Some(params) => serde_json::from_value(params).map_err(|error| {
            Box::new(json_rpc_error_response(
                StatusCode::BAD_REQUEST,
                Value::Null,
                -32602,
                format!("invalid tools/list params: {error}"),
                None,
                CURRENT_PROTOCOL_VERSION,
            ))
        }),
        None => Ok(ToolsListParams { cursor: None }),
    }
}

fn parse_call_tool_params(params: Option<Value>) -> HttpResponseResult<CallToolParams> {
    let Some(params) = params else {
        return Err(Box::new(json_rpc_error_response(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32602,
            "tools/call params are required",
            None,
            CURRENT_PROTOCOL_VERSION,
        )));
    };

    serde_json::from_value(params).map_err(|error| {
        Box::new(json_rpc_error_response(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32602,
            format!("invalid tools/call params: {error}"),
            None,
            CURRENT_PROTOCOL_VERSION,
        ))
    })
}

fn parse_rag_search_tool_input(
    arguments: Option<Value>,
) -> std::result::Result<RagSearchToolInput, String> {
    let Some(arguments) = arguments else {
        return Err("tool arguments are required".to_string());
    };
    let Value::Object(object) = arguments else {
        return Err("tool arguments must be a JSON object".to_string());
    };

    let query = object
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "query must be a non-empty string".to_string())?
        .to_string();

    let top_k = match object.get("topK") {
        Some(value) => {
            let parsed = value
                .as_u64()
                .ok_or_else(|| "topK must be an integer".to_string())?
                as usize;
            if !(1..=MAX_TOOL_TOP_K).contains(&parsed) {
                return Err(format!("topK must be between 1 and {MAX_TOOL_TOP_K}"));
            }
            parsed
        }
        None => DEFAULT_TOOL_TOP_K,
    };

    let min_score = match object.get("minScore") {
        Some(value) => {
            let parsed = value
                .as_f64()
                .ok_or_else(|| "minScore must be a number".to_string())?
                as f32;
            if !(0.0..=1.0).contains(&parsed) {
                return Err("minScore must be between 0 and 1".to_string());
            }
            parsed
        }
        None => DEFAULT_TOOL_MIN_SCORE,
    };

    Ok(RagSearchToolInput {
        query,
        top_k,
        min_score,
    })
}

fn select_initialize_protocol(requested: &str) -> HttpResponseResult<&'static str> {
    match requested {
        CURRENT_PROTOCOL_VERSION => Ok(CURRENT_PROTOCOL_VERSION),
        LEGACY_PROTOCOL_VERSION => Ok(LEGACY_PROTOCOL_VERSION),
        _ => Err(Box::new(json_rpc_error_response(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32602,
            format!("unsupported MCP protocol version: {requested}"),
            Some(json!({
                "supportedProtocolVersions": [CURRENT_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION]
            })),
            CURRENT_PROTOCOL_VERSION,
        ))),
    }
}

fn negotiated_protocol_version(
    request: &JsonRpcRequest,
    headers: &HeaderMap,
) -> HttpResponseResult<&'static str> {
    if request.method == "initialize" {
        return Ok(CURRENT_PROTOCOL_VERSION);
    }

    let requested = headers
        .get(mcp_protocol_version_header())
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(CURRENT_PROTOCOL_VERSION);

    match requested {
        CURRENT_PROTOCOL_VERSION => Ok(CURRENT_PROTOCOL_VERSION),
        LEGACY_PROTOCOL_VERSION => Ok(LEGACY_PROTOCOL_VERSION),
        _ => Err(Box::new(json_rpc_error_response(
            StatusCode::BAD_REQUEST,
            request.id.clone().unwrap_or(Value::Null),
            -32602,
            format!("unsupported MCP protocol version header: {requested}"),
            Some(json!({
                "supportedProtocolVersions": [CURRENT_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION]
            })),
            CURRENT_PROTOCOL_VERSION,
        ))),
    }
}

fn validate_local_http_request(headers: &HeaderMap) -> HttpResponseResult<()> {
    if let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    {
        if !is_loopback_host(strip_host_port(host)) {
            return Err(Box::new(text_response(
                StatusCode::FORBIDDEN,
                "host header must target a loopback address",
            )));
        }
    }

    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        let parsed = reqwest::Url::parse(origin).map_err(|_| {
            Box::new(text_response(
                StatusCode::FORBIDDEN,
                "origin header must be a valid URL",
            ))
        })?;
        let host = parsed.host_str().unwrap_or_default();
        if !is_loopback_host(host) {
            return Err(Box::new(text_response(
                StatusCode::FORBIDDEN,
                "origin header must use a loopback host",
            )));
        }
    }

    Ok(())
}

fn strip_host_port(host: &str) -> &str {
    if host.starts_with('[') {
        return host
            .strip_prefix('[')
            .and_then(|value| value.split(']').next())
            .unwrap_or(host);
    }

    host.split(':').next().unwrap_or(host)
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn build_tool_success_result(result: RagSearchResult) -> Value {
    let structured_content = serde_json::to_value(&result).unwrap_or_else(|_| json!({}));
    json!({
        "content": [
            {
                "type": "text",
                "text": build_search_summary_text(&result)
            },
            {
                "type": "text",
                "text": serde_json::to_string_pretty(&structured_content).unwrap_or_else(|_| "{}".to_string())
            }
        ],
        "structuredContent": structured_content,
        "isError": false
    })
}

fn build_tool_pending_result(result: RagSearchResult) -> Value {
    let message = if result.hit_count > 0 {
        format!(
            "RAG 索引仍在构建中，当前 {} 条命中只是部分结果，不能当成稳定事实来源。",
            result.hit_count
        )
    } else {
        "RAG 索引仍在构建中，当前不能返回稳定检索结果。".to_string()
    };
    let structured_content = json!({
        "error": message,
        "pendingIndexing": true,
        "partialResult": result,
    });
    json!({
        "content": [
            {
                "type": "text",
                "text": message
            },
            {
                "type": "text",
                "text": serde_json::to_string_pretty(&structured_content).unwrap_or_else(|_| "{}".to_string())
            }
        ],
        "structuredContent": structured_content,
        "isError": true
    })
}

fn build_tool_error_result(message: impl Into<String>) -> Value {
    let message = message.into();
    let structured_content = json!({
        "error": message,
    });
    json!({
        "content": [
            {
                "type": "text",
                "text": message
            },
            {
                "type": "text",
                "text": serde_json::to_string_pretty(&structured_content).unwrap_or_else(|_| "{}".to_string())
            }
        ],
        "structuredContent": structured_content,
        "isError": true
    })
}

fn build_search_summary_text(result: &RagSearchResult) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "Retrieved {} hit(s) for query {:?}.",
        result.hit_count, result.query
    );
    if result.pending_indexing && result.hit_count == 0 {
        let _ = writeln!(
            text,
            "The RAG index still has pending files, so empty results may be temporary."
        );
    }

    for (index, hit) in result.hits.iter().enumerate() {
        let _ = writeln!(
            text,
            "\n[{}] {} (chunk {}, lines {}-{}, paragraph {}, score {:.4}, distance {:.4})\n{}",
            index + 1,
            hit.path,
            hit.chunk_index,
            hit.line_start,
            hit.line_end,
            hit.paragraph_line_start,
            hit.score,
            hit.distance,
            hit.text
        );
    }

    text.trim().to_string()
}

fn json_rpc_success_response(id: Value, result: Value, protocol_version: &str) -> Response<Body> {
    json_response(
        StatusCode::OK,
        json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "result": result,
        }),
        protocol_version,
    )
}

fn json_rpc_error_response(
    status: StatusCode,
    id: Value,
    code: i64,
    message: impl Into<String>,
    data: Option<Value>,
    protocol_version: &str,
) -> Response<Body> {
    let message = message.into();
    let error = match data {
        Some(data) => json!({
            "code": code,
            "message": message,
            "data": data,
        }),
        None => json!({
            "code": code,
            "message": message,
        }),
    };

    json_response(
        status,
        json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "error": error,
        }),
        protocol_version,
    )
}

fn json_response(status: StatusCode, payload: Value, protocol_version: &str) -> Response<Body> {
    let mut response = Json(payload).into_response();
    *response.status_mut() = status;
    response.headers_mut().insert(
        mcp_protocol_version_header(),
        HeaderValue::from_str(protocol_version)
            .unwrap_or_else(|_| HeaderValue::from_static(CURRENT_PROTOCOL_VERSION)),
    );
    response
}

fn text_response(status: StatusCode, message: &str) -> Response<Body> {
    let mut response = Response::new(Body::from(message.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

fn mcp_protocol_version_header() -> HeaderName {
    HeaderName::from_static(MCP_PROTOCOL_VERSION_HEADER)
}

#[cfg(test)]
mod tests {
    use super::{
        build_search_summary_text, build_tool_pending_result, parse_rag_search_tool_input,
        select_initialize_protocol, RagSearchResult, RAG_MCP_TOOL_NAME,
    };
    use crate::services::rag_query::RagSearchHit;
    use serde_json::json;

    #[test]
    fn initialize_accepts_supported_versions() {
        assert_eq!(
            select_initialize_protocol("2025-06-18").expect("current protocol should be supported"),
            "2025-06-18"
        );
        assert_eq!(
            select_initialize_protocol("2025-03-26").expect("legacy protocol should be supported"),
            "2025-03-26"
        );
    }

    #[test]
    fn tool_input_rejects_invalid_top_k() {
        let error = parse_rag_search_tool_input(Some(json!({
            "query": "settings",
            "topK": 0,
        })))
        .expect_err("invalid topK should fail");
        assert!(error.contains("topK"));
    }

    #[test]
    fn search_summary_contains_hit_metadata() {
        let summary = build_search_summary_text(&RagSearchResult {
            query: "settings".to_string(),
            hit_count: 1,
            pending_indexing: false,
            hits: vec![RagSearchHit {
                source_root: "/docs".to_string(),
                absolute_path: "/docs/ARCHITECTURE.md".to_string(),
                path: "~/docs/ARCHITECTURE.md".to_string(),
                chunk_index: 2,
                line_start: 10,
                line_end: 18,
                paragraph_line_start: 9,
                heading_path: vec!["Architecture".to_string()],
                text: "launcher and MCP".to_string(),
                distance: 0.2,
                score: 0.8,
            }],
        });

        assert!(summary.contains("ARCHITECTURE.md"));
        assert!(summary.contains("lines 10-18"));
        assert!(summary.contains("score 0.8000"));
        assert!(!RAG_MCP_TOOL_NAME.is_empty());
    }

    #[test]
    fn pending_result_is_reported_as_error_with_partial_hits() {
        let payload = build_tool_pending_result(RagSearchResult {
            query: "settings".to_string(),
            hit_count: 1,
            pending_indexing: true,
            hits: vec![RagSearchHit {
                source_root: "/docs".to_string(),
                absolute_path: "/docs/ARCHITECTURE.md".to_string(),
                path: "~/docs/ARCHITECTURE.md".to_string(),
                chunk_index: 2,
                line_start: 10,
                line_end: 18,
                paragraph_line_start: 9,
                heading_path: vec!["Architecture".to_string()],
                text: "launcher and MCP".to_string(),
                distance: 0.2,
                score: 0.8,
            }],
        });

        assert_eq!(payload["isError"], true);
        assert_eq!(payload["structuredContent"]["pendingIndexing"], true);
        assert_eq!(payload["structuredContent"]["partialResult"]["hitCount"], 1);
    }
}
