use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    pub root_path: String,
    pub recent_roots: Vec<String>,
    pub home_path: Option<String>,
    pub display_home_as_tilde: bool,
}
