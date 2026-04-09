use std::{
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

use anyhow::{Context, Result};
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard};
#[cfg(target_os = "macos")]
use objc2_app_kit::NSPasteboard;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{
    domain::clipboard::ClipboardHistoryEntry,
    infrastructure::config::{safe_write, ConfigStore},
};

const CLIPBOARD_HISTORY_FILE_NAME: &str = "clipboard-history.toml";
#[cfg(target_os = "macos")]
const MACOS_ANSI_V_KEYCODE: u16 = 0x09;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct StoredClipboardHistory {
    entries: Vec<ClipboardHistoryEntry>,
}

pub struct ClipboardHistoryStore {
    history_path: PathBuf,
    cached_history: Mutex<Option<Vec<ClipboardHistoryEntry>>>,
}

impl ClipboardHistoryStore {
    fn cached_history_guard(&self) -> MutexGuard<'_, Option<Vec<ClipboardHistoryEntry>>> {
        match self.cached_history.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("clipboard history cache lock poisoned; recovering cached state");
                poisoned.into_inner()
            }
        }
    }

    pub fn new() -> Result<Self> {
        let config_dir = ConfigStore::config_dir()?;
        std::fs::create_dir_all(&config_dir).with_context(|| {
            format!(
                "failed to create clipboard history config directory: {}",
                config_dir.display()
            )
        })?;

        Ok(Self {
            history_path: config_dir.join(CLIPBOARD_HISTORY_FILE_NAME),
            cached_history: Mutex::new(None),
        })
    }

    pub async fn load_entries(&self) -> Result<Vec<ClipboardHistoryEntry>> {
        {
            let cached_history = self.cached_history_guard();
            if let Some(entries) = cached_history.as_ref() {
                return Ok(entries.clone());
            }
        }

        if !self.history_path.exists() {
            self.save_entries(&[]).await?;
            return Ok(Vec::new());
        }

        let content = tokio::fs::read_to_string(&self.history_path)
            .await
            .with_context(|| {
                format!(
                    "failed to read clipboard history file: {}",
                    self.history_path.display()
                )
            })?;
        let parsed: StoredClipboardHistory =
            toml::from_str(&content).context("failed to parse clipboard history content")?;

        let mut cached_history = self.cached_history_guard();
        *cached_history = Some(parsed.entries.clone());
        Ok(parsed.entries)
    }

    pub async fn save_entries(&self, entries: &[ClipboardHistoryEntry]) -> Result<()> {
        let content = toml::to_string_pretty(&StoredClipboardHistory {
            entries: entries.to_vec(),
        })
        .context("failed to serialize clipboard history content")?;
        safe_write(&self.history_path, &content).await?;

        let mut cached_history = self.cached_history_guard();
        *cached_history = Some(entries.to_vec());
        Ok(())
    }
}

pub fn read_clipboard_text() -> Result<Option<String>> {
    let mut clipboard = Clipboard::new().context("failed to access clipboard")?;
    Ok(clipboard.get_text().ok())
}

pub fn write_clipboard_text(text: &str) -> Result<()> {
    let mut clipboard = Clipboard::new().context("failed to access clipboard")?;
    clipboard
        .set_text(text.to_string())
        .context("failed to write clipboard text")
}

pub fn send_paste_shortcut() -> Result<()> {
    let mut enigo =
        Enigo::new(&enigo::Settings::default()).context("failed to create enigo instance")?;

    #[cfg(target_os = "macos")]
    {
        enigo.key(Key::Meta, Direction::Press)?;
        enigo.raw(MACOS_ANSI_V_KEYCODE, Direction::Click)?;
        enigo.key(Key::Meta, Direction::Release)?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        enigo.key(Key::Control, Direction::Press)?;
        enigo.key(Key::Unicode('v'), Direction::Press)?;
        enigo.key(Key::Unicode('v'), Direction::Release)?;
        enigo.key(Key::Control, Direction::Release)?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
pub fn current_clipboard_change_count() -> Option<isize> {
    Some(NSPasteboard::generalPasteboard().changeCount())
}

#[cfg(not(target_os = "macos"))]
pub fn current_clipboard_change_count() -> Option<isize> {
    None
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::ClipboardHistoryStore;

    #[tokio::test]
    async fn load_entries_recovers_from_poisoned_cache_lock() {
        let history_path = unique_temp_path("clipboard-history");
        let store = ClipboardHistoryStore {
            history_path,
            cached_history: Mutex::new(Some(Vec::new())),
        };

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = store
                .cached_history
                .lock()
                .expect("cache lock should succeed");
            panic!("poison clipboard history cache lock");
        }));

        let entries = store
            .load_entries()
            .await
            .expect("poisoned clipboard cache lock should recover");
        assert!(entries.is_empty());
    }

    fn unique_temp_path(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}.toml"))
    }
}
