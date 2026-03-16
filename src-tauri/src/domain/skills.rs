use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicSkillCatalog {
    pub root_path: String,
    pub exists: bool,
    pub skills: Vec<PublicSkillEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicSkillEntry {
    pub id: String,
    pub directory_name: String,
    pub relative_path: String,
    pub meta: PublicSkillMeta,
    pub directory_count: usize,
    pub file_count: usize,
    pub tree: SkillTreeNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PublicSkillMeta {
    pub name: Option<String>,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub license: Option<String>,
    #[serde(default)]
    pub metadata: Vec<PublicSkillMetaEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicSkillMetaEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillTreeNode {
    pub name: String,
    pub relative_path: String,
    pub kind: SkillTreeNodeKind,
    #[serde(default)]
    pub children: Vec<SkillTreeNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillTreeNodeKind {
    Directory,
    File,
}
