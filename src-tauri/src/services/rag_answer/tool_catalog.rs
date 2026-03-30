use std::{
    collections::HashMap,
    env,
    process::Command,
    sync::{Mutex, OnceLock},
};

use serde_json::{json, Value};

use super::{
    HostSystemContext, QuestionAnswerProtocol, ResponsesToolCompatibilityMode, ToolCatalog,
    OPEN_TARGET_TOOL_NAME, RAG_QUERY_TOOL_NAME, READ_DOCUMENT_EXCERPT_TOOL_NAME,
    READ_FILE_TOOL_NAME, RESPONSES_TOOL_COMPATIBILITY_CACHE,
};
use crate::domain::acp::{AcpMcpServerConfig, AcpNameValuePair};

static HOST_SYSTEM_CONTEXT: OnceLock<HostSystemContext> = OnceLock::new();

pub(super) fn build_tool_catalog(
    mcp_servers: &[AcpMcpServerConfig],
    protocol: QuestionAnswerProtocol,
) -> ToolCatalog {
    let mut request_tools = vec![
        build_read_file_tool(protocol),
        build_read_document_excerpt_tool(protocol),
        build_rag_query_tool(protocol),
        build_open_target_tool(protocol),
    ];
    let mut available_names = vec![
        READ_FILE_TOOL_NAME.to_string(),
        READ_DOCUMENT_EXCERPT_TOOL_NAME.to_string(),
        RAG_QUERY_TOOL_NAME.to_string(),
        OPEN_TARGET_TOOL_NAME.to_string(),
    ];
    let mut skipped_mcp_servers = Vec::new();

    for server in mcp_servers {
        if protocol == QuestionAnswerProtocol::ChatCompletions {
            let name = match server {
                AcpMcpServerConfig::Http(server) => &server.name,
                AcpMcpServerConfig::Sse(server) => &server.name,
                AcpMcpServerConfig::Stdio(server) => &server.name,
            };
            skipped_mcp_servers.push(name.clone());
            continue;
        }

        match server {
            AcpMcpServerConfig::Http(server) => {
                available_names.push(format!("mcp:{}", server.name));
                request_tools.push(json!({
                    "type": "mcp",
                    "server_label": server.name,
                    "server_url": server.url,
                    "headers": name_value_pairs_to_json_object(&server.headers),
                    "require_approval": "never",
                }));
            }
            AcpMcpServerConfig::Sse(server) => {
                available_names.push(format!("mcp:{}", server.name));
                request_tools.push(json!({
                    "type": "mcp",
                    "server_label": server.name,
                    "server_url": server.url,
                    "headers": name_value_pairs_to_json_object(&server.headers),
                    "require_approval": "never",
                }));
            }
            AcpMcpServerConfig::Stdio(server) => {
                skipped_mcp_servers.push(server.name.clone());
            }
        }
    }

    ToolCatalog {
        request_tools,
        available_names,
        skipped_mcp_servers,
    }
}

pub(super) fn tool_catalog_without_mcp_tools(tool_catalog: &ToolCatalog) -> ToolCatalog {
    let mut skipped_mcp_servers = tool_catalog.skipped_mcp_servers.clone();
    skipped_mcp_servers.extend(
        tool_catalog
            .request_tools
            .iter()
            .filter(|tool| is_mcp_tool_definition(tool))
            .filter_map(|tool| {
                tool.get("server_label")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            }),
    );

    ToolCatalog {
        request_tools: tool_catalog
            .request_tools
            .iter()
            .filter(|tool| !is_mcp_tool_definition(tool))
            .cloned()
            .collect(),
        available_names: tool_catalog
            .available_names
            .iter()
            .filter(|name| !name.starts_with("mcp:"))
            .cloned()
            .collect(),
        skipped_mcp_servers,
    }
}

pub(super) fn tool_catalog_without_all_tools(tool_catalog: &ToolCatalog) -> ToolCatalog {
    let mut stripped = tool_catalog_without_mcp_tools(tool_catalog);
    stripped.request_tools.clear();
    stripped.available_names.clear();
    stripped
}

pub(super) fn responses_tool_compatibility_cache_key(base_url: &str, model: &str) -> String {
    format!("responses::{base_url}::{model}")
}

pub(super) fn load_cached_responses_tool_compatibility(
    compatibility_cache_key: &str,
) -> ResponsesToolCompatibilityMode {
    responses_tool_compatibility_cache()
        .lock()
        .expect("responses tool compatibility cache lock poisoned")
        .get(compatibility_cache_key)
        .copied()
        .unwrap_or(ResponsesToolCompatibilityMode::Full)
}

pub(super) fn store_cached_responses_tool_compatibility(
    compatibility_cache_key: &str,
    mode: ResponsesToolCompatibilityMode,
) {
    let mut cache = responses_tool_compatibility_cache()
        .lock()
        .expect("responses tool compatibility cache lock poisoned");
    let entry = cache
        .entry(compatibility_cache_key.to_string())
        .or_insert(ResponsesToolCompatibilityMode::Full);
    if mode > *entry {
        *entry = mode;
    }
}

pub(super) fn responses_tool_compatibility_cache(
) -> &'static Mutex<HashMap<String, ResponsesToolCompatibilityMode>> {
    RESPONSES_TOOL_COMPATIBILITY_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn is_mcp_tool_definition(tool: &Value) -> bool {
    tool.get("type").and_then(Value::as_str) == Some("mcp")
}

fn build_read_file_tool(protocol: QuestionAnswerProtocol) -> Value {
    build_function_tool(
        protocol,
        READ_FILE_TOOL_NAME,
        "Read exact lines from a local text file after you already know which file matters. Access is restricted to the current workspace root and explicitly configured RAG source roots. Prefer calling wabity.rag.query first to locate evidence, then use this tool to verify the precise file path and line range you want to cite. Do not use this as a blind file discovery tool.",
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path, ~/ path, or workspace-relative path of the file to read, but it must resolve inside the current workspace root or an explicit RAG source root. Use a concrete path you already identified from retrieval results."
                },
                "line_start": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-based starting line number. Choose the smallest range that still captures the exact evidence you need."
                },
                "line_count": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": super::MAX_READ_FILE_LINES,
                    "description": "Number of lines to read. Keep the window tight; expand only if the first slice is insufficient."
                }
            },
            "required": ["path", "line_start", "line_count"]
        }),
    )
}

fn build_read_document_excerpt_tool(protocol: QuestionAnswerProtocol) -> Value {
    build_function_tool(
        protocol,
        READ_DOCUMENT_EXCERPT_TOOL_NAME,
        "Read a normalized excerpt for a previously retrieved indexed document chunk. Use this for PDFs or other extracted documents whose original file bytes do not map cleanly to line numbers. Pass the concrete path and chunk_index from a prior wabity.rag.query result. Do not use this as a blind document discovery tool.",
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path, ~/ path, or workspace-relative path of the document to inspect. It must resolve inside the current workspace root or an explicit RAG source root."
                },
                "chunk_index": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Chunk index returned by wabity.rag.query for the hit you want to inspect."
                }
            },
            "required": ["path", "chunk_index"]
        }),
    )
}

fn build_rag_query_tool(protocol: QuestionAnswerProtocol) -> Value {
    build_function_tool(
        protocol,
        RAG_QUERY_TOOL_NAME,
        "Search the local RAG index to find likely evidence before answering repository or documentation questions. Use this first when you do not yet know which file or section is relevant. Then follow up with wabity.read_file_lines for text files or wabity.read_document_excerpt for extracted documents such as PDFs before making specific claims.",
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language retrieval query. Write it in terms of the concept, behavior, API, error, or file/topic you need evidence for."
                },
                "top_k": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 20,
                    "description": "How many hits to return. Start small for focused queries; increase only when the first pass is too narrow."
                },
                "min_score": {
                    "type": "number",
                    "minimum": 0.0,
                    "maximum": 1.0,
                    "description": "Minimum similarity score. Lower it only if an initial focused search returns too few relevant hits."
                }
            },
            "required": ["query"]
        }),
    )
}

fn build_open_target_tool(protocol: QuestionAnswerProtocol) -> Value {
    let host_context = current_host_system_context();
    build_function_tool(
        protocol,
        OPEN_TARGET_TOOL_NAME,
        &open_target_tool_description(host_context),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "target": {
                    "type": "string",
                    "description": "The URL, absolute path, ~/ path, or workspace-relative path to open. For local paths, only targets inside the current workspace root or explicit RAG source roots are allowed."
                }
            },
            "required": ["target"]
        }),
    )
}

fn current_host_system_context() -> &'static HostSystemContext {
    HOST_SYSTEM_CONTEXT.get_or_init(detect_host_system_context)
}

fn detect_host_system_context() -> HostSystemContext {
    HostSystemContext {
        os_name: host_os_name().to_string(),
        os_version: host_os_version(),
        package_managers: detect_available_package_managers(),
    }
}

pub(super) fn open_target_tool_description(host_context: &HostSystemContext) -> String {
    let os_summary = host_context
        .os_version
        .as_deref()
        .map(|version| format!("{} {}", host_context.os_name, version))
        .unwrap_or_else(|| host_context.os_name.clone());
    let package_manager_summary = if host_context.package_managers.is_empty() {
        "none detected on PATH".to_string()
    } else {
        host_context.package_managers.join(", ")
    };

    format!(
        "Open a URL, local file, or directory with the operating system default application. Use this only when the current user request explicitly asks you to open something. Local path access is restricted to the current workspace root and explicitly configured RAG source roots. Current host environment: OS={os_summary}; package managers on PATH={package_manager_summary}."
    )
}

fn host_os_name() -> &'static str {
    match env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

fn host_os_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        return run_command_for_single_line("sw_vers", &["-productVersion"]);
    }

    #[cfg(target_os = "linux")]
    {
        return std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|content| {
                content.lines().find_map(|line| {
                    line.strip_prefix("PRETTY_NAME=")
                        .map(trim_quoted_value)
                        .filter(|value| !value.is_empty())
                })
            })
            .or_else(|| run_command_for_single_line("uname", &["-sr"]));
    }

    #[cfg(target_os = "windows")]
    {
        return run_command_for_single_line("cmd", &["/C", "ver"]);
    }

    #[allow(unreachable_code)]
    None
}

fn run_command_for_single_line(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(target_os = "linux")]
fn trim_quoted_value(raw: &str) -> String {
    raw.trim().trim_matches('"').to_string()
}

fn detect_available_package_managers() -> Vec<String> {
    const CANDIDATES: &[&str] = &[
        "brew", "apt", "apt-get", "dnf", "yum", "pacman", "zypper", "nix", "winget", "choco",
        "scoop", "cargo", "npm", "pnpm", "yarn", "bun", "pip", "pip3", "uv", "poetry", "conda",
        "mamba", "gem", "go",
    ];

    CANDIDATES
        .iter()
        .copied()
        .filter(|candidate| command_exists(candidate))
        .map(ToOwned::to_owned)
        .collect()
}

fn command_exists(command: &str) -> bool {
    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };

    let path_exts = executable_suffixes();
    env::split_paths(&path_var).any(|directory| {
        path_exts.iter().any(|suffix| {
            let candidate = if suffix.is_empty() {
                directory.join(command)
            } else {
                directory.join(format!("{command}{suffix}"))
            };
            candidate.is_file()
        })
    })
}

fn executable_suffixes() -> Vec<String> {
    #[cfg(windows)]
    {
        let suffixes = env::var_os("PATHEXT")
            .and_then(|value| value.into_string().ok())
            .map(|value| {
                value
                    .split(';')
                    .filter(|suffix| !suffix.trim().is_empty())
                    .map(|suffix| suffix.to_ascii_lowercase())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![".exe".to_string(), ".cmd".to_string(), ".bat".to_string()]);

        let mut all = vec![String::new()];
        all.extend(suffixes);
        return all;
    }

    #[cfg(not(windows))]
    {
        vec![String::new()]
    }
}

fn build_function_tool(
    protocol: QuestionAnswerProtocol,
    name: &str,
    description: &str,
    parameters: Value,
) -> Value {
    match protocol {
        QuestionAnswerProtocol::Responses => json!({
            "type": "function",
            "name": name,
            "description": description,
            "strict": true,
            "parameters": parameters,
        }),
        QuestionAnswerProtocol::ChatCompletions => json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "strict": true,
                "parameters": parameters,
            },
        }),
    }
}

fn name_value_pairs_to_json_object(pairs: &[AcpNameValuePair]) -> Value {
    let mut object = serde_json::Map::new();
    for pair in pairs {
        object.insert(pair.name.clone(), Value::String(pair.value.clone()));
    }
    Value::Object(object)
}
