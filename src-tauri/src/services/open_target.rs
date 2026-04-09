use std::{
    collections::HashSet,
    ffi::OsStr,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::{domain::execution::ExecutionResult, infrastructure::opener};

const OPEN_TARGET_COMMAND_ALIASES: [&str; 1] = ["/open"];

#[derive(Debug, Clone, Default)]
pub struct OpenTargetService;

#[derive(Debug, Clone, PartialEq, Eq)]
enum OpenTarget {
    Url(String),
    Path(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedTarget {
    pub kind: String,
    pub target: String,
}

impl OpenTargetService {
    pub fn new() -> Self {
        Self
    }

    pub fn open_action(
        &self,
        raw_text: &str,
        workspace_root: Option<&Path>,
    ) -> Result<ExecutionResult> {
        let opened =
            self.open_input_target(extract_open_target_payload(raw_text), workspace_root)?;
        Ok(success_result_from_opened_target(&opened))
    }

    pub fn open_path(&self, path: &Path) -> Result<()> {
        opener::open_path(path)
    }

    pub fn open_input_target(
        &self,
        raw_target: &str,
        workspace_root: Option<&Path>,
    ) -> Result<OpenedTarget> {
        open_resolved_target(resolve_target(raw_target, workspace_root, None)?)
    }

    pub fn open_input_target_with_allowed_roots(
        &self,
        raw_target: &str,
        allowed_roots: &[PathBuf],
    ) -> Result<OpenedTarget> {
        open_resolved_target(resolve_target(raw_target, None, Some(allowed_roots))?)
    }
}

impl OpenTarget {
    fn open(&self) -> Result<()> {
        match self {
            Self::Url(url) => opener::open_url(url),
            Self::Path(path) => opener::open_path(path),
        }
    }

    fn opened_target(&self) -> OpenedTarget {
        match self {
            Self::Url(url) => OpenedTarget {
                kind: "url".to_string(),
                target: url.clone(),
            },
            Self::Path(path) => OpenedTarget {
                kind: path_kind(path).to_string(),
                target: path.display().to_string(),
            },
        }
    }
}

fn resolve_target(
    raw_target: &str,
    workspace_root: Option<&Path>,
    allowed_roots: Option<&[PathBuf]>,
) -> Result<OpenTarget> {
    let payload = raw_target.trim();
    if payload.is_empty() {
        bail!("请输入要打开的链接、文件或目录");
    }

    if has_explicit_uri_scheme(payload) {
        return Ok(OpenTarget::Url(payload.to_string()));
    }

    if let Some(path) = resolve_existing_path(payload, workspace_root, allowed_roots)? {
        return Ok(OpenTarget::Path(path));
    }

    if input_looks_like_path(payload) {
        bail!("路径不存在或不可访问: {payload}");
    }

    if let Some(url) = normalize_http_url(payload) {
        return Ok(OpenTarget::Url(url));
    }

    bail!("无法识别要打开的目标，请输入链接、文件或目录路径")
}

fn extract_open_target_payload(raw_text: &str) -> &str {
    extract_prefixed_payload(raw_text, &OPEN_TARGET_COMMAND_ALIASES)
        .unwrap_or(raw_text)
        .trim()
}

fn extract_prefixed_payload<'a>(text: &'a str, aliases: &[&str]) -> Option<&'a str> {
    let trimmed = text.trim_start();

    for alias in aliases {
        let Some(remainder) = trimmed.strip_prefix(alias) else {
            continue;
        };

        if remainder.is_empty() {
            return None;
        }

        let first = remainder.chars().next()?;
        if !first.is_whitespace() && !matches!(first, '/' | '\\' | '.' | '~' | '#') {
            continue;
        }

        return Some(remainder.trim_start());
    }

    None
}

fn resolve_existing_path(
    input: &str,
    workspace_root: Option<&Path>,
    allowed_roots: Option<&[PathBuf]>,
) -> Result<Option<PathBuf>> {
    let candidates = build_candidate_paths(input, workspace_root, allowed_roots);
    if candidates.is_empty() {
        return Ok(None);
    }

    let enforce_unique_match = allowed_roots.is_some()
        && !Path::new(input).is_absolute()
        && !input.starts_with("~/")
        && !input.starts_with("~\\");
    let mut resolved_matches = Vec::new();

    for candidate in candidates {
        match candidate.canonicalize() {
            Ok(path) => {
                if !path.is_file() && !path.is_dir() {
                    bail!("不是可打开的文件或目录: {}", path.display());
                }
                ensure_path_is_allowed(&path, allowed_roots)?;
                if !resolved_matches.iter().any(|existing| existing == &path) {
                    resolved_matches.push(path);
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("无法解析要打开的路径: {}", candidate.display()));
            }
        }
    }

    if resolved_matches.len() == 1 {
        return Ok(Some(resolved_matches.remove(0)));
    }

    if enforce_unique_match && resolved_matches.len() > 1 {
        let matches = resolved_matches
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        bail!("相对路径命中多个允许目录中的目标，请改用绝对路径或更精确路径: {matches}");
    }

    Ok(None)
}

fn open_resolved_target(target: OpenTarget) -> Result<OpenedTarget> {
    target.open()?;
    Ok(target.opened_target())
}

fn ensure_path_is_allowed(path: &Path, allowed_roots: Option<&[PathBuf]>) -> Result<()> {
    let Some(allowed_roots) = allowed_roots else {
        return Ok(());
    };

    if crate::services::rag::path_is_within_roots(path, allowed_roots) {
        return Ok(());
    }

    bail!(
        "路径超出允许范围，只能打开当前 workspace 或显式配置的 RAG 目录: {}",
        path.display()
    )
}

fn build_candidate_paths(
    input: &str,
    workspace_root: Option<&Path>,
    allowed_roots: Option<&[PathBuf]>,
) -> Vec<PathBuf> {
    let expanded = expand_home(input).unwrap_or_else(|| PathBuf::from(input));
    if expanded.is_absolute() || is_windows_drive_path(input) {
        return vec![expanded];
    }

    if let Some(allowed_roots) = allowed_roots {
        let mut seen = HashSet::new();
        return allowed_roots
            .iter()
            .filter(|root| seen.insert(normalize_allowed_root_key(root)))
            .map(|root| root.join(&expanded))
            .collect();
    }

    workspace_root
        .map(|root| vec![root.join(expanded)])
        .unwrap_or_default()
}

fn normalize_allowed_root_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn expand_home(input: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;

    if input == "~" {
        return Some(home);
    }

    input
        .strip_prefix("~/")
        .or_else(|| input.strip_prefix("~\\"))
        .map(|suffix| home.join(suffix))
}

fn has_explicit_uri_scheme(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };

    if scheme.len() == 1 && is_windows_drive_path(value) {
        return false;
    }

    !scheme.is_empty()
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn normalize_http_url(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }

    if text.starts_with("http://") || text.starts_with("https://") {
        return Some(text.to_string());
    }

    if text.contains('.') && !text.contains(' ') {
        return Some(format!("https://{text}"));
    }

    None
}

fn input_looks_like_path(value: &str) -> bool {
    value == "~"
        || value.starts_with("~/")
        || value.starts_with("~\\")
        || value.starts_with("./")
        || value.starts_with(".\\")
        || value.starts_with("../")
        || value.starts_with("..\\")
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('/')
        || value.contains('\\')
        || is_windows_drive_path(value)
}

fn is_windows_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn path_kind(path: &Path) -> &'static str {
    if path.extension() == Some(OsStr::new("app")) {
        return "application";
    }

    if path.is_dir() {
        "directory"
    } else {
        "file"
    }
}

fn success_result_from_opened_target(opened: &OpenedTarget) -> ExecutionResult {
    let secondary_text = match opened.kind.as_str() {
        "url" => "已交给系统默认程序打开链接".to_string(),
        "directory" => "已交给系统默认程序打开目录".to_string(),
        "application" => "已交给系统默认程序启动应用".to_string(),
        _ => "已交给系统默认程序打开路径".to_string(),
    };

    ExecutionResult::success(
        Some(opened.target.clone()),
        Some(secondary_text),
        None,
        vec![],
        true,
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        extract_open_target_payload, has_explicit_uri_scheme, normalize_http_url, resolve_target,
        OpenTarget,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("wabity-open-target-{name}-{suffix}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn open_command_strips_prefix_before_resolution() {
        assert_eq!(extract_open_target_payload("/open README.md"), "README.md");
    }

    #[test]
    fn attached_open_command_prefix_is_not_treated_as_payload() {
        assert_eq!(
            extract_open_target_payload("/opentauri.app"),
            "/opentauri.app"
        );
    }

    #[test]
    fn explicit_uri_scheme_is_detected() {
        assert!(has_explicit_uri_scheme("mailto:test@example.com"));
        assert!(has_explicit_uri_scheme("https://tauri.app"));
        assert!(!has_explicit_uri_scheme("C:\\Windows"));
    }

    #[test]
    fn bare_domain_is_normalized_to_https() {
        assert_eq!(
            normalize_http_url("tauri.app").as_deref(),
            Some("https://tauri.app")
        );
    }

    #[test]
    fn existing_workspace_relative_path_is_opened_as_path() {
        let workspace = temp_dir("workspace");
        let file_path = workspace.join("README.md");
        fs::write(&file_path, "hello").unwrap();

        let resolved = resolve_target("README.md", Some(&workspace), None).unwrap();

        assert_eq!(
            resolved,
            OpenTarget::Path(file_path.canonicalize().unwrap())
        );
    }

    #[test]
    fn non_existing_domain_like_input_falls_back_to_url() {
        let workspace = temp_dir("url");
        let resolved = resolve_target("tauri.app", Some(&workspace), None).unwrap();

        assert_eq!(resolved, OpenTarget::Url("https://tauri.app".to_string()));
    }

    #[test]
    fn missing_path_like_input_returns_path_error() {
        let workspace = temp_dir("missing");
        let error = resolve_target("./missing.txt", Some(&workspace), None).unwrap_err();

        assert_eq!(error.to_string(), "路径不存在或不可访问: ./missing.txt");
    }

    #[test]
    fn relative_allowed_root_path_rejects_ambiguous_matches() {
        let root = temp_dir("ambiguous");
        let workspace = root.join("workspace");
        let docs = root.join("docs");
        fs::create_dir_all(workspace.join("guides")).unwrap();
        fs::create_dir_all(docs.join("guides")).unwrap();
        fs::write(workspace.join("guides/intro.md"), "workspace").unwrap();
        fs::write(docs.join("guides/intro.md"), "docs").unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let docs = docs.canonicalize().unwrap();

        let error = resolve_target("guides/intro.md", None, Some(&[workspace, docs])).unwrap_err();

        assert!(error
            .to_string()
            .contains("相对路径命中多个允许目录中的目标"));
    }

    #[test]
    fn equivalent_allowed_roots_do_not_create_false_ambiguity() {
        let workspace = temp_dir("dedup-roots");
        let file_path = workspace.join("guides/intro.md");
        fs::create_dir_all(file_path.parent().expect("guides parent")).unwrap();
        fs::write(&file_path, "workspace").unwrap();
        let canonical_workspace = workspace.canonicalize().unwrap();
        let dotted_workspace = canonical_workspace.join(".");

        let resolved = resolve_target(
            "guides/intro.md",
            None,
            Some(&[canonical_workspace.clone(), dotted_workspace]),
        )
        .expect("equivalent allowed roots should resolve uniquely");

        assert_eq!(
            resolved,
            OpenTarget::Path(file_path.canonicalize().unwrap())
        );
    }
}
