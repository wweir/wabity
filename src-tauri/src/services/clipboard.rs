use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tauri::{AppHandle, Emitter};
use tokio::{sync::Mutex, time::sleep};

use crate::{
    domain::clipboard::{
        ClipboardHistoryEntry, ClipboardHistorySnapshot, MAX_PINNED_CLIPBOARD_ENTRIES,
        MAX_RECENT_CLIPBOARD_ENTRIES,
    },
    infrastructure::clipboard::{
        current_clipboard_change_count, read_clipboard_text, send_paste_shortcut,
        write_clipboard_text, ClipboardHistoryStore,
    },
};

const CLIPBOARD_HISTORY_UPDATED_EVENT: &str = "clipboard-history-updated";
const CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(400);
const SUPPRESSED_CLIPBOARD_WRITE_TTL: Duration = Duration::from_secs(2);

#[derive(Debug)]
struct SuppressedClipboardWrite {
    text_hash: String,
    expires_at: Instant,
}

#[derive(Debug)]
struct ClipboardRuntimeState {
    entries: Vec<ClipboardHistoryEntry>,
    last_change_count: Option<isize>,
    last_polled_text_hash: Option<String>,
    suppressed_write: Option<SuppressedClipboardWrite>,
}

#[derive(Clone)]
pub struct ClipboardService {
    app_handle: AppHandle,
    store: std::sync::Arc<ClipboardHistoryStore>,
    state: std::sync::Arc<Mutex<ClipboardRuntimeState>>,
}

impl ClipboardService {
    pub async fn new(app_handle: AppHandle) -> Result<Self> {
        let store = std::sync::Arc::new(ClipboardHistoryStore::new()?);
        let entries = sanitize_entries(store.load_entries().await?);

        Ok(Self {
            app_handle,
            store,
            state: std::sync::Arc::new(Mutex::new(ClipboardRuntimeState {
                entries,
                last_change_count: None,
                last_polled_text_hash: None,
                suppressed_write: None,
            })),
        })
    }

    pub fn start(&self) {
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                if let Err(error) = service.poll_once().await {
                    tracing::warn!(?error, "failed to poll clipboard history");
                }
                sleep(CLIPBOARD_POLL_INTERVAL).await;
            }
        });
    }

    pub async fn snapshot(&self) -> ClipboardHistorySnapshot {
        let state = self.state.lock().await;
        build_snapshot(&state.entries)
    }

    pub async fn toggle_pin(&self, entry_id: &str) -> Result<ClipboardHistorySnapshot> {
        let mut state = self.state.lock().await;
        let entry_index = state
            .entries
            .iter()
            .position(|entry| entry.id == entry_id)
            .ok_or_else(|| anyhow::anyhow!("clipboard history entry not found: {entry_id}"))?;
        let now_ms = now_unix_ms();

        if state.entries[entry_index].pinned {
            state.entries[entry_index].pinned = false;
            state.entries[entry_index].pinned_at_ms = None;
        } else {
            let pinned_count = state.entries.iter().filter(|entry| entry.pinned).count();
            if pinned_count >= MAX_PINNED_CLIPBOARD_ENTRIES {
                bail!("最多只能固定 {MAX_PINNED_CLIPBOARD_ENTRIES} 条常用剪贴板");
            }

            state.entries[entry_index].pinned = true;
            state.entries[entry_index].pinned_at_ms = Some(now_ms);
        }

        prune_recent_entries(&mut state.entries);
        let snapshot = build_snapshot(&state.entries);
        self.persist_and_emit_locked(&state.entries, &snapshot)
            .await?;
        Ok(snapshot)
    }

    pub async fn delete_entry(&self, entry_id: &str) -> Result<ClipboardHistorySnapshot> {
        let mut state = self.state.lock().await;
        let original_len = state.entries.len();
        state.entries.retain(|entry| entry.id != entry_id);
        if state.entries.len() == original_len {
            bail!("clipboard history entry not found: {entry_id}");
        }

        let snapshot = build_snapshot(&state.entries);
        self.persist_and_emit_locked(&state.entries, &snapshot)
            .await?;
        Ok(snapshot)
    }

    pub async fn prepare_entry_for_external_paste(
        &self,
        entry_id: &str,
    ) -> Result<ClipboardHistorySnapshot> {
        let mut state = self.state.lock().await;
        let now_ms = now_unix_ms();
        let entry = state
            .entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("clipboard history entry not found: {entry_id}"))?;

        write_clipboard_text(&entry.text).context("failed to copy clipboard history entry")?;
        state.suppressed_write = Some(SuppressedClipboardWrite {
            text_hash: text_hash(&entry.text),
            expires_at: Instant::now() + SUPPRESSED_CLIPBOARD_WRITE_TTL,
        });
        mark_entry_seen(&mut state.entries, &entry.text, now_ms);
        let snapshot = build_snapshot(&state.entries);
        self.persist_and_emit_locked(&state.entries, &snapshot)
            .await?;
        Ok(snapshot)
    }

    pub async fn send_paste_shortcut(&self) -> Result<()> {
        tokio::task::spawn_blocking(send_paste_shortcut)
            .await
            .context("failed to join paste shortcut task")?
    }

    async fn poll_once(&self) -> Result<()> {
        let change_count = current_clipboard_change_count();
        let clipboard_text = read_clipboard_text()?;
        let mut state = self.state.lock().await;

        if change_count.is_some() && state.last_change_count == change_count {
            return Ok(());
        }
        if let Some(change_count) = change_count {
            state.last_change_count = Some(change_count);
        }

        let Some(clipboard_text) = normalize_clipboard_text(clipboard_text) else {
            state.last_polled_text_hash = None;
            return Ok(());
        };
        let clipboard_text_hash = text_hash(&clipboard_text);

        if change_count.is_none()
            && state.last_polled_text_hash.as_deref() == Some(&clipboard_text_hash)
        {
            return Ok(());
        }
        state.last_polled_text_hash = Some(clipboard_text_hash.clone());

        if should_ignore_suppressed_write(&mut state.suppressed_write, &clipboard_text_hash) {
            return Ok(());
        }

        let updated = record_observed_text(&mut state.entries, &clipboard_text, now_unix_ms());
        if !updated {
            return Ok(());
        }

        let snapshot = build_snapshot(&state.entries);
        self.persist_and_emit_locked(&state.entries, &snapshot)
            .await
    }

    async fn persist_and_emit_locked(
        &self,
        entries: &[ClipboardHistoryEntry],
        snapshot: &ClipboardHistorySnapshot,
    ) -> Result<()> {
        self.store.save_entries(entries).await?;
        self.app_handle
            .emit(CLIPBOARD_HISTORY_UPDATED_EVENT, snapshot.clone())
            .context("failed to emit clipboard history update event")
    }
}

fn sanitize_entries(entries: Vec<ClipboardHistoryEntry>) -> Vec<ClipboardHistoryEntry> {
    let mut deduped_entries = Vec::new();

    for mut entry in entries {
        if entry.text.trim().is_empty() {
            continue;
        }
        entry.id = text_hash(&entry.text);
        if entry.pinned && entry.pinned_at_ms.is_none() {
            entry.pinned_at_ms = Some(entry.last_seen_at_ms);
        }
        if deduped_entries
            .iter()
            .any(|current: &ClipboardHistoryEntry| current.id == entry.id)
        {
            continue;
        }
        deduped_entries.push(entry);
    }

    if deduped_entries.iter().filter(|entry| entry.pinned).count() > MAX_PINNED_CLIPBOARD_ENTRIES {
        let mut pinned_entries = deduped_entries
            .iter()
            .filter(|entry| entry.pinned)
            .cloned()
            .collect::<Vec<_>>();
        pinned_entries.sort_by(|left, right| {
            right
                .pinned_at_ms
                .unwrap_or(right.last_seen_at_ms)
                .cmp(&left.pinned_at_ms.unwrap_or(left.last_seen_at_ms))
        });
        let allowed_pinned_ids = pinned_entries
            .into_iter()
            .take(MAX_PINNED_CLIPBOARD_ENTRIES)
            .map(|entry| entry.id)
            .collect::<std::collections::HashSet<_>>();
        for entry in &mut deduped_entries {
            if entry.pinned && !allowed_pinned_ids.contains(&entry.id) {
                entry.pinned = false;
                entry.pinned_at_ms = None;
            }
        }
    }

    prune_recent_entries(&mut deduped_entries);
    deduped_entries
}

fn build_snapshot(entries: &[ClipboardHistoryEntry]) -> ClipboardHistorySnapshot {
    let mut pinned_entries = entries
        .iter()
        .filter(|entry| entry.pinned)
        .cloned()
        .collect::<Vec<_>>();
    pinned_entries.sort_by(|left, right| {
        right
            .pinned_at_ms
            .unwrap_or(right.last_seen_at_ms)
            .cmp(&left.pinned_at_ms.unwrap_or(left.last_seen_at_ms))
            .then_with(|| right.last_seen_at_ms.cmp(&left.last_seen_at_ms))
    });

    let mut recent_entries = entries
        .iter()
        .filter(|entry| !entry.pinned)
        .cloned()
        .collect::<Vec<_>>();
    recent_entries.sort_by(|left, right| right.last_seen_at_ms.cmp(&left.last_seen_at_ms));
    recent_entries.truncate(MAX_RECENT_CLIPBOARD_ENTRIES);

    ClipboardHistorySnapshot {
        pinned_entries,
        recent_entries,
    }
}

fn normalize_clipboard_text(text: Option<String>) -> Option<String> {
    let text = text?;
    if text.trim().is_empty() {
        return None;
    }

    Some(text)
}

fn mark_entry_seen(entries: &mut Vec<ClipboardHistoryEntry>, text: &str, now_ms: u64) {
    if let Some(entry) = entries.iter_mut().find(|entry| entry.text == text) {
        entry.last_seen_at_ms = now_ms;
        return;
    }

    entries.push(ClipboardHistoryEntry {
        id: text_hash(text),
        text: text.to_string(),
        pinned: false,
        last_seen_at_ms: now_ms,
        pinned_at_ms: None,
    });
    prune_recent_entries(entries);
}

fn record_observed_text(entries: &mut Vec<ClipboardHistoryEntry>, text: &str, now_ms: u64) -> bool {
    if let Some(entry) = entries.iter_mut().find(|entry| entry.text == text) {
        if entry.last_seen_at_ms == now_ms {
            return false;
        }
        entry.last_seen_at_ms = now_ms;
        return true;
    }

    entries.push(ClipboardHistoryEntry {
        id: text_hash(text),
        text: text.to_string(),
        pinned: false,
        last_seen_at_ms: now_ms,
        pinned_at_ms: None,
    });
    prune_recent_entries(entries);
    true
}

fn prune_recent_entries(entries: &mut Vec<ClipboardHistoryEntry>) {
    let mut recent_entries = entries
        .iter()
        .filter(|entry| !entry.pinned)
        .cloned()
        .collect::<Vec<_>>();
    recent_entries.sort_by(|left, right| right.last_seen_at_ms.cmp(&left.last_seen_at_ms));
    recent_entries.truncate(MAX_RECENT_CLIPBOARD_ENTRIES);
    let allowed_recent_ids = recent_entries
        .into_iter()
        .map(|entry| entry.id)
        .collect::<std::collections::HashSet<_>>();

    entries.retain(|entry| entry.pinned || allowed_recent_ids.contains(&entry.id));
}

fn should_ignore_suppressed_write(
    suppressed_write: &mut Option<SuppressedClipboardWrite>,
    text_hash: &str,
) -> bool {
    let Some(current) = suppressed_write.as_ref() else {
        return false;
    };
    if Instant::now() > current.expires_at {
        *suppressed_write = None;
        return false;
    }
    if current.text_hash != text_hash {
        return false;
    }

    *suppressed_write = None;
    true
}

fn text_hash(text: &str) -> String {
    format!("{:x}", md5::compute(text.as_bytes()))
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::{
        build_snapshot, normalize_clipboard_text, prune_recent_entries, record_observed_text,
        sanitize_entries, ClipboardHistoryEntry, MAX_PINNED_CLIPBOARD_ENTRIES,
        MAX_RECENT_CLIPBOARD_ENTRIES,
    };

    fn entry(
        id: &str,
        text: &str,
        pinned: bool,
        last_seen_at_ms: u64,
        pinned_at_ms: Option<u64>,
    ) -> ClipboardHistoryEntry {
        ClipboardHistoryEntry {
            id: id.to_string(),
            text: text.to_string(),
            pinned,
            last_seen_at_ms,
            pinned_at_ms,
        }
    }

    #[test]
    fn ignores_blank_clipboard_text() {
        assert_eq!(normalize_clipboard_text(Some("   ".to_string())), None);
        assert_eq!(
            normalize_clipboard_text(Some("hello".to_string())),
            Some("hello".to_string())
        );
    }

    #[test]
    fn keeps_only_latest_recent_entries() {
        let mut entries = (0..(MAX_RECENT_CLIPBOARD_ENTRIES + 2))
            .map(|index| {
                entry(
                    &format!("recent-{index}"),
                    &format!("recent-{index}"),
                    false,
                    index as u64,
                    None,
                )
            })
            .collect::<Vec<_>>();

        prune_recent_entries(&mut entries);

        assert_eq!(entries.len(), MAX_RECENT_CLIPBOARD_ENTRIES);
        assert!(entries.iter().all(|entry| entry.text != "recent-0"));
        assert!(entries.iter().all(|entry| entry.text != "recent-1"));
    }

    #[test]
    fn snapshot_splits_pinned_and_recent_without_duplicates() {
        let snapshot = build_snapshot(&[
            entry("a", "alpha", true, 20, Some(10)),
            entry("b", "beta", false, 30, None),
            entry("c", "charlie", false, 10, None),
        ]);

        assert_eq!(snapshot.pinned_entries.len(), 1);
        assert_eq!(snapshot.recent_entries.len(), 2);
        assert_eq!(snapshot.pinned_entries[0].text, "alpha");
        assert_eq!(snapshot.recent_entries[0].text, "beta");
    }

    #[test]
    fn recording_existing_text_refreshes_last_seen_instead_of_duplication() {
        let mut entries = vec![entry("alpha", "alpha", false, 10, None)];

        let updated = record_observed_text(&mut entries, "alpha", 20);

        assert!(updated);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].last_seen_at_ms, 20);
    }

    #[test]
    fn sanitize_entries_drops_blank_and_excess_recent_entries() {
        let sanitized = sanitize_entries(
            (0..(MAX_RECENT_CLIPBOARD_ENTRIES + MAX_PINNED_CLIPBOARD_ENTRIES + 3))
                .map(|index| {
                    let pinned = index < (MAX_PINNED_CLIPBOARD_ENTRIES + 1);
                    let text = format!("text-{index}");
                    entry(
                        &format!("entry-{index}"),
                        if index == 2 { "   " } else { &text },
                        pinned,
                        index as u64,
                        pinned.then_some(index as u64),
                    )
                })
                .collect(),
        );

        let pinned_count = sanitized.iter().filter(|entry| entry.pinned).count();
        let recent_count = sanitized.iter().filter(|entry| !entry.pinned).count();

        assert_eq!(pinned_count, MAX_PINNED_CLIPBOARD_ENTRIES);
        assert_eq!(recent_count, MAX_RECENT_CLIPBOARD_ENTRIES);
        assert!(sanitized.iter().all(|entry| !entry.text.trim().is_empty()));
    }
}
