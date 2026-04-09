use std::{
    collections::HashSet,
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use ignore::{WalkBuilder, WalkState};
use tokio::task;

use crate::{
    domain::{application::InstalledAppMatch, execution::ExecutionResult},
    infrastructure::opener,
};

pub const APPLICATION_CACHE_STALE_AFTER: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone)]
pub struct ApplicationService {
    snapshot: Arc<RwLock<ApplicationCatalogSnapshot>>,
}

#[derive(Debug, Clone)]
struct ApplicationCatalogSnapshot {
    entries: Arc<Vec<ApplicationRecord>>,
    built_at_ms: Option<u64>,
    refreshing: bool,
    version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApplicationRecord {
    name: String,
    aliases: Vec<String>,
    path: String,
    normalized_name: String,
    normalized_aliases: Vec<String>,
}

impl ApplicationService {
    pub fn new() -> Result<Self> {
        Ok(Self {
            snapshot: Arc::new(RwLock::new(ApplicationCatalogSnapshot {
                entries: Arc::new(Vec::new()),
                built_at_ms: None,
                refreshing: false,
                version: 0,
            })),
        })
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<InstalledAppMatch>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }

        let entries =
            self.search_entries(APPLICATION_CACHE_STALE_AFTER, build_application_index)?;
        Ok(rank_matches(entries.as_ref(), needle, limit))
    }

    pub fn launch(&self, path: &str) -> Result<ExecutionResult> {
        let target = validate_app_bundle_path(path)?;
        let app_name = localized_app_name(&target).unwrap_or_else(|| normalize_app_name(&target));
        opener::open_path(&target)?;

        Ok(ExecutionResult::success(
            Some(app_name),
            Some("已启动应用".to_string()),
            None,
            vec![],
            true,
        ))
    }

    pub async fn refresh_now(&self) -> Result<()> {
        self.refresh_now_with(build_application_index).await
    }

    async fn refresh_now_with<F>(&self, build: F) -> Result<()>
    where
        F: FnOnce() -> Result<Vec<ApplicationRecord>> + Send + 'static,
    {
        if !self.begin_refresh()? {
            return Ok(());
        }

        let build_result = match task::spawn_blocking(build).await {
            Ok(result) => result,
            Err(error) => {
                self.finish_failed_refresh()?;
                return Err(error).context("failed to join application index refresh task");
            }
        };
        match build_result {
            Ok(entries) => {
                self.replace_snapshot(entries)?;
                Ok(())
            }
            Err(error) => {
                self.finish_failed_refresh()?;
                Err(error)
            }
        }
    }

    fn entries_snapshot(&self) -> Result<Arc<Vec<ApplicationRecord>>> {
        Ok(self.read_snapshot_state()?.entries)
    }

    fn entries_for_search<F>(&self, build: F) -> Result<Arc<Vec<ApplicationRecord>>>
    where
        F: FnOnce() -> Result<Vec<ApplicationRecord>>,
    {
        let started_refresh = self.begin_refresh()?;
        match build() {
            Ok(entries) => {
                self.replace_snapshot(entries)?;
                self.entries_snapshot()
            }
            Err(error) => {
                if started_refresh {
                    self.finish_failed_refresh()?;
                }
                Err(error)
            }
        }
    }

    fn read_snapshot_state(&self) -> Result<ApplicationCatalogSnapshot> {
        let guard = read_snapshot_lock(&self.snapshot, "read");
        Ok(guard.clone())
    }

    fn begin_refresh(&self) -> Result<bool> {
        let mut guard = write_snapshot_lock(&self.snapshot, "write");
        if guard.refreshing {
            return Ok(false);
        }
        guard.refreshing = true;
        Ok(true)
    }

    fn replace_snapshot(&self, entries: Vec<ApplicationRecord>) -> Result<()> {
        self.write_snapshot(entries, false)
    }
    fn write_snapshot(&self, entries: Vec<ApplicationRecord>, refreshing: bool) -> Result<()> {
        let mut guard = write_snapshot_lock(&self.snapshot, "update");
        let version = guard.version.saturating_add(1);
        guard.entries = Arc::new(entries);
        guard.built_at_ms = Some(current_time_ms());
        guard.refreshing = refreshing;
        guard.version = version;
        tracing::info!(
            count = guard.entries.len(),
            refreshing = guard.refreshing,
            version = guard.version,
            "application cache refreshed"
        );
        Ok(())
    }

    fn finish_failed_refresh(&self) -> Result<()> {
        let mut guard = write_snapshot_lock(&self.snapshot, "update");
        guard.refreshing = false;
        Ok(())
    }

    fn should_refresh(&self, max_age: Duration) -> Result<bool> {
        let guard = read_snapshot_lock(&self.snapshot, "read");
        if guard.refreshing {
            return Ok(false);
        }

        if guard.entries.is_empty() || guard.built_at_ms.is_none() {
            return Ok(true);
        }

        Ok(cache_age_exceeded(guard.built_at_ms, max_age))
    }

    pub fn schedule_refresh_if_stale(&self, max_age: Duration) {
        let should_refresh = match self.should_refresh(max_age) {
            Ok(should_refresh) => should_refresh,
            Err(error) => {
                tracing::warn!(?error, "failed to inspect application cache state");
                return;
            }
        };
        if !should_refresh {
            return;
        }

        self.spawn_refresh_task();
    }

    fn spawn_refresh_task(&self) {
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = service.refresh_now().await {
                tracing::warn!(?error, "failed to refresh application cache");
            }
        });
    }

    fn search_entries<F>(&self, max_age: Duration, build: F) -> Result<Arc<Vec<ApplicationRecord>>>
    where
        F: FnOnce() -> Result<Vec<ApplicationRecord>>,
    {
        if self.snapshot_needs_refresh(max_age)? {
            return self.entries_for_search(build);
        }

        self.entries_snapshot()
    }

    fn snapshot_needs_refresh(&self, max_age: Duration) -> Result<bool> {
        let snapshot = self.read_snapshot_state()?;
        Ok(snapshot.entries.is_empty() || cache_age_exceeded(snapshot.built_at_ms, max_age))
    }
}

fn read_snapshot_lock<'a>(
    snapshot: &'a Arc<RwLock<ApplicationCatalogSnapshot>>,
    action: &str,
) -> std::sync::RwLockReadGuard<'a, ApplicationCatalogSnapshot> {
    match snapshot.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                action,
                "application cache lock poisoned; recovering cached state"
            );
            poisoned.into_inner()
        }
    }
}

fn write_snapshot_lock<'a>(
    snapshot: &'a Arc<RwLock<ApplicationCatalogSnapshot>>,
    action: &str,
) -> std::sync::RwLockWriteGuard<'a, ApplicationCatalogSnapshot> {
    match snapshot.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                action,
                "application cache lock poisoned; recovering cached state"
            );
            poisoned.into_inner()
        }
    }
}

fn build_application_index() -> Result<Vec<ApplicationRecord>> {
    let started_at = std::time::Instant::now();
    #[cfg(target_os = "macos")]
    {
        let roots = application_roots();
        let entries = Mutex::new(Vec::new());
        let seen_paths = Mutex::new(HashSet::new());

        for root in roots {
            let walker = WalkBuilder::new(&root)
                .hidden(false)
                .ignore(false)
                .git_ignore(false)
                .git_global(false)
                .git_exclude(false)
                .parents(false)
                .threads(
                    std::thread::available_parallelism()
                        .map(|parallelism| parallelism.get())
                        .unwrap_or(4),
                )
                .build_parallel();

            walker.run(|| {
                let entries = &entries;
                let seen_paths = &seen_paths;
                Box::new(move |entry| {
                    let dir_entry = match entry {
                        Ok(dir_entry) => dir_entry,
                        Err(_) => return WalkState::Continue,
                    };

                    let path = dir_entry.path();
                    let Some(file_type) = dir_entry.file_type() else {
                        return WalkState::Continue;
                    };

                    if !file_type.is_dir() {
                        return WalkState::Continue;
                    }

                    if path.extension() != Some(OsStr::new("app")) {
                        return WalkState::Continue;
                    }

                    let canonical = match path.canonicalize() {
                        Ok(canonical) => canonical,
                        Err(_) => path.to_path_buf(),
                    };

                    if let Ok(mut guard) = seen_paths.lock() {
                        if !guard.insert(canonical.clone()) {
                            return WalkState::Skip;
                        }
                    }

                    if let Ok(mut guard) = entries.lock() {
                        let bundle_name = normalize_app_name(&canonical);
                        let display_name =
                            localized_app_name(&canonical).unwrap_or_else(|| bundle_name.clone());
                        let aliases = build_aliases(&display_name, &bundle_name);
                        guard.push(ApplicationRecord {
                            name: display_name.clone(),
                            normalized_name: normalize_search_text(&display_name),
                            normalized_aliases: aliases
                                .iter()
                                .map(|alias| normalize_search_text(alias))
                                .collect(),
                            aliases,
                            path: canonical.to_string_lossy().into_owned(),
                        });
                    }

                    WalkState::Skip
                })
            });
        }

        let mut entries = entries
            .into_inner()
            .map_err(|_| anyhow::anyhow!("application index mutex poisoned"))?;
        entries.sort_by(|left, right| {
            right
                .name
                .cmp(&left.name)
                .reverse()
                .then_with(|| {
                    app_directory_priority(Path::new(&right.path))
                        .cmp(&app_directory_priority(Path::new(&left.path)))
                        .reverse()
                })
                .then_with(|| left.path.len().cmp(&right.path.len()))
                .then_with(|| left.path.cmp(&right.path))
        });

        tracing::info!(
            count = entries.len(),
            total_name_bytes = entries.iter().map(|entry| entry.name.len()).sum::<usize>(),
            total_alias_bytes = entries
                .iter()
                .map(|entry| entry.aliases.iter().map(String::len).sum::<usize>())
                .sum::<usize>(),
            total_path_bytes = entries.iter().map(|entry| entry.path.len()).sum::<usize>(),
            elapsed_ms = started_at.elapsed().as_millis(),
            "application index ready"
        );
        Ok(entries)
    }

    #[cfg(not(target_os = "macos"))]
    {
        tracing::info!(
            elapsed_ms = started_at.elapsed().as_millis(),
            "application index build completed on unsupported platform"
        );
        Ok(Vec::new())
    }
}

#[cfg(target_os = "macos")]
fn application_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];

    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }

    roots.into_iter().filter(|root| root.is_dir()).collect()
}

fn rank_matches(
    entries: &[ApplicationRecord],
    needle: &str,
    limit: usize,
) -> Vec<InstalledAppMatch> {
    let normalized_limit = limit.clamp(1, 20);
    let matcher = SkimMatcherV2::default()
        .smart_case()
        .element_limit(1024 * 1024 * 1024);
    let normalized_needle = normalize_search_text(needle);
    let mut ranked = Vec::with_capacity(normalized_limit);

    for entry in entries {
        if let Some(score) = score_entry(&matcher, entry, &normalized_needle) {
            ranked.push(InstalledAppMatch {
                name: entry.name.clone(),
                path: entry.path.clone(),
                score: saturating_score(score),
            });
        }
    }

    ranked.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| {
                app_directory_priority(Path::new(&right.path))
                    .cmp(&app_directory_priority(Path::new(&left.path)))
            })
            .then_with(|| left.path.len().cmp(&right.path.len()))
            .then_with(|| left.path.cmp(&right.path))
    });
    ranked.truncate(normalized_limit);
    ranked
}

fn score_entry(
    matcher: &SkimMatcherV2,
    entry: &ApplicationRecord,
    normalized_needle: &str,
) -> Option<i64> {
    let lowercase_path = normalize_search_text(&entry.path);
    let alias_score = entry
        .normalized_aliases
        .iter()
        .filter_map(|alias| matcher.fuzzy_match(alias, normalized_needle))
        .max();
    let mut score = match (
        matcher.fuzzy_match(&entry.normalized_name, normalized_needle),
        alias_score,
    ) {
        (Some(name_score), Some(alias_score)) => name_score.max(alias_score.saturating_add(80)),
        (Some(name_score), None) => name_score,
        (None, Some(alias_score)) => alias_score.saturating_add(80),
        (None, None) => return None,
    };

    if entry.normalized_name.starts_with(normalized_needle) {
        score = score.saturating_add(220);
    } else if entry.normalized_name.contains(normalized_needle) {
        score = score.saturating_add(140);
    }

    if lowercase_path.contains(normalized_needle) {
        score = score.saturating_add(36);
    }

    score = score.saturating_add(i64::from(
        app_directory_priority(Path::new(&entry.path)) * 12,
    ));
    Some(score)
}

fn app_directory_priority(path: &Path) -> u8 {
    let path_string = path.to_string_lossy();

    if path_string.starts_with("/System/Applications/") {
        return 0;
    }

    if path_string.starts_with("/Applications/") {
        return 1;
    }

    if path_string.contains("/Applications/") {
        return 2;
    }

    1
}

fn normalize_app_name(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_else(|| OsStr::new(""))
        .to_string_lossy()
        .into_owned()
}

#[cfg(target_os = "macos")]
fn localized_app_name(path: &Path) -> Option<String> {
    let output = Command::new("mdls")
        .arg("-raw")
        .arg("-name")
        .arg("kMDItemDisplayName")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let display_name = String::from_utf8(output.stdout).ok()?;
    normalize_display_name(&display_name)
}

#[cfg(not(target_os = "macos"))]
fn localized_app_name(_path: &Path) -> Option<String> {
    None
}

fn normalize_display_name(raw_name: &str) -> Option<String> {
    let trimmed = raw_name.trim();
    if trimmed.is_empty() || trimmed == "(null)" {
        return None;
    }

    Some(
        trimmed
            .strip_suffix(".app")
            .unwrap_or(trimmed)
            .trim()
            .to_string(),
    )
}

fn build_aliases(primary_name: &str, bundle_name: &str) -> Vec<String> {
    let mut aliases = Vec::new();

    for candidate in [primary_name, bundle_name] {
        let normalized = candidate.trim();
        if normalized.is_empty() || aliases.iter().any(|existing| existing == normalized) {
            continue;
        }
        aliases.push(normalized.to_string());
    }

    aliases
}

fn normalize_search_text(value: &str) -> String {
    value.trim().to_lowercase()
}

fn cache_age_exceeded(built_at_ms: Option<u64>, max_age: Duration) -> bool {
    let Some(built_at_ms) = built_at_ms else {
        return true;
    };
    let age_ms = current_time_ms().saturating_sub(built_at_ms);
    age_ms > max_age.as_millis() as u64
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn validate_app_bundle_path(path: &str) -> Result<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        anyhow::bail!("application path is empty");
    }

    let target = PathBuf::from(trimmed);
    if target.extension() != Some(OsStr::new("app")) {
        anyhow::bail!("application path must point to a .app bundle");
    }
    if !target.is_dir() {
        anyhow::bail!("application bundle does not exist: {}", target.display());
    }

    Ok(target)
}

fn saturating_score(score: i64) -> u16 {
    score.clamp(0, i64::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        sync::Arc,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::{
        app_directory_priority, build_aliases, cache_age_exceeded, current_time_ms,
        normalize_app_name, normalize_display_name, rank_matches, validate_app_bundle_path,
        ApplicationCatalogSnapshot, ApplicationRecord, ApplicationService,
        APPLICATION_CACHE_STALE_AFTER,
    };

    #[test]
    fn rank_prefers_prefix_match() {
        let entries = vec![
            ApplicationRecord {
                name: "Safari".to_string(),
                aliases: vec!["Safari".to_string()],
                path: "/Applications/Safari.app".to_string(),
                normalized_name: "safari".to_string(),
                normalized_aliases: vec!["safari".to_string()],
            },
            ApplicationRecord {
                name: "Arc Browser".to_string(),
                aliases: vec!["Arc Browser".to_string()],
                path: "/Applications/Arc Browser.app".to_string(),
                normalized_name: "arc browser".to_string(),
                normalized_aliases: vec!["arc browser".to_string()],
            },
        ];

        let matches = rank_matches(&entries, "sa", 5);
        assert_eq!(
            matches.first().map(|item| item.name.as_str()),
            Some("Safari")
        );
    }

    #[test]
    fn rank_prefers_non_system_path_when_scores_tie() {
        let entries = vec![
            ApplicationRecord {
                name: "Calendar".to_string(),
                aliases: vec!["Calendar".to_string()],
                path: "/System/Applications/Calendar.app".to_string(),
                normalized_name: "calendar".to_string(),
                normalized_aliases: vec!["calendar".to_string()],
            },
            ApplicationRecord {
                name: "Calendar".to_string(),
                aliases: vec!["Calendar".to_string()],
                path: "/Applications/Calendar.app".to_string(),
                normalized_name: "calendar".to_string(),
                normalized_aliases: vec!["calendar".to_string()],
            },
        ];

        let matches = rank_matches(&entries, "calendar", 5);
        assert_eq!(
            matches.first().map(|item| item.path.as_str()),
            Some("/Applications/Calendar.app")
        );
    }

    #[test]
    fn normalize_name_strips_app_suffix() {
        assert_eq!(
            normalize_app_name(Path::new("/Applications/Visual Studio Code.app")),
            "Visual Studio Code"
        );
    }

    #[test]
    fn normalize_display_name_strips_app_suffix() {
        assert_eq!(normalize_display_name("微信.app"), Some("微信".to_string()));
        assert_eq!(normalize_display_name("(null)"), None);
    }

    #[test]
    fn aliases_keep_primary_and_bundle_names() {
        assert_eq!(
            build_aliases("微信", "WeChat"),
            vec!["微信".to_string(), "WeChat".to_string()]
        );
    }

    #[test]
    fn rank_matches_bundle_alias_for_localized_app() {
        let entries = vec![ApplicationRecord {
            name: "微信".to_string(),
            aliases: vec!["微信".to_string(), "WeChat".to_string()],
            path: "/Applications/WeChat.app".to_string(),
            normalized_name: "微信".to_string(),
            normalized_aliases: vec!["微信".to_string(), "wechat".to_string()],
        }];

        let matches = rank_matches(&entries, "wechat", 5);
        assert_eq!(matches.first().map(|item| item.name.as_str()), Some("微信"));
    }

    #[test]
    fn cache_age_reports_stale_entries() {
        assert!(cache_age_exceeded(Some(0), Duration::from_secs(1)));
        assert!(!cache_age_exceeded(
            Some(current_time_ms()),
            Duration::from_secs(60)
        ));
    }

    #[test]
    fn validate_bundle_rejects_empty_path() {
        let error = validate_app_bundle_path("").expect_err("empty path should fail");
        assert!(error.to_string().contains("empty"));
    }

    #[test]
    fn validate_bundle_accepts_existing_app_directory() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let app_dir = std::env::temp_dir().join(format!("wabity-test-{unique}.app"));

        fs::create_dir_all(&app_dir).expect("failed to create app dir");
        let target = validate_app_bundle_path(app_dir.to_string_lossy().as_ref())
            .expect("app dir should be valid");
        assert_eq!(target, app_dir);
        let _ = fs::remove_dir_all(app_dir);
    }

    #[test]
    fn applications_directory_priority_is_stable() {
        assert!(
            app_directory_priority(Path::new("/Applications/Foo.app"))
                > app_directory_priority(Path::new("/System/Applications/Foo.app"))
        );
    }

    #[test]
    fn search_entries_clears_refreshing_after_direct_build() {
        let service = ApplicationService::new().expect("service should initialize");
        *service.snapshot.write().expect("snapshot lock should work") =
            ApplicationCatalogSnapshot {
                entries: Arc::new(Vec::new()),
                built_at_ms: None,
                refreshing: true,
                version: 7,
            };

        let entries = service
            .entries_for_search(|| {
                Ok(vec![ApplicationRecord {
                    name: "Arc".to_string(),
                    aliases: vec!["Arc".to_string()],
                    path: "/Applications/Arc.app".to_string(),
                    normalized_name: "arc".to_string(),
                    normalized_aliases: vec!["arc".to_string()],
                }])
            })
            .expect("fallback build should succeed");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Arc");
        let snapshot = service
            .snapshot
            .read()
            .expect("snapshot lock should work after fallback");
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].name, "Arc");
        assert!(!snapshot.refreshing);
        assert_eq!(snapshot.version, 8);
    }

    #[test]
    fn search_entries_rebuilds_stale_snapshot_before_returning_results() {
        let service = ApplicationService::new().expect("service should initialize");
        *service.snapshot.write().expect("snapshot lock should work") =
            ApplicationCatalogSnapshot {
                entries: Arc::new(vec![ApplicationRecord {
                    name: "Old".to_string(),
                    aliases: vec!["Old".to_string()],
                    path: "/Applications/Old.app".to_string(),
                    normalized_name: "old".to_string(),
                    normalized_aliases: vec!["old".to_string()],
                }]),
                built_at_ms: Some(current_time_ms().saturating_sub(20 * 60 * 1000)),
                refreshing: false,
                version: 3,
            };

        let entries = service
            .search_entries(APPLICATION_CACHE_STALE_AFTER, || {
                Ok(vec![ApplicationRecord {
                    name: "Fresh".to_string(),
                    aliases: vec!["Fresh".to_string()],
                    path: "/Applications/Fresh.app".to_string(),
                    normalized_name: "fresh".to_string(),
                    normalized_aliases: vec!["fresh".to_string()],
                }])
            })
            .expect("stale application snapshot should rebuild before returning");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Fresh");

        let snapshot = service
            .snapshot
            .read()
            .expect("snapshot lock should work after rebuild");
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].name, "Fresh");
        assert!(!snapshot.refreshing);
        assert_eq!(snapshot.version, 4);
    }

    #[test]
    fn search_entries_rebuilds_stale_snapshot_even_while_refreshing() {
        let service = ApplicationService::new().expect("service should initialize");
        *service.snapshot.write().expect("snapshot lock should work") =
            ApplicationCatalogSnapshot {
                entries: Arc::new(vec![ApplicationRecord {
                    name: "Old".to_string(),
                    aliases: vec!["Old".to_string()],
                    path: "/Applications/Old.app".to_string(),
                    normalized_name: "old".to_string(),
                    normalized_aliases: vec!["old".to_string()],
                }]),
                built_at_ms: Some(current_time_ms().saturating_sub(20 * 60 * 1000)),
                refreshing: true,
                version: 9,
            };

        let entries = service
            .search_entries(APPLICATION_CACHE_STALE_AFTER, || {
                Ok(vec![ApplicationRecord {
                    name: "Fresh".to_string(),
                    aliases: vec!["Fresh".to_string()],
                    path: "/Applications/Fresh.app".to_string(),
                    normalized_name: "fresh".to_string(),
                    normalized_aliases: vec!["fresh".to_string()],
                }])
            })
            .expect("stale refreshing snapshot should still rebuild before returning");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Fresh");

        let snapshot = service
            .snapshot
            .read()
            .expect("snapshot lock should work after refreshing rebuild");
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].name, "Fresh");
        assert!(!snapshot.refreshing);
        assert_eq!(snapshot.version, 10);
    }

    #[tokio::test]
    async fn refresh_now_clears_refreshing_when_background_build_panics() {
        let service = ApplicationService::new().expect("service should initialize");

        let error = service
            .refresh_now_with(|| -> anyhow::Result<Vec<ApplicationRecord>> {
                panic!("panic during application refresh");
            })
            .await
            .expect_err("panic in refresh task should surface as an error");

        assert!(error
            .to_string()
            .contains("failed to join application index refresh task"));
        assert!(
            !service
                .snapshot
                .read()
                .expect("snapshot lock should work after panic")
                .refreshing
        );
    }

    #[test]
    fn entries_snapshot_recovers_from_poisoned_snapshot_lock() {
        let service = ApplicationService::new().expect("service should initialize");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = service
                .snapshot
                .write()
                .expect("snapshot lock should succeed");
            panic!("poison application snapshot lock");
        }));

        let entries = service
            .entries_snapshot()
            .expect("poisoned application snapshot lock should recover");
        assert!(entries.is_empty());
    }
}
