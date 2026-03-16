use serde::{Deserialize, Serialize};

use crate::domain::query::InputMode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionDescriptor {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub aliases: Vec<String>,
    pub keywords: Vec<String>,
    pub supported_input_modes: Vec<InputMode>,
    pub category: String,
    pub priority: u16,
}

impl ActionDescriptor {
    pub fn supports(&self, mode: InputMode) -> bool {
        self.supported_input_modes.contains(&mode)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionMatch {
    pub descriptor: ActionDescriptor,
    pub score: i32,
}
