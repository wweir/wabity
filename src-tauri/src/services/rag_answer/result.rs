use std::collections::{BTreeSet, HashSet};

use anyhow::Result;
use serde::Serialize;

use super::AnswerOutcome;

use crate::domain::{
    acp::AcpActionEvent,
    execution::{
        ExecutionCitation, ExecutionConversationState, ExecutionResult, ExecutionToolCall,
    },
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RagAnswerPayload {
    kind: &'static str,
    render: &'static str,
    response_id: Option<String>,
    conversation_state: ExecutionConversationState,
    reasoning: Option<String>,
    citations: Vec<ExecutionCitation>,
    retrieval: RagRetrievalPayload,
    actions: Vec<AcpActionEvent>,
    tools: RagToolUsagePayload,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RagRetrievalPayload {
    query: String,
    match_count: usize,
    file_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RagToolUsagePayload {
    available: Vec<String>,
    skipped: Vec<String>,
    calls: Vec<ExecutionToolCall>,
}

pub(super) fn deduplicate_and_number_citations(
    citations: Vec<ExecutionCitation>,
) -> Vec<ExecutionCitation> {
    let mut seen = HashSet::new();
    citations
        .into_iter()
        .filter(|citation| {
            seen.insert(format!(
                "{}:{}:{:?}:{:?}:{:?}:{:?}",
                citation.absolute_path,
                citation.chunk_index,
                citation.line_start,
                citation.line_end,
                citation.page_start,
                citation.page_end
            ))
        })
        .enumerate()
        .map(|(index, mut citation)| {
            citation.id = index + 1;
            citation
        })
        .collect()
}

pub(super) fn build_execution_result(
    question: &str,
    protocol_label: String,
    answer: AnswerOutcome,
    conversation_state: ExecutionConversationState,
    available_tools: Vec<String>,
    skipped_tools: Vec<String>,
) -> Result<ExecutionResult> {
    let citations = deduplicate_and_number_citations(answer.citations);
    let retrieval = RagRetrievalPayload {
        query: question.to_string(),
        match_count: citations.len(),
        file_count: citations
            .iter()
            .map(|citation| citation.absolute_path.clone())
            .collect::<BTreeSet<_>>()
            .len(),
    };
    let secondary_text = if answer.tool_calls.is_empty() {
        format!("{protocol_label} 未调用工具，直接生成了回答")
    } else {
        format!(
            "{protocol_label} 已执行 {} 次工具调用，引用 {} 个文件",
            answer.tool_calls.len(),
            retrieval.file_count
        )
    };
    let secondary_text = if skipped_tools.is_empty() {
        secondary_text
    } else {
        format!(
            "{secondary_text}；跳过 {} 个当前协议不支持的 MCP server",
            skipped_tools.len()
        )
    };
    let response_id = answer.response_id.clone();

    Ok(ExecutionResult::success(
        Some(answer.final_answer),
        Some(secondary_text),
        Some(serde_json::to_value(RagAnswerPayload {
            kind: "rag_answer",
            render: "markdown",
            response_id: response_id.clone(),
            conversation_state,
            reasoning: answer.reasoning,
            citations,
            retrieval,
            tools: RagToolUsagePayload {
                available: available_tools,
                skipped: skipped_tools,
                calls: answer.tool_calls,
            },
            actions: answer.actions,
        })?),
        vec!["copy_text"],
        false,
    ))
}
