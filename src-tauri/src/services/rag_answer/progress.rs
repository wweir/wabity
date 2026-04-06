use std::sync::Arc;

use anyhow::Error;

use crate::domain::{acp::AcpActionEvent, execution::ExecutionProgressEvent};

use super::{
    LocalToolCall, OPEN_TARGET_TOOL_NAME, RAG_QUERY_TOOL_NAME, READ_DOCUMENT_EXCERPT_TOOL_NAME,
    READ_FILE_TOOL_NAME,
};

pub(super) fn build_round_action(round: usize, tool_count: usize) -> AcpActionEvent {
    AcpActionEvent {
        kind: "info".to_string(),
        title: format!("第 {round} 步"),
        correlation_id: None,
        detail: Some(format!("模型发起了 {tool_count} 个工具调用")),
    }
}

pub(super) fn emit_question_progress(
    progress_event_tx: Option<&Arc<dyn Fn(ExecutionProgressEvent) + Send + Sync>>,
    status_text: &str,
) {
    emit_question_progress_with_partial(progress_event_tx, status_text, None);
}

pub(super) fn emit_question_progress_with_partial(
    progress_event_tx: Option<&Arc<dyn Fn(ExecutionProgressEvent) + Send + Sync>>,
    status_text: &str,
    partial_text: Option<String>,
) {
    let Some(progress_event_tx) = progress_event_tx else {
        return;
    };

    progress_event_tx(ExecutionProgressEvent {
        action_id: "rag_answer".to_string(),
        status_text: status_text.to_string(),
        partial_text,
    });
}

pub(super) fn emit_question_partial_answer(
    progress_event_tx: Option<&Arc<dyn Fn(ExecutionProgressEvent) + Send + Sync>>,
    partial_text: &str,
) {
    emit_question_progress_with_partial(
        progress_event_tx,
        "文档问答 · 正在输出回答",
        Some(partial_text.to_string()),
    );
}

pub(super) fn clear_question_partial_answer(
    progress_event_tx: Option<&Arc<dyn Fn(ExecutionProgressEvent) + Send + Sync>>,
    status_text: &str,
) {
    emit_question_progress_with_partial(progress_event_tx, status_text, Some(String::new()));
}

pub(super) fn should_retry_without_stream(streaming_enabled: bool, error: &Error) -> bool {
    if !streaming_enabled {
        return false;
    }

    let message = error.to_string().to_ascii_lowercase();
    (message.contains("stream") || message.contains("sse"))
        && [
            "unsupported",
            "not supported",
            "does not support",
            "disabled",
            "invalid",
            "unexpected",
        ]
        .iter()
        .any(|needle| message.contains(needle))
}

pub(super) fn summarize_local_tool_progress(local_calls: &[LocalToolCall]) -> &'static str {
    let rag_query_count = local_calls
        .iter()
        .filter(|call| call.name == RAG_QUERY_TOOL_NAME)
        .count();
    let read_file_count = local_calls
        .iter()
        .filter(|call| call.name == READ_FILE_TOOL_NAME)
        .count();
    let read_document_count = local_calls
        .iter()
        .filter(|call| call.name == READ_DOCUMENT_EXCERPT_TOOL_NAME)
        .count();
    let open_target_count = local_calls
        .iter()
        .filter(|call| call.name == OPEN_TARGET_TOOL_NAME)
        .count();

    match (
        rag_query_count > 0,
        read_file_count > 0,
        read_document_count > 0,
        open_target_count > 0,
    ) {
        (true, true, _, _) | (true, _, true, _) => "文档问答 · 正在检索索引并读取证据",
        (true, false, false, true) => "文档问答 · 正在检索并打开目标",
        (true, false, false, false) => "文档问答 · 正在检索本地文档索引",
        (false, true, _, true) | (false, _, true, true) => "文档问答 · 正在读取证据并打开目标",
        (false, true, _, false) | (false, _, true, false) => "文档问答 · 正在读取证据",
        (false, false, false, true) => "文档问答 · 正在打开目标",
        (false, false, false, false) => "文档问答 · 正在执行模型请求的工具",
    }
}
