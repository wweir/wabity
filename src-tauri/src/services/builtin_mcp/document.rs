use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::{
    domain::acp::{BuiltinMcpModuleKey, BuiltinMcpModuleStatus},
    services::{
        document_extract::load_readable_document_text,
        rag::{self, load_document_excerpt_for_chunk},
    },
};

use super::{
    build_tool_error_result, build_tool_success_result, execute_with_timeout,
    BuiltinMcpRuntimeConfig, BuiltinMcpToolDefinition,
};

pub(super) const READ_FILE_TOOL_NAME: &str = "wabity.read_file_lines";
pub(super) const READ_DOCUMENT_EXCERPT_TOOL_NAME: &str = "wabity.read_document_excerpt";
const MAX_READ_FILE_LINES: usize = 240;
const MAX_READ_FILE_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadFileToolLine {
    number: usize,
    text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadFileToolResult {
    absolute_path: String,
    path: String,
    line_start: usize,
    line_end: usize,
    line_count: usize,
    lines: Vec<ReadFileToolLine>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadDocumentExcerptToolResult {
    absolute_path: String,
    path: String,
    document_kind: String,
    chunk_index: i32,
    line_start: Option<i32>,
    line_end: Option<i32>,
    paragraph_line_start: Option<i32>,
    page_start: Option<i32>,
    page_end: Option<i32>,
    heading_path: Vec<String>,
    anchor_label: Option<String>,
    text: String,
}

pub(super) fn module_status() -> BuiltinMcpModuleStatus {
    BuiltinMcpModuleStatus {
        key: BuiltinMcpModuleKey::Document,
        title: "文档读取".to_string(),
        summary: "按精确行号或 chunk 读取允许范围内的文本与文档摘录。".to_string(),
        tool_count: 2,
    }
}

pub(super) fn tool_definitions() -> Vec<BuiltinMcpToolDefinition> {
    vec![
        BuiltinMcpToolDefinition {
            name: READ_FILE_TOOL_NAME,
            title: "Wabity Read File Lines",
            description: "Read exact lines from a local text file. Access is restricted to the current workspace root and explicitly configured RAG source roots."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path, ~/ path, or workspace-relative path of the file to read."
                    },
                    "line_start": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "1-based starting line number."
                    },
                    "line_count": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_READ_FILE_LINES,
                        "description": "Number of lines to read."
                    }
                },
                "required": ["path", "line_start", "line_count"]
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "absolutePath": { "type": "string" },
                    "path": { "type": "string" },
                    "lineStart": { "type": "integer" },
                    "lineEnd": { "type": "integer" },
                    "lineCount": { "type": "integer" },
                    "lines": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "number": { "type": "integer" },
                                "text": { "type": "string" }
                            },
                            "required": ["number", "text"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["absolutePath", "path", "lineStart", "lineEnd", "lineCount", "lines"],
                "additionalProperties": false
            }),
        },
        BuiltinMcpToolDefinition {
            name: READ_DOCUMENT_EXCERPT_TOOL_NAME,
            title: "Wabity Read Document Excerpt",
            description: "Read a normalized excerpt for a previously indexed document chunk."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path, ~/ path, or workspace-relative path of the document to inspect."
                    },
                    "chunk_index": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Chunk index returned by retrieval."
                    }
                },
                "required": ["path", "chunk_index"]
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "absolutePath": { "type": "string" },
                    "path": { "type": "string" },
                    "documentKind": { "type": "string" },
                    "chunkIndex": { "type": "integer" },
                    "lineStart": { "type": ["integer", "null"] },
                    "lineEnd": { "type": ["integer", "null"] },
                    "paragraphLineStart": { "type": ["integer", "null"] },
                    "pageStart": { "type": ["integer", "null"] },
                    "pageEnd": { "type": ["integer", "null"] },
                    "headingPath": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "anchorLabel": { "type": ["string", "null"] },
                    "text": { "type": "string" }
                },
                "required": [
                    "absolutePath",
                    "path",
                    "documentKind",
                    "chunkIndex",
                    "lineStart",
                    "lineEnd",
                    "paragraphLineStart",
                    "pageStart",
                    "pageEnd",
                    "headingPath",
                    "anchorLabel",
                    "text"
                ],
                "additionalProperties": false
            }),
        },
    ]
}

pub(super) fn handles_tool(name: &str) -> bool {
    matches!(name, READ_FILE_TOOL_NAME | READ_DOCUMENT_EXCERPT_TOOL_NAME)
}

pub(super) async fn execute_tool(
    tool_name: &str,
    runtime_config: &BuiltinMcpRuntimeConfig,
    arguments: Option<Value>,
) -> Value {
    let result = match tool_name {
        READ_FILE_TOOL_NAME => execute_read_file_tool(runtime_config, arguments).await,
        READ_DOCUMENT_EXCERPT_TOOL_NAME => {
            execute_read_document_excerpt_tool(runtime_config, arguments).await
        }
        _ => Err(anyhow::anyhow!("unknown document tool: {tool_name}")),
    };

    match result {
        Ok((structured_content, summary)) => build_tool_success_result(structured_content, summary),
        Err(error) => build_tool_error_result(error.to_string()),
    }
}

async fn execute_read_file_tool(
    runtime_config: &BuiltinMcpRuntimeConfig,
    arguments: Option<Value>,
) -> Result<(Value, String)> {
    let arguments = arguments.context("tool arguments are required")?;
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("read_file_lines.path 不能为空")?;
    let line_start = value_as_usize(&arguments, "line_start")?.max(1);
    let line_count = value_as_usize(&arguments, "line_count")?.clamp(1, MAX_READ_FILE_LINES);
    let resolved_path =
        prepare_readable_document_path(runtime_config, path, "read_file_lines", "文本文件").await?;

    let text = tokio::task::spawn_blocking({
        let resolved_path = resolved_path.clone();
        move || load_readable_document_text(&resolved_path)
    })
    .await
    .context("读取文档文本任务失败")?
    .with_context(|| format!("无法读取可索引文档文本: {}", resolved_path.display()))?;

    let lines = text.lines().collect::<Vec<_>>();
    let start_index = line_start.saturating_sub(1);
    let selected = lines
        .iter()
        .enumerate()
        .skip(start_index)
        .take(line_count)
        .map(|(index, line)| ReadFileToolLine {
            number: index + 1,
            text: (*line).to_string(),
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        bail!(
            "请求的文件范围为空: {}:{}+{}",
            resolved_path.display(),
            line_start,
            line_count
        );
    }

    let line_end = selected
        .last()
        .map(|line| line.number)
        .unwrap_or(line_start);
    let result = ReadFileToolResult {
        absolute_path: resolved_path.to_string_lossy().into_owned(),
        path: resolved_path.to_string_lossy().into_owned(),
        line_start,
        line_end,
        line_count: selected.len(),
        lines: selected,
    };

    Ok((
        serde_json::to_value(&result).unwrap_or_else(|_| json!({})),
        format!("{}:{}-{}", result.path, result.line_start, result.line_end),
    ))
}

async fn execute_read_document_excerpt_tool(
    runtime_config: &BuiltinMcpRuntimeConfig,
    arguments: Option<Value>,
) -> Result<(Value, String)> {
    let arguments = arguments.context("tool arguments are required")?;
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("read_document_excerpt.path 不能为空")?;
    let chunk_index = value_as_i32(&arguments, "chunk_index")?;
    if chunk_index < 0 {
        bail!("read_document_excerpt.chunk_index 不能为负数");
    }

    let resolved_path =
        prepare_readable_document_path(runtime_config, path, "read_document_excerpt", "已索引文档")
            .await?;

    let excerpt = execute_with_timeout(
        READ_DOCUMENT_EXCERPT_TOOL_NAME,
        super::BUILTIN_MCP_TOOL_EXECUTION_TIMEOUT,
        async {
            tokio::task::spawn_blocking({
                let resolved_path = resolved_path.clone();
                move || load_document_excerpt_for_chunk(&resolved_path, chunk_index)
            })
            .await
            .context("读取文档摘录任务失败")?
            .with_context(|| format!("无法读取文档摘录: {}", resolved_path.display()))
        },
    )
    .await
    .map_err(anyhow::Error::msg)?;

    let result = ReadDocumentExcerptToolResult {
        absolute_path: resolved_path.to_string_lossy().into_owned(),
        path: resolved_path.to_string_lossy().into_owned(),
        document_kind: excerpt.document_kind.as_str().to_string(),
        chunk_index: excerpt.chunk_index,
        line_start: excerpt.line_start,
        line_end: excerpt.line_end,
        paragraph_line_start: excerpt.paragraph_line_start,
        page_start: excerpt.page_start,
        page_end: excerpt.page_end,
        heading_path: excerpt.heading_path,
        anchor_label: excerpt.anchor_label,
        text: excerpt.text,
    };

    let location_summary =
        if let (Some(page_start), Some(page_end)) = (result.page_start, result.page_end) {
            if page_start == page_end {
                format!("page {page_start}")
            } else {
                format!("pages {page_start}-{page_end}")
            }
        } else if let (Some(line_start), Some(line_end)) = (result.line_start, result.line_end) {
            format!("lines {line_start}-{line_end}")
        } else {
            format!("chunk {}", result.chunk_index)
        };

    Ok((
        serde_json::to_value(&result).unwrap_or_else(|_| json!({})),
        format!("{} · {}", result.path, location_summary),
    ))
}

async fn prepare_readable_document_path(
    runtime_config: &BuiltinMcpRuntimeConfig,
    path: &str,
    tool_name: &str,
    target_label: &str,
) -> Result<PathBuf> {
    let allowed_roots = rag::collect_document_access_roots(
        Path::new(&runtime_config.workspace_root),
        &runtime_config.rag_settings,
    );
    let resolved_path = resolve_readable_file_path(path, &allowed_roots)?;
    let metadata = fs::metadata(&resolved_path)
        .await
        .with_context(|| format!("无法读取文件 metadata: {}", resolved_path.display()))?;
    if metadata.len() > MAX_READ_FILE_BYTES {
        bail!(
            "文件过大，{tool_name} 只允许读取不超过 {} KB 的{target_label}",
            MAX_READ_FILE_BYTES / 1024
        );
    }

    Ok(resolved_path)
}

fn resolve_readable_file_path(path: &str, allowed_roots: &[PathBuf]) -> Result<PathBuf> {
    let candidate = if let Some(remainder) = path.strip_prefix("~/") {
        dirs::home_dir()
            .map(|home| home.join(remainder))
            .context("无法展开 ~/ 路径，因为 HOME 不可用")?
    } else {
        let candidate = PathBuf::from(path);
        if candidate.is_absolute() {
            candidate
        } else {
            allowed_roots
                .first()
                .cloned()
                .context("当前没有可访问的文档根目录")?
                .join(candidate)
        }
    };
    let resolved = candidate
        .canonicalize()
        .with_context(|| format!("无法解析文件路径: {}", candidate.display()))?;
    ensure_file(&resolved)?;
    if !rag::path_is_within_roots(&resolved, allowed_roots) {
        bail!(
            "文件路径超出允许范围，只能读取当前 workspace 或显式配置的 RAG 目录: {}",
            resolved.display()
        );
    }
    Ok(resolved)
}

fn ensure_file(path: &Path) -> Result<()> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("无法读取文件 metadata: {}", path.display()))?;
    if metadata.is_dir() {
        bail!("目标不是文件: {}", path.display());
    }
    Ok(())
}

fn value_as_usize(arguments: &Value, key: &str) -> Result<usize> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .with_context(|| format!("{key} 必须是非负整数"))
}

fn value_as_i32(arguments: &Value, key: &str) -> Result<i32> {
    arguments
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .with_context(|| format!("{key} 必须是整数"))
}
