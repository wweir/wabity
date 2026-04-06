use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunningProcessKind {
    App,
    Process,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningProcessMatch {
    pub pid: u32,
    pub display_name: String,
    pub process_name: String,
    pub executable_path: Option<String>,
    pub app_bundle_path: Option<String>,
    pub kind: RunningProcessKind,
    pub score: i32,
}
