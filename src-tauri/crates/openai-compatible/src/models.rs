use std::collections::BTreeMap;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCompatibleModelEntry {
    pub id: String,
    pub identity_hint: Option<String>,
}

pub fn extract_model_ids(payload: &Value) -> Vec<String> {
    extract_model_entries(payload)
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

pub fn extract_model_entries(payload: &Value) -> Vec<OpenAiCompatibleModelEntry> {
    let mut models = BTreeMap::new();

    if let Some(items) = payload.get("data").and_then(Value::as_array) {
        for item in items {
            if let Some(model_id) = item
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let identity_hint = extract_model_identity_hint(item);
                models
                    .entry(model_id.to_string())
                    .and_modify(|current_hint: &mut Option<String>| {
                        if current_hint.is_none() && identity_hint.is_some() {
                            *current_hint = identity_hint.clone();
                        }
                    })
                    .or_insert(identity_hint);
            }
        }
    }

    models
        .into_iter()
        .map(|(id, identity_hint)| OpenAiCompatibleModelEntry { id, identity_hint })
        .collect()
}

fn extract_model_identity_hint(item: &Value) -> Option<String> {
    const DIRECT_KEYS: &[&str] = &[
        "digest",
        "sha256",
        "model_digest",
        "modelDigest",
        "model_sha256",
        "modelSha256",
        "checksum",
        "fingerprint",
        "model_fingerprint",
        "modelFingerprint",
    ];
    const NESTED_KEYS: &[&str] = &["details", "metadata", "model_info", "modelInfo"];

    for key in DIRECT_KEYS {
        if let Some(hint) = item
            .get(key)
            .and_then(Value::as_str)
            .and_then(normalize_model_identity_hint)
        {
            return Some(hint);
        }
    }

    for key in NESTED_KEYS {
        let Some(value) = item.get(key) else {
            continue;
        };
        for nested_key in DIRECT_KEYS {
            if let Some(hint) = value
                .get(nested_key)
                .and_then(Value::as_str)
                .and_then(normalize_model_identity_hint)
            {
                return Some(hint);
            }
        }
    }

    None
}

fn normalize_model_identity_hint(raw: &str) -> Option<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return None;
    }

    if let Some((algorithm, digest)) = trimmed.split_once(':') {
        if !algorithm.is_empty()
            && algorithm
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
            && digest.len() >= 16
            && digest.chars().all(|ch| ch.is_ascii_hexdigit())
        {
            return Some(format!("digest:{algorithm}:{digest}"));
        }
    }

    if trimmed.len() >= 16 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Some(format!("digest:hex:{trimmed}"));
    }

    None
}
