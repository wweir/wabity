use std::fmt::Write as _;

use serde_json::{json, Value};

use crate::{
    domain::acp::{BuiltinMcpModuleKey, BuiltinMcpModuleStatus},
    services::rag_query::{self, RagSearchResult},
};

use super::{
    build_tool_error_result, build_tool_pending_result, build_tool_success_result,
    execute_with_timeout, BuiltinMcpRequestState, BuiltinMcpRuntimeConfig,
    BuiltinMcpToolDefinition,
};

pub(super) const SEARCH_TOOL_NAME: &str = "wabity.rag.search";
const DEFAULT_TOOL_TOP_K: usize = 8;
const DEFAULT_TOOL_MIN_SCORE: f32 = 0.35;
const MAX_TOOL_TOP_K: usize = 20;

#[derive(Debug)]
struct RagSearchToolInput {
    query: String,
    top_k: usize,
    min_score: f32,
}

pub(super) fn module_status() -> BuiltinMcpModuleStatus {
    BuiltinMcpModuleStatus {
        key: BuiltinMcpModuleKey::Rag,
        title: "RAG 检索".to_string(),
        summary: "检索本地文档索引，返回命中 chunk、路径和分数。".to_string(),
        tool_count: 1,
    }
}

pub(super) fn tool_definitions() -> Vec<BuiltinMcpToolDefinition> {
    vec![BuiltinMcpToolDefinition {
        name: SEARCH_TOOL_NAME,
        title: "Wabity RAG Search",
        description:
            "Search Wabity's local document index and return the nearest indexed chunks with path, score, and chunk text."
                .to_string(),
        input_schema: json!({
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
                    "description": "Discard hits whose normalized similarity score is below this threshold."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        output_schema: json!({
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
                            "text": { "type": "string" },
                            "distance": { "type": "number" },
                            "score": { "type": "number" }
                        },
                        "required": [
                            "sourceRoot",
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
        }),
    }]
}

pub(super) fn handles_tool(name: &str) -> bool {
    name == SEARCH_TOOL_NAME
}

pub(super) async fn execute_tool(
    request_state: &BuiltinMcpRequestState,
    runtime_config: &BuiltinMcpRuntimeConfig,
    arguments: Option<Value>,
) -> Value {
    let tool_input = match parse_tool_input(arguments) {
        Ok(tool_input) => tool_input,
        Err(message) => return build_tool_error_result(message),
    };

    match execute_with_timeout(
        SEARCH_TOOL_NAME,
        super::BUILTIN_MCP_TOOL_EXECUTION_TIMEOUT,
        rag_query::search_chunks(
            &request_state.data_dir,
            &tool_input.query,
            &runtime_config.rag_settings,
            &runtime_config.llm_settings,
            tool_input.top_k,
            tool_input.min_score,
        ),
    )
    .await
    {
        Ok(result) => {
            let structured_content = rag_search_result_value(&result);
            if result.pending_indexing {
                let pending_message = build_pending_message(&result);
                build_tool_pending_result(structured_content, pending_message)
            } else {
                build_tool_success_result(structured_content, build_search_summary_text(&result))
            }
        }
        Err(error) => build_tool_error_result(error),
    }
}

fn rag_search_result_value(result: &RagSearchResult) -> Value {
    serde_json::to_value(result).unwrap_or_else(|_| json!({}))
}

fn parse_tool_input(arguments: Option<Value>) -> Result<RagSearchToolInput, String> {
    let Some(arguments) = arguments else {
        return Err("tool arguments are required".to_string());
    };
    let Value::Object(object) = arguments else {
        return Err("tool arguments must be a JSON object".to_string());
    };
    if let Some(key) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "query" | "topK" | "minScore"))
    {
        return Err(format!("unknown tool argument: {key}"));
    }

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

fn build_pending_message(result: &RagSearchResult) -> String {
    if result.hit_count > 0 {
        format!(
            "RAG 索引仍在构建中，当前 {} 条命中只是部分结果，不能当成稳定事实来源。",
            result.hit_count
        )
    } else {
        "RAG 索引仍在构建中，当前不能返回稳定检索结果。".to_string()
    }
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
        let location = rag_hit_location(hit);
        let _ = writeln!(
            text,
            "\n[{}] {} (chunk {}, {}, score {:.4}, distance {:.4})\n{}",
            index + 1,
            hit.path,
            hit.chunk_index,
            location,
            hit.score,
            hit.distance,
            hit.text
        );
    }

    text.trim().to_string()
}

fn rag_hit_location(hit: &rag_query::RagSearchHit) -> String {
    if let (Some(page_start), Some(page_end)) = (hit.page_start, hit.page_end) {
        if page_start == page_end {
            return format!("page {page_start}");
        }

        return format!("pages {page_start}-{page_end}");
    }

    if let (Some(line_start), Some(line_end)) = (hit.line_start, hit.line_end) {
        let paragraph = hit
            .paragraph_line_start
            .map(|line| format!(", paragraph {line}"))
            .unwrap_or_default();
        return format!("lines {line_start}-{line_end}{paragraph}");
    }

    format!("chunk {}", hit.chunk_index)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::rag_search_result_value;
    use crate::services::{
        document_extract::DocumentKind,
        rag_query::{RagSearchHit, RagSearchResult},
    };

    #[test]
    fn pending_rag_result_keeps_declared_output_shape() {
        let value = rag_search_result_value(&RagSearchResult {
            query: "test".to_string(),
            hit_count: 1,
            pending_indexing: true,
            hits: vec![RagSearchHit {
                source_root: "/tmp".to_string(),
                absolute_path: "/tmp/doc.md".to_string(),
                path: "doc.md".to_string(),
                document_kind: DocumentKind::Markdown,
                chunk_index: 0,
                line_start: Some(1),
                line_end: Some(3),
                paragraph_line_start: Some(1),
                page_start: None,
                page_end: None,
                heading_path: vec!["Intro".to_string()],
                anchor_label: None,
                text: "hello".to_string(),
                distance: 0.1,
                score: 0.9,
                vector_score: 0.9,
                lexical_score: 0.0,
                has_vector_signal: true,
                retrieval_boost: 0.0,
            }],
        });

        assert_eq!(value["pendingIndexing"], json!(true));
        assert_eq!(value["query"], json!("test"));
        assert_eq!(value["hitCount"], json!(1));
        assert!(value.get("partialResult").is_none());
        assert!(value.get("error").is_none());
    }
}
