use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose, Engine as _};
use serde_json::json;

use crate::{
    domain::execution::{ExecutionRequest, ExecutionResult},
    services::command_prefix::extract_prefixed_payload,
};

const UPPERCASE_TEXT_COMMAND_ALIASES: [&str; 1] = ["/upper"];
const TITLE_CASE_TEXT_COMMAND_ALIASES: [&str; 1] = ["/title"];
const LOWERCASE_TEXT_COMMAND_ALIASES: [&str; 1] = ["/lower"];
const CAMEL_CASE_TEXT_COMMAND_ALIASES: [&str; 1] = ["/camel"];
const SNAKE_CASE_TEXT_COMMAND_ALIASES: [&str; 1] = ["/snake"];
const WORD_COUNT_COMMAND_ALIASES: [&str; 1] = ["/words"];
const LINE_COUNT_COMMAND_ALIASES: [&str; 1] = ["/lines"];
const TRIM_WHITESPACE_COMMAND_ALIASES: [&str; 1] = ["/trim"];
const UNIQUE_LINES_COMMAND_ALIASES: [&str; 1] = ["/unique"];
const SORT_LINES_COMMAND_ALIASES: [&str; 1] = ["/sort"];
const JSON_FORMAT_COMMAND_ALIASES: [&str; 3] = ["/format", "/fmt", "/json"];
const BASE64_COMMAND_ALIASES: [&str; 1] = ["/base64"];
const MARKDOWN_RENDER_COMMAND_ALIASES: [&str; 2] = ["/md", "/markdown"];

#[derive(Debug, Clone, Default)]
pub struct ExecutorService;

impl ExecutorService {
    pub fn new() -> Self {
        Self
    }

    pub fn execute(&self, request: &ExecutionRequest) -> Result<ExecutionResult> {
        let normalized_text = action_payload_or_text(&request.action_id, &request.query.raw_text);

        match request.action_id.as_str() {
            "copy_text" => Ok(success_result(
                Some(normalized_text.to_string()),
                Some("已通过前端副作用复制到剪贴板".to_string()),
                Some(json!({ "effect": "copy_to_clipboard" })),
                vec!["trim_whitespace", "uppercase_text"],
                false,
            )),
            "uppercase_text" => execute_text_transform(
                normalized_text,
                |text| text.to_uppercase(),
                "文本已转为大写",
            ),
            "title_case_text" => {
                execute_text_transform(normalized_text, convert_title_case, "文本已转为标题格式")
            }
            "lowercase_text" => execute_text_transform(
                normalized_text,
                |text| text.to_lowercase(),
                "文本已转为小写",
            ),
            "camel_case_text" => execute_text_transform(
                normalized_text,
                |text| convert_lines(text, LineTransform::CamelCase),
                "文本已转为驼峰",
            ),
            "snake_case_text" => execute_text_transform(
                normalized_text,
                |text| convert_lines(text, LineTransform::SnakeCase),
                "文本已转为下划线",
            ),
            "word_count" => Ok(success_result(
                Some(count_words(normalized_text).to_string()),
                Some("词数统计完成".to_string()),
                Some(json!({ "metric": "word_count" })),
                vec![],
                false,
            )),
            "line_count" => Ok(success_result(
                Some(count_non_empty_lines(normalized_text).to_string()),
                Some("行数统计完成".to_string()),
                Some(json!({ "metric": "line_count" })),
                vec![],
                false,
            )),
            "trim_whitespace" => execute_text_transform(
                normalized_text,
                normalize_whitespace,
                "行尾与首尾空白已清理",
            ),
            "unique_lines" => execute_text_transform(normalized_text, unique_lines, "重复行已去重"),
            "sort_lines" => execute_text_transform(normalized_text, sort_lines, "文本已按行排序"),
            "json_pretty_print" => execute_pretty_json(normalized_text),
            "base64_text" => execute_base64_text(normalized_text),
            "markdown_render" => execute_markdown_render(normalized_text),
            other => bail!("unknown action id: {other}"),
        }
    }
}

fn execute_pretty_json(text: &str) -> Result<ExecutionResult> {
    let json_input = extract_json_format_payload(text).unwrap_or(text).trim();
    let json_value =
        serde_json::from_str::<serde_json::Value>(json_input).context("input is not valid json")?;
    let pretty = serde_json::to_string_pretty(&json_value).context("failed to format json")?;

    Ok(success_result(
        Some(pretty),
        Some("JSON 已格式化".to_string()),
        None,
        vec!["copy_text"],
        false,
    ))
}

fn execute_text_transform(
    text: &str,
    transform: impl FnOnce(&str) -> String,
    success_message: &str,
) -> Result<ExecutionResult> {
    if text.is_empty() {
        bail!("请输入要转换的内容");
    }

    Ok(success_result(
        Some(transform(text)),
        Some(success_message.to_string()),
        None,
        vec!["copy_text"],
        false,
    ))
}

fn execute_base64_text(text: &str) -> Result<ExecutionResult> {
    let payload = extract_base64_payload(text).unwrap_or(text).trim();
    if payload.is_empty() {
        bail!("请输入要编码或解码的内容");
    }

    if let Some(decoded) = try_decode_base64_text(payload)? {
        return Ok(success_result(
            Some(decoded),
            Some("Base64 已解码".to_string()),
            None,
            vec!["copy_text"],
            false,
        ));
    }

    Ok(success_result(
        Some(general_purpose::STANDARD.encode(payload.as_bytes())),
        Some("文本已编码为 Base64".to_string()),
        None,
        vec!["copy_text"],
        false,
    ))
}

fn execute_markdown_render(text: &str) -> Result<ExecutionResult> {
    let payload = extract_markdown_render_payload(text).unwrap_or(text).trim();
    if payload.is_empty() {
        bail!("请输入要渲染的 Markdown 内容");
    }

    Ok(success_result(
        Some(payload.to_string()),
        Some("Markdown 已准备渲染".to_string()),
        Some(json!({ "render": "markdown" })),
        vec!["copy_text"],
        false,
    ))
}

fn action_payload_or_text<'a>(action_id: &str, text: &'a str) -> &'a str {
    match action_id {
        "copy_text" => text.trim(),
        "uppercase_text" => extract_prefixed_payload(text, &UPPERCASE_TEXT_COMMAND_ALIASES)
            .unwrap_or(text)
            .trim(),
        "title_case_text" => extract_prefixed_payload(text, &TITLE_CASE_TEXT_COMMAND_ALIASES)
            .unwrap_or(text)
            .trim(),
        "lowercase_text" => extract_prefixed_payload(text, &LOWERCASE_TEXT_COMMAND_ALIASES)
            .unwrap_or(text)
            .trim(),
        "camel_case_text" => extract_prefixed_payload(text, &CAMEL_CASE_TEXT_COMMAND_ALIASES)
            .unwrap_or(text)
            .trim(),
        "snake_case_text" => extract_prefixed_payload(text, &SNAKE_CASE_TEXT_COMMAND_ALIASES)
            .unwrap_or(text)
            .trim(),
        "word_count" => extract_prefixed_payload(text, &WORD_COUNT_COMMAND_ALIASES)
            .unwrap_or(text)
            .trim(),
        "line_count" => extract_prefixed_payload(text, &LINE_COUNT_COMMAND_ALIASES)
            .unwrap_or(text)
            .trim(),
        "trim_whitespace" => extract_prefixed_payload(text, &TRIM_WHITESPACE_COMMAND_ALIASES)
            .unwrap_or(text)
            .trim(),
        "unique_lines" => extract_prefixed_payload(text, &UNIQUE_LINES_COMMAND_ALIASES)
            .unwrap_or(text)
            .trim(),
        "sort_lines" => extract_prefixed_payload(text, &SORT_LINES_COMMAND_ALIASES)
            .unwrap_or(text)
            .trim(),
        "json_pretty_print" => extract_json_format_payload(text).unwrap_or(text).trim(),
        "base64_text" => extract_base64_payload(text).unwrap_or(text).trim(),
        "markdown_render" => extract_markdown_render_payload(text).unwrap_or(text).trim(),
        _ => text.trim(),
    }
}

fn extract_json_format_payload(text: &str) -> Option<&str> {
    extract_prefixed_payload(text, &JSON_FORMAT_COMMAND_ALIASES)
}

fn extract_base64_payload(text: &str) -> Option<&str> {
    extract_prefixed_payload(text, &BASE64_COMMAND_ALIASES)
}

fn extract_markdown_render_payload(text: &str) -> Option<&str> {
    extract_prefixed_payload(text, &MARKDOWN_RENDER_COMMAND_ALIASES)
}

fn try_decode_base64_text(text: &str) -> Result<Option<String>> {
    let Some(normalized) = normalize_base64_candidate(text) else {
        return Ok(None);
    };

    let decoded = match general_purpose::STANDARD.decode(normalized.as_bytes()) {
        Ok(decoded) => decoded,
        Err(_) => return Ok(None),
    };

    match String::from_utf8(decoded) {
        Ok(decoded_text) => Ok(Some(decoded_text)),
        Err(_) => Ok(None),
    }
}

fn normalize_base64_candidate(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return None;
    }

    let remainder = trimmed.len() % 4;
    if remainder == 1 {
        return None;
    }

    if remainder == 0 {
        return Some(trimmed.to_string());
    }

    let mut normalized = trimmed.to_string();
    normalized.push_str(&"=".repeat(4 - remainder));
    Some(normalized)
}

fn normalize_whitespace(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn convert_title_case(text: &str) -> String {
    let mut transformed = String::with_capacity(text.len());
    let mut at_word_start = true;

    for character in text.chars() {
        if character.is_alphanumeric() {
            if at_word_start {
                transformed.extend(character.to_uppercase());
                at_word_start = false;
            } else {
                transformed.extend(character.to_lowercase());
            }
        } else {
            transformed.push(character);
            at_word_start = true;
        }
    }

    transformed
}

#[derive(Debug, Clone, Copy)]
enum LineTransform {
    CamelCase,
    SnakeCase,
}

fn convert_lines(text: &str, transform: LineTransform) -> String {
    text.split('\n')
        .map(|line| convert_line_case(line.strip_suffix('\r').unwrap_or(line), transform))
        .collect::<Vec<_>>()
        .join("\n")
}

fn convert_line_case(line: &str, transform: LineTransform) -> String {
    let words = split_words(line);
    if words.is_empty() {
        return String::new();
    }

    match transform {
        LineTransform::CamelCase => {
            words
                .iter()
                .enumerate()
                .fold(String::new(), |mut transformed, (index, word)| {
                    if index == 0 {
                        transformed.push_str(word);
                    } else {
                        transformed.push_str(&uppercase_first(word));
                    }
                    transformed
                })
        }
        LineTransform::SnakeCase => words.join("_"),
    }
}

fn split_words(line: &str) -> Vec<String> {
    let characters = line.chars().collect::<Vec<_>>();
    let mut words = Vec::new();
    let mut current = String::new();

    for (index, character) in characters.iter().copied().enumerate() {
        if !character.is_alphanumeric() {
            push_normalized_word(&mut words, &mut current);
            continue;
        }

        let previous = current.chars().last();
        let next = characters.get(index + 1).copied();
        let should_split = matches!(
            previous,
            Some(previous_character)
                if (previous_character.is_lowercase() || previous_character.is_ascii_digit())
                    && character.is_uppercase()
        ) || matches!(
            (previous, next),
            (Some(previous_character), Some(next_character))
                if previous_character.is_uppercase()
                    && character.is_uppercase()
                    && next_character.is_lowercase()
        );

        if should_split {
            push_normalized_word(&mut words, &mut current);
        }

        current.push(character);
    }

    push_normalized_word(&mut words, &mut current);
    words
}

fn push_normalized_word(words: &mut Vec<String>, current: &mut String) {
    if current.is_empty() {
        return;
    }

    words.push(current.to_lowercase());
    current.clear();
}

fn uppercase_first(word: &str) -> String {
    let mut characters = word.chars();
    let Some(first_character) = characters.next() else {
        return String::new();
    };

    let mut transformed = first_character.to_uppercase().collect::<String>();
    transformed.push_str(characters.as_str());
    transformed
}

fn unique_lines(text: &str) -> String {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| seen.insert((*line).to_string()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn sort_lines(text: &str) -> String {
    let mut lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect::<Vec<_>>();
    lines.sort();
    lines.join("\n")
}

fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

fn count_non_empty_lines(text: &str) -> usize {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .count()
}

fn success_result(
    primary_text: Option<String>,
    secondary_text: Option<String>,
    structured_payload: Option<serde_json::Value>,
    next_actions: Vec<&str>,
    should_close_launcher: bool,
) -> ExecutionResult {
    ExecutionResult::success(
        primary_text,
        secondary_text,
        structured_payload,
        next_actions,
        should_close_launcher,
    )
}

#[cfg(test)]
mod tests {
    use super::ExecutorService;
    use crate::domain::{
        execution::ExecutionRequest,
        query::{InputMode, QueryPayload, SourceMetadata},
    };

    fn request(action_id: &str, raw_text: &str) -> ExecutionRequest {
        ExecutionRequest {
            action_id: action_id.to_string(),
            query: QueryPayload {
                mode: InputMode::Multiline,
                raw_text: raw_text.to_string(),
                segments: raw_text.lines().map(ToString::to_string).collect(),
                language: None,
                source_metadata: SourceMetadata {
                    created_at_ms: 0,
                    source_hint: "test".to_string(),
                },
            },
            conversation: Vec::new(),
            conversation_state: None,
        }
    }

    #[test]
    fn uppercase_action_transforms_text() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("uppercase_text", "hello"))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("HELLO"));
    }

    #[test]
    fn lowercase_action_transforms_text() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("lowercase_text", "Hello WORLD"))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("hello world"));
    }

    #[test]
    fn title_case_action_transforms_text() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request(
                "title_case_text",
                "/title hello_world\nHTTP server",
            ))
            .unwrap();

        assert_eq!(
            result.primary_text.as_deref(),
            Some("Hello_World\nHttp Server")
        );
    }

    #[test]
    fn uppercase_action_rejects_empty_payload() {
        let service = ExecutorService::new();
        let error = service.execute(&request("uppercase_text", "")).unwrap_err();

        assert_eq!(error.to_string(), "请输入要转换的内容");
    }

    #[test]
    fn camel_case_action_transforms_each_line() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request(
                "camel_case_text",
                "/camel hello_world\nHTTP server",
            ))
            .unwrap();

        assert_eq!(
            result.primary_text.as_deref(),
            Some("helloWorld\nhttpServer")
        );
    }

    #[test]
    fn snake_case_action_transforms_mixed_input() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("snake_case_text", "helloWorld-HTTPServer"))
            .unwrap();

        assert_eq!(
            result.primary_text.as_deref(),
            Some("hello_world_http_server")
        );
    }

    #[test]
    fn copy_action_preserves_raw_text() {
        let service = ExecutorService::new();
        let result = service.execute(&request("copy_text", "hello")).unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("hello"));
    }

    #[test]
    fn line_count_strips_slash_command_prefix() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("line_count", "/lines first\n\nsecond"))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("2"));
    }

    #[test]
    fn uppercase_action_accepts_short_slash_prefix_with_attached_payload() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("uppercase_text", "/uhello"))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("HELLO"));
    }

    #[test]
    fn uppercase_action_accepts_short_slash_prefix_with_whitespace_payload() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("uppercase_text", "/up hello"))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("HELLO"));
    }

    #[test]
    fn unique_action_deduplicates_lines() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("unique_lines", "/unique alpha\nbeta\nalpha\nbeta"))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("alpha\nbeta"));
    }

    #[test]
    fn sort_action_sorts_lines() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("sort_lines", "/sort beta\nalpha\ngamma"))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("alpha\nbeta\ngamma"));
    }

    #[test]
    fn json_action_formats_payload() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("json_pretty_print", "{\"a\":1}"))
            .unwrap();

        assert!(result
            .primary_text
            .as_deref()
            .unwrap()
            .contains("\n  \"a\": 1\n"));
    }

    #[test]
    fn json_action_accepts_format_command_prefix() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("json_pretty_print", "/format {\"a\":1}"))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("{\n  \"a\": 1\n}"));
    }

    #[test]
    fn base64_action_decodes_base64_payload() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("base64_text", "/base64 dGVzdA=="))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("test"));
    }

    #[test]
    fn base64_action_encodes_plain_text_payload() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("base64_text", "/base64 hello"))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("aGVsbG8="));
    }

    #[test]
    fn markdown_action_returns_payload_and_render_metadata() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("markdown_render", "/md # Title"))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("# Title"));
        assert_eq!(
            result
                .structured_payload
                .as_ref()
                .and_then(|payload| payload.get("render"))
                .and_then(|render| render.as_str()),
            Some("markdown")
        );
    }

    #[test]
    fn markdown_action_accepts_attached_payload() {
        let service = ExecutorService::new();
        let result = service
            .execute(&request("markdown_render", "/md# Title"))
            .unwrap();

        assert_eq!(result.primary_text.as_deref(), Some("# Title"));
    }
}
