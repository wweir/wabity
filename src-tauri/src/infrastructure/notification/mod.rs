use std::sync::Arc;

use anyhow::Result;
use tauri::AppHandle;

use crate::domain::notification::{CompletionNotification, NotificationPermissionState};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod noop;

pub trait SystemNotificationBackend: Send + Sync {
    fn permission_state(&self) -> Result<NotificationPermissionState>;
    fn notify(&self, payload: &CompletionNotification) -> Result<()>;
}

pub fn create_system_notification_backend(
    app_handle: AppHandle,
) -> Arc<dyn SystemNotificationBackend> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::MacOsSystemNotificationBackend::new(app_handle))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app_handle;
        Arc::new(noop::NoopNotificationBackend)
    }
}
