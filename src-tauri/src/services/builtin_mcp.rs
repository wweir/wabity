mod document;
mod rag;

use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};

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

use crate::domain::{
    acp::{
        AcpMcpServerConfig, AcpMcpServerHttpConfig, BuiltinMcpConfig, BuiltinMcpModuleKey,
        BuiltinMcpModuleStatus, BuiltinMcpServerStatus,
    },
    settings::{LlmSettings, RagSettings},
};

const JSON_RPC_VERSION: &str = "2.0";
const CURRENT_PROTOCOL_VERSION: &str = "2025-06-18";
const LEGACY_PROTOCOL_VERSION: &str = "2025-03-26";
const MCP_PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
const INTERNAL_HTTP_HOST: &str = "127.0.0.1";
const INTERNAL_HTTP_PORT: u16 = 43189;
const BUILTIN_MCP_PATH: &str = "/internal/mcp";
const LEGACY_RAG_MCP_PATH: &str = "/internal/mcp/rag";
pub const BUILTIN_MCP_SERVER_NAME: &str = "Wabity Built-in MCP";
const BUILTIN_MCP_SERVER_INFO_NAME: &str = "wabity-builtin-mcp";
const MAX_HTTP_BODY_BYTES: usize = 512 * 1024;
pub(super) const BUILTIN_MCP_TOOL_EXECUTION_TIMEOUT: Duration = Duration::from_secs(20);

type HttpResponseResult<T> = std::result::Result<T, Box<Response<Body>>>;

#[derive(Clone)]
pub struct BuiltinMcpServerService {
    inner: Arc<BuiltinMcpServerInner>,
}

struct BuiltinMcpServerInner {
    data_dir: PathBuf,
    runtime_config: Arc<AsyncRwLock<BuiltinMcpRuntimeConfig>>,
    status: Arc<AsyncRwLock<BuiltinMcpServerStatus>>,
}

#[derive(Clone)]
pub(super) struct BuiltinMcpRequestState {
    pub(super) data_dir: PathBuf,
    runtime_config: Arc<AsyncRwLock<BuiltinMcpRuntimeConfig>>,
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

#[derive(Debug, Clone)]
pub(super) struct BuiltinMcpRuntimeConfig {
    pub(super) workspace_root: PathBuf,
    pub(super) rag_settings: RagSettings,
    pub(super) llm_settings: LlmSettings,
    pub(super) builtin_config: BuiltinMcpConfig,
}

#[derive(Debug, Clone)]
pub(super) struct BuiltinMcpToolDefinition {
    pub(super) name: &'static str,
    pub(super) title: &'static str,
    pub(super) description: String,
    pub(super) input_schema: Value,
    pub(super) output_schema: Value,
}

impl BuiltinMcpServerService {
    pub fn new(
        data_dir: PathBuf,
        workspace_root: PathBuf,
        rag_settings: RagSettings,
        llm_settings: LlmSettings,
        builtin_config: BuiltinMcpConfig,
    ) -> Self {
        Self {
            inner: Arc::new(BuiltinMcpServerInner {
                data_dir,
                runtime_config: Arc::new(AsyncRwLock::new(BuiltinMcpRuntimeConfig {
                    workspace_root,
                    rag_settings,
                    llm_settings,
                    builtin_config,
                })),
                status: Arc::new(AsyncRwLock::new(BuiltinMcpServerStatus {
                    server: builtin_server_config(),
                    running: false,
                    last_error: None,
                    available_modules: available_modules(),
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
                        "failed to bind internal HTTP server on {}:{} for built-in MCP: {error}",
                        INTERNAL_HTTP_HOST, INTERNAL_HTTP_PORT
                    ))
                    .await;
                    tracing::warn!(?error, "failed to start built-in MCP server");
                    return;
                }
            };

        self.update_status(true, None).await;

        let router = Router::new()
            .route(BUILTIN_MCP_PATH, get(handle_get).post(handle_post))
            .route(LEGACY_RAG_MCP_PATH, get(handle_get).post(handle_post))
            .with_state(Arc::new(BuiltinMcpRequestState {
                data_dir: self.inner.data_dir.clone(),
                runtime_config: self.inner.runtime_config.clone(),
            }));
        let status = self.inner.status.clone();

        tauri::async_runtime::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                tracing::warn!(?error, "built-in MCP server exited unexpectedly");
                let mut guard = status.write().await;
                guard.running = false;
                guard.last_error =
                    Some(format!("built-in MCP server exited unexpectedly: {error}"));
            }
        });
    }

    pub async fn status(&self) -> BuiltinMcpServerStatus {
        self.inner.status.read().await.clone()
    }

    pub async fn apply_runtime_config(
        &self,
        workspace_root: PathBuf,
        rag_settings: RagSettings,
        llm_settings: LlmSettings,
        builtin_config: BuiltinMcpConfig,
    ) {
        let mut runtime_config = self.inner.runtime_config.write().await;
        runtime_config.workspace_root = workspace_root;
        runtime_config.rag_settings = rag_settings;
        runtime_config.llm_settings = llm_settings;
        runtime_config.builtin_config = builtin_config;
    }

    async fn set_error(&self, error: String) {
        self.update_status(false, Some(error)).await;
    }

    async fn update_status(&self, running: bool, last_error: Option<String>) {
        let mut status = self.inner.status.write().await;
        status.running = running;
        status.last_error = last_error;
        status.available_modules = available_modules();
    }
}

pub fn builtin_server_config() -> AcpMcpServerConfig {
    AcpMcpServerConfig::Http(AcpMcpServerHttpConfig {
        name: BUILTIN_MCP_SERVER_NAME.to_string(),
        url: builtin_server_url(),
        headers: Vec::new(),
    })
}

pub fn builtin_server_url() -> String {
    format!(
        "http://{}:{}{}",
        INTERNAL_HTTP_HOST, INTERNAL_HTTP_PORT, BUILTIN_MCP_PATH
    )
}

pub fn is_builtin_server(server: &AcpMcpServerConfig) -> bool {
    let expected_urls = [builtin_server_url(), legacy_rag_server_url()];
    match server {
        AcpMcpServerConfig::Http(server) => matches_builtin_server_url(&server.url, &expected_urls),
        AcpMcpServerConfig::Sse(server) => matches_builtin_server_url(&server.url, &expected_urls),
        AcpMcpServerConfig::Stdio(_) => false,
    }
}

fn legacy_rag_server_url() -> String {
    format!(
        "http://{}:{}{}",
        INTERNAL_HTTP_HOST, INTERNAL_HTTP_PORT, LEGACY_RAG_MCP_PATH
    )
}

fn matches_builtin_server_url(url: &str, expected_urls: &[String]) -> bool {
    let normalized_url = url.trim().trim_end_matches('/');
    expected_urls
        .iter()
        .any(|expected| normalized_url == expected.trim_end_matches('/'))
}

fn available_modules() -> Vec<BuiltinMcpModuleStatus> {
    vec![rag::module_status(), document::module_status()]
}

fn normalize_enabled_modules(config: &BuiltinMcpConfig) -> Vec<BuiltinMcpModuleKey> {
    if !config.enabled {
        return Vec::new();
    }

    let mut enabled_modules = config.enabled_modules.clone();
    enabled_modules.sort();
    enabled_modules.dedup();
    enabled_modules
}

fn enabled_tool_definitions(config: &BuiltinMcpConfig) -> Vec<BuiltinMcpToolDefinition> {
    let mut tools = Vec::new();
    let enabled_modules = normalize_enabled_modules(config);
    for module_key in enabled_modules {
        match module_key {
            BuiltinMcpModuleKey::Rag => tools.extend(rag::tool_definitions()),
            BuiltinMcpModuleKey::Document => tools.extend(document::tool_definitions()),
        }
    }
    tools
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
    State(state): State<Arc<BuiltinMcpRequestState>>,
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
        return handle_notification(request);
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
            let runtime_config = state.runtime_config.read().await.clone();
            let enabled_modules = normalize_enabled_modules(&runtime_config.builtin_config);

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
                        "name": BUILTIN_MCP_SERVER_INFO_NAME,
                        "title": BUILTIN_MCP_SERVER_NAME,
                        "version": env!("WABITY_APP_VERSION"),
                    },
                    "instructions": build_initialize_instructions(&enabled_modules)
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

            let runtime_config = state.runtime_config.read().await.clone();
            let tools = enabled_tool_definitions(&runtime_config.builtin_config);
            let tools = tools
                .into_iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "title": tool.title,
                        "description": tool.description,
                        "inputSchema": tool.input_schema,
                        "outputSchema": tool.output_schema,
                    })
                })
                .collect::<Vec<_>>();

            json_rpc_success_response(request_id, json!({ "tools": tools }), protocol_version)
        }
        "tools/call" => {
            let params = match parse_call_tool_params(request.params) {
                Ok(params) => params,
                Err(response) => return *response,
            };

            let runtime_config = state.runtime_config.read().await.clone();
            let enabled_modules = normalize_enabled_modules(&runtime_config.builtin_config);
            let enabled_tools = enabled_tool_definitions(&runtime_config.builtin_config);
            let tool_is_enabled = enabled_tools.iter().any(|tool| tool.name == params.name);
            if !tool_is_enabled {
                let message = if rag::handles_tool(&params.name)
                    || document::handles_tool(&params.name)
                {
                    "tool is currently disabled by built-in MCP module configuration".to_string()
                } else {
                    format!("unknown tool: {}", params.name)
                };
                return json_rpc_success_response(
                    request_id,
                    build_tool_error_result(message),
                    protocol_version,
                );
            }

            let tool_result = if enabled_modules.contains(&BuiltinMcpModuleKey::Rag)
                && rag::handles_tool(&params.name)
            {
                rag::execute_tool(&state, &runtime_config, params.arguments).await
            } else if enabled_modules.contains(&BuiltinMcpModuleKey::Document)
                && document::handles_tool(&params.name)
            {
                document::execute_tool(&params.name, &runtime_config, params.arguments).await
            } else {
                build_tool_error_result(format!("unknown tool: {}", params.name))
            };

            json_rpc_success_response(request_id, tool_result, protocol_version)
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

fn build_initialize_instructions(enabled_modules: &[BuiltinMcpModuleKey]) -> String {
    if enabled_modules.is_empty() {
        return "No built-in MCP modules are currently enabled. The endpoint is running, but tools/list will be empty until the user enables at least one built-in module in settings."
            .to_string();
    }

    let module_names = enabled_modules
        .iter()
        .map(|module_key| match module_key {
            BuiltinMcpModuleKey::Rag => "rag search",
            BuiltinMcpModuleKey::Document => "document reading",
        })
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "Use Wabity's built-in MCP tools for {}. All local file access remains restricted to the current workspace root and explicitly configured RAG source roots.",
        module_names
    )
}

fn handle_notification(_request: JsonRpcRequest) -> Response<Body> {
    StatusCode::ACCEPTED.into_response()
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

pub(super) async fn execute_with_timeout<T, F>(
    tool_name: &str,
    timeout_duration: Duration,
    future: F,
) -> std::result::Result<T, String>
where
    F: Future<Output = anyhow::Result<T>>,
{
    match tokio::time::timeout(timeout_duration, future).await {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_) => Err(format!(
            "tool {} timed out after {} ms",
            tool_name,
            timeout_duration.as_millis()
        )),
    }
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

pub(super) fn build_tool_success_result(structured_content: Value, summary: String) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": summary
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

pub(super) fn build_tool_pending_result(structured_content: Value, message: String) -> Value {
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

pub(super) fn build_tool_error_result(message: impl Into<String>) -> Value {
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

fn json_rpc_success_response(
    id: Value,
    result: Value,
    protocol_version: &'static str,
) -> Response<Body> {
    json_rpc_response(
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
    code: i32,
    message: impl Into<String>,
    data: Option<Value>,
    protocol_version: &'static str,
) -> Response<Body> {
    let mut error = json!({
        "code": code,
        "message": message.into(),
    });
    if let Some(data) = data {
        error["data"] = data;
    }

    json_rpc_response(
        status,
        json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "error": error,
        }),
        protocol_version,
    )
}

fn json_rpc_response(
    status: StatusCode,
    payload: Value,
    protocol_version: &'static str,
) -> Response<Body> {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(
        mcp_protocol_version_header(),
        HeaderValue::from_static(protocol_version),
    );
    (status, headers, Json(payload)).into_response()
}

fn text_response(status: StatusCode, text: &str) -> Response<Body> {
    let mut response = Response::new(Body::from(text.to_string()));
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
