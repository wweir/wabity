use anyhow::{Context, Result};
use tauri::AppHandle;

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
use tauri_plugin_autostart::ManagerExt as _;

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
pub fn sync_autostart(app: &AppHandle, enabled: bool) -> Result<()> {
    let manager = app.autolaunch();
    let is_enabled = manager
        .is_enabled()
        .context("failed to read autostart status")?;

    if is_enabled == enabled {
        return Ok(());
    }

    if enabled {
        manager.enable().context("failed to enable autostart")?;
    } else {
        manager.disable().context("failed to disable autostart")?;
    }

    Ok(())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub fn sync_autostart(_app: &AppHandle, enabled: bool) -> Result<()> {
    if enabled {
        anyhow::bail!("autostart is not supported on this target");
    }

    Ok(())
}
