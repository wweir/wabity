use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::{future::Future, path::PathBuf, time::Duration};
use tokio::fs;

use super::{
    parsing::{compact_snippet, display_path, ensure_file, value_as_i32, value_as_usize},
    ExecutedToolCall, LocalToolCall, QuestionToolRuntime, CITATION_SNIPPET_MAX_CHARS,
    DEFAULT_RAG_TOOL_MIN_SCORE, DEFAULT_RAG_TOOL_TOP_K, MAX_READ_FILE_BYTES, MAX_READ_FILE_LINES,
    OPEN_TARGET_TOOL_NAME, RAG_QUERY_TOOL_NAME, READ_DOCUMENT_EXCERPT_TOOL_NAME,
    READ_FILE_TOOL_NAME,
};
use crate::{
    domain::execution::{ExecutionCitation, ExecutionToolCall},
    services::{
        document_extract::load_readable_document_text,
        open_target::{OpenTargetService, OpenedTarget},
        rag,
        rag_query::{self, RagSearchHit},
    },
};

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenTargetToolResult {
    requested_target: String,
    opened: OpenedTarget,
}

pub(super) async fn execute_local_tool_call(
    runtime: &QuestionToolRuntime<'_>,
    call: LocalToolCall,
) -> Result<ExecutedToolCall> {
    let executed = execute_with_local_tool_timeout(&call.name, async {
        match call.name.as_str() {
            READ_FILE_TOOL_NAME => execute_read_file_tool(runtime, &call.arguments).await,
            READ_DOCUMENT_EXCERPT_TOOL_NAME => {
                execute_read_document_excerpt_tool(runtime, &call.arguments).await
            }
            RAG_QUERY_TOOL_NAME => execute_rag_query_tool(runtime, &call.arguments).await,
            OPEN_TARGET_TOOL_NAME => execute_open_target_tool(runtime, &call.arguments).await,
            _ => Err(anyhow::anyhow!("未知内置工具: {}", call.name)),
        }
    })
    .await;

    match executed {
        Ok(mut executed) => {
            executed.call_id = call.call_id;
            Ok(executed)
        }
        Err(error) => Ok(ExecutedToolCall {
            call_id: call.call_id,
            name: call.name.clone(),
            output: serde_json::to_string(&json!({
                "ok": false,
                "error": error.to_string(),
            }))?,
            citations: Vec::new(),
            trace: ExecutionToolCall {
                name: call.name,
                source: "builtin".to_string(),
                status: "error".to_string(),
                summary: error.to_string(),
            },
        }),
    }
}

pub(super) async fn execute_with_timeout<T, F>(
    tool_name: &str,
    timeout_duration: Duration,
    future: F,
) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    match tokio::time::timeout(timeout_duration, future).await {
        Ok(result) => result,
        Err(_) => bail!(
            "内置工具 {} 执行超时（>{} ms）",
            tool_name,
            timeout_duration.as_millis()
        ),
    }
}

async fn execute_with_local_tool_timeout<T, F>(tool_name: &str, future: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    execute_with_timeout(tool_name, super::LOCAL_TOOL_EXECUTION_TIMEOUT, future).await
}

pub(super) async fn execute_read_file_tool(
    runtime: &QuestionToolRuntime<'_>,
    arguments: &Value,
) -> Result<ExecutedToolCall> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("read_file_lines.path 不能为空")?;
    let line_start = value_as_usize(arguments, "line_start")?.max(1);
    let line_count = value_as_usize(arguments, "line_count")?.clamp(1, MAX_READ_FILE_LINES);
    let resolved_path =
        prepare_readable_document_path(runtime, path, "read_file_lines", "文本文件").await?;

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
        path: display_path(&resolved_path),
        line_start,
        line_end,
        line_count: selected.len(),
        lines: selected.clone(),
    };
    let snippet = compact_snippet(
        &selected
            .iter()
            .map(|line| format!("{}: {}", line.number, line.text))
            .collect::<Vec<_>>()
            .join("\n"),
        CITATION_SNIPPET_MAX_CHARS,
    );

    Ok(ExecutedToolCall {
        call_id: String::new(),
        name: READ_FILE_TOOL_NAME.to_string(),
        output: serde_json::to_string(&result)?,
        citations: vec![ExecutionCitation {
            id: 0,
            absolute_path: result.absolute_path.clone(),
            path: result.path.clone(),
            document_kind: "plain_text".to_string(),
            chunk_index: -1,
            line_start: Some(i32::try_from(result.line_start).unwrap_or(i32::MAX)),
            line_end: Some(i32::try_from(result.line_end).unwrap_or(i32::MAX)),
            paragraph_line_start: Some(i32::try_from(result.line_start).unwrap_or(i32::MAX)),
            page_start: None,
            page_end: None,
            heading_path: Vec::new(),
            anchor_label: None,
            score: 1.0,
            distance: 0.0,
            snippet,
        }],
        trace: ExecutionToolCall {
            name: READ_FILE_TOOL_NAME.to_string(),
            source: "builtin".to_string(),
            status: "ok".to_string(),
            summary: format!("{}:{}-{}", result.path, result.line_start, result.line_end),
        },
    })
}

pub(super) async fn execute_read_document_excerpt_tool(
    runtime: &QuestionToolRuntime<'_>,
    arguments: &Value,
) -> Result<ExecutedToolCall> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("read_document_excerpt.path 不能为空")?;
    let chunk_index = value_as_i32(arguments, "chunk_index")?;
    if chunk_index < 0 {
        bail!("read_document_excerpt.chunk_index 不能为负数");
    }

    let resolved_path =
        prepare_readable_document_path(runtime, path, "read_document_excerpt", "已索引文档")
            .await?;

    let excerpt = tokio::task::spawn_blocking({
        let resolved_path = resolved_path.clone();
        move || rag::load_document_excerpt_for_chunk(&resolved_path, chunk_index)
    })
    .await
    .context("读取文档摘录任务失败")?
    .with_context(|| format!("无法读取文档摘录: {}", resolved_path.display()))?;
    let result = ReadDocumentExcerptToolResult {
        absolute_path: resolved_path.to_string_lossy().into_owned(),
        path: display_path(&resolved_path),
        document_kind: excerpt.document_kind.as_str().to_string(),
        chunk_index: excerpt.chunk_index,
        line_start: excerpt.line_start,
        line_end: excerpt.line_end,
        paragraph_line_start: excerpt.paragraph_line_start,
        page_start: excerpt.page_start,
        page_end: excerpt.page_end,
        heading_path: excerpt.heading_path.clone(),
        anchor_label: excerpt.anchor_label.clone(),
        text: excerpt.text.clone(),
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

    Ok(ExecutedToolCall {
        call_id: String::new(),
        name: READ_DOCUMENT_EXCERPT_TOOL_NAME.to_string(),
        output: serde_json::to_string(&result)?,
        citations: vec![ExecutionCitation {
            id: 0,
            absolute_path: result.absolute_path.clone(),
            path: result.path.clone(),
            document_kind: result.document_kind.clone(),
            chunk_index: result.chunk_index,
            line_start: result.line_start,
            line_end: result.line_end,
            paragraph_line_start: result.paragraph_line_start,
            page_start: result.page_start,
            page_end: result.page_end,
            heading_path: result.heading_path.clone(),
            anchor_label: result.anchor_label.clone(),
            score: 1.0,
            distance: 0.0,
            snippet: compact_snippet(&result.text, CITATION_SNIPPET_MAX_CHARS),
        }],
        trace: ExecutionToolCall {
            name: READ_DOCUMENT_EXCERPT_TOOL_NAME.to_string(),
            source: "builtin".to_string(),
            status: "ok".to_string(),
            summary: format!("{} · {}", result.path, location_summary),
        },
    })
}

pub(super) async fn execute_rag_query_tool(
    runtime: &QuestionToolRuntime<'_>,
    arguments: &Value,
) -> Result<ExecutedToolCall> {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("rag.query.query 不能为空")?;
    let top_k = arguments
        .get("top_k")
        .map(|_| value_as_usize(arguments, "top_k"))
        .transpose()?
        .unwrap_or(DEFAULT_RAG_TOOL_TOP_K);
    let min_score = arguments
        .get("min_score")
        .and_then(Value::as_f64)
        .map(|value| value.clamp(0.0, 1.0) as f32)
        .unwrap_or(DEFAULT_RAG_TOOL_MIN_SCORE);
    let result = rag_query::search_chunks(
        runtime.data_dir,
        query,
        runtime.rag_settings,
        runtime.llm_settings,
        top_k,
        min_score,
    )
    .await?;
    let citations = result
        .hits
        .iter()
        .map(citation_from_rag_hit)
        .collect::<Vec<_>>();

    Ok(ExecutedToolCall {
        call_id: String::new(),
        name: RAG_QUERY_TOOL_NAME.to_string(),
        output: serde_json::to_string(&result)?,
        citations,
        trace: ExecutionToolCall {
            name: RAG_QUERY_TOOL_NAME.to_string(),
            source: "builtin".to_string(),
            status: if result.pending_indexing {
                "error".to_string()
            } else {
                "ok".to_string()
            },
            summary: if result.pending_indexing {
                format!(
                    "query=`{}` hits={} pending_indexing=true",
                    result.query, result.hit_count
                )
            } else {
                format!("query=`{}` hits={}", result.query, result.hit_count)
            },
        },
    })
}

pub(super) async fn execute_open_target_tool(
    runtime: &QuestionToolRuntime<'_>,
    arguments: &Value,
) -> Result<ExecutedToolCall> {
    if !runtime.allow_open_target {
        bail!("当前问题没有明确要求打开目标，禁止调用 wabity.system.open");
    }

    let target = arguments
        .get("target")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("system.open.target 不能为空")?;
    let allowed_roots = document_access_roots(runtime);
    let opened =
        OpenTargetService::new().open_input_target_with_allowed_roots(target, &allowed_roots)?;
    let result = OpenTargetToolResult {
        requested_target: target.to_string(),
        opened,
    };

    Ok(ExecutedToolCall {
        call_id: String::new(),
        name: OPEN_TARGET_TOOL_NAME.to_string(),
        output: serde_json::to_string(&result)?,
        citations: Vec::new(),
        trace: ExecutionToolCall {
            name: OPEN_TARGET_TOOL_NAME.to_string(),
            source: "builtin".to_string(),
            status: "ok".to_string(),
            summary: format!("opened {}", result.opened.target),
        },
    })
}

fn citation_from_rag_hit(hit: &RagSearchHit) -> ExecutionCitation {
    ExecutionCitation {
        id: 0,
        absolute_path: hit.absolute_path.clone(),
        path: hit.path.clone(),
        document_kind: hit.document_kind.as_str().to_string(),
        chunk_index: hit.chunk_index,
        line_start: hit.line_start,
        line_end: hit.line_end,
        paragraph_line_start: hit.paragraph_line_start,
        page_start: hit.page_start,
        page_end: hit.page_end,
        heading_path: hit.heading_path.clone(),
        anchor_label: hit.anchor_label.clone(),
        score: hit.score,
        distance: hit.distance,
        snippet: compact_snippet(&hit.text, CITATION_SNIPPET_MAX_CHARS),
    }
}

fn document_access_roots(runtime: &QuestionToolRuntime<'_>) -> Vec<PathBuf> {
    rag::collect_document_access_roots(runtime.workspace_root, runtime.rag_settings)
}

async fn prepare_readable_document_path(
    runtime: &QuestionToolRuntime<'_>,
    path: &str,
    tool_name: &str,
    target_label: &str,
) -> Result<PathBuf> {
    let allowed_roots = document_access_roots(runtime);
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

pub(super) fn resolve_readable_file_path(path: &str, allowed_roots: &[PathBuf]) -> Result<PathBuf> {
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
