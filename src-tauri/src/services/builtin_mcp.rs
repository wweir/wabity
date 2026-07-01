mod document;
mod rag;

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use pi::sdk::{
    default_tool_registry, Config as PiConfig, ContentBlock, TextContent, Tool, ToolFactory,
    ToolOutput, ToolRegistry, ToolUpdate,
};
use serde_json::{json, Value};

use crate::{
    domain::{
        acp::{
            AcpMcpServerConfig, BuiltinAgentToolStatus, BuiltinMcpConfig, BuiltinMcpModuleKey,
            BuiltinMcpModuleStatus,
        },
        settings::{LlmSettings, RagSettings},
    },
    services::tool_timeout::execute_with_timeout,
};

const INTERNAL_HTTP_HOST: &str = "127.0.0.1";
const INTERNAL_HTTP_PORT: u16 = 43189;
const BUILTIN_MCP_PATH: &str = "/internal/mcp";
const LEGACY_RAG_MCP_PATH: &str = "/internal/mcp/rag";
#[cfg(test)]
pub const BUILTIN_MCP_SERVER_NAME: &str = "Wabity Built-in MCP";
pub(super) const BUILTIN_MCP_TOOL_EXECUTION_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Default)]
pub struct BuiltinMcpServerService;

#[derive(Clone)]
pub(super) struct BuiltinMcpRequestState {
    pub(super) data_dir: PathBuf,
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
}

#[derive(Clone)]
pub struct BuiltinMcpToolFactory {
    data_dir: PathBuf,
    runtime_config: Arc<BuiltinMcpRuntimeConfig>,
}

struct BuiltinMcpAgentTool {
    data_dir: PathBuf,
    runtime_config: Arc<BuiltinMcpRuntimeConfig>,
    definition: BuiltinMcpToolDefinition,
}

impl BuiltinMcpToolFactory {
    pub fn new(
        data_dir: PathBuf,
        workspace_root: PathBuf,
        rag_settings: RagSettings,
        llm_settings: LlmSettings,
        builtin_config: BuiltinMcpConfig,
    ) -> Self {
        Self {
            data_dir,
            runtime_config: Arc::new(BuiltinMcpRuntimeConfig {
                workspace_root,
                rag_settings,
                llm_settings,
                builtin_config,
            }),
        }
    }
}

impl ToolFactory for BuiltinMcpToolFactory {
    fn create_tool_registry(
        &self,
        enabled: &[&str],
        cwd: &std::path::Path,
        config: &PiConfig,
    ) -> ToolRegistry {
        let mut registry = default_tool_registry(enabled, cwd, config);
        if self.runtime_config.builtin_config.enabled {
            registry.extend(
                enabled_tool_definitions(&self.runtime_config.builtin_config)
                    .into_iter()
                    .map(|definition| {
                        Box::new(BuiltinMcpAgentTool {
                            data_dir: self.data_dir.clone(),
                            runtime_config: self.runtime_config.clone(),
                            definition,
                        }) as Box<dyn Tool>
                    }),
            );
        }
        registry
    }
}

#[async_trait]
impl Tool for BuiltinMcpAgentTool {
    fn name(&self) -> &str {
        self.definition.name
    }

    fn label(&self) -> &str {
        self.definition.title
    }

    fn description(&self) -> &str {
        &self.definition.description
    }

    fn parameters(&self) -> Value {
        self.definition.input_schema.clone()
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        input: Value,
        _on_update: Option<Box<dyn Fn(ToolUpdate) + Send + Sync>>,
    ) -> pi::sdk::Result<ToolOutput> {
        let request_state = BuiltinMcpRequestState {
            data_dir: self.data_dir.clone(),
        };
        let result = dispatch_tool_call(
            &request_state,
            self.runtime_config.as_ref(),
            self.definition.name,
            Some(input),
        )
        .await;
        Ok(mcp_result_to_tool_output(result))
    }
}

fn mcp_result_to_tool_output(result: Value) -> ToolOutput {
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .map(|text| ContentBlock::Text(TextContent::new(text.to_string())))
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| vec![ContentBlock::Text(TextContent::new(result.to_string()))]);

    let details = result.get("structuredContent").cloned();

    ToolOutput {
        content,
        details,
        is_error,
    }
}

impl BuiltinMcpServerService {
    pub fn new() -> Self {
        Self
    }

    pub async fn status(&self) -> BuiltinAgentToolStatus {
        BuiltinAgentToolStatus {
            available_modules: available_modules(),
        }
    }
}

#[cfg(test)]
pub fn builtin_server_config() -> AcpMcpServerConfig {
    AcpMcpServerConfig::Http(crate::domain::acp::AcpMcpServerHttpConfig {
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

async fn dispatch_tool_call(
    state: &BuiltinMcpRequestState,
    runtime_config: &BuiltinMcpRuntimeConfig,
    tool_name: &str,
    arguments: Option<Value>,
) -> Value {
    let enabled_modules = normalize_enabled_modules(&runtime_config.builtin_config);
    if enabled_modules.contains(&BuiltinMcpModuleKey::Rag) && rag::handles_tool(tool_name) {
        rag::execute_tool(state, runtime_config, arguments).await
    } else if enabled_modules.contains(&BuiltinMcpModuleKey::Document)
        && document::handles_tool(tool_name)
    {
        document::execute_tool(tool_name, runtime_config, arguments).await
    } else {
        let message = if rag::handles_tool(tool_name) || document::handles_tool(tool_name) {
            "tool is currently disabled by Agent tool module configuration".to_string()
        } else {
            format!("unknown tool: {tool_name}")
        };
        build_tool_error_result(message)
    }
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
        "isError": false
    })
}

pub(super) fn build_tool_error_result(message: impl std::fmt::Display) -> Value {
    let message = message.to_string();
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::build_tool_pending_result;

    #[test]
    fn pending_tool_result_is_not_marked_as_error() {
        let result = build_tool_pending_result(
            json!({
                "pendingIndexing": true,
                "partialResult": {
                    "query": "test"
                }
            }),
            "pending".to_string(),
        );

        assert_eq!(result["isError"], json!(false));
        assert_eq!(result["structuredContent"]["pendingIndexing"], json!(true));
    }
}
