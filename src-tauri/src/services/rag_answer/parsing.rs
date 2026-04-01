use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::services::command_prefix::extract_prefixed_payload;

pub(super) fn question_payload<'a>(raw_text: &'a str, aliases: &[&str]) -> &'a str {
    let trimmed = raw_text.trim_start();
    if trimmed.starts_with('/') {
        return extract_prefixed_payload(raw_text, aliases)
            .unwrap_or("")
            .trim();
    }

    raw_text.trim()
}

pub(super) fn question_explicitly_requests_open(question: &str) -> bool {
    let normalized = question.trim().to_ascii_lowercase();
    question.contains("打开")
        || question.contains("开启")
        || normalized.starts_with("open ")
        || normalized.contains(" open ")
        || normalized.starts_with("launch ")
        || normalized.contains(" launch ")
        || normalized.starts_with("/open ")
}

pub(super) fn parse_tool_arguments(value: &Value) -> Result<Value> {
    if value.is_object() {
        return Ok(value.clone());
    }
    let raw = value
        .as_str()
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .context("tool call arguments 不能为空")?;
    serde_json::from_str(raw).with_context(|| format!("tool call arguments 不是有效 JSON: {raw}"))
}

pub(super) fn value_as_usize(arguments: &Value, key: &str) -> Result<usize> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .with_context(|| format!("{key} 必须是正整数"))
}

pub(super) fn value_as_i32(arguments: &Value, key: &str) -> Result<i32> {
    arguments
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .with_context(|| format!("{key} 必须是 32 位整数"))
}

pub(super) fn display_path(path: &Path) -> String {
    let Some(home) = dirs::home_dir() else {
        return path.to_string_lossy().into_owned();
    };
    if let Ok(relative) = path.strip_prefix(&home) {
        return format!("~/{}", relative.to_string_lossy());
    }
    path.to_string_lossy().into_owned()
}

pub(super) fn compact_snippet(text: &str, max_chars: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }

    let mut snippet = compact.chars().take(max_chars).collect::<String>();
    snippet.push_str("...");
    snippet
}

pub(super) fn json_string_to_pretty_text(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .map(|value| json_value_to_pretty_text(&value))
        .unwrap_or_else(|_| raw.to_string())
}

pub(super) fn json_value_to_pretty_text(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

pub(super) fn ensure_file(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("不是可读取的文件: {}", path.display());
    }
    Ok(())
}
