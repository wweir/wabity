use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpAgentConfig {
    pub id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub shell_command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<AcpMcpServerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AcpAgentCatalog {
    pub agents: Vec<AcpAgentConfig>,
    pub default_agent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AcpMcpServerCatalog {
    pub servers: Vec<AcpMcpServerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpNameValuePair {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum AcpMcpServerConfig {
    Stdio(AcpMcpServerStdioConfig),
    Http(AcpMcpServerHttpConfig),
    Sse(AcpMcpServerSseConfig),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpMcpServerStdioConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<AcpNameValuePair>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpMcpServerHttpConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<AcpNameValuePair>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpMcpServerSseConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<AcpNameValuePair>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AcpSessionStatus {
    Starting,
    Idle,
    Running,
    Error,
    Exited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AcpSessionErrorLevel {
    Recoverable,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AcpMessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpActionEvent {
    pub kind: String,
    pub title: String,
    pub correlation_id: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AcpMessageBlock {
    Thought { content: String },
    Actions { items: Vec<AcpActionEvent> },
    Content { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpSessionMessage {
    pub id: String,
    pub role: AcpMessageRole,
    pub blocks: Vec<AcpMessageBlock>,
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpSessionSummary {
    pub session_id: String,
    pub workspace_root: String,
    pub title: String,
    pub agent_id: Option<String>,
    pub agent_name: String,
    pub status: AcpSessionStatus,
    pub error_level: Option<AcpSessionErrorLevel>,
    pub attention: bool,
    pub is_active: bool,
    pub last_error: Option<String>,
    pub last_updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpSessionDetail {
    pub session: AcpSessionSummary,
    pub messages: Vec<AcpSessionMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRestoreNotice {
    pub session_id: String,
    pub workspace_root: String,
    pub message: String,
}
