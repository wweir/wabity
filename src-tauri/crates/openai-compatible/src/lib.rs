mod client;
mod extract;
mod models;
mod parsing;
mod streaming;

pub use client::{normalize_base_url, OpenAiCompatibleClient, OpenAiCompatibleResponseFormat};
pub use extract::{
    describe_chat_completions_response_issue, extract_chat_completions_message_parts,
    extract_chat_completions_text, extract_responses_text, extract_text_content,
    ChatCompletionsMessageParts,
};
pub use models::{extract_model_entries, extract_model_ids, OpenAiCompatibleModelEntry};
pub use parsing::{
    body_preview, extract_provider_error_message, parse_json_or_sse_payload, parse_json_payload,
};

#[cfg(test)]
use streaming::StreamingPayloadCollector;

#[cfg(test)]
mod tests {
    use super::{
        body_preview, describe_chat_completions_response_issue,
        extract_chat_completions_message_parts, extract_chat_completions_text,
        extract_model_entries, extract_model_ids, extract_provider_error_message,
        extract_responses_text, normalize_base_url, parse_json_or_sse_payload,
        OpenAiCompatibleModelEntry, StreamingPayloadCollector,
    };
    use serde_json::json;

    #[test]
    fn normalize_base_url_trims_trailing_slash() {
        let normalized = normalize_base_url(
            " https://api.openai.example.com/v1/ ",
            "LLM provider base URL",
        )
        .expect("base url should normalize");

        assert_eq!(normalized, "https://api.openai.example.com/v1");
    }

    #[test]
    fn parse_json_or_sse_payload_accepts_plain_json() {
        let payload = parse_json_or_sse_payload(
            r#"{"id":"resp_123","output":[{"type":"message","content":[{"type":"output_text","text":"ok"}]}]}"#,
            "responses payload",
        )
        .expect("plain JSON response should parse");

        assert_eq!(payload["id"], "resp_123");
    }

    #[test]
    fn parse_json_or_sse_payload_accepts_sse_completed_event() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "event: response.created\n",
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_ignore\"}}\n\n",
                "event: response.completed\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"ok\"}]}]}}\n\n",
                "data: [DONE]\n",
            ),
            "responses payload",
        )
        .expect("SSE completed response should parse");

        assert_eq!(payload["id"], "resp_123");
    }

    #[test]
    fn parse_json_or_sse_payload_reconstructs_responses_delta_stream() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "event: response.created\n",
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_123\",\"output\":[]}}\n\n",
                "event: response.output_item.added\n",
                "data: {\"type\":\"response.output_item.added\",\"response_id\":\"resp_123\",\"output_index\":0,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n",
                "event: response.output_text.delta\n",
                "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"content_index\":0,\"delta\":\"hel\"}\n\n",
                "event: response.output_text.delta\n",
                "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"content_index\":0,\"delta\":\"lo\"}\n\n",
                "data: [DONE]\n",
            ),
            "responses payload",
        )
        .expect("responses delta stream should reconstruct");

        assert_eq!(payload["id"], "resp_123");
        assert_eq!(extract_responses_text(&payload).as_deref(), Some("hello"));
    }

    #[test]
    fn parse_json_or_sse_payload_preserves_responses_delta_boundary_spaces() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "event: response.output_item.added\n",
                "data: {\"type\":\"response.output_item.added\",\"response_id\":\"resp_123\",\"output_index\":0,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n",
                "event: response.output_text.delta\n",
                "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"content_index\":0,\"delta\":\"The\"}\n\n",
                "event: response.output_text.delta\n",
                "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"content_index\":0,\"delta\":\" advantage\"}\n\n",
                "event: response.output_text.delta\n",
                "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"content_index\":0,\"delta\":\" of doing this\"}\n\n",
                "data: [DONE]\n",
            ),
            "responses payload",
        )
        .expect("responses delta stream should reconstruct");

        assert_eq!(
            extract_responses_text(&payload).as_deref(),
            Some("The advantage of doing this")
        );
    }

    #[test]
    fn streaming_payload_collector_emits_responses_text_deltas() {
        let mut collector = StreamingPayloadCollector::default();
        let mut deltas = Vec::new();

        collector
            .push_bytes_with_text_stream(
                concat!(
                    "event: response.output_item.added\n",
                    "data: {\"type\":\"response.output_item.added\",\"response_id\":\"resp_123\",\"output_index\":0,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n",
                    "event: response.output_text.delta\n",
                    "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"content_index\":0,\"delta\":\"hel\"}\n\n",
                    "event: response.output_text.delta\n",
                    "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"content_index\":0,\"delta\":\"lo\"}\n\n",
                    "data: [DONE]\n",
                )
                .as_bytes(),
                &mut |delta| deltas.push(delta.to_string()),
            )
            .expect("stream chunk should parse");

        let payload = collector
            .finish("responses payload")
            .expect("stream should finish");
        assert_eq!(deltas, vec!["hel".to_string(), "lo".to_string()]);
        assert_eq!(extract_responses_text(&payload).as_deref(), Some("hello"));
    }

    #[test]
    fn streaming_payload_collector_emits_responses_delta_boundary_spaces() {
        let mut collector = StreamingPayloadCollector::default();
        let mut deltas = Vec::new();

        collector
            .push_bytes_with_text_stream(
                concat!(
                    "event: response.output_item.added\n",
                    "data: {\"type\":\"response.output_item.added\",\"response_id\":\"resp_123\",\"output_index\":0,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n",
                    "event: response.output_text.delta\n",
                    "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"content_index\":0,\"delta\":\"The\"}\n\n",
                    "event: response.output_text.delta\n",
                    "data: {\"type\":\"response.output_text.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"content_index\":0,\"delta\":\" advantage\"}\n\n",
                    "data: [DONE]\n",
                )
                .as_bytes(),
                &mut |delta| deltas.push(delta.to_string()),
            )
            .expect("stream chunk should parse");

        assert_eq!(deltas, vec!["The".to_string(), " advantage".to_string()]);
    }

    #[test]
    fn parse_json_or_sse_payload_reconstructs_chat_completion_chunk_stream() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hel\"}}]}\n\n",
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n",
            ),
            "chat/completions payload",
        )
        .expect("chat completion chunk stream should reconstruct");

        assert_eq!(payload["id"], "chatcmpl_123");
        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn parse_json_or_sse_payload_preserves_chat_delta_boundary_spaces() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"The\"}}]}\n\n",
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" advantage\"}}]}\n\n",
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" of doing this\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n",
            ),
            "chat/completions payload",
        )
        .expect("chat completion chunk stream should reconstruct");

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("The advantage of doing this")
        );
    }

    #[test]
    fn streaming_payload_collector_emits_chat_text_deltas() {
        let mut collector = StreamingPayloadCollector::default();
        let mut deltas = Vec::new();

        collector
            .push_bytes_with_text_stream(
                concat!(
                    "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hel\"}}]}\n\n",
                    "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n",
                )
                .as_bytes(),
                &mut |delta| deltas.push(delta.to_string()),
            )
            .expect("stream chunk should parse");

        let payload = collector
            .finish("chat payload")
            .expect("stream should finish");
        assert_eq!(deltas, vec!["hel".to_string(), "lo".to_string()]);
        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn streaming_payload_collector_emits_chat_delta_boundary_spaces() {
        let mut collector = StreamingPayloadCollector::default();
        let mut deltas = Vec::new();

        collector
            .push_bytes_with_text_stream(
                concat!(
                    "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"The\"}}]}\n\n",
                    "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" advantage\"},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n",
                )
                .as_bytes(),
                &mut |delta| deltas.push(delta.to_string()),
            )
            .expect("stream chunk should parse");

        let payload = collector
            .finish("chat payload")
            .expect("stream should finish");
        assert_eq!(deltas, vec!["The".to_string(), " advantage".to_string()]);
        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("The advantage")
        );
    }

    #[test]
    fn parse_json_or_sse_payload_reconstructs_chat_completion_reasoning_content_stream() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"hel\"}}]}\n\n",
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n",
            ),
            "chat/completions payload",
        )
        .expect("chat completion reasoning chunk stream should reconstruct");

        assert_eq!(payload["id"], "chatcmpl_123");
        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn parse_json_or_sse_payload_routes_reasoning_items_embedded_in_chat_content() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":[{\"type\":\"thinking\",\"text\":\"step 1\"}]}}]}\n\n",
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":[{\"type\":\"text\",\"text\":\"final answer\"}]},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n",
            ),
            "chat/completions payload",
        )
        .expect("chat completion mixed content chunk stream should reconstruct");

        let parts = extract_chat_completions_message_parts(&payload)
            .expect("chat completion mixed content chunk should yield message parts");
        assert_eq!(parts.content.as_deref(), Some("final answer"));
        assert_eq!(parts.reasoning.as_deref(), Some("step 1"));
    }

    #[test]
    fn parse_json_or_sse_payload_reconstructs_chat_tool_call_stream() {
        let payload = parse_json_or_sse_payload(
            concat!(
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_123\",\"type\":\"function\",\"function\":{\"name\":\"wabity.rag.query\",\"arguments\":\"{\\\"q\\\":\\\"hel\"}}]}}]}\n\n",
                "data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"lo\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
                "data: [DONE]\n",
            ),
            "chat/completions payload",
        )
        .expect("chat completion tool call chunk stream should reconstruct");

        assert_eq!(
            payload["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "wabity.rag.query"
        );
        assert_eq!(
            payload["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
            "{\"q\":\"hello\"}"
        );
    }

    #[test]
    fn streaming_payload_collector_handles_split_sse_frames() {
        let chunks = [
            b"data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.comple".as_slice(),
            b"tion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hel\"}}]}\n\n".as_slice(),
            b"data: {\"id\":\"chatcmpl_123\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n".as_slice(),
            b"data: [DONE]\n".as_slice(),
        ];

        let mut collector = StreamingPayloadCollector::default();
        for chunk in chunks {
            collector
                .push_bytes(chunk)
                .expect("split SSE chunk should stream");
        }
        let payload = collector.finish("chat/completions payload").unwrap();

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn body_preview_collapses_whitespace_and_truncates() {
        let preview = body_preview(&format!("  a\n b\t{}  ", "x".repeat(300)));

        assert!(preview.starts_with("a b"));
        assert!(preview.ends_with("..."));
        assert!(preview.len() <= 243);
    }

    #[test]
    fn extract_provider_error_message_prefers_nested_fields() {
        let message = extract_provider_error_message(
            r#"{"error":{"details":[{"message":"quota exceeded"}]}}"#,
        );

        assert_eq!(message, "quota exceeded");
    }

    #[test]
    fn extract_responses_text_reads_output_text_and_refusal_parts() {
        let payload = json!({
            "output": [
                {
                    "type": "message",
                    "content": [
                        { "type": "output_text", "text": "first" },
                        { "type": "refusal", "refusal": "second" }
                    ]
                }
            ]
        });

        assert_eq!(
            extract_responses_text(&payload).as_deref(),
            Some("first\nsecond")
        );
    }

    #[test]
    fn extract_chat_completions_text_reads_array_content() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": [
                            { "type": "text", "text": "hello" },
                            { "type": "text", "text": { "value": "world" } }
                        ]
                    }
                }
            ]
        });

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("hello\nworld")
        );
    }

    #[test]
    fn extract_chat_completions_text_keeps_array_content_part_boundaries() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": [
                            { "type": "text", "text": "hello" },
                            { "type": "text", "text": "world" }
                        ]
                    }
                }
            ]
        });

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("hello\nworld")
        );
    }

    #[test]
    fn extract_chat_completions_text_reads_output_text_parts() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": [
                            { "type": "output_text", "output_text": "hello" },
                            { "type": "output_text", "output_text": { "value": "world" } }
                        ]
                    }
                }
            ]
        });

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("hello\nworld")
        );
    }

    #[test]
    fn extract_chat_completions_text_reads_message_level_refusal() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": [],
                        "refusal": "I can not comply."
                    }
                }
            ]
        });

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("I can not comply.")
        );
    }

    #[test]
    fn extract_chat_completions_text_reads_reasoning_content_when_content_is_empty() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "reasoning_content": "final answer from compatibility field"
                    }
                }
            ]
        });

        assert_eq!(
            extract_chat_completions_text(&payload).as_deref(),
            Some("final answer from compatibility field")
        );
    }

    #[test]
    fn extract_chat_completions_message_parts_separates_content_and_reasoning() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "final answer",
                        "reasoning_content": "step 1\nstep 2"
                    }
                }
            ]
        });

        let parts = extract_chat_completions_message_parts(&payload)
            .expect("chat/completions payload should produce message parts");

        assert_eq!(parts.content.as_deref(), Some("final answer"));
        assert_eq!(parts.reasoning.as_deref(), Some("step 1\nstep 2"));
    }

    #[test]
    fn extract_chat_completions_message_parts_reads_reasoning_from_content_items() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": [
                            {
                                "type": "thinking",
                                "text": "step 1"
                            },
                            {
                                "type": "text",
                                "text": "final answer"
                            }
                        ]
                    }
                }
            ]
        });

        let parts = extract_chat_completions_message_parts(&payload)
            .expect("chat/completions payload should produce message parts");

        assert_eq!(parts.content.as_deref(), Some("final answer"));
        assert_eq!(parts.reasoning.as_deref(), Some("step 1"));
    }

    #[test]
    fn extract_chat_completions_message_parts_splits_think_tagged_content() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": "<think>step 1</think>\n\nfinal answer"
                    }
                }
            ]
        });

        let parts = extract_chat_completions_message_parts(&payload)
            .expect("chat/completions payload should produce message parts");

        assert_eq!(parts.content.as_deref(), Some("final answer"));
        assert_eq!(parts.reasoning.as_deref(), Some("step 1"));
    }

    #[test]
    fn extract_chat_completions_message_parts_strips_self_closing_think_marker_from_content() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": "<think>step 1</think>\n\n<think />\n\nfinal answer"
                    }
                }
            ]
        });

        let parts = extract_chat_completions_message_parts(&payload)
            .expect("chat/completions payload should produce message parts");

        assert_eq!(parts.content.as_deref(), Some("final answer"));
        assert_eq!(parts.reasoning.as_deref(), Some("step 1"));
    }

    #[test]
    fn describe_chat_completions_response_issue_summarizes_empty_content() {
        let payload = json!({
            "choices": [
                {
                    "finish_reason": "stop",
                    "message": {
                        "role": "assistant",
                        "content": [],
                        "tool_calls": []
                    }
                }
            ]
        });

        let summary = describe_chat_completions_response_issue(&payload);

        assert!(summary.contains("finish_reason=stop"));
        assert!(summary.contains("tool_calls=0"));
        assert!(summary.contains("content=array(len=0)"));
        assert!(summary.contains("reasoning_content=missing"));
    }

    #[test]
    fn extract_model_ids_deduplicates_and_sorts_ids() {
        let payload = json!({
            "data": [
                { "id": "gpt-4.1" },
                { "id": "gpt-4.1-mini" },
                { "id": "gpt-4.1" }
            ]
        });

        assert_eq!(
            extract_model_ids(&payload),
            vec!["gpt-4.1".to_string(), "gpt-4.1-mini".to_string()]
        );
    }

    #[test]
    fn extract_model_entries_preserves_digest_identity_hint() {
        let payload = json!({
            "data": [
                {
                    "id": "embedding-a",
                    "digest": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                },
                {
                    "id": "embedding-b",
                    "details": {
                        "model_digest": "0123456789abcdef0123456789abcdef"
                    }
                }
            ]
        });

        assert_eq!(
            extract_model_entries(&payload),
            vec![
                OpenAiCompatibleModelEntry {
                    id: "embedding-a".to_string(),
                    identity_hint: Some(
                        "digest:sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
                    ),
                },
                OpenAiCompatibleModelEntry {
                    id: "embedding-b".to_string(),
                    identity_hint: Some(
                        "digest:hex:0123456789abcdef0123456789abcdef".to_string()
                    ),
                }
            ]
        );
    }
}
