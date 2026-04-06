use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use std::process::Command;

use anyhow::{bail, Result};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System};

use crate::domain::{
    execution::{ExecutionResult, ExecutionStatus},
    process::{RunningProcessKind, RunningProcessMatch},
};

const KILL_COMMAND_ALIASES: [&str; 1] = ["/kill"];

#[derive(Debug, Clone, Default)]
pub struct ProcessService;

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
        Self
    }

    pub fn search_running(&self, query: &str, limit: usize) -> Result<Vec<RunningProcessMatch>> {
        let needle = query.trim();
        if needle.is_empty() {
            return Ok(Vec::new());
        }

        let current_pid = current_pid_u32()?;
        let processes = collect_process_records(current_pid);
        Ok(rank_processes(&processes, needle, limit))
    }

    pub fn kill_action(&self, raw_text: &str) -> Result<ExecutionResult> {
        let target_text = extract_kill_payload(raw_text).unwrap_or(raw_text).trim();
        if target_text.is_empty() {
            return Ok(warning_result(
                None,
                "请输入要终止的应用名称、进程名称或 pid:<id>",
            ));
        }

        let current_pid = current_pid_u32()?;
        let processes = collect_process_records(current_pid);
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
}

fn current_pid_u32() -> Result<u32> {
    Ok(sysinfo::get_current_pid()
        .map_err(|error| anyhow::anyhow!("failed to resolve current process id: {error}"))?
        .as_u32())
}

fn collect_process_records(current_pid: u32) -> Vec<ProcessRecord> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::everything(),
    );

    let mut processes = system
        .processes()
        .values()
        .filter_map(|process| process_record_from_sysinfo(process, current_pid))
        .collect::<Vec<_>>();
    processes.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.pid.cmp(&right.pid))
    });
    processes
}

fn process_record_from_sysinfo(
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
        .and_then(|path| localized_app_name(Path::new(path)))
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        bundle_display_name, extract_app_bundle_path, normalize_display_name, rank_processes,
        resolve_kill_target, KillTargetResolution, ProcessRecord,
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
}
