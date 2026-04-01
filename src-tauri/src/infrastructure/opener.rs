use std::path::Path;

use anyhow::{Context, Result};

pub fn open_url(url: &str) -> Result<()> {
    tauri_plugin_opener::open_url(url, None::<&str>)
        .with_context(|| format!("failed to open url with system opener: {url}"))
}

pub fn open_path(path: &Path) -> Result<()> {
    tauri_plugin_opener::open_path(path, None::<&str>)
        .with_context(|| format!("failed to open path with system opener: {}", path.display()))
}
