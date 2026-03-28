use std::sync::OnceLock;

use anyhow::Result;
use mac_notification_sys::{set_application, Notification};
use tauri::AppHandle;
use tauri_plugin_notification::{NotificationExt, PermissionState};

use crate::domain::notification::{CompletionNotification, NotificationPermissionState};

use super::SystemNotificationBackend;

static NOTIFICATION_APPLICATION_INIT: OnceLock<()> = OnceLock::new();

#[derive(Clone)]
pub struct MacOsSystemNotificationBackend {
    app_handle: AppHandle,
}

impl MacOsSystemNotificationBackend {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }

    fn ensure_notification_application(&self) {
        let bundle_identifier = self.app_handle.config().identifier.clone();
        NOTIFICATION_APPLICATION_INIT.get_or_init(|| {
            if let Err(error) = set_application(bundle_identifier.as_str()) {
                tracing::warn!(
                    ?error,
                    bundle_identifier,
                    "failed to bind macOS notification sender to Wabity bundle identifier; falling back to default notification sender"
                );
            }
        });
    }
}

impl SystemNotificationBackend for MacOsSystemNotificationBackend {
    fn permission_state(&self) -> Result<NotificationPermissionState> {
        let permission = self.app_handle.notification().permission_state()?;
        Ok(map_permission_state(permission))
    }

    fn notify(&self, payload: &CompletionNotification) -> Result<()> {
        self.ensure_notification_application();

        let mut notification = Notification::new();
        notification.title(&payload.title).message(&payload.body);
        notification.send()?;
        Ok(())
    }
}

fn map_permission_state(permission: PermissionState) -> NotificationPermissionState {
    match permission {
        PermissionState::Granted => NotificationPermissionState::Granted,
        PermissionState::Denied => NotificationPermissionState::Denied,
        PermissionState::Prompt => NotificationPermissionState::Prompt,
        PermissionState::PromptWithRationale => NotificationPermissionState::PromptWithRationale,
    }
}
