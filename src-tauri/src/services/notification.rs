use std::sync::Arc;

use tauri::AppHandle;
use tokio::sync::RwLock as AsyncRwLock;

use crate::{
    domain::notification::{
        CompletionNotification, NotificationContentPreview, NotificationOutcome,
        NotificationPermissionState, NotificationSettings, NotificationTrigger,
    },
    infrastructure::{
        config::ConfigStore,
        notification::{create_system_notification_backend, SystemNotificationBackend},
        window,
    },
    state::ShortcutRuntimeState,
};

const NOTIFICATION_PREVIEW_MAX_CHARS: usize = 140;
const NOTIFICATION_PREVIEW_MAX_SEGMENTS: usize = 2;

#[derive(Clone)]
pub struct NotificationService {
    app_handle: AppHandle,
    shortcut_state: ShortcutRuntimeState,
    config_store: Arc<AsyncRwLock<ConfigStore>>,
    backend: Arc<dyn SystemNotificationBackend>,
}

impl NotificationService {
    pub fn new(
        app_handle: AppHandle,
        shortcut_state: ShortcutRuntimeState,
        config_store: Arc<AsyncRwLock<ConfigStore>>,
    ) -> Self {
        let backend = create_system_notification_backend(app_handle.clone());
        Self {
            app_handle,
            shortcut_state,
            config_store,
            backend,
        }
    }

    pub async fn notify_question_answer_success(&self, response_preview: Option<String>) {
        let payload = CompletionNotification {
            trigger: NotificationTrigger::QuestionAnswer,
            outcome: NotificationOutcome::Success,
            title: "文档问答已完成".to_string(),
            body: "返回 launcher 查看完整回答".to_string(),
            preview: response_preview,
        };
        self.maybe_notify(payload).await;
    }

    pub async fn notify_question_answer_failure(&self, failure_detail: Option<String>) {
        let payload = CompletionNotification {
            trigger: NotificationTrigger::QuestionAnswer,
            outcome: NotificationOutcome::Error,
            title: "文档问答未完成".to_string(),
            body: summarize_question_answer_failure(failure_detail.as_deref()),
            preview: None,
        };
        self.maybe_notify(payload).await;
    }

    pub async fn notify_acp_prompt_success(
        &self,
        agent_name: &str,
        response_preview: Option<String>,
    ) {
        let normalized_agent_name = agent_name.trim();
        let title = if normalized_agent_name.is_empty() {
            "Agent 已完成当前任务".to_string()
        } else {
            format!("{normalized_agent_name} 已完成当前任务")
        };
        let payload = CompletionNotification {
            trigger: NotificationTrigger::AcpPrompt,
            outcome: NotificationOutcome::Success,
            title,
            body: "返回 launcher 查看完整结果".to_string(),
            preview: response_preview,
        };
        self.maybe_notify(payload).await;
    }

    pub async fn notify_acp_prompt_failure(
        &self,
        agent_name: &str,
        failure_detail: Option<String>,
    ) {
        let normalized_agent_name = agent_name.trim();
        let title = if normalized_agent_name.is_empty() {
            "Agent 未完成当前任务".to_string()
        } else {
            format!("{normalized_agent_name} 未完成当前任务")
        };
        let payload = CompletionNotification {
            trigger: NotificationTrigger::AcpPrompt,
            outcome: NotificationOutcome::Error,
            title,
            body: summarize_acp_failure(failure_detail.as_deref()),
            preview: None,
        };
        self.maybe_notify(payload).await;
    }

    async fn maybe_notify(&self, payload: CompletionNotification) {
        let settings = match self.notification_settings().await {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(?error, "failed to load notification settings");
                return;
            }
        };

        let launcher_foreground = match window::launcher_is_effectively_foreground(
            &self.app_handle,
            &self.shortcut_state,
        ) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(?error, "failed to inspect launcher foreground state");
                return;
            }
        };

        let permission_state = match self.backend.permission_state() {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(?error, "failed to inspect notification permission state");
                return;
            }
        };

        if !should_emit_completion_notification(
            &settings,
            payload.trigger,
            launcher_foreground,
            permission_state,
        ) {
            return;
        }

        let rendered_payload = render_completion_notification(payload, settings.content_preview);

        if let Err(error) = self.backend.notify(&rendered_payload) {
            tracing::warn!(
                ?error,
                trigger = ?rendered_payload.trigger,
                outcome = ?rendered_payload.outcome,
                "failed to show system notification"
            );
        }
    }

    async fn notification_settings(&self) -> anyhow::Result<NotificationSettings> {
        let store = self.config_store.read().await;
        Ok(store.load().await?.notification)
    }
}

fn render_completion_notification(
    payload: CompletionNotification,
    content_preview: NotificationContentPreview,
) -> CompletionNotification {
    let body = match content_preview {
        NotificationContentPreview::Hidden => payload.body.clone(),
        NotificationContentPreview::Brief => {
            summarize_notification_preview(payload.preview.as_deref())
                .unwrap_or_else(|| payload.body.clone())
        }
    };

    CompletionNotification { body, ..payload }
}

fn should_emit_completion_notification(
    settings: &NotificationSettings,
    trigger: NotificationTrigger,
    launcher_foreground: bool,
    permission_state: NotificationPermissionState,
) -> bool {
    if !settings.enabled {
        return false;
    }

    let trigger_enabled = match trigger {
        NotificationTrigger::QuestionAnswer => settings.notify_question_answer_completion,
        NotificationTrigger::AcpPrompt => settings.notify_acp_prompt_completion,
    };
    if !trigger_enabled {
        return false;
    }

    if settings.only_when_launcher_in_background && launcher_foreground {
        return false;
    }

    permission_state == NotificationPermissionState::Granted
}

fn summarize_notification_preview(text: Option<&str>) -> Option<String> {
    let mut preview = String::new();
    let mut collected_segments = 0usize;
    let mut inside_fenced_code = false;

    for line in text.unwrap_or_default().lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            inside_fenced_code = !inside_fenced_code;
            continue;
        }
        if inside_fenced_code || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let normalized = normalize_notification_preview_line(trimmed);
        if normalized.is_empty() {
            continue;
        }

        if !preview.is_empty() {
            preview.push(' ');
        }
        preview.push_str(&normalized);
        collected_segments += 1;

        if collected_segments >= NOTIFICATION_PREVIEW_MAX_SEGMENTS
            || preview.chars().count() >= NOTIFICATION_PREVIEW_MAX_CHARS
        {
            break;
        }
    }

    if preview.is_empty() {
        None
    } else {
        Some(truncate_notification_text(
            preview.trim().to_string(),
            NOTIFICATION_PREVIEW_MAX_CHARS,
        ))
    }
}

fn normalize_notification_preview_line(line: &str) -> String {
    let stripped = line
        .trim_start_matches(['#', '>', '`', '*', '+', '-', '|'])
        .trim_start();
    let stripped = strip_ordered_list_prefix(stripped).unwrap_or(stripped);

    collapse_whitespace(stripped)
}

fn strip_ordered_list_prefix(line: &str) -> Option<&str> {
    let digit_count = line
        .chars()
        .take_while(|char| char.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }

    let suffix = line.get(digit_count..)?;
    let remainder = suffix
        .strip_prefix(". ")
        .or_else(|| suffix.strip_prefix(") "))?;
    Some(remainder.trim_start())
}

fn collapse_whitespace(text: &str) -> String {
    let mut collapsed = String::new();
    let mut previous_was_whitespace = false;

    for char in text.chars() {
        if char.is_whitespace() {
            if !collapsed.is_empty() && !previous_was_whitespace {
                collapsed.push(' ');
            }
            previous_was_whitespace = true;
            continue;
        }

        collapsed.push(char);
        previous_was_whitespace = false;
    }

    collapsed.trim().to_string()
}

fn truncate_notification_text(text: String, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text;
    }

    let truncated = text
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    format!("{}...", truncated.trim_end())
}

fn summarize_question_answer_failure(detail: Option<&str>) -> String {
    let normalized = detail.unwrap_or_default().to_ascii_lowercase();

    if normalized.contains("timeout") || normalized.contains("timed out") {
        return "回答超时，请回到 launcher 重试".to_string();
    }
    if normalized.contains("rate limit") || normalized.contains("429") {
        return "模型限流，请稍后重试".to_string();
    }
    if normalized.contains("unauthorized")
        || normalized.contains("forbidden")
        || normalized.contains("api key")
        || normalized.contains("401")
        || normalized.contains("403")
    {
        return "模型鉴权失败，请检查问答模型配置".to_string();
    }

    "回答失败，请回到 launcher 查看详情".to_string()
}

fn summarize_acp_failure(detail: Option<&str>) -> String {
    let normalized = detail.unwrap_or_default().to_ascii_lowercase();

    if normalized.contains("timeout") || normalized.contains("timed out") {
        return "Agent 响应超时，请回到 launcher 查看详情".to_string();
    }
    if normalized.contains("exited")
        || normalized.contains("exit")
        || normalized.contains("broken pipe")
        || normalized.contains("已退出")
    {
        return "Agent 已退出，请回到 launcher 查看详情".to_string();
    }
    if normalized.contains("permission denied") || normalized.contains("operation not permitted") {
        return "Agent 权限不足，请回到 launcher 查看详情".to_string();
    }

    "Agent 执行失败，请回到 launcher 查看详情".to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        render_completion_notification, should_emit_completion_notification,
        summarize_notification_preview,
    };
    use crate::domain::notification::{
        CompletionNotification, NotificationContentPreview, NotificationOutcome,
        NotificationPermissionState, NotificationSettings, NotificationTrigger,
    };

    #[test]
    fn notification_emission_requires_global_enable() {
        let settings = NotificationSettings {
            enabled: false,
            ..NotificationSettings::default()
        };

        assert!(!should_emit_completion_notification(
            &settings,
            NotificationTrigger::QuestionAnswer,
            false,
            NotificationPermissionState::Granted,
        ));
    }

    #[test]
    fn notification_emission_respects_trigger_specific_toggle() {
        let settings = NotificationSettings {
            notify_acp_prompt_completion: false,
            ..NotificationSettings {
                enabled: true,
                ..NotificationSettings::default()
            }
        };

        assert!(!should_emit_completion_notification(
            &settings,
            NotificationTrigger::AcpPrompt,
            false,
            NotificationPermissionState::Granted,
        ));
        assert!(should_emit_completion_notification(
            &settings,
            NotificationTrigger::QuestionAnswer,
            false,
            NotificationPermissionState::Granted,
        ));
    }

    #[test]
    fn notification_emission_skips_foreground_when_background_only_enabled() {
        let settings = NotificationSettings {
            enabled: true,
            only_when_launcher_in_background: true,
            ..NotificationSettings::default()
        };

        assert!(!should_emit_completion_notification(
            &settings,
            NotificationTrigger::QuestionAnswer,
            true,
            NotificationPermissionState::Granted,
        ));
    }

    #[test]
    fn notification_emission_requires_granted_permission() {
        let settings = NotificationSettings {
            enabled: true,
            ..NotificationSettings::default()
        };

        assert!(!should_emit_completion_notification(
            &settings,
            NotificationTrigger::QuestionAnswer,
            false,
            NotificationPermissionState::Unsupported,
        ));
        assert!(!should_emit_completion_notification(
            &settings,
            NotificationTrigger::QuestionAnswer,
            false,
            NotificationPermissionState::Prompt,
        ));
    }

    #[test]
    fn notification_preview_extracts_meaningful_lines() {
        let preview = summarize_notification_preview(Some(
            "\n# 总结\n\n```rust\nlet hidden = true;\n```\n- 第一条回答\n第二条补充\n",
        ));

        assert_eq!(preview.as_deref(), Some("第一条回答 第二条补充"));
    }

    #[test]
    fn notification_preview_can_be_hidden() {
        let payload = CompletionNotification {
            trigger: NotificationTrigger::QuestionAnswer,
            outcome: NotificationOutcome::Success,
            title: "文档问答已完成".to_string(),
            body: "返回 launcher 查看完整回答".to_string(),
            preview: Some("这是通知摘要".to_string()),
        };

        let rendered = render_completion_notification(payload, NotificationContentPreview::Hidden);

        assert_eq!(rendered.body, "返回 launcher 查看完整回答");
    }

    #[test]
    fn notification_preview_uses_brief_mode_when_available() {
        let payload = CompletionNotification {
            trigger: NotificationTrigger::QuestionAnswer,
            outcome: NotificationOutcome::Success,
            title: "文档问答已完成".to_string(),
            body: "返回 launcher 查看完整回答".to_string(),
            preview: Some("第一行回答\n第二行回答".to_string()),
        };

        let rendered = render_completion_notification(payload, NotificationContentPreview::Brief);

        assert_eq!(rendered.body, "第一行回答 第二行回答");
    }
}
