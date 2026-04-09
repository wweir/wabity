use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
        Arc, Mutex, RwLock,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use ignore::{WalkBuilder, WalkState};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

#[cfg(target_os = "macos")]
use std::process::Command;

use crate::domain::file_search::FileSearchMatch;

const FILE_SEARCH_CACHE_CAPACITY: usize = 4;
const FILE_SEARCH_WATCH_DEBOUNCE_WINDOW: Duration = Duration::from_millis(180);
const FILE_SEARCH_WATCH_IDLE_POLL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
pub struct FileSearchService {
    cache: Arc<RwLock<FileSearchCache>>,
}

#[derive(Debug, Default)]
struct FileSearchCache {
    workspaces: HashMap<PathBuf, Arc<WorkspaceIndexHandle>>,
    next_access_tick: u64,
}

#[derive(Debug)]
struct WorkspaceIndexHandle {
    snapshot: Arc<RwLock<Arc<Vec<FileRecord>>>>,
    last_access_tick: AtomicU64,
    watcher: Mutex<Option<WorkspaceWatcherHandle>>,
}

#[derive(Debug)]
struct WorkspaceWatcherHandle {
    watcher: RecommendedWatcher,
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileRecord {
    path: String,
}

impl FileRecord {
    fn file_name(&self) -> &str {
        file_name_from_record_path(&self.path)
    }

    fn parent(&self) -> &str {
        parent_from_record_path(&self.path)
    }
}

impl FileSearchService {
    pub fn new() -> Result<Self> {
        Ok(Self {
            cache: Arc::new(RwLock::new(FileSearchCache::default())),
        })
    }

    pub fn search(
        &self,
        workspace_root: &Path,
        query: &str,
        limit: usize,
    ) -> Result<Vec<FileSearchMatch>> {
        let needle = query.trim().trim_start_matches('@').trim();
        if !is_file_search_ready(needle) {
            return Ok(Vec::new());
        }

        if self.should_use_native_index(workspace_root)? {
            if let Some(matches) = self.search_with_native_index(workspace_root, needle, limit)? {
                return Ok(matches);
            }
        }

        let handle = self.workspace_index(workspace_root)?;
        let snapshot = handle.snapshot()?;
        Ok(rank_matches(snapshot.as_ref(), needle, limit))
    }

    fn workspace_index(&self, workspace_root: &Path) -> Result<Arc<WorkspaceIndexHandle>> {
        let workspace_root = normalize_workspace_cache_key(workspace_root);
        {
            let mut write_guard = self
                .cache
                .write()
                .map_err(|_| anyhow::anyhow!("failed to update file index cache state"))?;
            let access_tick = next_access_tick(&mut write_guard);
            if let Some(handle) = write_guard.workspaces.get(&workspace_root) {
                handle.mark_access(access_tick);
                return Ok(Arc::clone(handle));
            }
        }

        let built = Arc::new(WorkspaceIndexHandle::build(&workspace_root)?);
        if !built.has_watcher() {
            tracing::warn!(
                "file search watcher unavailable; caching a static workspace snapshot instead"
            );
        }

        let mut write_guard = self
            .cache
            .write()
            .map_err(|_| anyhow::anyhow!("failed to update file index cache state"))?;
        let access_tick = next_access_tick(&mut write_guard);

        if let Some(existing) = write_guard.workspaces.get(&workspace_root) {
            built.shutdown_watcher();
            existing.mark_access(access_tick);
            return Ok(Arc::clone(existing));
        }

        cache_workspace_index(
            &mut write_guard,
            &workspace_root,
            Arc::clone(&built),
            access_tick,
        );
        Ok(built)
    }

    fn search_with_native_index(
        &self,
        workspace_root: &Path,
        needle: &str,
        limit: usize,
    ) -> Result<Option<Vec<FileSearchMatch>>> {
        #[cfg(target_os = "macos")]
        {
            match self.search_with_spotlight(workspace_root, needle, limit) {
                Ok(Some(matches)) => {
                    tracing::debug!(
                        query_len = needle.chars().count(),
                        count = matches.len(),
                        "file search resolved through spotlight"
                    );
                    return Ok(Some(matches));
                }
                Ok(None) => {
                    tracing::debug!(
                        query_len = needle.chars().count(),
                        "spotlight returned no file matches, falling back"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        ?error,
                        query_len = needle.chars().count(),
                        "spotlight search failed, falling back"
                    );
                }
            }
        }

        Ok(None)
    }

    #[cfg(target_os = "macos")]
    fn should_use_native_index(&self, workspace_root: &Path) -> Result<bool> {
        Ok(self.cached_workspace_snapshot(workspace_root)?.is_some())
    }

    #[cfg(not(target_os = "macos"))]
    fn should_use_native_index(&self, _workspace_root: &Path) -> Result<bool> {
        Ok(false)
    }

    #[cfg(target_os = "macos")]
    fn search_with_spotlight(
        &self,
        workspace_root: &Path,
        needle: &str,
        limit: usize,
    ) -> Result<Option<Vec<FileSearchMatch>>> {
        let output = Command::new("mdfind")
            .arg("-onlyin")
            .arg(workspace_root)
            .arg("-name")
            .arg(needle)
            .output()
            .context("failed to spawn mdfind for file search")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("mdfind exited with status {}: {stderr}", output.status);
        }

        let stdout = String::from_utf8(output.stdout).context("mdfind returned non-utf8 output")?;
        let candidates = collect_path_records(stdout.lines().map(str::trim), workspace_root);
        let candidates = match self.cached_workspace_snapshot(workspace_root)? {
            Some(snapshot) => filter_records_to_snapshot(candidates, snapshot.as_ref()),
            None => filter_records_with_workspace_rules(candidates, workspace_root),
        };

        if candidates.is_empty() {
            return Ok(None);
        }

        Ok(Some(rank_matches(&candidates, needle, limit)))
    }

    #[cfg(target_os = "macos")]
    fn cached_workspace_snapshot(
        &self,
        workspace_root: &Path,
    ) -> Result<Option<Arc<Vec<FileRecord>>>> {
        let workspace_root = normalize_workspace_cache_key(workspace_root);
        let mut write_guard = self
            .cache
            .write()
            .map_err(|_| anyhow::anyhow!("failed to update file index cache state"))?;
        let access_tick = next_access_tick(&mut write_guard);
        let Some(handle) = write_guard.workspaces.get(&workspace_root) else {
            return Ok(None);
        };
        handle.mark_access(access_tick);
        handle.snapshot().map(Some)
    }
}

fn normalize_workspace_cache_key(workspace_root: &Path) -> PathBuf {
    workspace_root
        .canonicalize()
        .unwrap_or_else(|_| normalize_update_path(workspace_root))
}

impl WorkspaceIndexHandle {
    fn build(workspace_root: &Path) -> Result<Self> {
        tracing::info!("building workspace file index");

        let root = workspace_root.to_path_buf();
        let initial_records = build_initial_record_map(&root)?;
        let snapshot = Arc::new(RwLock::new(Arc::new(snapshot_from_records_map(
            &initial_records,
        ))));
        let records = Arc::new(Mutex::new(initial_records));
        let watcher = start_workspace_watcher(&root, Arc::clone(&records), Arc::clone(&snapshot));

        let count = snapshot
            .read()
            .map_err(|_| anyhow::anyhow!("failed to read workspace file snapshot"))?
            .len();
        let total_path_bytes = records
            .lock()
            .map(|guard| {
                guard
                    .values()
                    .map(|record| record.path.len())
                    .sum::<usize>()
            })
            .unwrap_or_default();
        let (estimated_file_name_bytes, estimated_parent_bytes) = {
            let snapshot_guard = snapshot
                .read()
                .map_err(|_| anyhow::anyhow!("failed to read workspace file snapshot"))?;
            (
                total_file_name_bytes(snapshot_guard.as_ref()),
                total_parent_bytes(snapshot_guard.as_ref()),
            )
        };
        tracing::info!(
            count,
            total_path_bytes,
            estimated_file_name_bytes,
            estimated_parent_bytes,
            "workspace file index ready"
        );

        Ok(Self {
            snapshot,
            last_access_tick: AtomicU64::new(0),
            watcher: Mutex::new(watcher),
        })
    }

    fn mark_access(&self, tick: u64) {
        self.last_access_tick.store(tick, Ordering::Relaxed);
    }

    fn has_watcher(&self) -> bool {
        self.watcher
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    fn snapshot(&self) -> Result<Arc<Vec<FileRecord>>> {
        self.snapshot
            .read()
            .map(|snapshot| Arc::clone(&snapshot))
            .map_err(|_| anyhow::anyhow!("failed to read workspace file snapshot"))
    }

    fn shutdown_watcher(&self) {
        let watcher = match self.watcher.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => None,
        };
        if let Some(watcher) = watcher {
            watcher.shutdown();
        }
    }
}

impl Drop for WorkspaceIndexHandle {
    fn drop(&mut self) {
        self.shutdown_watcher();
    }
}

impl WorkspaceWatcherHandle {
    fn shutdown(mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        drop(self.watcher);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn start_workspace_watcher(
    root: &Path,
    records: Arc<Mutex<HashMap<String, FileRecord>>>,
    snapshot: Arc<RwLock<Arc<Vec<FileRecord>>>>,
) -> Option<WorkspaceWatcherHandle> {
    let (event_tx, event_rx) = mpsc::channel::<notify::Result<Event>>();
    let root = root.to_path_buf();
    let mut watcher = match notify::recommended_watcher(move |result| {
        if event_tx.send(result).is_err() {
            tracing::debug!("file search watcher callback dropped because receiver is gone");
        }
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(?error, "failed to create file search watcher");
            return None;
        }
    };

    if let Err(error) = watcher.watch(&root, RecursiveMode::Recursive) {
        tracing::warn!(?error, "failed to watch workspace for file search updates");
        return None;
    }

    let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_stop_flag = Arc::clone(&stop_flag);
    let worker = match thread::Builder::new()
        .name(format!(
            "wabity-file-search-{}",
            FILE_SEARCH_WATCH_THREAD_ID.fetch_add(1, Ordering::Relaxed)
        ))
        .spawn(move || {
            run_workspace_watch_loop(root, records, snapshot, event_rx, worker_stop_flag);
        }) {
        Ok(worker) => worker,
        Err(error) => {
            tracing::warn!(?error, "failed to spawn file search watcher worker");
            return None;
        }
    };

    Some(WorkspaceWatcherHandle {
        watcher,
        stop_flag,
        worker: Some(worker),
    })
}

static FILE_SEARCH_WATCH_THREAD_ID: AtomicUsize = AtomicUsize::new(1);

fn run_workspace_watch_loop(
    root: PathBuf,
    records: Arc<Mutex<HashMap<String, FileRecord>>>,
    snapshot: Arc<RwLock<Arc<Vec<FileRecord>>>>,
    event_rx: Receiver<notify::Result<Event>>,
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
) {
    loop {
        if stop_flag.load(Ordering::Relaxed) {
            return;
        }

        let first_event = match event_rx.recv_timeout(FILE_SEARCH_WATCH_IDLE_POLL) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };

        let mut events = vec![first_event];
        let deadline = Instant::now() + FILE_SEARCH_WATCH_DEBOUNCE_WINDOW;
        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            match event_rx.recv_timeout(remaining) {
                Ok(event) => events.push(event),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        if let Err(error) = apply_event_batch(&root, &records, &snapshot, events) {
            tracing::warn!(?error, "failed to apply incremental file search update");
        }
    }
}

fn apply_event_batch(
    root: &Path,
    records: &Arc<Mutex<HashMap<String, FileRecord>>>,
    snapshot: &Arc<RwLock<Arc<Vec<FileRecord>>>>,
    events: Vec<notify::Result<Event>>,
) -> Result<()> {
    let (remove_targets, rescan_targets) = collect_update_targets(root, events);
    if remove_targets.is_empty() && rescan_targets.is_empty() {
        return Ok(());
    }

    apply_path_updates(root, records, snapshot, &remove_targets, &rescan_targets)
}

fn apply_path_updates(
    root: &Path,
    records: &Arc<Mutex<HashMap<String, FileRecord>>>,
    snapshot: &Arc<RwLock<Arc<Vec<FileRecord>>>>,
    remove_targets: &BTreeSet<PathBuf>,
    rescan_targets: &BTreeSet<PathBuf>,
) -> Result<()> {
    let mut guard = records
        .lock()
        .map_err(|_| anyhow::anyhow!("failed to update workspace file records"))?;

    for target in remove_targets {
        remove_records_for_path(&mut guard, root, target);
    }

    for target in rescan_targets {
        remove_records_for_path(&mut guard, root, target);
        upsert_records_for_path(&mut guard, root, target)?;
    }

    let next_snapshot = Arc::new(snapshot_from_records_map(&guard));
    drop(guard);

    let mut snapshot_guard = snapshot
        .write()
        .map_err(|_| anyhow::anyhow!("failed to replace workspace file snapshot"))?;
    *snapshot_guard = next_snapshot;
    Ok(())
}

fn collect_update_targets(
    root: &Path,
    events: Vec<notify::Result<Event>>,
) -> (BTreeSet<PathBuf>, BTreeSet<PathBuf>) {
    let mut remove_targets = BTreeSet::new();
    let mut rescan_targets = BTreeSet::new();

    for event in events {
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(?error, "file search watcher received invalid event");
                continue;
            }
        };

        for path in event.paths {
            let Some(update_target) = classify_update_target(root, &path) else {
                continue;
            };
            match update_target {
                UpdateTarget::Rescan(target) => {
                    rescan_targets.insert(target);
                }
                UpdateTarget::Remove(target) => {
                    remove_targets.insert(target);
                }
            }
        }
    }

    prune_nested_paths(&mut remove_targets);
    prune_nested_paths(&mut rescan_targets);
    (remove_targets, rescan_targets)
}

enum UpdateTarget {
    Remove(PathBuf),
    Rescan(PathBuf),
}

fn classify_update_target(root: &Path, path: &Path) -> Option<UpdateTarget> {
    let path = normalize_update_path(path);

    if let Some(scope) = ignore_rule_rescan_scope(root, &path) {
        return Some(UpdateTarget::Rescan(scope));
    }

    if !path.starts_with(root) || should_skip(&path) {
        return None;
    }

    if path.exists() {
        Some(UpdateTarget::Rescan(path.to_path_buf()))
    } else {
        Some(UpdateTarget::Remove(path.to_path_buf()))
    }
}

fn normalize_update_path(path: &Path) -> PathBuf {
    if let Ok(canonical_path) = path.canonicalize() {
        return canonical_path;
    }

    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name()) {
        if let Ok(canonical_parent) = parent.canonicalize() {
            return canonical_parent.join(file_name);
        }
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

fn ignore_rule_rescan_scope(root: &Path, path: &Path) -> Option<PathBuf> {
    if !path.starts_with(root) {
        return None;
    }

    if path
        .file_name()
        .is_some_and(|name| name == ".gitignore" || name == ".ignore")
    {
        return Some(path.parent().unwrap_or(root).to_path_buf());
    }

    let exclude_path = root.join(".git").join("info").join("exclude");
    if path == exclude_path {
        return Some(root.to_path_buf());
    }

    None
}

fn prune_nested_paths(paths: &mut BTreeSet<PathBuf>) {
    let ordered = paths.iter().cloned().collect::<Vec<_>>();
    let mut kept = BTreeSet::new();

    'outer: for candidate in ordered {
        for existing in &kept {
            if candidate.starts_with(existing) {
                continue 'outer;
            }
        }
        kept.insert(candidate);
    }

    *paths = kept;
}

fn build_initial_record_map(workspace_root: &Path) -> Result<HashMap<String, FileRecord>> {
    let records = build_record_vec(workspace_root)?;
    Ok(records
        .into_iter()
        .map(|record| (record.path.clone(), record))
        .collect())
}

fn build_record_vec(workspace_root: &Path) -> Result<Vec<FileRecord>> {
    let files = Mutex::new(Vec::new());
    let walker = configured_walk_builder(workspace_root)
        .threads(
            std::thread::available_parallelism()
                .map(|parallelism| parallelism.get())
                .unwrap_or(4),
        )
        .build_parallel();

    walker.run(|| {
        let files = &files;
        Box::new(move |entry| {
            let dir_entry = match entry {
                Ok(dir_entry) => dir_entry,
                Err(_) => return WalkState::Continue,
            };

            let path = dir_entry.path();
            let file_type = match dir_entry.file_type() {
                Some(file_type) => file_type,
                None => return WalkState::Continue,
            };

            if !file_type.is_file() || should_skip(path) {
                return WalkState::Continue;
            }

            if let Some(record) = build_record_for_path(path) {
                if let Ok(mut guard) = files.lock() {
                    guard.push(record);
                }
            }

            WalkState::Continue
        })
    });

    files
        .into_inner()
        .map_err(|_| anyhow::anyhow!("file index mutex poisoned"))
}

fn upsert_records_for_path(
    records: &mut HashMap<String, FileRecord>,
    workspace_root: &Path,
    path: &Path,
) -> Result<()> {
    if !(path.is_file() || path.is_dir()) {
        return Ok(());
    }

    for record in collect_path_records_for_update(workspace_root, path)? {
        records.insert(record.path.clone(), record);
    }

    Ok(())
}

fn collect_path_records_for_update(
    workspace_root: &Path,
    target: &Path,
) -> Result<Vec<FileRecord>> {
    let target = target.to_path_buf();
    let mut builder = configured_walk_builder(workspace_root);
    builder.filter_entry({
        let target = target.clone();
        move |entry| {
            let path = entry.path();
            path.starts_with(&target) || target.starts_with(path)
        }
    });

    let mut records = Vec::new();
    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(?error, "failed to walk subtree for file search update");
                continue;
            }
        };

        let path = entry.path();
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() || should_skip(path) || !path.starts_with(&target) {
            continue;
        }

        if let Some(record) = build_record_for_path(path) {
            records.push(record);
        }
    }
    Ok(records)
}

fn configured_walk_builder(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true);
    builder
}

fn build_record_for_path(path: &Path) -> Option<FileRecord> {
    if !path.is_file() || should_skip(path) {
        return None;
    }

    Some(FileRecord {
        path: path.to_string_lossy().into_owned(),
    })
}

fn remove_records_for_path(records: &mut HashMap<String, FileRecord>, root: &Path, target: &Path) {
    if !target.starts_with(root) {
        return;
    }

    let target_prefix = target.to_string_lossy().into_owned();
    records.retain(|path, _| {
        path != &target_prefix
            && !Path::new(path).starts_with(target)
            && !path.starts_with(&format!("{target_prefix}/"))
    });
}

fn snapshot_from_records_map(records: &HashMap<String, FileRecord>) -> Vec<FileRecord> {
    let mut snapshot = records.values().cloned().collect::<Vec<_>>();
    snapshot.sort_by(|left, right| left.path.cmp(&right.path));
    snapshot
}

fn eviction_root(
    workspaces: &HashMap<PathBuf, Arc<WorkspaceIndexHandle>>,
    protected_root: &Path,
) -> Option<PathBuf> {
    workspaces
        .iter()
        .filter(|(root, _)| root.as_path() != protected_root)
        .min_by_key(|(_, handle)| handle.last_access_tick.load(Ordering::Relaxed))
        .map(|(root, _)| root.clone())
}

fn cache_workspace_index(
    cache: &mut FileSearchCache,
    workspace_root: &Path,
    handle: Arc<WorkspaceIndexHandle>,
    access_tick: u64,
) {
    handle.mark_access(access_tick);
    cache
        .workspaces
        .insert(workspace_root.to_path_buf(), Arc::clone(&handle));

    while cache.workspaces.len() > FILE_SEARCH_CACHE_CAPACITY {
        let Some(eviction_root) = eviction_root(&cache.workspaces, workspace_root) else {
            break;
        };
        if let Some(stale) = cache.workspaces.remove(&eviction_root) {
            stale.shutdown_watcher();
        }
    }
}

fn next_access_tick(cache: &mut FileSearchCache) -> u64 {
    let next_tick = cache.next_access_tick.saturating_add(1);
    cache.next_access_tick = next_tick;
    next_tick
}

fn is_file_search_ready(needle: &str) -> bool {
    let trimmed = needle.trim();
    if trimmed.is_empty() {
        return false;
    }

    let non_english_letter_count = trimmed
        .chars()
        .filter(|character| character.is_alphabetic() && !character.is_ascii_alphabetic())
        .count();
    if non_english_letter_count >= 1 {
        return true;
    }

    trimmed
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .count()
        >= 2
}

fn rank_matches(entries: &[FileRecord], needle: &str, limit: usize) -> Vec<FileSearchMatch> {
    let normalized_limit = limit.clamp(1, 20);
    let matcher = SkimMatcherV2::default()
        .smart_case()
        .element_limit(1024 * 1024 * 1024);
    let mut ranked = Vec::with_capacity(normalized_limit);

    for entry in entries {
        if let Some(score) = score_entry(&matcher, entry, needle) {
            push_ranked(
                &mut ranked,
                FileSearchMatch {
                    path: entry.path.clone(),
                    file_name: entry.file_name().to_string(),
                    parent: entry.parent().to_string(),
                    score: saturating_score(score),
                },
                normalized_limit,
            );
        }
    }

    ranked.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
    });
    ranked
}

fn score_entry(matcher: &SkimMatcherV2, entry: &FileRecord, needle: &str) -> Option<i64> {
    let path_score = matcher.fuzzy_match(&entry.path, needle);
    let file_name_score = matcher
        .fuzzy_match(entry.file_name(), needle)
        .map(|score| score.saturating_add(100));

    match (file_name_score, path_score) {
        (Some(file_name_score), Some(path_score)) => Some(file_name_score.max(path_score)),
        (Some(file_name_score), None) => Some(file_name_score),
        (None, Some(path_score)) => Some(path_score),
        (None, None) => None,
    }
}

fn file_name_from_record_path(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn parent_from_record_path(path: &str) -> &str {
    path.rsplit_once(['/', '\\'])
        .map(|(parent, _)| parent)
        .unwrap_or_default()
}

fn total_file_name_bytes(records: &[FileRecord]) -> usize {
    records.iter().map(|record| record.file_name().len()).sum()
}

fn total_parent_bytes(records: &[FileRecord]) -> usize {
    records.iter().map(|record| record.parent().len()).sum()
}

fn saturating_score(score: i64) -> u16 {
    score.clamp(0, i64::from(u16::MAX)) as u16
}

fn push_ranked(ranked: &mut Vec<FileSearchMatch>, candidate: FileSearchMatch, limit: usize) {
    ranked.push(candidate);
    ranked.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
    });

    if ranked.len() > limit {
        ranked.pop();
    }
}

fn should_skip(path: &Path) -> bool {
    let path_string = path.to_string_lossy().replace('\\', "/");

    [
        "/.git/",
        "/node_modules/",
        "/target/",
        "/.Trash/",
        "/Library/Caches/",
        "/Library/Containers/",
        "/.npm/",
        "/.cargo/registry/",
        "/.cache/",
    ]
    .iter()
    .any(|segment| path_string.contains(segment))
}

fn collect_path_records<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    root: &Path,
) -> Vec<FileRecord> {
    let mut records = Vec::new();

    for raw_path in paths {
        if raw_path.is_empty() {
            continue;
        }

        let path = PathBuf::from(raw_path);
        if !path.starts_with(root) || !path.is_file() || should_skip(&path) {
            continue;
        }

        if let Some(record) = build_record_for_path(&path) {
            records.push(record);
        }
    }

    records
}

fn filter_records_to_snapshot(
    records: Vec<FileRecord>,
    snapshot: &[FileRecord],
) -> Vec<FileRecord> {
    records
        .into_iter()
        .filter(|record| {
            snapshot
                .binary_search_by(|entry| entry.path.cmp(&record.path))
                .is_ok()
        })
        .collect()
}

fn filter_records_with_workspace_rules(
    records: Vec<FileRecord>,
    workspace_root: &Path,
) -> Vec<FileRecord> {
    if records.is_empty() {
        return records;
    }

    let candidate_paths = records
        .iter()
        .map(|record| PathBuf::from(&record.path))
        .collect::<Vec<_>>();
    let candidate_lookup = candidate_paths.iter().cloned().collect::<HashSet<_>>();
    let mut builder = configured_walk_builder(workspace_root);
    builder.filter_entry({
        let candidate_paths = candidate_paths.clone();
        move |entry| {
            let path = entry.path();
            candidate_paths
                .iter()
                .any(|candidate| candidate.starts_with(path))
        }
    });

    let mut allowed_paths = HashSet::new();
    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(
                    ?error,
                    root = %workspace_root.display(),
                    "failed to validate spotlight file search candidates"
                );
                continue;
            }
        };

        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file() && candidate_lookup.contains(entry.path()) {
            allowed_paths.insert(entry.into_path());
        }
    }

    records
        .into_iter()
        .filter(|record| allowed_paths.contains(Path::new(&record.path)))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        apply_path_updates, build_initial_record_map, cache_workspace_index, collect_path_records,
        collect_update_targets, eviction_root, filter_records_to_snapshot,
        filter_records_with_workspace_rules, is_file_search_ready, rank_matches, should_skip,
        snapshot_from_records_map, FileRecord, FileSearchCache, FileSearchService,
        WorkspaceIndexHandle,
    };
    use notify::Event;
    use std::{
        collections::{BTreeSet, HashMap},
        path::PathBuf,
        sync::{atomic::AtomicU64, Arc, Mutex, RwLock},
    };

    fn record(path: &str) -> FileRecord {
        FileRecord {
            path: path.to_string(),
        }
    }

    #[test]
    fn fuzzy_search_prefers_close_filename_match() {
        let entries = vec![
            record("/Users/demo/Documents/wabity-plan.md"),
            record("/Users/demo/Desktop/random-note.txt"),
        ];

        let matches = rank_matches(&entries, "wbp", 5);
        assert_eq!(
            matches.first().map(|item| item.file_name.as_str()),
            Some("wabity-plan.md")
        );
    }

    #[test]
    fn fuzzy_search_handles_mixed_case_query_without_panicking() {
        let entries = vec![
            record("/Users/demo/Documents/aaA.md"),
            record("/Users/demo/Documents/alpha.md"),
        ];

        let matches = rank_matches(&entries, "aA", 5);
        assert_eq!(
            matches.first().map(|item| item.file_name.as_str()),
            Some("aaA.md")
        );
    }

    #[test]
    fn collect_path_records_filters_out_paths_outside_root() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("wabity-file-search-{unique}"));
        let inside_dir = root.join("Documents");
        let inside_file = inside_dir.join("kept.txt");
        let outside_file = std::env::temp_dir().join(format!("wabity-outside-{unique}.txt"));

        fs::create_dir_all(&inside_dir).expect("failed to create inside directory");
        fs::write(&inside_file, "demo").expect("failed to create inside file");
        fs::write(&outside_file, "demo").expect("failed to create outside file");

        let records = collect_path_records(
            [
                inside_file.to_string_lossy().as_ref(),
                outside_file.to_string_lossy().as_ref(),
                "",
            ],
            Path::new(&root),
        );

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].file_name(), "kept.txt");

        let _ = fs::remove_file(&inside_file);
        let _ = fs::remove_file(&outside_file);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn should_skip_recognizes_windows_style_ignored_directories() {
        assert!(should_skip(Path::new(r"C:\repo\node_modules\pkg\index.js")));
        assert!(should_skip(Path::new(r"C:\repo\target\debug\app.exe")));
        assert!(should_skip(Path::new(r"C:\repo\.git\config")));
    }

    #[test]
    fn spotlight_candidates_respect_cached_snapshot_filtering() {
        let kept = record("/workspace/kept.txt");
        let ignored = record("/workspace/dist/ignored.txt");

        let filtered =
            filter_records_to_snapshot(vec![kept.clone(), ignored], std::slice::from_ref(&kept));

        assert_eq!(filtered, vec![kept]);
    }

    #[test]
    fn spotlight_candidates_respect_workspace_rules_without_cached_snapshot() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("wabity-file-search-spotlight-{unique}"));
        let kept_file = root.join("kept.txt");
        let ignored_dir = root.join("dist");
        let ignored_file = ignored_dir.join("ignored.txt");

        fs::create_dir_all(&ignored_dir).expect("failed to create ignored directory");
        fs::create_dir_all(root.join(".git")).expect("failed to create git directory");
        fs::write(root.join(".gitignore"), "dist/\n").expect("failed to write gitignore");
        fs::write(&kept_file, "kept").expect("failed to write kept file");
        fs::write(&ignored_file, "ignored").expect("failed to write ignored file");

        let filtered = filter_records_with_workspace_rules(
            vec![
                record(&kept_file.to_string_lossy()),
                record(&ignored_file.to_string_lossy()),
            ],
            &root,
        );

        assert_eq!(
            filtered
                .iter()
                .map(FileRecord::file_name)
                .collect::<Vec<_>>(),
            vec!["kept.txt"]
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_file_search_requires_cached_snapshot_before_using_spotlight() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("wabity-file-search-native-index-{unique}"));
        let file = root.join("kept.txt");

        fs::create_dir_all(&root).expect("failed to create root");
        fs::write(&file, "kept").expect("failed to write file");

        let service = FileSearchService::new().expect("file search service should initialize");
        assert!(!service
            .should_use_native_index(&root)
            .expect("missing cache state should be readable"));

        let handle = service
            .workspace_index(&root)
            .expect("workspace index should build");
        drop(handle);

        assert!(service
            .should_use_native_index(&root)
            .expect("cached state should be readable"));

        let _ = fs::remove_file(&file);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_search_ready_accepts_single_non_english_letter() {
        assert!(is_file_search_ready("文"));
        assert!(is_file_search_ready("ab"));
        assert!(!is_file_search_ready("a"));
        assert!(!is_file_search_ready(""));
    }

    #[test]
    fn incremental_updates_add_and_remove_records() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("wabity-file-search-update-{unique}"));
        fs::create_dir_all(&root).expect("failed to create root");
        let initial_file = root.join("initial.txt");
        fs::write(&initial_file, "hello").expect("failed to write initial file");

        let records = Arc::new(Mutex::new(HashMap::from([(
            initial_file.to_string_lossy().into_owned(),
            record(&initial_file.to_string_lossy()),
        )])));
        let snapshot = Arc::new(RwLock::new(Arc::new(snapshot_from_records_map(
            &records.lock().expect("failed to lock records"),
        ))));

        let added_file = root.join("added.txt");
        fs::write(&added_file, "hello").expect("failed to write added file");

        apply_path_updates(
            &root,
            &records,
            &snapshot,
            &BTreeSet::new(),
            &BTreeSet::from([added_file.clone()]),
        )
        .expect("failed to apply add update");

        let added_snapshot = snapshot.read().expect("failed to read snapshot");
        assert!(added_snapshot
            .iter()
            .any(|record| record.file_name() == "added.txt"));
        drop(added_snapshot);

        fs::remove_file(&initial_file).expect("failed to remove initial file");
        apply_path_updates(
            &root,
            &records,
            &snapshot,
            &BTreeSet::from([initial_file.clone()]),
            &BTreeSet::new(),
        )
        .expect("failed to apply remove update");

        let removed_snapshot = snapshot.read().expect("failed to read snapshot");
        assert!(!removed_snapshot
            .iter()
            .any(|record| record.file_name() == "initial.txt"));

        let _ = fs::remove_file(&added_file);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn incremental_updates_preserve_gitignore_filtering_for_single_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("wabity-file-search-ignore-{unique}"));
        let ignored_dir = root.join("dist");
        let ignored_file = ignored_dir.join("ignored.txt");

        fs::create_dir_all(&ignored_dir).expect("failed to create ignored directory");
        fs::create_dir_all(root.join(".git")).expect("failed to create git directory");
        fs::write(root.join(".gitignore"), "dist/\n").expect("failed to write gitignore");
        fs::write(&ignored_file, "hello").expect("failed to write ignored file");

        let records =
            build_initial_record_map(&root).expect("initial index should respect gitignore");
        assert!(
            !records.contains_key(ignored_file.to_string_lossy().as_ref()),
            "ignored file should not appear in initial index"
        );

        let records = Arc::new(Mutex::new(records));
        let snapshot = Arc::new(RwLock::new(Arc::new(snapshot_from_records_map(
            &records.lock().expect("failed to lock records"),
        ))));

        apply_path_updates(
            &root,
            &records,
            &snapshot,
            &BTreeSet::new(),
            &BTreeSet::from([ignored_file.clone()]),
        )
        .expect("ignored file update should not fail");

        let updated_snapshot = snapshot.read().expect("failed to read snapshot");
        assert!(
            !updated_snapshot
                .iter()
                .any(|record| record.path == ignored_file.to_string_lossy()),
            "ignored file should stay excluded after incremental refresh"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn gitignore_update_rebuilds_parent_scope_and_removes_now_ignored_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let raw_root =
            std::env::temp_dir().join(format!("wabity-file-search-gitignore-add-{unique}"));
        let ignored_dir = raw_root.join("dist");

        fs::create_dir_all(&ignored_dir).expect("failed to create ignored directory");
        fs::create_dir_all(raw_root.join(".git")).expect("failed to create git directory");
        let root = raw_root
            .canonicalize()
            .expect("canonicalize workspace root");
        let ignored_file = root.join("dist").join("ignored.txt");
        fs::write(&ignored_file, "hello").expect("failed to write ignored file");

        let records = Arc::new(Mutex::new(
            build_initial_record_map(&root).expect("initial index should build without ignore"),
        ));
        let snapshot = Arc::new(RwLock::new(Arc::new(snapshot_from_records_map(
            &records.lock().expect("failed to lock records"),
        ))));
        assert!(snapshot
            .read()
            .expect("failed to read initial snapshot")
            .iter()
            .any(|record| record.path == ignored_file.to_string_lossy()));

        let gitignore_path = root.join(".gitignore");
        fs::write(&gitignore_path, "dist/\n").expect("failed to write gitignore");
        let (_, rescan_targets) = collect_update_targets(
            &root,
            vec![Ok(Event {
                kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                    notify::event::DataChange::Content,
                )),
                paths: vec![gitignore_path.clone()],
                attrs: Default::default(),
            })],
        );

        assert_eq!(rescan_targets, BTreeSet::from([root.clone()]));
        apply_path_updates(
            &root,
            &records,
            &snapshot,
            &BTreeSet::new(),
            &rescan_targets,
        )
        .expect("gitignore scope refresh should succeed");

        let updated_snapshot = snapshot.read().expect("failed to read updated snapshot");
        assert!(
            !updated_snapshot
                .iter()
                .any(|record| record.path == ignored_file.to_string_lossy()),
            "newly ignored file should be removed after gitignore refresh"
        );

        let _ = fs::remove_dir_all(&raw_root);
    }

    #[test]
    fn gitignore_removal_rebuilds_parent_scope_and_restores_unignored_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let raw_root =
            std::env::temp_dir().join(format!("wabity-file-search-gitignore-remove-{unique}"));
        let ignored_dir = raw_root.join("dist");

        fs::create_dir_all(&ignored_dir).expect("failed to create ignored directory");
        fs::create_dir_all(raw_root.join(".git")).expect("failed to create git directory");
        let root = raw_root
            .canonicalize()
            .expect("canonicalize workspace root");
        let ignored_file = root.join("dist").join("ignored.txt");
        let gitignore_path = root.join(".gitignore");
        fs::write(&gitignore_path, "dist/\n").expect("failed to write gitignore");
        fs::write(&ignored_file, "hello").expect("failed to write ignored file");

        let records = Arc::new(Mutex::new(
            build_initial_record_map(&root).expect("initial index should respect gitignore"),
        ));
        let snapshot = Arc::new(RwLock::new(Arc::new(snapshot_from_records_map(
            &records.lock().expect("failed to lock records"),
        ))));
        assert!(
            !snapshot
                .read()
                .expect("failed to read initial snapshot")
                .iter()
                .any(|record| record.path == ignored_file.to_string_lossy()),
            "ignored file should be absent before removing gitignore"
        );

        fs::remove_file(&gitignore_path).expect("failed to remove gitignore");
        let (_, rescan_targets) = collect_update_targets(
            &root,
            vec![Ok(Event {
                kind: notify::EventKind::Remove(notify::event::RemoveKind::File),
                paths: vec![gitignore_path.clone()],
                attrs: Default::default(),
            })],
        );

        assert_eq!(rescan_targets, BTreeSet::from([root.clone()]));
        apply_path_updates(
            &root,
            &records,
            &snapshot,
            &BTreeSet::new(),
            &rescan_targets,
        )
        .expect("gitignore removal refresh should succeed");

        let updated_snapshot = snapshot.read().expect("failed to read updated snapshot");
        assert!(
            updated_snapshot
                .iter()
                .any(|record| record.path == ignored_file.to_string_lossy()),
            "file should be restored after gitignore removal"
        );

        let _ = fs::remove_dir_all(&raw_root);
    }

    #[test]
    fn deleted_directory_event_with_dotdot_path_removes_nested_records() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("wabity-file-search-remove-dir-{unique}"));
        let nested_dir = root.join("nested");
        let nested_file = nested_dir.join("a.txt");

        fs::create_dir_all(&nested_dir).expect("failed to create nested directory");
        fs::write(&nested_file, "hello").expect("failed to write nested file");

        let records = Arc::new(Mutex::new(HashMap::from([(
            nested_file.to_string_lossy().into_owned(),
            record(&nested_file.to_string_lossy()),
        )])));
        let snapshot = Arc::new(RwLock::new(Arc::new(snapshot_from_records_map(
            &records.lock().expect("failed to lock records"),
        ))));

        fs::remove_dir_all(&nested_dir).expect("failed to remove nested directory");
        let (remove_targets, rescan_targets) = collect_update_targets(
            &root,
            vec![Ok(Event {
                kind: notify::EventKind::Remove(notify::event::RemoveKind::Folder),
                paths: vec![root.join("missing/../nested")],
                attrs: Default::default(),
            })],
        );

        assert!(rescan_targets.is_empty());
        assert_eq!(remove_targets, BTreeSet::from([nested_dir.clone()]));

        apply_path_updates(
            &root,
            &records,
            &snapshot,
            &remove_targets,
            &BTreeSet::new(),
        )
        .expect("directory remove update should succeed");

        let updated_snapshot = snapshot.read().expect("failed to read updated snapshot");
        assert!(
            !updated_snapshot
                .iter()
                .any(|record| record.path == nested_file.to_string_lossy()),
            "deleted directory contents should be removed after path normalization"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn eviction_skips_protected_workspace_and_uses_oldest_access() {
        let protected_root = Path::new("/protected").to_path_buf();
        let make_handle = |tick| {
            Arc::new(WorkspaceIndexHandle {
                snapshot: Arc::new(RwLock::new(Arc::new(Vec::new()))),
                last_access_tick: AtomicU64::new(tick),
                watcher: Mutex::new(None),
            })
        };

        let protected_handle = Arc::new(WorkspaceIndexHandle {
            snapshot: Arc::new(RwLock::new(Arc::new(Vec::new()))),
            last_access_tick: AtomicU64::new(0),
            watcher: Mutex::new(None),
        });
        let old_handle = make_handle(1);
        let new_handle = make_handle(9);

        let workspaces = HashMap::from([
            (protected_root.clone(), protected_handle),
            (PathBuf::from("/old"), old_handle),
            (PathBuf::from("/new"), new_handle),
        ]);

        assert_eq!(
            eviction_root(&workspaces, &protected_root),
            Some(PathBuf::from("/old"))
        );
    }

    #[test]
    fn refreshed_access_prevents_recent_workspace_from_eviction() {
        let protected_root = Path::new("/protected").to_path_buf();
        let refreshed_handle = Arc::new(WorkspaceIndexHandle {
            snapshot: Arc::new(RwLock::new(Arc::new(Vec::new()))),
            last_access_tick: AtomicU64::new(1),
            watcher: Mutex::new(None),
        });
        let stale_handle = Arc::new(WorkspaceIndexHandle {
            snapshot: Arc::new(RwLock::new(Arc::new(Vec::new()))),
            last_access_tick: AtomicU64::new(2),
            watcher: Mutex::new(None),
        });
        let protected_handle = Arc::new(WorkspaceIndexHandle {
            snapshot: Arc::new(RwLock::new(Arc::new(Vec::new()))),
            last_access_tick: AtomicU64::new(0),
            watcher: Mutex::new(None),
        });

        refreshed_handle.mark_access(9);

        let workspaces = HashMap::from([
            (protected_root.clone(), protected_handle),
            (PathBuf::from("/refreshed"), refreshed_handle),
            (PathBuf::from("/stale"), stale_handle),
        ]);

        assert_eq!(
            eviction_root(&workspaces, &protected_root),
            Some(PathBuf::from("/stale"))
        );
    }

    #[test]
    fn workspace_handle_without_watcher_is_still_cached() {
        let handle = Arc::new(WorkspaceIndexHandle {
            snapshot: Arc::new(RwLock::new(Arc::new(Vec::new()))),
            last_access_tick: AtomicU64::new(0),
            watcher: Mutex::new(None),
        });
        let workspace_root = Path::new("/cached-without-watcher");
        let mut cache = FileSearchCache::default();
        let access_tick = super::next_access_tick(&mut cache);

        cache_workspace_index(&mut cache, workspace_root, Arc::clone(&handle), access_tick);

        assert_eq!(cache.workspaces.len(), 1);
        assert!(cache.workspaces.contains_key(workspace_root));
        assert!(!handle.has_watcher());
    }

    #[test]
    fn workspace_index_reuses_cache_for_equivalent_root_paths() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("wabity-file-search-cache-root-{unique}"));
        fs::create_dir_all(&root).expect("failed to create workspace root");
        fs::write(root.join("a.txt"), "hello").expect("failed to write workspace file");

        let service = FileSearchService::new().expect("file search service should initialize");
        let canonical_root = root.canonicalize().expect("canonicalize workspace root");
        let dotted_root = canonical_root.join(".");

        let first = service
            .workspace_index(&canonical_root)
            .expect("workspace index should build");
        let second = service
            .workspace_index(&dotted_root)
            .expect("equivalent workspace root should reuse cache");

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            service
                .cache
                .read()
                .expect("failed to read file index cache")
                .workspaces
                .len(),
            1
        );

        let _ = fs::remove_dir_all(&root);
    }
}
