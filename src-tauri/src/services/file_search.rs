use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
};

use anyhow::{Context, Result};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use ignore::{WalkBuilder, WalkState};

#[cfg(target_os = "macos")]
use std::process::Command;

use crate::domain::file_search::FileSearchMatch;

#[derive(Debug, Clone)]
pub struct FileSearchService {
    index: Arc<RwLock<Option<CachedIndex>>>,
}

#[derive(Debug, Clone)]
struct CachedIndex {
    root: PathBuf,
    entries: Arc<Vec<FileRecord>>,
}

#[derive(Debug, Clone)]
struct FileRecord {
    path: String,
    file_name: String,
    parent: String,
}

impl FileSearchService {
    pub fn new() -> Result<Self> {
        Ok(Self {
            index: Arc::new(RwLock::new(None)),
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

        if let Some(matches) = self.search_with_native_index(workspace_root, needle, limit)? {
            return Ok(matches);
        }

        let entries = self.index(workspace_root)?;
        Ok(rank_matches(entries.as_ref(), needle, limit))
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
                        root = %workspace_root.display(),
                        query = needle,
                        count = matches.len(),
                        "file search resolved through spotlight"
                    );
                    return Ok(Some(matches));
                }
                Ok(None) => {
                    tracing::debug!(
                        query = needle,
                        "spotlight returned no file matches, falling back"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        ?error,
                        query = needle,
                        "spotlight search failed, falling back"
                    );
                }
            }
        }

        Ok(None)
    }

    fn index(&self, workspace_root: &Path) -> Result<Arc<Vec<FileRecord>>> {
        let read_guard = self
            .index
            .read()
            .map_err(|_| anyhow::anyhow!("failed to read file index state"))?;
        if let Some(cached) = read_guard.as_ref() {
            if cached.root == workspace_root {
                return Ok(Arc::clone(&cached.entries));
            }
        }
        drop(read_guard);

        let built = self.build_index(workspace_root)?;
        let mut guard = self
            .index
            .write()
            .map_err(|_| anyhow::anyhow!("failed to write file index state"))?;

        if let Some(cached) = guard.as_ref() {
            if cached.root == workspace_root {
                return Ok(Arc::clone(&cached.entries));
            }
        }

        *guard = Some(CachedIndex {
            root: workspace_root.to_path_buf(),
            entries: Arc::clone(&built),
        });
        Ok(built)
    }

    fn build_index(&self, workspace_root: &Path) -> Result<Arc<Vec<FileRecord>>> {
        tracing::info!(root = %workspace_root.display(), "building workspace file index");

        let files = Mutex::new(Vec::new());
        let walker = WalkBuilder::new(workspace_root)
            .hidden(false)
            .ignore(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .parents(true)
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

                let path_string = path.to_string_lossy().into_owned();
                let file_name = path
                    .file_name()
                    .unwrap_or_else(|| OsStr::new(""))
                    .to_string_lossy()
                    .into_owned();
                let parent = path
                    .parent()
                    .map(|parent| parent.to_string_lossy().into_owned())
                    .unwrap_or_default();

                if let Ok(mut guard) = files.lock() {
                    guard.push(FileRecord {
                        path: path_string,
                        file_name,
                        parent,
                    });
                }

                WalkState::Continue
            })
        });

        let files = files
            .into_inner()
            .map_err(|_| anyhow::anyhow!("file index mutex poisoned"))?;

        tracing::info!(
            root = %workspace_root.display(),
            count = files.len(),
            "workspace file index ready"
        );
        Ok(Arc::new(files))
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
            .with_context(|| format!("failed to spawn mdfind for query `{needle}`"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("mdfind exited with status {}: {stderr}", output.status);
        }

        let stdout = String::from_utf8(output.stdout).context("mdfind returned non-utf8 output")?;
        let candidates = collect_path_records(stdout.lines().map(str::trim), workspace_root);

        if candidates.is_empty() {
            return Ok(None);
        }

        Ok(Some(rank_matches(&candidates, needle, limit)))
    }
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
                    file_name: entry.file_name.clone(),
                    parent: entry.parent.clone(),
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
        .fuzzy_match(&entry.file_name, needle)
        .map(|score| score.saturating_add(100));

    match (file_name_score, path_score) {
        (Some(file_name_score), Some(path_score)) => Some(file_name_score.max(path_score)),
        (Some(file_name_score), None) => Some(file_name_score),
        (None, Some(path_score)) => Some(path_score),
        (None, None) => None,
    }
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
    let path_string = path.to_string_lossy();

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

        records.push(FileRecord {
            path: path.to_string_lossy().into_owned(),
            file_name: path
                .file_name()
                .unwrap_or_else(|| OsStr::new(""))
                .to_string_lossy()
                .into_owned(),
            parent: path
                .parent()
                .map(|parent| parent.to_string_lossy().into_owned())
                .unwrap_or_default(),
        });
    }

    records
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{collect_path_records, is_file_search_ready, rank_matches, FileRecord};

    #[test]
    fn fuzzy_search_prefers_close_filename_match() {
        let entries = vec![
            FileRecord {
                path: "/Users/demo/Documents/wabity-plan.md".to_string(),
                file_name: "wabity-plan.md".to_string(),
                parent: "/Users/demo/Documents".to_string(),
            },
            FileRecord {
                path: "/Users/demo/Desktop/random-note.txt".to_string(),
                file_name: "random-note.txt".to_string(),
                parent: "/Users/demo/Desktop".to_string(),
            },
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
            FileRecord {
                path: "/Users/demo/Documents/aaA.md".to_string(),
                file_name: "aaA.md".to_string(),
                parent: "/Users/demo/Documents".to_string(),
            },
            FileRecord {
                path: "/Users/demo/Documents/alpha.md".to_string(),
                file_name: "alpha.md".to_string(),
                parent: "/Users/demo/Documents".to_string(),
            },
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
        assert_eq!(records[0].file_name, "kept.txt");

        let _ = fs::remove_file(&inside_file);
        let _ = fs::remove_file(&outside_file);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_search_ready_accepts_single_non_english_letter() {
        assert!(is_file_search_ready("文"));
        assert!(is_file_search_ready("ab"));
        assert!(!is_file_search_ready("a"));
        assert!(!is_file_search_ready(""));
    }
}
