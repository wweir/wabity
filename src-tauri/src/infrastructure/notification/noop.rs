use anyhow::Result;

use crate::domain::notification::{CompletionNotification, NotificationPermissionState};

use super::SystemNotificationBackend;

#[derive(Clone, Copy, Default)]
pub struct NoopNotificationBackend;

impl SystemNotificationBackend for NoopNotificationBackend {
    fn permission_state(&self) -> Result<NotificationPermissionState> {
        Ok(NotificationPermissionState::Unsupported)
    }

    fn notify(&self, _payload: &CompletionNotification) -> Result<()> {
        Ok(())
    }
}
