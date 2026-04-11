use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::{
    domain::settings::{LlmProviderConfig, LlmSettings, RagSettings},
    services::document_extract::DocumentKind,
};

use super::{
    embedding::embedding_fingerprint,
    model::{
        EmbeddingTargetIdentity, RagRuntimeInputs, RagRuntimeStartMode, ResolvedRagConfig,
        RAG_DB_DIR_NAME, RAG_SQLITE_DB_FILE_NAME,
    },
};

impl RagRuntimeInputs {
    pub(crate) fn from_settings(settings: &RagSettings, llm_settings: &LlmSettings) -> Self {
        let embedding_provider =
            settings
                .embedding_provider_id
                .as_deref()
                .and_then(|provider_id| {
                    llm_settings
                        .providers
                        .iter()
                        .find(|provider| provider.id == provider_id)
                        .cloned()
                });

        Self {
            settings: settings.clone(),
            embedding_provider,
        }
    }

    pub(crate) fn source_directory_set(&self) -> BTreeSet<String> {
        self.settings
            .source_directories
            .iter()
            .map(|directory| normalize_runtime_source_directory(directory))
            .filter(|directory| !directory.is_empty())
            .collect()
    }

    pub(crate) fn ignore_glob_set(&self) -> BTreeSet<String> {
        self.settings
            .ignore_globs
            .iter()
            .map(|pattern| pattern.trim().to_string())
            .filter(|pattern| !pattern.is_empty())
            .collect()
    }
}

pub(crate) fn rag_database_path(config_dir: &Path) -> PathBuf {
    config_dir.join(RAG_DB_DIR_NAME)
}

pub(crate) fn rag_sqlite_database_path(data_dir: &Path) -> PathBuf {
    rag_database_path(data_dir).join(RAG_SQLITE_DB_FILE_NAME)
}

pub(crate) fn classify_rag_runtime_start(
    previous: Option<&RagRuntimeInputs>,
    next: &RagRuntimeInputs,
) -> RagRuntimeStartMode {
    let Some(previous) = previous else {
        return RagRuntimeStartMode::ReuseIndex;
    };

    let source_directories_changed = previous.source_directory_set() != next.source_directory_set();
    let ignore_globs_changed = previous.ignore_glob_set() != next.ignore_glob_set();
    let embedding_target_changed = rag_embedding_target_changed(previous, next);

    if source_directories_changed || ignore_globs_changed || embedding_target_changed {
        RagRuntimeStartMode::RebuildIndex
    } else {
        RagRuntimeStartMode::ReuseIndex
    }
}

pub(crate) fn rag_embedding_target_changed(
    previous: &RagRuntimeInputs,
    next: &RagRuntimeInputs,
) -> bool {
    effective_embedding_target(previous) != effective_embedding_target(next)
}

pub(crate) fn rag_settings_disabled(settings: &RagSettings) -> bool {
    settings
        .source_directories
        .iter()
        .all(|directory| directory.trim().is_empty())
        && settings
            .embedding_provider_id
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
}

pub(crate) fn effective_embedding_target(
    inputs: &RagRuntimeInputs,
) -> Option<EmbeddingTargetIdentity> {
    let provider = inputs.embedding_provider.as_ref()?;
    Some(infer_embedding_target_identity(
        provider.base_url.trim().trim_end_matches('/'),
        provider.model_name(),
        provider.model_identity_hint.as_deref(),
    ))
}

pub(crate) fn infer_embedding_target_identity(
    normalized_base_url: &str,
    model_name: &str,
    model_identity_hint: Option<&str>,
) -> EmbeddingTargetIdentity {
    if let Some(identity_hint) = model_identity_hint.and_then(parse_stable_model_identity_hint) {
        return identity_hint;
    }

    if let Some(digest) = extract_stable_model_digest(model_name) {
        return EmbeddingTargetIdentity::StableModel {
            namespace: "digest",
            model_identity: digest,
        };
    }

    if let Some(namespace) = managed_embedding_model_namespace(normalized_base_url) {
        return EmbeddingTargetIdentity::StableModel {
            namespace,
            model_identity: model_name.to_string(),
        };
    }

    EmbeddingTargetIdentity::EndpointBound {
        normalized_base_url: normalized_base_url.to_string(),
        model_identity: model_name.to_string(),
    }
}

pub(crate) fn parse_stable_model_identity_hint(
    identity_hint: &str,
) -> Option<EmbeddingTargetIdentity> {
    let trimmed = identity_hint.trim();
    let digest = trimmed.strip_prefix("digest:")?;
    (!digest.is_empty()).then_some(EmbeddingTargetIdentity::StableModel {
        namespace: "digest",
        model_identity: digest.to_ascii_lowercase(),
    })
}

pub(crate) fn extract_stable_model_digest(model_name: &str) -> Option<String> {
    let normalized_model = model_name.trim().to_ascii_lowercase();
    let marker = "sha256:";
    let start = normalized_model.find(marker)? + marker.len();
    let digest = normalized_model[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_hexdigit())
        .collect::<String>();
    (digest.len() == 64).then_some(format!("{marker}{digest}"))
}

pub(crate) fn managed_embedding_model_namespace(normalized_base_url: &str) -> Option<&'static str> {
    let parsed = reqwest::Url::parse(normalized_base_url).ok()?;
    if parsed.query().is_some() {
        return None;
    }

    let host = parsed.host_str()?.to_ascii_lowercase();
    let path = parsed.path().trim_end_matches('/');
    if host == "api.openai.com" && path == "/v1" {
        Some("openai")
    } else {
        None
    }
}

pub(crate) fn normalize_runtime_source_directory(directory: &str) -> String {
    let trimmed = directory.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Ok(canonical_path) = std::fs::canonicalize(trimmed) {
        return normalize_path_string(&canonical_path);
    }

    trim_trailing_path_separators(trimmed)
}

pub(crate) fn trim_trailing_path_separators(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let normalized = trimmed.replace('\\', "/");
    let without_trailing = normalized.trim_end_matches('/');
    if without_trailing.is_empty() {
        normalized
    } else {
        without_trailing.to_string()
    }
}

pub(crate) fn resolve_rag_config(
    settings: &RagSettings,
    llm_settings: &LlmSettings,
) -> Result<ResolvedRagConfig> {
    let provider = resolve_embedding_provider(settings, llm_settings)?;
    let embedding_fingerprint = embedding_fingerprint(provider)?;
    if settings.source_directories.is_empty() {
        bail!("RAG 至少需要一个扫描目录");
    }

    let mut source_roots = Vec::new();
    let mut seen_roots = HashSet::new();
    for source_directory in &settings.source_directories {
        let root = std::fs::canonicalize(source_directory).with_context(|| {
            format!("failed to resolve RAG source directory: {source_directory}")
        })?;
        if !root.is_dir() {
            bail!("RAG source path is not a directory: {}", root.display());
        }
        if seen_roots.insert(root.clone()) {
            source_roots.push(root);
        }
    }

    Ok(ResolvedRagConfig {
        source_roots,
        ignore_globs: std::sync::Arc::new(build_ignore_glob_set(&settings.ignore_globs)?),
        embedding_fingerprint,
        provider: provider.clone(),
    })
}

pub(crate) fn resolve_embedding_provider<'a>(
    settings: &RagSettings,
    llm_settings: &'a LlmSettings,
) -> Result<&'a LlmProviderConfig> {
    let provider_id = settings
        .embedding_provider_id
        .as_deref()
        .context("RAG 扫描前必须选择一个 embedding provider")?;
    let provider = llm_settings
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .with_context(|| format!("RAG 选择的 embedding provider 不存在: {provider_id}"))?;
    if !provider.resolved_profile().can_handle_embedding() {
        bail!("RAG 只接受启用了 embedding 能力的 provider");
    }
    if provider.base_url.trim().is_empty() {
        bail!("RAG embedding provider base URL 不能为空");
    }
    if provider.resolved_profile().model_name().is_none() {
        bail!("RAG embedding provider model 不能为空");
    }

    Ok(provider)
}

pub(crate) fn resolve_source_root_for_path<'a>(
    roots: &'a [PathBuf],
    path: &Path,
) -> Option<&'a PathBuf> {
    roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
}

pub(crate) fn build_ignore_glob_set(patterns: &[String]) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            continue;
        }
        builder.add(
            Glob::new(trimmed)
                .with_context(|| format!("invalid RAG ignore glob pattern: {trimmed}"))?,
        );
    }

    let matcher = builder
        .build()
        .context("failed to build RAG ignore glob set")?;
    if matcher.is_empty() {
        Ok(None)
    } else {
        Ok(Some(matcher))
    }
}

pub(crate) fn should_skip_path(root: &Path, path: &Path, ignore_matcher: Option<&GlobSet>) -> bool {
    let Some(ignore_matcher) = ignore_matcher else {
        return false;
    };
    let relative_path = path.strip_prefix(root).unwrap_or(path);
    ignore_matcher.is_match(relative_path)
}

pub(crate) fn normalize_path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn collect_document_access_roots(
    workspace_root: &Path,
    rag_settings: &RagSettings,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    push_document_access_root(&mut roots, &mut seen, workspace_root);
    for directory in &rag_settings.source_directories {
        let trimmed = directory.trim();
        if trimmed.is_empty() {
            continue;
        }
        push_document_access_root(&mut roots, &mut seen, Path::new(trimmed));
    }

    roots
}

fn push_document_access_root(roots: &mut Vec<PathBuf>, seen: &mut HashSet<String>, path: &Path) {
    let Ok(canonical_root) = path.canonicalize() else {
        return;
    };
    if !canonical_root.is_dir() {
        return;
    }

    let normalized_root = normalize_path_string(&canonical_root);
    if seen.insert(normalized_root) {
        roots.push(canonical_root);
    }
}

pub(crate) fn path_is_within_roots(path: &Path, roots: &[PathBuf]) -> bool {
    path.starts_with_any(roots)
}

pub(crate) fn display_path_for_prompt(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let Some(home_dir) = dirs::home_dir() else {
        return normalized;
    };
    let home = normalize_path_string(&home_dir);
    if normalized == home {
        return "~".to_string();
    }
    if let Some(stripped) = normalized.strip_prefix(&(home.clone() + "/")) {
        return format!("~/{stripped}");
    }
    normalized
}

pub(crate) fn parse_heading_path(raw: &str) -> Result<Vec<String>> {
    serde_json::from_str(raw).context("failed to parse heading path metadata")
}

pub(crate) fn parse_document_kind(raw: &str) -> Result<DocumentKind> {
    match raw {
        "plain_text" => Ok(DocumentKind::PlainText),
        "markdown" => Ok(DocumentKind::Markdown),
        "pdf" => Ok(DocumentKind::Pdf),
        "docx" => Ok(DocumentKind::Docx),
        _ => bail!("failed to parse document kind metadata: {raw}"),
    }
}

pub(crate) fn system_time_to_unix_ms(value: SystemTime) -> Option<i64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

pub(crate) fn now_unix_ms() -> i64 {
    system_time_to_unix_ms(SystemTime::now()).unwrap_or_default()
}

pub(crate) trait PathStartsWithAny {
    fn starts_with_any(&self, prefixes: &[PathBuf]) -> bool;
}

impl PathStartsWithAny for Path {
    fn starts_with_any(&self, prefixes: &[PathBuf]) -> bool {
        prefixes.iter().any(|prefix| self.starts_with(prefix))
    }
}
