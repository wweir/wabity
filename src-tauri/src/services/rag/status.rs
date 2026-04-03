use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use tauri::AppHandle;
use tokio::sync::RwLock as AsyncRwLock;

use crate::domain::rag::{RagRuntimePhase, RagRuntimeStatus};
use crate::infrastructure::window;

use super::{config::now_unix_ms, model::RuntimeProgress};

pub(super) async fn set_runtime_status(
    app_handle: Option<&AppHandle>,
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_guard: Option<(&Arc<AtomicU64>, u64)>,
    phase: RagRuntimePhase,
    progress: RuntimeProgress,
    last_error: Option<String>,
) {
    if let Some((runtime_generation, generation)) = runtime_guard {
        if runtime_generation.load(Ordering::SeqCst) != generation {
            return;
        }
    }

    let mut status = runtime_status.write().await;
    if let Some((runtime_generation, generation)) = runtime_guard {
        if runtime_generation.load(Ordering::SeqCst) != generation {
            return;
        }
    }

    *status = RagRuntimeStatus {
        phase,
        scanned_file_count: progress.scanned_file_count,
        completed_file_count: progress.completed_file_count,
        total_file_count: progress.total_file_count,
        pending_file_count: progress.pending_file_count,
        last_error,
        updated_at_ms: u64::try_from(now_unix_ms()).unwrap_or_default(),
    };
    if let Some(app_handle) = app_handle {
        if let Err(error) = window::emit_rag_runtime_status(app_handle, &status) {
            tracing::debug!(?error, "failed to emit RAG runtime status event");
        }
    }
}

pub(super) async fn set_runtime_status_for_generation(
    app_handle: Option<&AppHandle>,
    runtime_status: &Arc<AsyncRwLock<RagRuntimeStatus>>,
    runtime_generation: &Arc<AtomicU64>,
    generation: u64,
    phase: RagRuntimePhase,
    progress: RuntimeProgress,
    last_error: Option<String>,
) {
    set_runtime_status(
        app_handle,
        runtime_status,
        Some((runtime_generation, generation)),
        phase,
        progress,
        last_error,
    )
    .await;
}
