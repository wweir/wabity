use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationTrigger {
    QuestionAnswer,
    AcpPrompt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationOutcome {
    Success,
    Error,
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationPermissionState {
    Granted,
    Denied,
    Prompt,
    PromptWithRationale,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionNotification {
    pub trigger: NotificationTrigger,
    pub outcome: NotificationOutcome,
    pub title: String,
    pub body: String,
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotificationContentPreview {
    Hidden,
    #[default]
    Brief,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSettings {
    pub enabled: bool,
    pub notify_question_answer_completion: bool,
    pub notify_acp_prompt_completion: bool,
    pub only_when_launcher_in_background: bool,
    #[serde(default)]
    pub content_preview: NotificationContentPreview,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            notify_question_answer_completion: true,
            notify_acp_prompt_completion: true,
            only_when_launcher_in_background: true,
            content_preview: NotificationContentPreview::Brief,
        }
    }
}
