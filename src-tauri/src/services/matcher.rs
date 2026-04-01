use anyhow::Result;

use crate::{
    domain::{
        actions::{ActionDescriptor, ActionMatch},
        query::{InputMode, QueryPayload},
    },
    services::command_prefix::extract_prefixed_payload,
};

const JSON_FORMAT_COMMAND_ALIASES: [&str; 3] = ["/format", "/fmt", "/json"];
const BASE64_COMMAND_ALIASES: [&str; 1] = ["/base64"];
const MARKDOWN_RENDER_COMMAND_ALIASES: [&str; 2] = ["/md", "/markdown"];
const TRANSLATE_COMMAND_ALIASES: [&str; 3] = ["/translate", "/fy", "/tr"];
const RAG_ANSWER_COMMAND_ALIASES: [&str; 3] = ["/ask", "/qa", "/docs"];

#[derive(Debug, Clone)]
pub struct MatcherService {
    actions: Vec<ActionDescriptor>,
}

impl MatcherService {
    pub fn new() -> Self {
        Self {
            actions: builtin_actions(),
        }
    }

    pub fn match_actions(&self, query: &QueryPayload) -> Result<Vec<ActionMatch>> {
        let normalized = query.normalized_text();
        let mut matches = self
            .actions
            .iter()
            .filter(|action| action.supports(query.mode))
            .filter_map(|action| score_action(action, query, &normalized))
            .collect::<Vec<_>>();

        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.descriptor.title.cmp(&right.descriptor.title))
        });

        Ok(matches)
    }
}

fn score_action(
    action: &ActionDescriptor,
    query: &QueryPayload,
    normalized_query: &str,
) -> Option<ActionMatch> {
    let command_token = slash_command_token(normalized_query).unwrap_or(normalized_query);

    if query.is_empty() {
        return Some(ActionMatch {
            descriptor: action.clone(),
            score: i32::from(action.priority),
        });
    }

    let title = action.title.to_lowercase();
    let aliases = action.aliases.join(" ").to_lowercase();
    let keywords = action.keywords.join(" ").to_lowercase();

    let mut score = i32::from(action.priority);
    let mut matched = false;

    if title.starts_with(command_token) {
        score += 90;
        matched = true;
    } else if title.contains(command_token) {
        score += 55;
        matched = true;
    }

    if aliases.contains(command_token) {
        score += 36;
        matched = true;
    }

    if keywords.contains(command_token) {
        score += 28;
        matched = true;
    }

    if command_token.starts_with('/') && action.aliases.iter().any(|alias| alias == command_token) {
        score += 42;
        matched = true;
    }

    if normalized_query.starts_with("http") && action.id == "open_target" {
        score += 60;
        matched = true;
    }

    if action.id == "json_pretty_print"
        && (extract_json_format_payload(&query.raw_text).is_some()
            || normalized_query.contains('{'))
    {
        score += 44;
        matched = true;
    }

    if action.id == "base64_text" && extract_base64_payload(&query.raw_text).is_some() {
        score += 44;
        matched = true;
    }

    if action.id == "markdown_render" && extract_markdown_render_payload(&query.raw_text).is_some()
    {
        score += 44;
        matched = true;
    }

    if action.id == "translate_text" && extract_translate_payload(&query.raw_text).is_some() {
        score += 44;
        matched = true;
    }

    if action.id == "rag_answer" && extract_rag_answer_payload(&query.raw_text).is_some() {
        score += 44;
        matched = true;
    }

    if !matched {
        return None;
    }

    if query.mode == InputMode::Multiline {
        score += 10;
    }

    Some(ActionMatch {
        descriptor: action.clone(),
        score,
    })
}

fn builtin_actions() -> Vec<ActionDescriptor> {
    use InputMode::{Clipboard, Inline, Multiline, Ocr, Selection};

    vec![
        ActionDescriptor {
            id: "open_target".to_string(),
            title: "打开目标".to_string(),
            summary: "打开当前输入的链接、文件或目录".to_string(),
            aliases: vec!["/open".to_string()],
            keywords: vec![
                "url".to_string(),
                "browser".to_string(),
                "link".to_string(),
                "open".to_string(),
                "file".to_string(),
                "folder".to_string(),
                "path".to_string(),
            ],
            supported_input_modes: vec![Inline, Clipboard, Selection],
            category: "system".to_string(),
            priority: 120,
        },
        ActionDescriptor {
            id: "clipboard_history".to_string(),
            title: "历史剪贴板".to_string(),
            summary: "打开最近文本剪贴板历史并支持回贴".to_string(),
            aliases: vec!["/clip".to_string(), "/paste".to_string()],
            keywords: vec![
                "clipboard".to_string(),
                "paste".to_string(),
                "history".to_string(),
                "clip".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "system".to_string(),
            priority: 97,
        },
        ActionDescriptor {
            id: "uppercase_text".to_string(),
            title: "转大写".to_string(),
            summary: "把文本转换为全大写".to_string(),
            aliases: vec!["/upper".to_string()],
            keywords: vec![
                "uppercase".to_string(),
                "text".to_string(),
                "transform".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 100,
        },
        ActionDescriptor {
            id: "title_case_text".to_string(),
            title: "转标题".to_string(),
            summary: "把每个词的首字母转为大写".to_string(),
            aliases: vec!["/title".to_string()],
            keywords: vec![
                "title".to_string(),
                "titlecase".to_string(),
                "capitalize".to_string(),
                "text".to_string(),
                "transform".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 99,
        },
        ActionDescriptor {
            id: "lowercase_text".to_string(),
            title: "转小写".to_string(),
            summary: "把文本转换为全小写".to_string(),
            aliases: vec!["/lower".to_string()],
            keywords: vec![
                "lowercase".to_string(),
                "text".to_string(),
                "transform".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 99,
        },
        ActionDescriptor {
            id: "camel_case_text".to_string(),
            title: "转驼峰".to_string(),
            summary: "把文本转换为 camelCase".to_string(),
            aliases: vec!["/camel".to_string()],
            keywords: vec![
                "camel".to_string(),
                "camelcase".to_string(),
                "text".to_string(),
                "transform".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 98,
        },
        ActionDescriptor {
            id: "snake_case_text".to_string(),
            title: "转下划线".to_string(),
            summary: "把文本转换为 snake_case".to_string(),
            aliases: vec!["/snake".to_string()],
            keywords: vec![
                "snake".to_string(),
                "snakecase".to_string(),
                "underscore".to_string(),
                "text".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 97,
        },
        ActionDescriptor {
            id: "word_count".to_string(),
            title: "统计词数".to_string(),
            summary: "统计输入中的单词数量".to_string(),
            aliases: vec!["/words".to_string()],
            keywords: vec!["count".to_string(), "words".to_string(), "text".to_string()],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 95,
        },
        ActionDescriptor {
            id: "line_count".to_string(),
            title: "统计行数".to_string(),
            summary: "统计输入中的行数".to_string(),
            aliases: vec!["/lines".to_string()],
            keywords: vec!["count".to_string(), "lines".to_string(), "text".to_string()],
            supported_input_modes: vec![Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 92,
        },
        ActionDescriptor {
            id: "trim_whitespace".to_string(),
            title: "清理空白".to_string(),
            summary: "清理每行首尾和整体多余空白".to_string(),
            aliases: vec!["/trim".to_string()],
            keywords: vec![
                "trim".to_string(),
                "normalize".to_string(),
                "text".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 90,
        },
        ActionDescriptor {
            id: "unique_lines".to_string(),
            title: "去重".to_string(),
            summary: "按行保留首次出现的内容并去重".to_string(),
            aliases: vec!["/unique".to_string()],
            keywords: vec![
                "unique".to_string(),
                "dedupe".to_string(),
                "lines".to_string(),
                "text".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 89,
        },
        ActionDescriptor {
            id: "sort_lines".to_string(),
            title: "排序".to_string(),
            summary: "按行进行字典序排序".to_string(),
            aliases: vec!["/sort".to_string()],
            keywords: vec![
                "sort".to_string(),
                "lines".to_string(),
                "order".to_string(),
                "text".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 87,
        },
        ActionDescriptor {
            id: "json_pretty_print".to_string(),
            title: "格式化 JSON".to_string(),
            summary: "格式化并缩进 JSON 内容".to_string(),
            aliases: JSON_FORMAT_COMMAND_ALIASES
                .iter()
                .map(ToString::to_string)
                .collect(),
            keywords: vec![
                "json".to_string(),
                "format".to_string(),
                "pretty".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Clipboard],
            category: "text".to_string(),
            priority: 88,
        },
        ActionDescriptor {
            id: "base64_text".to_string(),
            title: "Base64 编解码".to_string(),
            summary: "自动识别 Base64，命中则解码，否则编码".to_string(),
            aliases: BASE64_COMMAND_ALIASES
                .iter()
                .map(ToString::to_string)
                .collect(),
            keywords: vec![
                "base64".to_string(),
                "encode".to_string(),
                "decode".to_string(),
                "text".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 86,
        },
        ActionDescriptor {
            id: "markdown_render".to_string(),
            title: "渲染 Markdown".to_string(),
            summary: "按 Markdown 语义渲染当前输入内容".to_string(),
            aliases: MARKDOWN_RENDER_COMMAND_ALIASES
                .iter()
                .map(ToString::to_string)
                .collect(),
            keywords: vec![
                "markdown".to_string(),
                "render".to_string(),
                "preview".to_string(),
                "md".to_string(),
                "text".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 85,
        },
        ActionDescriptor {
            id: "translate_text".to_string(),
            title: "翻译".to_string(),
            summary: "调用已配置翻译 LLM 翻译文本".to_string(),
            aliases: TRANSLATE_COMMAND_ALIASES
                .iter()
                .map(ToString::to_string)
                .collect(),
            keywords: vec![
                "translate".to_string(),
                "translation".to_string(),
                "fy".to_string(),
                "tr".to_string(),
                "text".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Ocr, Clipboard, Selection],
            category: "text".to_string(),
            priority: 96,
        },
        ActionDescriptor {
            id: "rag_answer".to_string(),
            title: "文档问答".to_string(),
            summary: "基于本地 RAG 索引回答问题".to_string(),
            aliases: RAG_ANSWER_COMMAND_ALIASES
                .iter()
                .map(ToString::to_string)
                .collect(),
            keywords: vec![
                "ask".to_string(),
                "qa".to_string(),
                "docs".to_string(),
                "knowledge".to_string(),
                "rag".to_string(),
            ],
            supported_input_modes: vec![Inline, Multiline, Clipboard, Selection],
            category: "knowledge".to_string(),
            priority: 94,
        },
    ]
}

fn slash_command_token(normalized_query: &str) -> Option<&str> {
    if !normalized_query.starts_with('/') {
        return None;
    }

    normalized_query.split_whitespace().next()
}

fn extract_json_format_payload(raw_text: &str) -> Option<&str> {
    extract_prefixed_payload(raw_text, &JSON_FORMAT_COMMAND_ALIASES)
}

fn extract_base64_payload(raw_text: &str) -> Option<&str> {
    extract_prefixed_payload(raw_text, &BASE64_COMMAND_ALIASES)
}

fn extract_markdown_render_payload(raw_text: &str) -> Option<&str> {
    extract_prefixed_payload(raw_text, &MARKDOWN_RENDER_COMMAND_ALIASES)
}

fn extract_translate_payload(raw_text: &str) -> Option<&str> {
    extract_prefixed_payload(raw_text, &TRANSLATE_COMMAND_ALIASES)
}

fn extract_rag_answer_payload(raw_text: &str) -> Option<&str> {
    extract_prefixed_payload(raw_text, &RAG_ANSWER_COMMAND_ALIASES)
}

#[cfg(test)]
mod tests {
    use super::MatcherService;
    use crate::domain::query::{InputMode, QueryPayload, SourceMetadata};

    fn query(mode: InputMode, raw_text: &str) -> QueryPayload {
        QueryPayload {
            mode,
            raw_text: raw_text.to_string(),
            segments: raw_text.lines().map(ToString::to_string).collect(),
            language: None,
            source_metadata: SourceMetadata {
                created_at_ms: 0,
                source_hint: "test".to_string(),
            },
        }
    }

    #[test]
    fn url_query_prefers_open_target_action() {
        let service = MatcherService::new();
        let matches = service
            .match_actions(&query(InputMode::Inline, "https://tauri.app"))
            .unwrap();

        assert_eq!(
            matches.first().map(|item| item.descriptor.id.as_str()),
            Some("open_target")
        );
    }

    #[test]
    fn multiline_query_filters_out_inline_only_actions() {
        let service = MatcherService::new();
        let matches = service
            .match_actions(&query(InputMode::Multiline, "/open\na\nb"))
            .unwrap();

        assert!(matches.iter().all(|item| {
            item.descriptor
                .supported_input_modes
                .contains(&InputMode::Multiline)
        }));
    }

    #[test]
    fn slash_query_only_exposes_executable_actions() {
        let service = MatcherService::new();
        let matches = service
            .match_actions(&query(InputMode::Inline, "/"))
            .unwrap();

        assert_eq!(matches.len(), 16);
        assert!(matches
            .iter()
            .any(|item| item.descriptor.id.as_str() == "clipboard_history"));
        assert!(matches
            .iter()
            .all(|item| item.descriptor.id.as_str() != "copy_text"));
    }

    #[test]
    fn format_command_with_payload_matches_json_action() {
        let service = MatcherService::new();
        let matches = service
            .match_actions(&query(InputMode::Inline, "/fmt {\"a\":1}"))
            .unwrap();

        assert_eq!(
            matches.first().map(|item| item.descriptor.id.as_str()),
            Some("json_pretty_print")
        );
    }

    #[test]
    fn base64_command_with_payload_matches_base64_action() {
        let service = MatcherService::new();
        let matches = service
            .match_actions(&query(InputMode::Inline, "/base64 dGVzdA=="))
            .unwrap();

        assert_eq!(
            matches.first().map(|item| item.descriptor.id.as_str()),
            Some("base64_text")
        );
    }

    #[test]
    fn base64_command_is_available_for_ocr_and_selection_inputs() {
        let service = MatcherService::new();

        for mode in [InputMode::Ocr, InputMode::Selection] {
            let matches = service
                .match_actions(&query(mode, "/base64 dGVzdA=="))
                .unwrap();

            assert_eq!(
                matches.first().map(|item| item.descriptor.id.as_str()),
                Some("base64_text")
            );
        }
    }

    #[test]
    fn markdown_command_with_payload_matches_markdown_action() {
        let service = MatcherService::new();
        let matches = service
            .match_actions(&query(InputMode::Inline, "/md # title"))
            .unwrap();

        assert_eq!(
            matches.first().map(|item| item.descriptor.id.as_str()),
            Some("markdown_render")
        );
    }

    #[test]
    fn markdown_command_with_attached_payload_matches_markdown_action() {
        let service = MatcherService::new();
        let matches = service
            .match_actions(&query(InputMode::Inline, "/md# title"))
            .unwrap();

        assert_eq!(
            matches.first().map(|item| item.descriptor.id.as_str()),
            Some("markdown_render")
        );
    }

    #[test]
    fn upper_command_with_payload_matches_uppercase_action() {
        let service = MatcherService::new();
        let matches = service
            .match_actions(&query(InputMode::Inline, "/upper hello"))
            .unwrap();

        assert_eq!(
            matches.first().map(|item| item.descriptor.id.as_str()),
            Some("uppercase_text")
        );
    }

    #[test]
    fn title_command_with_payload_matches_title_case_action() {
        let service = MatcherService::new();
        let matches = service
            .match_actions(&query(InputMode::Inline, "/title hello world"))
            .unwrap();

        assert_eq!(
            matches.first().map(|item| item.descriptor.id.as_str()),
            Some("title_case_text")
        );
    }

    #[test]
    fn translate_command_with_payload_matches_translate_action() {
        let service = MatcherService::new();
        let matches = service
            .match_actions(&query(InputMode::Inline, "/fy hello world"))
            .unwrap();

        assert_eq!(
            matches.first().map(|item| item.descriptor.id.as_str()),
            Some("translate_text")
        );
    }

    #[test]
    fn rag_command_with_payload_matches_rag_answer_action() {
        let service = MatcherService::new();
        let matches = service
            .match_actions(&query(InputMode::Inline, "/ask 解释 embedding 缓存"))
            .unwrap();

        assert_eq!(
            matches.first().map(|item| item.descriptor.id.as_str()),
            Some("rag_answer")
        );
    }
}
