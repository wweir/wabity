use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    Inline,
    Multiline,
    Ocr,
    Clipboard,
    Selection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMetadata {
    pub created_at_ms: u64,
    pub source_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryPayload {
    pub mode: InputMode,
    pub raw_text: String,
    pub segments: Vec<String>,
    pub language: Option<String>,
    pub source_metadata: SourceMetadata,
}

impl QueryPayload {
    pub fn normalized_text(&self) -> String {
        self.raw_text.trim().to_lowercase()
    }

    pub fn is_empty(&self) -> bool {
        self.raw_text.trim().is_empty()
    }
}
