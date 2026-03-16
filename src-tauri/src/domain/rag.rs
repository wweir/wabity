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
    pub finished_at_ms: u64,
}
