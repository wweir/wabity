use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagScanResult {
    pub database_path: String,
    pub source_count: usize,
    pub scanned_file_count: usize,
    pub indexed_file_count: usize,
    pub skipped_file_count: usize,
    pub chunk_count: usize,
    pub warning_count: usize,
    pub recent_warnings: Vec<String>,
    pub finished_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RagRuntimePhase {
    #[default]
    Idle,
    Scanning,
    Indexing,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagRuntimeStatus {
    pub phase: RagRuntimePhase,
    pub scanned_file_count: usize,
    pub completed_file_count: usize,
    pub total_file_count: usize,
    pub pending_file_count: usize,
    pub warning_count: usize,
    pub recent_warnings: Vec<String>,
    pub last_error: Option<String>,
    pub updated_at_ms: u64,
}

impl Default for RagRuntimeStatus {
    fn default() -> Self {
        Self {
            phase: RagRuntimePhase::Idle,
            scanned_file_count: 0,
            completed_file_count: 0,
            total_file_count: 0,
            pending_file_count: 0,
            warning_count: 0,
            recent_warnings: Vec::new(),
            last_error: None,
            updated_at_ms: 0,
        }
    }
}
