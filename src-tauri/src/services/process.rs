use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "macos")]
use std::process::Command;

use anyhow::{bail, Context, Result};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System};
use tokio::task;

use crate::domain::{
    execution::{ExecutionResult, ExecutionStatus},
    process::{RunningProcessKind, RunningProcessMatch},
};

const KILL_COMMAND_ALIASES: [&str; 1] = ["/kill"];
pub const PROCESS_CACHE_STALE_AFTER: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct ProcessService {
    snapshot: Arc<RwLock<ProcessCatalogSnapshot>>,
    #[cfg(target_os = "macos")]
    localized_name_cache: Arc<RwLock<HashMap<String, Option<String>>>>,
}

#[derive(Debug, Clone)]
struct ProcessCatalogSnapshot {
    entries: Arc<Vec<ProcessRecord>>,
    built_at_ms: Option<u64>,
    refreshing: bool,
    version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessRecord {
    pid: u32,
    display_name: String,
    process_name: String,
    executable_path: Option<String>,
    app_bundle_path: Option<String>,
    kind: RunningProcessKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KillTargetResolution {
    Unique(ProcessRecord),
    NotFound,
    Ambiguous(Vec<ProcessRecord>),
}

struct ProcessSearchCandidate {
    value: String,
    base_score: i32,
}

impl ProcessService {
    pub fn new() -> Self {
        Self {
            snapshot: Arc::new(RwLock::new(ProcessCatalogSnapshot {
                entries: Arc::new(Vec::new()),
                built_at_ms: None,
                refreshing: false,
                version: 0,
            })),
            #[cfg(target_os = "macos")]
            localized_name_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn search_running(&self, query: &str, limit: usize) -> Result<Vec<RunningProcessMatch>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }

        let entries = self.entries_for_search(PROCESS_CACHE_STALE_AFTER)?;
        Ok(rank_processes(entries.as_ref(), needle, limit))
    }

    pub async fn refresh_now(&self) -> Result<()> {
        self.refresh_now_with({
            let service = self.clone();
            move || service.collect_process_records_fresh()
        })
        .await
    }

    async fn refresh_now_with<F>(&self, build: F) -> Result<()>
    where
        F: FnOnce() -> Result<Vec<ProcessRecord>> + Send + 'static,
    {
        if !self.begin_refresh()? {
            return Ok(());
        }

        let build_result = match task::spawn_blocking(build).await {
            Ok(result) => result,
            Err(error) => {
                self.finish_failed_refresh()?;
                return Err(error).context("failed to join process cache refresh task");
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

    pub fn kill_action(&self, raw_text: &str) -> Result<ExecutionResult> {
        let target_text = extract_kill_payload(raw_text).unwrap_or(raw_text).trim();
        if target_text.is_empty() {
            return Ok(warning_result(
                None,
                "请输入要终止的应用名称、进程名称或 pid:<id>",
            ));
        }

        let processes = self.collect_process_records_fresh()?;
        match resolve_kill_target(target_text, &processes) {
            KillTargetResolution::NotFound => Ok(warning_result(
                Some(target_text.to_string()),
                "未找到运行中的目标",
            )),
            KillTargetResolution::Ambiguous(matches) => Ok(warning_result(
                Some(target_text.to_string()),
                &format!(
                    "命中多个运行中的目标，请改用补全后的 pid 再执行：{}",
                    format_ambiguous_targets(&matches)
                ),
            )),
            KillTargetResolution::Unique(target) => terminate_process(&target),
        }
    }

    fn collect_process_records_fresh(&self) -> Result<Vec<ProcessRecord>> {
        let started_at = std::time::Instant::now();
        let current_pid = current_pid_u32()?;
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::everything(),
        );

        let mut processes = system
            .processes()
            .values()
            .filter_map(|process| self.process_record_from_sysinfo(process, current_pid))
            .collect::<Vec<_>>();
        processes.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.pid.cmp(&right.pid))
        });
        tracing::info!(
            count = processes.len(),
            total_display_name_bytes = processes
                .iter()
                .map(|process| process.display_name.len())
                .sum::<usize>(),
            total_process_name_bytes = processes
                .iter()
                .map(|process| process.process_name.len())
                .sum::<usize>(),
            total_path_bytes = processes
                .iter()
                .map(|process| {
                    process
                        .executable_path
                        .as_ref()
                        .map(String::len)
                        .unwrap_or_default()
                        .saturating_add(
                            process
                                .app_bundle_path
                                .as_ref()
                                .map(String::len)
                                .unwrap_or_default(),
                        )
                })
                .sum::<usize>(),
            elapsed_ms = started_at.elapsed().as_millis(),
            "process cache refreshed"
        );
        Ok(processes)
    }

    fn entries_for_search(&self, max_age: Duration) -> Result<Arc<Vec<ProcessRecord>>> {
        let snapshot = self.read_snapshot_state()?;
        if snapshot.entries.is_empty() {
            return self.entries_for_search_with(|| self.collect_process_records_fresh());
        }

        if cache_age_exceeded(snapshot.built_at_ms, max_age) {
            self.schedule_refresh_if_stale(max_age);
        }

        Ok(snapshot.entries)
    }

    fn entries_for_search_with<F>(&self, build: F) -> Result<Arc<Vec<ProcessRecord>>>
    where
        F: FnOnce() -> Result<Vec<ProcessRecord>>,
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

    fn read_snapshot_state(&self) -> Result<ProcessCatalogSnapshot> {
        let guard = read_snapshot_lock(&self.snapshot, "read");
        Ok(guard.clone())
    }

    fn schedule_refresh_if_stale(&self, max_age: Duration) {
        let should_refresh = match self.snapshot_needs_refresh(max_age) {
            Ok(should_refresh) => should_refresh,
            Err(error) => {
                tracing::warn!(
                    error = format_args!("{:#}", error),
                    "failed to inspect process cache state"
                );
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
                tracing::warn!(
                    error = format_args!("{:#}", error),
                    "failed to refresh process cache"
                );
            }
        });
    }

    fn process_record_from_sysinfo(
        &self,
        process: &sysinfo::Process,
        current_pid: u32,
    ) -> Option<ProcessRecord> {
        let pid = process.pid().as_u32();
        if pid == current_pid {
            return None;
        }

        let process_name = process.name().to_string_lossy().trim().to_string();
        if process_name.is_empty() {
            return None;
        }

        let executable_path = process
            .exe()
            .map(|path| path.to_string_lossy().into_owned());
        let app_bundle_path = process
            .exe()
            .and_then(extract_app_bundle_path)
            .map(|path| path.to_string_lossy().into_owned());
        let display_name = app_bundle_path
            .as_deref()
            .and_then(|path| self.localized_app_name(Path::new(path)))
            .or_else(|| {
                app_bundle_path
                    .as_deref()
                    .and_then(|path| bundle_display_name(Path::new(path)))
            })
            .unwrap_or_else(|| process_name.clone());

        Some(ProcessRecord {
            pid,
            display_name,
            process_name,
            executable_path,
            app_bundle_path: app_bundle_path.clone(),
            kind: if app_bundle_path.is_some() {
                RunningProcessKind::App
            } else {
                RunningProcessKind::Process
            },
        })
    }

    fn entries_snapshot(&self) -> Result<Arc<Vec<ProcessRecord>>> {
        Ok(self.read_snapshot_state()?.entries)
    }

    fn begin_refresh(&self) -> Result<bool> {
        let mut guard = write_snapshot_lock(&self.snapshot, "write");
        if guard.refreshing {
            return Ok(false);
        }
        guard.refreshing = true;
        Ok(true)
    }

    fn replace_snapshot(&self, entries: Vec<ProcessRecord>) -> Result<()> {
        let mut guard = write_snapshot_lock(&self.snapshot, "update");
        let version = guard.version.saturating_add(1);
        guard.entries = Arc::new(entries);
        guard.built_at_ms = Some(current_time_ms());
        guard.refreshing = false;
        guard.version = version;
        tracing::info!(
            count = guard.entries.len(),
            version = guard.version,
            "process cache snapshot replaced"
        );
        Ok(())
    }

    fn finish_failed_refresh(&self) -> Result<()> {
        let mut guard = write_snapshot_lock(&self.snapshot, "update");
        guard.refreshing = false;
        Ok(())
    }

    fn snapshot_needs_refresh(&self, max_age: Duration) -> Result<bool> {
        let snapshot = self.read_snapshot_state()?;
        if snapshot.refreshing {
            return Ok(false);
        }

        Ok(snapshot.entries.is_empty() || cache_age_exceeded(snapshot.built_at_ms, max_age))
    }

    #[cfg(target_os = "macos")]
    fn localized_app_name(&self, path: &Path) -> Option<String> {
        let cache_key = path.to_string_lossy().into_owned();
        let guard = read_localized_name_cache(&self.localized_name_cache);
        if let Some(cached) = guard.get(&cache_key).cloned() {
            return cached;
        }

        let resolved = localized_app_name_uncached(path);
        write_localized_name_cache(&self.localized_name_cache).insert(cache_key, resolved.clone());
        resolved
    }

    #[cfg(not(target_os = "macos"))]
    fn localized_app_name(&self, _path: &Path) -> Option<String> {
        None
    }
}

fn read_snapshot_lock<'a>(
    snapshot: &'a Arc<RwLock<ProcessCatalogSnapshot>>,
    action: &str,
) -> std::sync::RwLockReadGuard<'a, ProcessCatalogSnapshot> {
    match snapshot.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                action,
                "process cache lock poisoned; recovering cached state"
            );
            poisoned.into_inner()
        }
    }
}

fn write_snapshot_lock<'a>(
    snapshot: &'a Arc<RwLock<ProcessCatalogSnapshot>>,
    action: &str,
) -> std::sync::RwLockWriteGuard<'a, ProcessCatalogSnapshot> {
    match snapshot.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                action,
                "process cache lock poisoned; recovering cached state"
            );
            poisoned.into_inner()
        }
    }
}

#[cfg(target_os = "macos")]
fn read_localized_name_cache<'a>(
    cache: &'a Arc<RwLock<HashMap<String, Option<String>>>>,
) -> std::sync::RwLockReadGuard<'a, HashMap<String, Option<String>>> {
    match cache.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("process localized name cache lock poisoned; recovering cached state");
            poisoned.into_inner()
        }
    }
}

#[cfg(target_os = "macos")]
fn write_localized_name_cache<'a>(
    cache: &'a Arc<RwLock<HashMap<String, Option<String>>>>,
) -> std::sync::RwLockWriteGuard<'a, HashMap<String, Option<String>>> {
    match cache.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("process localized name cache lock poisoned; recovering cached state");
            poisoned.into_inner()
        }
    }
}

fn current_pid_u32() -> Result<u32> {
    Ok(sysinfo::get_current_pid()
        .map_err(|error| anyhow::anyhow!("failed to resolve current process id: {error}"))?
        .as_u32())
}

fn extract_app_bundle_path(executable_path: &Path) -> Option<PathBuf> {
    executable_path
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .map(Path::to_path_buf)
}

fn bundle_display_name(bundle_path: &Path) -> Option<String> {
    bundle_path
        .file_stem()
        .map(|name| name.to_string_lossy().trim().to_string())
        .filter(|name| !name.is_empty())
}

fn normalize_text(value: &str) -> String {
    let trimmed = value.trim();
    let stripped = trimmed.strip_suffix(".app").unwrap_or(trimmed);
    stripped.to_lowercase()
}

fn parse_pid_target(value: &str) -> Option<u32> {
    let pid_text = value
        .trim()
        .strip_prefix("pid:")
        .or_else(|| value.trim().strip_prefix("PID:"))?
        .trim();
    pid_text.parse::<u32>().ok()
}

#[cfg(target_os = "macos")]
fn localized_app_name_uncached(path: &Path) -> Option<String> {
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

fn rank_processes(
    processes: &[ProcessRecord],
    query: &str,
    limit: usize,
) -> Vec<RunningProcessMatch> {
    if limit == 0 {
        return Vec::new();
    }

    let normalized_query = normalize_text(query);
    let matcher = SkimMatcherV2::default().ignore_case();
    let mut matches = processes
        .iter()
        .filter_map(|record| {
            let score = score_process_match(record, &normalized_query, &matcher)?;
            Some(RunningProcessMatch {
                pid: record.pid,
                display_name: record.display_name.clone(),
                process_name: record.process_name.clone(),
                executable_path: record.executable_path.clone(),
                app_bundle_path: record.app_bundle_path.clone(),
                kind: record.kind,
                score,
            })
        })
        .collect::<Vec<_>>();

    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| left.pid.cmp(&right.pid))
    });
    matches.truncate(limit);
    matches
}

fn score_process_match(
    record: &ProcessRecord,
    normalized_query: &str,
    matcher: &SkimMatcherV2,
) -> Option<i32> {
    if normalized_query.is_empty() {
        return None;
    }

    let pid_text = record.pid.to_string();
    if pid_text.starts_with(normalized_query) {
        return Some(420 - (pid_text.len() as i32));
    }

    let mut best_score: Option<i32> = None;
    for candidate in process_search_candidates(record) {
        let normalized_candidate = normalize_text(&candidate.value);
        let candidate_score = if normalized_candidate == normalized_query {
            Some(candidate.base_score + 260)
        } else if normalized_candidate.starts_with(normalized_query) {
            Some(candidate.base_score + 180)
        } else if normalized_candidate.contains(normalized_query) {
            Some(candidate.base_score + 110)
        } else {
            matcher
                .fuzzy_match(&candidate.value, normalized_query)
                .map(|score| candidate.base_score + score as i32)
        };

        if let Some(candidate_score) = candidate_score {
            best_score =
                Some(best_score.map_or(candidate_score, |current| current.max(candidate_score)));
        }
    }

    best_score
}

fn process_search_candidates(record: &ProcessRecord) -> Vec<ProcessSearchCandidate> {
    let mut candidates = vec![
        ProcessSearchCandidate {
            value: record.display_name.clone(),
            base_score: 180,
        },
        ProcessSearchCandidate {
            value: record.process_name.clone(),
            base_score: 160,
        },
    ];

    if let Some(executable_path) = record.executable_path.as_deref() {
        if let Some(file_name) = Path::new(executable_path).file_name() {
            let executable_name = file_name.to_string_lossy().into_owned();
            if !executable_name.is_empty() {
                candidates.push(ProcessSearchCandidate {
                    value: executable_name,
                    base_score: 120,
                });
            }
        }
    }

    candidates
}

fn resolve_kill_target(target_text: &str, processes: &[ProcessRecord]) -> KillTargetResolution {
    if let Some(pid) = parse_pid_target(target_text) {
        return processes
            .iter()
            .find(|record| record.pid == pid)
            .cloned()
            .map(KillTargetResolution::Unique)
            .unwrap_or(KillTargetResolution::NotFound);
    }

    let normalized_target = normalize_text(target_text);
    let mut exact_matches = processes
        .iter()
        .filter(|record| {
            normalize_text(&record.display_name) == normalized_target
                || normalize_text(&record.process_name) == normalized_target
        })
        .cloned()
        .collect::<Vec<_>>();
    exact_matches.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.pid.cmp(&right.pid))
    });

    match exact_matches.len() {
        0 => KillTargetResolution::NotFound,
        1 => KillTargetResolution::Unique(exact_matches.remove(0)),
        _ => KillTargetResolution::Ambiguous(exact_matches),
    }
}

fn terminate_process(target: &ProcessRecord) -> Result<ExecutionResult> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(target.pid)]),
        true,
        ProcessRefreshKind::everything(),
    );

    let Some(process) = system.process(Pid::from_u32(target.pid)) else {
        return Ok(warning_result(
            Some(target.display_name.clone()),
            "目标进程已经退出",
        ));
    };

    #[cfg(unix)]
    let terminated = process.kill_with(Signal::Term).unwrap_or(false);
    #[cfg(not(unix))]
    let terminated = process.kill();

    if !terminated {
        bail!("无法终止 {} (pid {})", target.display_name, target.pid);
    }

    Ok(ExecutionResult::success(
        Some(format!("{} (pid {})", target.display_name, target.pid)),
        Some("已请求终止目标进程".to_string()),
        None,
        vec![],
        true,
    ))
}

fn extract_kill_payload(text: &str) -> Option<&str> {
    crate::services::command_prefix::extract_prefixed_payload(text, &KILL_COMMAND_ALIASES)
}

fn format_ambiguous_targets(matches: &[ProcessRecord]) -> String {
    matches
        .iter()
        .map(|record| format!("{} (pid {})", record.display_name, record.pid))
        .collect::<Vec<_>>()
        .join("、")
}

fn warning_result(primary_text: Option<String>, message: &str) -> ExecutionResult {
    ExecutionResult {
        status: ExecutionStatus::Warning,
        primary_text,
        secondary_text: Some(message.to_string()),
        structured_payload: None,
        next_actions: Vec::new(),
        should_close_launcher: false,
    }
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

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc, time::Duration};

    use super::{
        bundle_display_name, cache_age_exceeded, extract_app_bundle_path, normalize_display_name,
        rank_processes, resolve_kill_target, KillTargetResolution, ProcessCatalogSnapshot,
        ProcessRecord, ProcessService,
    };
    use crate::domain::process::RunningProcessKind;

    fn process_record(pid: u32, display_name: &str, process_name: &str) -> ProcessRecord {
        ProcessRecord {
            pid,
            display_name: display_name.to_string(),
            process_name: process_name.to_string(),
            executable_path: Some(format!("/tmp/{process_name}")),
            app_bundle_path: None,
            kind: RunningProcessKind::Process,
        }
    }

    #[test]
    fn extracts_bundle_path_from_executable_path() {
        let executable_path = Path::new("/Applications/WeChat.app/Contents/MacOS/WeChat");
        let bundle_path = extract_app_bundle_path(executable_path).expect("bundle path");
        assert_eq!(bundle_path, Path::new("/Applications/WeChat.app"));
        assert_eq!(bundle_display_name(&bundle_path).as_deref(), Some("WeChat"));
    }

    #[test]
    fn resolves_exact_name_to_unique_process() {
        let processes = vec![process_record(101, "WeChat", "WeChat")];
        let resolution = resolve_kill_target("wechat", &processes);
        assert!(matches!(resolution, KillTargetResolution::Unique(_)));
    }

    #[test]
    fn resolves_duplicate_exact_name_as_ambiguous() {
        let processes = vec![
            process_record(101, "node", "node"),
            process_record(202, "node", "node"),
        ];
        let resolution = resolve_kill_target("node", &processes);
        assert!(
            matches!(resolution, KillTargetResolution::Ambiguous(matches) if matches.len() == 2)
        );
    }

    #[test]
    fn resolves_pid_target_exactly() {
        let processes = vec![
            process_record(101, "node", "node"),
            process_record(202, "pnpm", "pnpm"),
        ];
        let resolution = resolve_kill_target("pid:202", &processes);
        match resolution {
            KillTargetResolution::Unique(record) => assert_eq!(record.pid, 202),
            other => panic!("unexpected resolution: {other:?}"),
        }
    }

    #[test]
    fn ranks_display_name_prefix_above_weaker_match() {
        let processes = vec![
            process_record(101, "Google Chrome", "Google Chrome"),
            process_record(202, "Electron Helper", "Electron Helper"),
        ];
        let matches = rank_processes(&processes, "chr", 8);
        assert_eq!(matches.first().map(|item| item.pid), Some(101));
    }

    #[test]
    fn resolves_app_suffix_name_to_unique_process() {
        let processes = vec![process_record(101, "WeChat", "WeChat")];
        let resolution = resolve_kill_target("WeChat.app", &processes);
        assert!(matches!(resolution, KillTargetResolution::Unique(_)));
    }

    #[test]
    fn resolves_uppercase_pid_prefix() {
        let processes = vec![process_record(101, "node", "node")];
        let resolution = resolve_kill_target("PID:101", &processes);
        assert!(matches!(resolution, KillTargetResolution::Unique(_)));
    }

    #[test]
    fn normalize_display_name_strips_app_suffix() {
        assert_eq!(
            normalize_display_name("WeChat.app").as_deref(),
            Some("WeChat")
        );
        assert_eq!(normalize_display_name("(null)"), None);
    }

    #[test]
    fn cache_age_reports_stale_only_after_threshold() {
        let now_ms = super::current_time_ms();
        assert!(!cache_age_exceeded(Some(now_ms), Duration::from_secs(5)));
        assert!(cache_age_exceeded(
            Some(now_ms.saturating_sub(6_000)),
            Duration::from_secs(5)
        ));
    }

    #[test]
    fn search_entries_clears_refreshing_after_direct_build() {
        let service = ProcessService::new();
        *service.snapshot.write().expect("snapshot lock should work") = ProcessCatalogSnapshot {
            entries: Arc::new(Vec::new()),
            built_at_ms: None,
            refreshing: true,
            version: 11,
        };

        let entries = service
            .entries_for_search_with(|| Ok(vec![process_record(42, "Arc", "Arc")]))
            .expect("fallback build should succeed");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].pid, 42);
        let snapshot = service
            .snapshot
            .read()
            .expect("snapshot lock should work after fallback");
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].pid, 42);
        assert!(!snapshot.refreshing);
        assert_eq!(snapshot.version, 12);
    }

    #[test]
    fn search_entries_builds_snapshot_when_cache_is_empty() {
        let service = ProcessService::new();

        let entries = service
            .entries_for_search_with(|| Ok(vec![process_record(7, "Arc", "Arc")]))
            .expect("empty cache should build snapshot synchronously");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].pid, 7);

        let snapshot = service
            .snapshot
            .read()
            .expect("snapshot lock should work after synchronous build");
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].pid, 7);
        assert!(!snapshot.refreshing);
        assert_eq!(snapshot.version, 1);
    }

    #[tokio::test]
    async fn search_running_returns_stale_snapshot_while_async_refresh_is_pending() {
        let service = ProcessService::new();
        *service.snapshot.write().expect("snapshot lock should work") = ProcessCatalogSnapshot {
            entries: Arc::new(vec![process_record(7, "Old", "Old")]),
            built_at_ms: Some(super::current_time_ms().saturating_sub(20_000)),
            refreshing: false,
            version: 3,
        };

        let entries = service
            .search_running("old", 5)
            .expect("stale process snapshot should still return cached results immediately");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].pid, 7);

        let snapshot = service
            .snapshot
            .read()
            .expect("snapshot lock should work after immediate return");
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].pid, 7);
    }

    #[tokio::test]
    async fn refresh_now_clears_refreshing_when_background_build_panics() {
        let service = ProcessService::new();

        let error = service
            .refresh_now_with(|| -> anyhow::Result<Vec<ProcessRecord>> {
                panic!("panic during process refresh");
            })
            .await
            .expect_err("panic in refresh task should surface as an error");

        assert!(error
            .to_string()
            .contains("failed to join process cache refresh task"));
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
        let service = ProcessService::new();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = service
                .snapshot
                .write()
                .expect("snapshot lock should succeed");
            panic!("poison process snapshot lock");
        }));

        let entries = service
            .entries_snapshot()
            .expect("poisoned process snapshot lock should recover");
        assert!(entries.is_empty());
    }
}
