use serde::{Deserialize, Serialize};

pub const MAX_PINNED_CLIPBOARD_ENTRIES: usize = 5;
pub const MAX_RECENT_CLIPBOARD_ENTRIES: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardHistoryEntry {
    pub id: String,
    pub text: String,
    pub pinned: bool,
    pub last_seen_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardHistorySnapshot {
    #[serde(default)]
    pub pinned_entries: Vec<ClipboardHistoryEntry>,
    #[serde(default)]
    pub recent_entries: Vec<ClipboardHistoryEntry>,
}
