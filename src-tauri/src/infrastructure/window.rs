use anyhow::{Context, Result};
#[cfg(target_os = "macos")]
use std::sync::mpsc;
use std::time::Duration;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow, WindowEvent,
};
use tokio::time::sleep;

#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSRunningApplication, NSWindowOcclusionState,
    NSWorkspace,
};

use crate::{
    domain::{execution::ExecutionResult, rag::RagRuntimeStatus},
    services::application::APPLICATION_CACHE_STALE_AFTER,
    state::{AppState, LauncherWindowSize, LauncherWindowViewMode, ShortcutRuntimeState},
};

const OCR_ERROR_EVENT: &str = "ocr-error";
const OCR_TRANSLATION_STARTED_EVENT: &str = "ocr-translation-started";
const OCR_TRANSLATION_STREAM_EVENT: &str = "ocr-translation-stream";
const OCR_TRANSLATION_RESULT_EVENT: &str = "ocr-translation-result";
const RAG_RUNTIME_STATUS_EVENT: &str = "rag-runtime-status";
const OPEN_CLIPBOARD_HISTORY_PANEL_EVENT: &str = "open-clipboard-history-panel";
const REVEAL_LAUNCHER_MAIN_PANEL_EVENT: &str = "reveal-launcher-main-panel";
const INSERT_CLIPBOARD_HISTORY_TEXT_INTO_LAUNCHER_EVENT: &str =
    "insert-clipboard-history-text-into-launcher";
const MAIN_WINDOW_LABEL: &str = "main";
const CLIPBOARD_WINDOW_LABEL: &str = "clipboard";
const CLIPBOARD_WINDOW_CURSOR_OFFSET_X: f64 = 12.0;
const CLIPBOARD_WINDOW_CURSOR_OFFSET_Y: f64 = 16.0;
const LAUNCHER_VERTICAL_CENTER_RATIO: f64 = 0.382;
const LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD: Duration = Duration::from_millis(250);
const LAUNCHER_SHOW_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD: Duration = Duration::from_millis(400);
const LAUNCHER_BLUR_AUTO_HIDE_CONFIRMATION_PERIOD: Duration = Duration::from_millis(180);
#[cfg(target_os = "macos")]
const LAUNCHER_BLUR_DISABLED_VISIBILITY_REINFORCEMENT_DELAY: Duration = Duration::from_millis(120);
#[cfg(target_os = "macos")]
const MACOS_LAUNCHER_PANEL_LEVEL: PanelLevel = PanelLevel::Status;
const LAUNCHER_VISIBLE_RESIZE_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD: Duration =
    Duration::from_millis(500);
const MIN_WINDOW_DIMENSION: f64 = 1.0;
const DEFAULT_MAIN_WINDOW_WIDTH: f64 = 768.0;
const DEFAULT_MAIN_WINDOW_HEIGHT: f64 = 280.0;
const DEFAULT_CLIPBOARD_WINDOW_WIDTH: f64 = 452.0;
const DEFAULT_CLIPBOARD_WINDOW_HEIGHT: f64 = 408.0;

#[cfg(target_os = "macos")]
use tauri_nspanel::{
    tauri_panel, CollectionBehavior, ManagerExt as PanelManagerExt, PanelHandle, PanelLevel,
    StyleMask, WebviewWindowExt,
};

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(LauncherPanel {
        config: {
            can_become_key_window: true,
            can_become_main_window: false,
            is_floating_panel: true
        }
    })
}

#[derive(Clone, Copy)]
struct WindowWorkArea {
    scale_factor: f64,
    position: LogicalPosition<f64>,
    size: LogicalSize<f64>,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug)]
struct MacOsPanelState {
    visible: bool,
    key_window: bool,
    occlusion_visible: bool,
    app_active: bool,
}

#[cfg(target_os = "macos")]
fn macos_panel_visibility_reinforcement_reason(
    panel_state: MacOsPanelState,
) -> Option<&'static str> {
    if !panel_state.visible {
        return Some("panel_not_visible");
    }

    if !panel_state.occlusion_visible {
        return Some("panel_not_occluded_visible");
    }

    None
}

#[cfg(target_os = "macos")]
fn macos_panel_toggle_reveal_reason(
    launcher_focused: bool,
    panel_state: Option<MacOsPanelState>,
) -> Option<&'static str> {
    if !launcher_focused {
        return Some("tauri_window_not_focused");
    }

    let panel_state = panel_state?;
    if !panel_state.visible {
        return Some("panel_not_visible");
    }

    if !panel_state.occlusion_visible {
        return Some("panel_not_occluded_visible");
    }

    if !panel_state.key_window {
        return Some("panel_not_key_window");
    }

    if !panel_state.app_active {
        return Some("app_inactive");
    }

    None
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OcrTranslationStartedPayload {
    source_mode: ShortcutTranslationSourceMode,
    source_text: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OcrTranslationResultPayload {
    source_mode: ShortcutTranslationSourceMode,
    source_text: String,
    result: ExecutionResult,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OcrTranslationStreamPayload {
    source_mode: ShortcutTranslationSourceMode,
    source_text: String,
    partial_text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardHistorySelectionMode {
    InsertIntoLauncher,
    PasteExternally,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenClipboardHistoryPanelPayload {
    selection_mode: ClipboardHistorySelectionMode,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RevealLauncherMainPanelPayload;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InsertClipboardHistoryTextIntoLauncherPayload {
    text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutTranslationSourceMode {
    Ocr,
    Selection,
}

#[cfg(target_os = "macos")]
fn frontmost_application_pid() -> Option<i32> {
    let workspace = NSWorkspace::sharedWorkspace();
    let frontmost_application = workspace.frontmostApplication()?;
    Some(frontmost_application.processIdentifier())
}

#[cfg(target_os = "macos")]
fn capture_frontmost_application_pid() -> Option<i32> {
    let frontmost_pid = frontmost_application_pid()?;
    let current_pid = NSRunningApplication::currentApplication().processIdentifier();

    if frontmost_pid <= 0 || frontmost_pid == current_pid {
        return None;
    }

    Some(frontmost_pid)
}

#[cfg(target_os = "macos")]
pub fn remember_clipboard_external_paste_target(app: &AppHandle) {
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    let target_pid = capture_frontmost_application_pid();
    shortcut_state.remember_clipboard_external_paste_target_pid(target_pid);

    tracing::info!(
        target_pid = ?target_pid,
        "remembered frontmost application for clipboard external paste"
    );
}

#[cfg(not(target_os = "macos"))]
pub fn remember_clipboard_external_paste_target(_app: &AppHandle) {}

#[cfg(target_os = "macos")]
pub fn reactivate_clipboard_external_paste_target(app: &AppHandle) -> Result<Option<i32>> {
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    let Some(target_pid) = shortcut_state.take_clipboard_external_paste_target_pid() else {
        tracing::info!("no remembered external paste target available");
        return Ok(None);
    };
    let window = main_window(app)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            let activated = (|| {
                let Some(mtm) = objc2::MainThreadMarker::new() else {
                    tracing::warn!(
                        target_pid,
                        "failed to acquire MainThreadMarker for clipboard external paste target activation"
                    );
                    return false;
                };

                let Some(target_application) =
                    NSRunningApplication::runningApplicationWithProcessIdentifier(
                        target_pid as libc::pid_t,
                    )
                else {
                    tracing::warn!(
                        target_pid,
                        "failed to reacquire remembered application for clipboard external paste"
                    );
                    return false;
                };

                if target_application.isTerminated() {
                    tracing::warn!(
                        target_pid,
                        "remembered application already terminated before clipboard external paste"
                    );
                    return false;
                }

                if target_application.isHidden() {
                    let unhidden = target_application.unhide();
                    tracing::info!(
                        target_pid,
                        unhidden,
                        "requested unhide for clipboard paste target"
                    );
                }

                let current_application = NSRunningApplication::currentApplication();
                let shared_application = NSApplication::sharedApplication(mtm);
                shared_application.yieldActivationToApplication(&target_application);
                shared_application.deactivate();

                let activation_options = NSApplicationActivationOptions::ActivateAllWindows;
                let activated = target_application
                    .activateFromApplication_options(&current_application, activation_options)
                    || target_application.activateWithOptions(activation_options);
                tracing::info!(
                    target_pid,
                    activated,
                    current_app_pid = current_application.processIdentifier(),
                    "requested cooperative activation for clipboard external paste target"
                );
                activated
            })();

            let _ = sender.send(activated);
        })
        .context("failed to schedule clipboard external paste target activation on main thread")?;

    let activated = receiver
        .recv_timeout(Duration::from_millis(250))
        .context("timed out waiting for clipboard external paste target activation")?;

    Ok(activated.then_some(target_pid))
}

#[cfg(not(target_os = "macos"))]
pub fn reactivate_clipboard_external_paste_target(_app: &AppHandle) -> Result<Option<i32>> {
    Ok(None)
}

#[cfg(target_os = "macos")]
pub async fn wait_for_frontmost_application_pid(target_pid: i32, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    let poll_interval = Duration::from_millis(20);

    loop {
        if frontmost_application_pid() == Some(target_pid) {
            return true;
        }

        if tokio::time::Instant::now() >= deadline {
            return false;
        }

        sleep(poll_interval).await;
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn wait_for_frontmost_application_pid(_target_pid: i32, _timeout: Duration) -> bool {
    false
}

pub fn configure_main_window(window: &WebviewWindow) -> Result<()> {
    apply_platform_window_behavior(window)?;
    set_default_window_position(window)?;

    let launcher_state = window
        .app_handle()
        .state::<ShortcutRuntimeState>()
        .inner()
        .clone();
    let launcher_window_label = window.label().to_string();
    let launcher_app_handle = window.app_handle().clone();
    window.on_window_event(move |event| {
        let window_label = launcher_window_label.clone();
        match event {
            WindowEvent::Focused(true) => {
                let sequence = launcher_state.cancel_launcher_blur_auto_hide_confirmation();
                tracing::info!(
                    window_label,
                    blur_auto_hide_sequence = sequence,
                    "launcher window gained focus"
                );
            }
            WindowEvent::Focused(false) => {
                #[cfg(target_os = "macos")]
                let tauri_reported_focus = launcher_window(&launcher_app_handle, &window_label)
                    .ok()
                    .and_then(|window| window.is_focused().ok());
                let blur_auto_hide_delay_ms = launcher_state
                    .launcher_blur_auto_hide_delay()
                    .map(|duration| duration.as_millis());

                #[cfg(not(target_os = "macos"))]
                let tauri_reported_focus: Option<bool> = None;

                tracing::info!(
                    window_label,
                    launcher_visible = launcher_state.is_launcher_visible(),
                    blur_auto_hide_enabled = launcher_state.is_launcher_blur_auto_hide_enabled(),
                    transient_window_interaction_active =
                        launcher_state.is_transient_window_interaction_active(),
                    tauri_reported_focus = ?tauri_reported_focus,
                    blur_auto_hide_delay_ms = ?blur_auto_hide_delay_ms,
                    "launcher window lost focus"
                );

                if !launcher_state.is_launcher_visible() {
                    let sequence = launcher_state.cancel_launcher_blur_auto_hide_confirmation();
                    tracing::info!(
                        window_label,
                        blur_auto_hide_sequence = sequence,
                        "ignoring launcher blur because launcher is already hidden"
                    );
                    return;
                }

                if launcher_state.is_launcher_pinned() {
                    let sequence = launcher_state.cancel_launcher_blur_auto_hide_confirmation();
                    launcher_state.arm_launcher_blur_auto_hide_suppression(
                        LAUNCHER_SHOW_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD,
                    );
                    #[cfg(target_os = "macos")]
                    {
                        match inspect_macos_panel_state_from_app(&launcher_app_handle, &window_label) {
                            Ok(panel_state) => {
                                tracing::info!(
                                    window_label,
                                    panel_visible = panel_state.visible,
                                    panel_key_window = panel_state.key_window,
                                    panel_occlusion_visible = panel_state.occlusion_visible,
                                    app_active = panel_state.app_active,
                                    "captured macOS panel state after blur while launcher is pinned"
                                );
                            }
                            Err(error) => {
                                tracing::warn!(
                                    error = format_args!("{:#}", error),
                                    window_label,
                                    "failed to inspect macOS panel state after blur while launcher is pinned"
                                );
                            }
                        }

                        schedule_macos_panel_visibility_reinforcement(
                            launcher_app_handle.clone(),
                            launcher_state.clone(),
                            window_label.clone(),
                        );
                    }
                    tracing::info!(
                        window_label,
                        blur_auto_hide_sequence = sequence,
                        "ignoring launcher blur because launcher is pinned"
                    );
                    return;
                }

                if !launcher_state.is_launcher_blur_auto_hide_enabled() {
                    let sequence = launcher_state.cancel_launcher_blur_auto_hide_confirmation();
                    launcher_state.arm_launcher_blur_auto_hide_suppression(
                        LAUNCHER_SHOW_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD,
                    );
                    #[cfg(target_os = "macos")]
                    {
                        match inspect_macos_panel_state_from_app(&launcher_app_handle, &window_label) {
                            Ok(panel_state) => {
                                tracing::info!(
                                    window_label,
                                    panel_visible = panel_state.visible,
                                    panel_key_window = panel_state.key_window,
                                    panel_occlusion_visible = panel_state.occlusion_visible,
                                    app_active = panel_state.app_active,
                                    "captured macOS panel state after blur while blur auto-hide is disabled"
                                );
                            }
                            Err(error) => {
                                tracing::warn!(
                                    error = format_args!("{:#}", error),
                                    window_label,
                                    "failed to inspect macOS panel state after blur while blur auto-hide is disabled"
                                );
                            }
                        }

                        schedule_macos_panel_visibility_reinforcement(
                            launcher_app_handle.clone(),
                            launcher_state.clone(),
                            window_label.clone(),
                        );
                    }
                    tracing::info!(
                        window_label,
                        blur_auto_hide_sequence = sequence,
                        "ignoring launcher blur because blur auto-hide is disabled"
                    );
                    return;
                }

                if tauri_reported_focus == Some(true) {
                    let sequence = launcher_state.cancel_launcher_blur_auto_hide_confirmation();
                    tracing::info!(
                        window_label,
                        blur_auto_hide_sequence = sequence,
                        "ignoring launcher blur because Tauri still reports the window as focused"
                    );
                    return;
                }

                if launcher_state.is_transient_window_interaction_active() {
                    tracing::info!(
                        window_label,
                        "skipping launcher auto-hide during transient window interaction"
                    );
                    return;
                }

                if launcher_state.launcher_blur_auto_hide_delay().is_some() {
                    tracing::info!(
                        window_label,
                        blur_auto_hide_delay_ms = ?blur_auto_hide_delay_ms,
                        "ignoring launcher blur during post-show stabilization window"
                    );
                    return;
                }

                let sequence = launcher_state.arm_launcher_blur_auto_hide_confirmation();
                let app_handle = launcher_app_handle.clone();
                let launcher_state = launcher_state.clone();
                tracing::info!(
                    window_label,
                    blur_auto_hide_sequence = sequence,
                    confirmation_delay_ms = LAUNCHER_BLUR_AUTO_HIDE_CONFIRMATION_PERIOD.as_millis(),
                    "scheduling launcher auto-hide after focus loss"
                );
                tauri::async_runtime::spawn(async move {
                    sleep(LAUNCHER_BLUR_AUTO_HIDE_CONFIRMATION_PERIOD).await;

                    if !launcher_state.should_execute_launcher_blur_auto_hide(sequence) {
                        tracing::info!(
                            window_label,
                            blur_auto_hide_sequence = sequence,
                            "cancelling launcher auto-hide because focus state changed"
                        );
                        return;
                    }

                    if !launcher_state.is_launcher_visible() {
                        tracing::info!(
                            window_label,
                            blur_auto_hide_sequence = sequence,
                            "cancelling launcher auto-hide because launcher became hidden"
                        );
                        return;
                    }

                    if launcher_state.is_transient_window_interaction_active() {
                        tracing::info!(
                            window_label,
                            blur_auto_hide_sequence = sequence,
                            "cancelling launcher auto-hide because transient interaction resumed"
                        );
                        return;
                    }

                    if launcher_state.launcher_blur_auto_hide_delay().is_some() {
                        tracing::info!(
                            window_label,
                            blur_auto_hide_sequence = sequence,
                            "cancelling launcher auto-hide because blur suppression re-armed"
                        );
                        return;
                    }

                    if launcher_state.is_launcher_pinned() {
                        let cancelled_sequence =
                            launcher_state.cancel_launcher_blur_auto_hide_confirmation();
                        tracing::info!(
                            window_label,
                            blur_auto_hide_sequence = cancelled_sequence,
                            "cancelling launcher auto-hide because launcher was pinned"
                        );
                        return;
                    }

                    if !launcher_state.is_launcher_blur_auto_hide_enabled() {
                        let cancelled_sequence =
                            launcher_state.cancel_launcher_blur_auto_hide_confirmation();
                        tracing::info!(
                            window_label,
                            blur_auto_hide_sequence = cancelled_sequence,
                            "cancelling launcher auto-hide because blur auto-hide was disabled"
                        );
                        return;
                    }

                    let window = match launcher_window(&app_handle, &window_label) {
                        Ok(window) => window,
                        Err(error) => {
                            tracing::warn!(
                                error = format_args!("{:#}", error),
                                window_label,
                                blur_auto_hide_sequence = sequence,
                                "failed to reacquire launcher window for confirmed auto-hide"
                            );
                            return;
                        }
                    };

                    #[cfg(target_os = "macos")]
                    match window.is_focused() {
                        Ok(true) => {
                            let cancelled_sequence =
                                launcher_state.cancel_launcher_blur_auto_hide_confirmation();
                            tracing::info!(
                                window_label,
                                blur_auto_hide_sequence = cancelled_sequence,
                                "cancelling launcher auto-hide because Tauri reports focus restored"
                            );
                            return;
                        }
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(
                                error = format_args!("{:#}", error),
                                window_label,
                                blur_auto_hide_sequence = sequence,
                                "failed to verify launcher focus state before confirmed auto-hide"
                            );
                        }
                    }

                    if let Err(error) = hide_launcher_window_statefully(
                        &window,
                        &launcher_state,
                        "focus_lost_confirmed",
                    ) {
                        tracing::warn!(error = format_args!("{:#}", error), "failed to auto-hide launcher window on blur");
                    }
                });
            }
            _ => {}
        }
    });

    Ok(())
}

pub fn configure_clipboard_window(window: &WebviewWindow) -> Result<()> {
    apply_platform_window_behavior(window)?;
    set_default_window_position(window)?;

    let shortcut_state = window
        .app_handle()
        .state::<ShortcutRuntimeState>()
        .inner()
        .clone();
    let app_handle = window.app_handle().clone();
    let window_label = window.label().to_string();
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(false) => {
            if !shortcut_state.is_clipboard_history_visible() {
                return;
            }

            if let Err(error) = hide_clipboard_window_by_label(&app_handle, "focus_lost", false) {
                tracing::warn!(
                    error = format_args!("{:#}", error),
                    window_label,
                    "failed to auto-hide clipboard history window on blur"
                );
            }
        }
        WindowEvent::Destroyed => {
            shortcut_state.set_clipboard_history_visible(false);
            shortcut_state.set_clipboard_window_preserves_launcher_focus(false);
        }
        _ => {}
    });

    Ok(())
}

pub fn toggle_main_window(app: &AppHandle) -> Result<()> {
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    let should_stop_after_hiding_clipboard =
        should_stop_toggle_after_hiding_clipboard(shortcut_state.inner());
    if shortcut_state.is_clipboard_history_visible() {
        hide_clipboard_window_by_label(app, "launcher_toggle", true)?;
        if should_stop_after_hiding_clipboard {
            return Ok(());
        }
    }

    let window = main_window(app)?;
    let launcher_state = shortcut_state;
    #[cfg(target_os = "macos")]
    let launcher_focused = window
        .is_focused()
        .context("failed to inspect launcher focus state")?;
    #[cfg(target_os = "macos")]
    let macos_panel_state = if launcher_state.is_launcher_visible() {
        match inspect_macos_panel_state(&window) {
            Ok(panel_state) => {
                tracing::info!(
                    window_label = window.label(),
                    panel_visible = panel_state.visible,
                    panel_key_window = panel_state.key_window,
                    panel_occlusion_visible = panel_state.occlusion_visible,
                    app_active = panel_state.app_active,
                    "captured macOS panel state for launcher toggle"
                );
                Some(panel_state)
            }
            Err(error) => {
                tracing::warn!(
                    error = format_args!("{:#}", error),
                    window_label = window.label(),
                    "failed to inspect macOS panel state for launcher toggle"
                );
                None
            }
        }
    } else {
        None
    };

    #[cfg(target_os = "macos")]
    tracing::info!(
        window_label = window.label(),
        launcher_visible = launcher_state.is_launcher_visible(),
        launcher_focused = launcher_focused,
        "toggle launcher window requested"
    );

    #[cfg(not(target_os = "macos"))]
    tracing::info!(
        window_label = window.label(),
        launcher_visible = launcher_state.is_launcher_visible(),
        "toggle launcher window requested"
    );

    if launcher_state.is_launcher_visible() {
        #[cfg(target_os = "macos")]
        if let Some(reason) = macos_panel_toggle_reveal_reason(launcher_focused, macos_panel_state)
        {
            tracing::info!(
                window_label = window.label(),
                reveal_reason = reason,
                launcher_focused,
                blur_auto_hide_enabled = launcher_state.is_launcher_blur_auto_hide_enabled(),
                panel_visible = macos_panel_state.map(|state| state.visible),
                panel_key_window = macos_panel_state.map(|state| state.key_window),
                panel_occlusion_visible = macos_panel_state.map(|state| state.occlusion_visible),
                app_active = macos_panel_state.map(|state| state.app_active),
                "launcher is marked visible but not effectively present on macOS; re-showing instead of hiding"
            );
            prepare_main_window_for_show(&window)?;
            launcher_state.cancel_launcher_blur_auto_hide_confirmation();
            launcher_state.arm_launcher_blur_auto_hide_suppression(
                LAUNCHER_SHOW_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD,
            );
            show_window(&window)?;
            order_main_window_front(&window)?;
            launcher_state.set_launcher_visible(true);
            return Ok(());
        }

        hide_main_window(app)?;
        return Ok(());
    }

    reveal_main_window(app)
}

fn default_window_size_for_view_mode(mode: LauncherWindowViewMode) -> LogicalSize<f64> {
    match mode {
        LauncherWindowViewMode::Main => {
            LogicalSize::new(DEFAULT_MAIN_WINDOW_WIDTH, DEFAULT_MAIN_WINDOW_HEIGHT)
        }
        LauncherWindowViewMode::ClipboardHistory => LogicalSize::new(
            DEFAULT_CLIPBOARD_WINDOW_WIDTH,
            DEFAULT_CLIPBOARD_WINDOW_HEIGHT,
        ),
    }
}

fn apply_window_size_for_view_mode(
    window: &WebviewWindow,
    shortcut_state: &ShortcutRuntimeState,
    mode: LauncherWindowViewMode,
    should_reposition_default: bool,
) -> Result<()> {
    let work_area = resolve_presentation_work_area(window, should_reposition_default)?;
    let cached_size = shortcut_state
        .cached_launcher_window_size(mode)
        .map(|size| LogicalSize::new(size.width, size.height))
        .unwrap_or_else(|| default_window_size_for_view_mode(mode));
    let next_size = work_area
        .map(|bounds| clamp_window_size_to_work_area(cached_size, bounds.size))
        .unwrap_or(cached_size);

    tracing::info!(
        window_label = window.label(),
        launcher_view_mode = ?mode,
        cached_width = cached_size.width,
        cached_height = cached_size.height,
        applied_width = next_size.width,
        applied_height = next_size.height,
        should_reposition_default,
        "applying cached launcher window size before show"
    );

    set_window_content_size(window, next_size)?;
    shortcut_state.set_launcher_view_mode(mode);
    shortcut_state.remember_launcher_window_size(
        mode,
        LauncherWindowSize {
            width: next_size.width,
            height: next_size.height,
        },
    );

    if should_reposition_default {
        set_default_window_position(window)?;
    } else {
        clamp_window_position_within_work_area(window, work_area, next_size)?;
    }

    Ok(())
}

pub fn reveal_main_window(app: &AppHandle) -> Result<()> {
    if app
        .state::<ShortcutRuntimeState>()
        .is_clipboard_history_visible()
    {
        hide_clipboard_window_by_label(app, "reveal_main_window", false)?;
    }

    let window = main_window(app)?;
    let launcher_state = app.state::<ShortcutRuntimeState>();
    let was_launcher_visible = launcher_state.is_launcher_visible();

    tracing::info!(
        window_label = window.label(),
        launcher_visible = was_launcher_visible,
        "revealing launcher window"
    );

    prepare_main_window_for_show(&window)?;
    launcher_state.cancel_launcher_blur_auto_hide_confirmation();

    if !was_launcher_visible {
        launcher_state.set_launcher_blur_auto_hide_enabled(true);
        launcher_state.arm_launcher_resize_reposition(LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD);
    }

    apply_window_size_for_view_mode(
        &window,
        launcher_state.inner(),
        LauncherWindowViewMode::Main,
        !was_launcher_visible,
    )?;
    window
        .emit(
            REVEAL_LAUNCHER_MAIN_PANEL_EVENT,
            RevealLauncherMainPanelPayload,
        )
        .context("failed to emit launcher main panel reveal event")?;

    launcher_state
        .arm_launcher_blur_auto_hide_suppression(LAUNCHER_SHOW_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD);
    show_window(&window)?;
    order_main_window_front(&window)?;
    launcher_state.set_launcher_visible(true);
    Ok(())
}

pub fn launcher_is_effectively_foreground(
    app: &AppHandle,
    shortcut_state: &ShortcutRuntimeState,
) -> Result<bool> {
    if !shortcut_state.is_launcher_visible() {
        return Ok(false);
    }

    let window = main_window(app)?;
    let launcher_focused = window
        .is_focused()
        .context("failed to inspect launcher focus state for notification")?;

    #[cfg(target_os = "macos")]
    {
        let panel_state = inspect_macos_panel_state(&window)?;
        Ok(macos_panel_toggle_reveal_reason(launcher_focused, Some(panel_state)).is_none())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(launcher_focused)
    }
}

pub fn show_main_window_with_error(app: &AppHandle, error_message: &str) -> Result<()> {
    if app
        .state::<ShortcutRuntimeState>()
        .is_clipboard_history_visible()
    {
        hide_clipboard_window_by_label(app, "show_error", false)?;
    }

    let window = main_window(app)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    emit_ocr_error_event(&window, error_message)?;
    prepare_main_window_for_show(&window)?;
    shortcut_state.cancel_launcher_blur_auto_hide_confirmation();
    shortcut_state.set_launcher_blur_auto_hide_enabled(true);
    shortcut_state.arm_launcher_resize_reposition(LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD);
    shortcut_state
        .arm_launcher_blur_auto_hide_suppression(LAUNCHER_SHOW_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD);
    apply_window_size_for_view_mode(
        &window,
        shortcut_state.inner(),
        LauncherWindowViewMode::Main,
        true,
    )?;
    show_window(&window)?;
    order_main_window_front(&window)?;
    shortcut_state.set_launcher_visible(true);
    Ok(())
}

pub fn emit_clipboard_history_panel_error(app: &AppHandle, error_message: &str) -> Result<()> {
    let window = clipboard_window(app)?;
    emit_ocr_error_event(&window, error_message)
}

pub fn show_main_window_with_shortcut_translation_started(
    app: &AppHandle,
    source_mode: ShortcutTranslationSourceMode,
    source_text: String,
) -> Result<()> {
    if app
        .state::<ShortcutRuntimeState>()
        .is_clipboard_history_visible()
    {
        hide_clipboard_window_by_label(app, "show_shortcut_translation", false)?;
    }

    let window = main_window(app)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    window
        .emit(
            OCR_TRANSLATION_STARTED_EVENT,
            OcrTranslationStartedPayload {
                source_mode,
                source_text,
            },
        )
        .context("failed to emit OCR translation started event")?;
    prepare_main_window_for_show(&window)?;
    shortcut_state.cancel_launcher_blur_auto_hide_confirmation();
    shortcut_state.set_launcher_blur_auto_hide_enabled(true);
    shortcut_state.arm_launcher_resize_reposition(LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD);
    shortcut_state
        .arm_launcher_blur_auto_hide_suppression(LAUNCHER_SHOW_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD);
    apply_window_size_for_view_mode(
        &window,
        shortcut_state.inner(),
        LauncherWindowViewMode::Main,
        true,
    )?;
    show_window(&window)?;
    order_main_window_front(&window)?;
    shortcut_state.set_launcher_visible(true);
    Ok(())
}

pub fn emit_shortcut_translation_result(
    app: &AppHandle,
    source_mode: ShortcutTranslationSourceMode,
    source_text: String,
    result: ExecutionResult,
) -> Result<()> {
    let window = main_window(app)?;
    window
        .emit(
            OCR_TRANSLATION_RESULT_EVENT,
            OcrTranslationResultPayload {
                source_mode,
                source_text,
                result,
            },
        )
        .context("failed to emit OCR translation result event")?;
    Ok(())
}

pub fn emit_shortcut_translation_stream(
    app: &AppHandle,
    source_mode: ShortcutTranslationSourceMode,
    source_text: String,
    partial_text: String,
) -> Result<()> {
    let window = main_window(app)?;
    window
        .emit(
            OCR_TRANSLATION_STREAM_EVENT,
            OcrTranslationStreamPayload {
                source_mode,
                source_text,
                partial_text,
            },
        )
        .context("failed to emit OCR translation stream event")?;
    Ok(())
}

pub fn emit_rag_runtime_status(app: &AppHandle, status: &RagRuntimeStatus) -> Result<()> {
    let window = main_window(app)?;
    window
        .emit(RAG_RUNTIME_STATUS_EVENT, status)
        .context("failed to emit RAG runtime status event")?;
    Ok(())
}

pub fn show_main_window_with_clipboard_history_panel(app: &AppHandle) -> Result<()> {
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    let selection_mode = if shortcut_state.is_clipboard_history_visible() {
        current_clipboard_history_selection_mode(shortcut_state.inner())
    } else {
        match launcher_is_effectively_foreground(app, shortcut_state.inner()) {
            Ok(true) => ClipboardHistorySelectionMode::InsertIntoLauncher,
            Ok(false) => ClipboardHistorySelectionMode::PasteExternally,
            Err(error) => {
                let clipboard_window = clipboard_window(app)?;
                tracing::warn!(
                    error = format_args!("{:#}", error),
                    window_label = clipboard_window.label(),
                    "failed to inspect launcher foreground state before opening clipboard history"
                );
                ClipboardHistorySelectionMode::PasteExternally
            }
        }
    };

    show_clipboard_history_panel(app, selection_mode)
}

pub fn show_clipboard_history_panel(
    app: &AppHandle,
    selection_mode: ClipboardHistorySelectionMode,
) -> Result<()> {
    let clipboard_window = clipboard_window(app)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();

    if selection_mode == ClipboardHistorySelectionMode::InsertIntoLauncher {
        #[cfg(target_os = "macos")]
        shortcut_state.remember_clipboard_external_paste_target_pid(None);
        if shortcut_state.is_launcher_visible() {
            hide_main_window(app)?;
        }
        shortcut_state.set_clipboard_window_preserves_launcher_focus(true);
    } else {
        if should_refresh_clipboard_external_paste_target(shortcut_state.inner(), selection_mode) {
            remember_clipboard_external_paste_target(app);
        }
        shortcut_state.set_clipboard_window_preserves_launcher_focus(false);
    }

    prepare_main_window_for_show(&clipboard_window)?;
    apply_window_size_for_view_mode(
        &clipboard_window,
        shortcut_state.inner(),
        LauncherWindowViewMode::ClipboardHistory,
        true,
    )?;
    clipboard_window
        .emit(
            OPEN_CLIPBOARD_HISTORY_PANEL_EVENT,
            OpenClipboardHistoryPanelPayload { selection_mode },
        )
        .context("failed to emit clipboard history panel event")?;
    show_window(&clipboard_window)?;
    order_main_window_front(&clipboard_window)?;
    shortcut_state.set_clipboard_history_visible(true);
    Ok(())
}

fn current_clipboard_history_selection_mode(
    shortcut_state: &ShortcutRuntimeState,
) -> ClipboardHistorySelectionMode {
    if shortcut_state.clipboard_window_preserves_launcher_focus() {
        ClipboardHistorySelectionMode::InsertIntoLauncher
    } else {
        ClipboardHistorySelectionMode::PasteExternally
    }
}

fn should_refresh_clipboard_external_paste_target(
    shortcut_state: &ShortcutRuntimeState,
    selection_mode: ClipboardHistorySelectionMode,
) -> bool {
    selection_mode == ClipboardHistorySelectionMode::PasteExternally
        && !shortcut_state.is_clipboard_history_visible()
}

fn should_stop_toggle_after_hiding_clipboard(shortcut_state: &ShortcutRuntimeState) -> bool {
    shortcut_state.is_clipboard_history_visible()
        && shortcut_state.clipboard_window_preserves_launcher_focus()
}

pub fn hide_main_window(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    hide_launcher_window_statefully(&window, shortcut_state.inner(), "explicit_hide")
}

pub fn hide_launcher_window(app: &AppHandle, window_label: Option<&str>) -> Result<()> {
    match window_label.unwrap_or(MAIN_WINDOW_LABEL) {
        CLIPBOARD_WINDOW_LABEL => hide_clipboard_window_by_label(app, "explicit_hide", false),
        _ => hide_main_window(app),
    }
}

pub fn dismiss_clipboard_history_panel(app: &AppHandle) -> Result<()> {
    let restore_main_focus = {
        let shortcut_state = app.state::<ShortcutRuntimeState>();
        shortcut_state.clipboard_window_preserves_launcher_focus()
    };

    hide_clipboard_window_by_label(app, "explicit_dismiss", restore_main_focus)
}

pub fn resize_launcher_window(
    app: &AppHandle,
    width: f64,
    height: f64,
    window_label: Option<&str>,
) -> Result<()> {
    let resolved_label = window_label.unwrap_or(MAIN_WINDOW_LABEL);
    let window = launcher_window(app, resolved_label)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    let launcher_visible = match resolved_label {
        CLIPBOARD_WINDOW_LABEL => shortcut_state.is_clipboard_history_visible(),
        _ => shortcut_state.is_launcher_visible(),
    };
    let should_reposition_default =
        !launcher_visible || shortcut_state.should_reposition_launcher_on_resize();
    let work_area = resolve_presentation_work_area(&window, should_reposition_default)?;
    let requested_size = LogicalSize::new(
        width.max(MIN_WINDOW_DIMENSION),
        height.max(MIN_WINDOW_DIMENSION),
    );
    let next_size = work_area
        .map(|bounds| clamp_window_size_to_work_area(requested_size, bounds.size))
        .unwrap_or(requested_size);
    let current_mode = view_mode_for_window_label(resolved_label);

    tracing::info!(
        window_label = window.label(),
        launcher_view_mode = ?current_mode,
        requested_width = width,
        requested_height = height,
        applied_width = next_size.width,
        applied_height = next_size.height,
        launcher_visible,
        should_reposition_default,
        "resizing launcher window"
    );

    if launcher_visible {
        shortcut_state.cancel_launcher_blur_auto_hide_confirmation();
        shortcut_state.arm_launcher_blur_auto_hide_suppression(
            LAUNCHER_VISIBLE_RESIZE_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD,
        );
        tracing::debug!(
            window_label = window.label(),
            suppression_ms = LAUNCHER_VISIBLE_RESIZE_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD.as_millis(),
            "re-arming blur auto-hide suppression because visible launcher is resizing"
        );
    }

    shortcut_state.remember_launcher_window_size(
        current_mode,
        LauncherWindowSize {
            width: next_size.width,
            height: next_size.height,
        },
    );

    set_window_content_size(&window, next_size)?;
    if should_reposition_default {
        set_default_window_position(&window)?;
    } else {
        clamp_window_position_within_work_area(&window, work_area, next_size)?;
    }

    Ok(())
}

fn set_window_content_size(window: &WebviewWindow, next_size: LogicalSize<f64>) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if has_macos_panel(window) {
            run_macos_panel_on_main_thread(window, move |panel| {
                panel.set_content_size(next_size.width, next_size.height);
            })?;
            return Ok(());
        }
    }

    window
        .set_size(next_size)
        .context("failed to resize launcher window")
}

fn launcher_window(app: &AppHandle, label: &str) -> Result<WebviewWindow> {
    app.get_webview_window(label)
        .with_context(|| format!("launcher window must exist: {label}"))
}

fn main_window(app: &AppHandle) -> Result<WebviewWindow> {
    launcher_window(app, MAIN_WINDOW_LABEL)
}

fn clipboard_window(app: &AppHandle) -> Result<WebviewWindow> {
    launcher_window(app, CLIPBOARD_WINDOW_LABEL)
}

fn view_mode_for_window_label(label: &str) -> LauncherWindowViewMode {
    match label {
        CLIPBOARD_WINDOW_LABEL => LauncherWindowViewMode::ClipboardHistory,
        _ => LauncherWindowViewMode::Main,
    }
}

fn apply_platform_window_behavior(window: &WebviewWindow) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        initialize_macos_panel(window)?;
    }

    Ok(())
}

fn set_default_window_position(window: &WebviewWindow) -> Result<()> {
    let Some(work_area) = resolve_target_window_work_area(window)? else {
        return Ok(());
    };

    let window_size = window
        .inner_size()
        .context("failed to get launcher window size for default position")?
        .to_logical::<f64>(work_area.scale_factor);

    let position = if window.label() == CLIPBOARD_WINDOW_LABEL {
        compute_clipboard_window_position(window, work_area, window_size)?
    } else {
        compute_default_window_position(work_area.position, work_area.size, window_size)
    };

    window
        .set_position(position)
        .context("failed to set launcher default position")
}

fn resolve_window_work_area(window: &WebviewWindow) -> Result<Option<WindowWorkArea>> {
    let Some(monitor) = resolve_window_monitor(window)? else {
        return Ok(None);
    };

    let scale_factor = monitor.scale_factor();
    let work_area = monitor.work_area();

    Ok(Some(WindowWorkArea {
        scale_factor,
        position: work_area.position.to_logical::<f64>(scale_factor),
        size: work_area.size.to_logical::<f64>(scale_factor),
    }))
}

fn resolve_target_window_work_area(window: &WebviewWindow) -> Result<Option<WindowWorkArea>> {
    let Some(monitor) = resolve_target_monitor(window)? else {
        return Ok(None);
    };

    let scale_factor = monitor.scale_factor();
    let work_area = monitor.work_area();

    Ok(Some(WindowWorkArea {
        scale_factor,
        position: work_area.position.to_logical::<f64>(scale_factor),
        size: work_area.size.to_logical::<f64>(scale_factor),
    }))
}

fn resolve_presentation_work_area(
    window: &WebviewWindow,
    should_reposition_default: bool,
) -> Result<Option<WindowWorkArea>> {
    let current_work_area = resolve_window_work_area(window)?;
    let target_work_area = resolve_target_window_work_area(window)?;

    Ok(select_presentation_work_area(
        current_work_area,
        target_work_area,
        should_reposition_default,
    ))
}

fn select_presentation_work_area(
    current_work_area: Option<WindowWorkArea>,
    target_work_area: Option<WindowWorkArea>,
    should_reposition_default: bool,
) -> Option<WindowWorkArea> {
    if should_reposition_default {
        target_work_area.or(current_work_area)
    } else {
        current_work_area.or(target_work_area)
    }
}

fn resolve_target_monitor(window: &WebviewWindow) -> Result<Option<tauri::Monitor>> {
    if let Ok(cursor_position) = window.cursor_position() {
        if let Ok(Some(cursor_monitor)) =
            window.monitor_from_point(cursor_position.x, cursor_position.y)
        {
            return Ok(Some(cursor_monitor));
        }
    }

    if let Some(current_monitor) = window
        .current_monitor()
        .context("failed to get current monitor for launcher default position")?
    {
        return Ok(Some(current_monitor));
    }

    window
        .primary_monitor()
        .context("failed to get primary monitor for launcher default position")
}

fn resolve_window_monitor(window: &WebviewWindow) -> Result<Option<tauri::Monitor>> {
    if let Some(current_monitor) = window
        .current_monitor()
        .context("failed to get current monitor for launcher resize bounds")?
    {
        return Ok(Some(current_monitor));
    }

    if let Ok(window_position) = window.outer_position() {
        if let Ok(Some(window_monitor)) =
            window.monitor_from_point(f64::from(window_position.x), f64::from(window_position.y))
        {
            return Ok(Some(window_monitor));
        }
    }

    window
        .primary_monitor()
        .context("failed to get primary monitor for launcher resize bounds")
}

fn clamp_window_size_to_work_area(
    requested_size: LogicalSize<f64>,
    work_area_size: LogicalSize<f64>,
) -> LogicalSize<f64> {
    let max_width = work_area_size.width.max(MIN_WINDOW_DIMENSION);
    let max_height = work_area_size.height.max(MIN_WINDOW_DIMENSION);

    LogicalSize::new(
        requested_size.width.clamp(MIN_WINDOW_DIMENSION, max_width),
        requested_size
            .height
            .clamp(MIN_WINDOW_DIMENSION, max_height),
    )
}

fn compute_default_window_position(
    work_area_position: LogicalPosition<f64>,
    work_area_size: LogicalSize<f64>,
    window_size: LogicalSize<f64>,
) -> LogicalPosition<f64> {
    let min_x = work_area_position.x;
    let min_y = work_area_position.y;
    let max_x = min_x + (work_area_size.width - window_size.width).max(0.0);
    let max_y = min_y + (work_area_size.height - window_size.height).max(0.0);
    let x = (min_x + (work_area_size.width - window_size.width) / 2.0).clamp(min_x, max_x);
    let y = (min_y + work_area_size.height * LAUNCHER_VERTICAL_CENTER_RATIO
        - window_size.height / 2.0)
        .clamp(min_y, max_y);

    LogicalPosition::new(x, y)
}

fn compute_clipboard_window_position(
    window: &WebviewWindow,
    work_area: WindowWorkArea,
    window_size: LogicalSize<f64>,
) -> Result<LogicalPosition<f64>> {
    let cursor_position = match window.cursor_position() {
        Ok(position) => Some(position.to_logical::<f64>(work_area.scale_factor)),
        Err(error) => {
            tracing::debug!(
                ?error,
                window_label = window.label(),
                "failed to read cursor position for clipboard window placement"
            );
            None
        }
    };

    Ok(compute_clipboard_window_position_from_cursor(
        work_area.position,
        work_area.size,
        window_size,
        cursor_position,
    ))
}

fn compute_clipboard_window_position_from_cursor(
    work_area_position: LogicalPosition<f64>,
    work_area_size: LogicalSize<f64>,
    window_size: LogicalSize<f64>,
    cursor_position: Option<LogicalPosition<f64>>,
) -> LogicalPosition<f64> {
    let Some(cursor_position) = cursor_position else {
        return compute_default_window_position(work_area_position, work_area_size, window_size);
    };

    let place_right = cursor_position.x + CLIPBOARD_WINDOW_CURSOR_OFFSET_X + window_size.width
        <= work_area_position.x + work_area_size.width;
    let place_below = cursor_position.y + CLIPBOARD_WINDOW_CURSOR_OFFSET_Y + window_size.height
        <= work_area_position.y + work_area_size.height;
    let target_x = if place_right {
        cursor_position.x + CLIPBOARD_WINDOW_CURSOR_OFFSET_X
    } else {
        cursor_position.x - window_size.width - CLIPBOARD_WINDOW_CURSOR_OFFSET_X
    };
    let target_y = if place_below {
        cursor_position.y + CLIPBOARD_WINDOW_CURSOR_OFFSET_Y
    } else {
        cursor_position.y - window_size.height - CLIPBOARD_WINDOW_CURSOR_OFFSET_Y
    };

    compute_visible_window_position(
        work_area_position,
        work_area_size,
        window_size,
        LogicalPosition::new(target_x, target_y),
    )
}

fn clamp_window_position_within_work_area(
    window: &WebviewWindow,
    work_area: Option<WindowWorkArea>,
    window_size: LogicalSize<f64>,
) -> Result<()> {
    let Some(work_area) = work_area else {
        return Ok(());
    };

    let current_position = match window.outer_position() {
        Ok(position) => position.to_logical::<f64>(work_area.scale_factor),
        Err(error) => {
            tracing::debug!(
                ?error,
                "failed to read launcher window position; falling back to default placement"
            );
            compute_default_window_position(work_area.position, work_area.size, window_size)
        }
    };
    let clamped_position = compute_visible_window_position(
        work_area.position,
        work_area.size,
        window_size,
        current_position,
    );

    window
        .set_position(clamped_position)
        .context("failed to clamp launcher window position within work area")
}

fn compute_visible_window_position(
    work_area_position: LogicalPosition<f64>,
    work_area_size: LogicalSize<f64>,
    window_size: LogicalSize<f64>,
    current_position: LogicalPosition<f64>,
) -> LogicalPosition<f64> {
    let min_x = work_area_position.x;
    let min_y = work_area_position.y;
    let max_x = min_x + (work_area_size.width - window_size.width).max(0.0);
    let max_y = min_y + (work_area_size.height - window_size.height).max(0.0);

    LogicalPosition::new(
        current_position.x.clamp(min_x, max_x),
        current_position.y.clamp(min_y, max_y),
    )
}

fn prepare_main_window_for_show(_window: &WebviewWindow) -> Result<()> {
    _window
        .app_handle()
        .state::<AppState>()
        .application()
        .schedule_refresh_if_stale(APPLICATION_CACHE_STALE_AFTER);
    Ok(())
}

fn order_main_window_front(window: &WebviewWindow) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        order_macos_window_front(window)?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn order_macos_window_front(window: &WebviewWindow) -> Result<()> {
    if has_macos_panel(window) {
        run_macos_panel_on_main_thread(window, |panel| {
            panel.order_front_regardless();
        })?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn show_macos_panel_without_focus(window: &WebviewWindow) -> Result<()> {
    window
        .app_handle()
        .show()
        .context("failed to show macOS app before reinforcing launcher panel visibility")?;

    run_macos_panel_on_main_thread(window, |panel| {
        panel.show();
        panel.order_front_regardless();
    })?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn initialize_macos_panel(window: &WebviewWindow) -> Result<()> {
    if window
        .app_handle()
        .get_webview_panel(window.label())
        .is_ok()
    {
        return Ok(());
    }

    let panel = window
        .to_panel::<LauncherPanel>()
        .context("failed to convert launcher window to NSPanel")?;

    panel.set_level(MACOS_LAUNCHER_PANEL_LEVEL.value());
    panel.set_style_mask(StyleMask::empty().borderless().nonactivating_panel().into());
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .can_join_all_spaces()
            .full_screen_auxiliary()
            .stationary()
            .into(),
    );
    panel.set_hides_on_deactivate(false);
    panel.set_becomes_key_only_if_needed(false);
    panel.set_works_when_modal(true);
    panel.set_movable_by_window_background(true);

    Ok(())
}

#[cfg(target_os = "macos")]
fn focus_macos_panel(window: &WebviewWindow) -> Result<()> {
    run_macos_panel_on_main_thread(window, |panel| {
        panel.show_and_make_key();
    })?;

    Ok(())
}

fn show_window(window: &WebviewWindow) -> Result<()> {
    tracing::info!(window_label = window.label(), "showing launcher window");

    #[cfg(target_os = "macos")]
    {
        if has_macos_panel(window) {
            focus_macos_panel(window)?;
            return Ok(());
        }
    }

    window.show().context("failed to show launcher window")?;
    window
        .unminimize()
        .context("failed to restore launcher window")?;
    window
        .set_focus()
        .context("failed to focus launcher window")?;
    Ok(())
}

fn hide_window(window: &WebviewWindow) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if has_macos_panel(window) {
            run_macos_panel_on_main_thread(window, |panel| {
                panel.hide();
            })?;
            return Ok(());
        }
    }

    window.hide().context("failed to hide launcher window")?;
    Ok(())
}

fn hide_launcher_window_statefully(
    window: &WebviewWindow,
    launcher_state: &ShortcutRuntimeState,
    reason: &'static str,
) -> Result<()> {
    let sequence = launcher_state.cancel_launcher_blur_auto_hide_confirmation();
    tracing::info!(
        window_label = window.label(),
        reason,
        blur_auto_hide_sequence = sequence,
        launcher_visible_before_hide = launcher_state.is_launcher_visible(),
        "hiding launcher window"
    );
    hide_window(window)?;
    launcher_state.set_launcher_visible(false);
    launcher_state.set_launcher_blur_auto_hide_enabled(true);
    launcher_state.clear_launcher_resize_reposition();
    launcher_state.clear_launcher_blur_auto_hide_suppression();
    Ok(())
}

fn hide_clipboard_window_statefully(
    window: &WebviewWindow,
    shortcut_state: &ShortcutRuntimeState,
    reason: &'static str,
    restore_main_focus: bool,
) -> Result<()> {
    tracing::info!(
        window_label = window.label(),
        reason,
        restore_main_focus,
        clipboard_visible_before_hide = shortcut_state.is_clipboard_history_visible(),
        "hiding clipboard history window"
    );
    hide_window(window)?;
    shortcut_state.set_clipboard_history_visible(false);

    let preserved_launcher_focus = shortcut_state.clipboard_window_preserves_launcher_focus();
    if preserved_launcher_focus {
        shortcut_state.set_clipboard_window_preserves_launcher_focus(false);
    }

    if restore_main_focus && preserved_launcher_focus {
        reveal_main_window(window.app_handle())?;
    }

    Ok(())
}

fn hide_clipboard_window_by_label(
    app: &AppHandle,
    reason: &'static str,
    restore_main_focus: bool,
) -> Result<()> {
    if !app
        .state::<ShortcutRuntimeState>()
        .is_clipboard_history_visible()
    {
        return Ok(());
    }

    let window = clipboard_window(app)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    hide_clipboard_window_statefully(&window, shortcut_state.inner(), reason, restore_main_focus)
}

fn emit_ocr_error_event(window: &WebviewWindow, error_message: &str) -> Result<()> {
    window
        .emit(OCR_ERROR_EVENT, error_message)
        .context("failed to emit OCR error event")
}

pub fn insert_clipboard_history_text_into_launcher(app: &AppHandle, text: String) -> Result<()> {
    let main_window = main_window(app)?;
    main_window
        .emit(
            INSERT_CLIPBOARD_HISTORY_TEXT_INTO_LAUNCHER_EVENT,
            InsertClipboardHistoryTextIntoLauncherPayload { text },
        )
        .context("failed to emit clipboard history insert event to launcher")?;

    hide_clipboard_window_by_label(app, "insert_into_launcher", true)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn has_macos_panel(window: &WebviewWindow) -> bool {
    window
        .app_handle()
        .get_webview_panel(window.label())
        .is_ok()
}

#[cfg(target_os = "macos")]
fn inspect_macos_panel_state_from_app(
    app: &AppHandle,
    window_label: &str,
) -> Result<MacOsPanelState> {
    let window = app
        .get_webview_window(window_label)
        .with_context(|| format!("macOS panel window must exist: {window_label}"))?;
    inspect_macos_panel_state(&window)
}

#[cfg(target_os = "macos")]
fn inspect_macos_panel_state(window: &WebviewWindow) -> Result<MacOsPanelState> {
    if !has_macos_panel(window) {
        anyhow::bail!("window is not backed by macOS NSPanel");
    }

    let (sender, receiver) = mpsc::sync_channel(1);
    run_macos_panel_on_main_thread(window, move |panel| {
        let panel_state = if let Some(mtm) = objc2::MainThreadMarker::new() {
            let application = NSApplication::sharedApplication(mtm);
            let native_panel = panel.as_panel();
            MacOsPanelState {
                visible: panel.is_visible(),
                key_window: native_panel.isKeyWindow(),
                occlusion_visible: native_panel
                    .occlusionState()
                    .contains(NSWindowOcclusionState::Visible),
                app_active: application.isActive(),
            }
        } else {
            MacOsPanelState {
                visible: panel.is_visible(),
                key_window: false,
                occlusion_visible: false,
                app_active: false,
            }
        };

        let _ = sender.send(panel_state);
    })?;

    receiver
        .recv_timeout(Duration::from_millis(250))
        .context("timed out waiting for macOS panel state inspection")
}

#[cfg(target_os = "macos")]
fn schedule_macos_panel_visibility_reinforcement(
    app_handle: AppHandle,
    launcher_state: ShortcutRuntimeState,
    window_label: String,
) {
    tauri::async_runtime::spawn(async move {
        sleep(LAUNCHER_BLUR_DISABLED_VISIBILITY_REINFORCEMENT_DELAY).await;

        if !launcher_state.is_launcher_visible()
            || launcher_state.is_launcher_blur_auto_hide_enabled()
        {
            return;
        }

        let window = match launcher_window(&app_handle, &window_label) {
            Ok(window) => window,
            Err(error) => {
                tracing::warn!(
                    error = format_args!("{:#}", error),
                    window_label,
                    "failed to reacquire launcher window for macOS visibility reinforcement"
                );
                return;
            }
        };

        match inspect_macos_panel_state(&window) {
            Ok(panel_state) => {
                tracing::info!(
                    window_label,
                    panel_visible = panel_state.visible,
                    panel_key_window = panel_state.key_window,
                    panel_occlusion_visible = panel_state.occlusion_visible,
                    app_active = panel_state.app_active,
                    "checked macOS panel state before blur-disabled visibility reinforcement"
                );

                let Some(reinforcement_reason) =
                    macos_panel_visibility_reinforcement_reason(panel_state)
                else {
                    return;
                };

                tracing::warn!(
                    window_label,
                    reinforcement_reason,
                    panel_visible = panel_state.visible,
                    panel_key_window = panel_state.key_window,
                    panel_occlusion_visible = panel_state.occlusion_visible,
                    app_active = panel_state.app_active,
                    "reinforcing macOS panel visibility after blur while blur auto-hide is disabled"
                );

                if let Err(error) = show_macos_panel_without_focus(&window) {
                    tracing::warn!(
                        error = format_args!("{:#}", error),
                        window_label,
                        "failed to reinforce macOS panel visibility without focus"
                    );
                    return;
                }

                match inspect_macos_panel_state(&window) {
                    Ok(panel_state_after) => {
                        tracing::info!(
                            window_label,
                            panel_visible = panel_state_after.visible,
                            panel_key_window = panel_state_after.key_window,
                            panel_occlusion_visible = panel_state_after.occlusion_visible,
                            app_active = panel_state_after.app_active,
                            "captured macOS panel state after blur-disabled visibility reinforcement"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = format_args!("{:#}", error),
                            window_label,
                            "failed to inspect macOS panel state after visibility reinforcement"
                        );
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = format_args!("{:#}", error),
                    window_label,
                    "failed to inspect macOS panel state before visibility reinforcement"
                );
            }
        }
    });
}

#[cfg(target_os = "macos")]
fn run_macos_panel_on_main_thread<F>(window: &WebviewWindow, operation: F) -> Result<()>
where
    F: FnOnce(PanelHandle<tauri::Wry>) + Send + 'static,
{
    let app_handle = window.app_handle().clone();
    let window_label = window.label().to_string();

    window
        .run_on_main_thread(move || {
            if let Ok(panel) = app_handle.get_webview_panel(&window_label) {
                operation(panel);
            }
        })
        .context("failed to schedule macOS panel operation on main thread")
}

#[cfg(test)]
mod tests {
    use super::{
        clamp_window_size_to_work_area, compute_clipboard_window_position_from_cursor,
        compute_default_window_position, compute_visible_window_position,
        current_clipboard_history_selection_mode, select_presentation_work_area,
        should_refresh_clipboard_external_paste_target, should_stop_toggle_after_hiding_clipboard,
        ClipboardHistorySelectionMode, LogicalPosition, LogicalSize, OcrTranslationResultPayload,
        OcrTranslationStartedPayload, ShortcutTranslationSourceMode, WindowWorkArea,
        CLIPBOARD_WINDOW_CURSOR_OFFSET_X, CLIPBOARD_WINDOW_CURSOR_OFFSET_Y,
        LAUNCHER_VERTICAL_CENTER_RATIO,
    };
    use crate::domain::execution::ExecutionResult;
    use crate::state::ShortcutRuntimeState;
    use serde_json::json;

    #[test]
    fn shortcut_translation_started_payload_uses_expected_wire_shape() {
        let payload = serde_json::to_value(OcrTranslationStartedPayload {
            source_mode: ShortcutTranslationSourceMode::Selection,
            source_text: "hello".to_string(),
        })
        .expect("payload should serialize");

        assert_eq!(
            payload,
            json!({
                "sourceMode": "selection",
                "sourceText": "hello",
            })
        );
    }

    #[test]
    fn shortcut_translation_result_payload_uses_expected_wire_shape() {
        let payload = serde_json::to_value(OcrTranslationResultPayload {
            source_mode: ShortcutTranslationSourceMode::Ocr,
            source_text: "hello".to_string(),
            result: ExecutionResult::success(
                Some("你好".to_string()),
                Some("已使用 test 进行翻译".to_string()),
                None,
                vec!["copy_text"],
                false,
            ),
        })
        .expect("payload should serialize");

        assert_eq!(
            payload,
            json!({
                "sourceMode": "ocr",
                "sourceText": "hello",
                "result": {
                    "status": "success",
                    "primaryText": "你好",
                    "secondaryText": "已使用 test 进行翻译",
                    "structuredPayload": null,
                    "nextActions": ["copy_text"],
                    "shouldCloseLauncher": false,
                }
            })
        );
    }

    #[test]
    fn default_position_centers_horizontally_and_uses_golden_vertical_center() {
        let position = compute_default_window_position(
            LogicalPosition::new(0.0, 24.0),
            LogicalSize::new(1440.0, 900.0),
            LogicalSize::new(768.0, 280.0),
        );

        assert_eq!(position.x, 336.0);
        assert_eq!(position.y, 227.8);

        let center_y = position.y + 140.0;
        assert_eq!(center_y, 24.0 + 900.0 * LAUNCHER_VERTICAL_CENTER_RATIO);
    }

    #[test]
    fn default_position_clamps_when_window_is_larger_than_work_area() {
        let position = compute_default_window_position(
            LogicalPosition::new(80.0, 40.0),
            LogicalSize::new(320.0, 180.0),
            LogicalSize::new(480.0, 220.0),
        );

        assert_eq!(position.x, 80.0);
        assert_eq!(position.y, 40.0);
    }

    #[test]
    fn clipboard_position_follows_cursor_with_offset() {
        let position = compute_clipboard_window_position_from_cursor(
            LogicalPosition::new(0.0, 24.0),
            LogicalSize::new(1440.0, 900.0),
            LogicalSize::new(428.0, 520.0),
            Some(LogicalPosition::new(320.0, 180.0)),
        );

        assert_eq!(position.x, 320.0 + CLIPBOARD_WINDOW_CURSOR_OFFSET_X);
        assert_eq!(position.y, 180.0 + CLIPBOARD_WINDOW_CURSOR_OFFSET_Y);
    }

    #[test]
    fn clipboard_position_flips_left_when_right_space_is_insufficient() {
        let position = compute_clipboard_window_position_from_cursor(
            LogicalPosition::new(0.0, 24.0),
            LogicalSize::new(1440.0, 900.0),
            LogicalSize::new(428.0, 520.0),
            Some(LogicalPosition::new(1320.0, 180.0)),
        );

        assert_eq!(position.x, 880.0);
        assert_eq!(position.y, 180.0 + CLIPBOARD_WINDOW_CURSOR_OFFSET_Y);
    }

    #[test]
    fn clipboard_position_flips_up_when_bottom_space_is_insufficient() {
        let position = compute_clipboard_window_position_from_cursor(
            LogicalPosition::new(0.0, 24.0),
            LogicalSize::new(1440.0, 900.0),
            LogicalSize::new(428.0, 520.0),
            Some(LogicalPosition::new(320.0, 860.0)),
        );

        assert_eq!(position.x, 320.0 + CLIPBOARD_WINDOW_CURSOR_OFFSET_X);
        assert_eq!(position.y, 324.0);
    }

    #[test]
    fn clipboard_position_stays_within_work_area() {
        let position = compute_clipboard_window_position_from_cursor(
            LogicalPosition::new(120.0, 40.0),
            LogicalSize::new(800.0, 600.0),
            LogicalSize::new(428.0, 520.0),
            Some(LogicalPosition::new(860.0, 620.0)),
        );

        assert_eq!(position.x, 420.0);
        assert_eq!(position.y, 84.0);
    }

    #[test]
    fn clipboard_position_falls_back_to_launcher_default_without_cursor() {
        let position = compute_clipboard_window_position_from_cursor(
            LogicalPosition::new(0.0, 24.0),
            LogicalSize::new(1440.0, 900.0),
            LogicalSize::new(428.0, 520.0),
            None,
        );

        assert_eq!(
            position,
            compute_default_window_position(
                LogicalPosition::new(0.0, 24.0),
                LogicalSize::new(1440.0, 900.0),
                LogicalSize::new(428.0, 520.0),
            )
        );
    }

    #[test]
    fn clipboard_reopen_keeps_insert_mode_while_window_preserves_launcher_focus() {
        let state = ShortcutRuntimeState::default();
        state.set_clipboard_history_visible(true);
        state.set_clipboard_window_preserves_launcher_focus(true);

        assert_eq!(
            current_clipboard_history_selection_mode(&state),
            ClipboardHistorySelectionMode::InsertIntoLauncher
        );
    }

    #[test]
    fn clipboard_first_external_open_refreshes_paste_target() {
        let state = ShortcutRuntimeState::default();

        assert!(should_refresh_clipboard_external_paste_target(
            &state,
            ClipboardHistorySelectionMode::PasteExternally
        ));
    }

    #[test]
    fn clipboard_reopen_does_not_refresh_external_paste_target() {
        let state = ShortcutRuntimeState::default();
        state.set_clipboard_history_visible(true);
        state.set_clipboard_window_preserves_launcher_focus(false);

        assert!(!should_refresh_clipboard_external_paste_target(
            &state,
            ClipboardHistorySelectionMode::PasteExternally
        ));
    }

    #[test]
    fn launcher_toggle_stops_after_hiding_clipboard_that_preserved_launcher_focus() {
        let state = ShortcutRuntimeState::default();
        state.set_clipboard_history_visible(true);
        state.set_clipboard_window_preserves_launcher_focus(true);

        assert!(should_stop_toggle_after_hiding_clipboard(&state));
    }

    #[test]
    fn launcher_toggle_continues_after_hiding_external_clipboard_window() {
        let state = ShortcutRuntimeState::default();
        state.set_clipboard_history_visible(true);
        state.set_clipboard_window_preserves_launcher_focus(false);

        assert!(!should_stop_toggle_after_hiding_clipboard(&state));
    }

    #[test]
    fn presentation_work_area_prefers_target_monitor_when_repositioning_default() {
        let current_work_area = WindowWorkArea {
            scale_factor: 2.0,
            position: LogicalPosition::new(0.0, 24.0),
            size: LogicalSize::new(1512.0, 982.0),
        };
        let target_work_area = WindowWorkArea {
            scale_factor: 1.0,
            position: LogicalPosition::new(1512.0, 0.0),
            size: LogicalSize::new(1920.0, 1080.0),
        };

        assert_eq!(
            select_presentation_work_area(Some(current_work_area), Some(target_work_area), true)
                .map(|work_area| work_area.size),
            Some(LogicalSize::new(1920.0, 1080.0))
        );
    }

    #[test]
    fn presentation_work_area_prefers_current_monitor_without_default_reposition() {
        let current_work_area = WindowWorkArea {
            scale_factor: 2.0,
            position: LogicalPosition::new(0.0, 24.0),
            size: LogicalSize::new(1512.0, 982.0),
        };
        let target_work_area = WindowWorkArea {
            scale_factor: 1.0,
            position: LogicalPosition::new(1512.0, 0.0),
            size: LogicalSize::new(1920.0, 1080.0),
        };

        assert_eq!(
            select_presentation_work_area(Some(current_work_area), Some(target_work_area), false)
                .map(|work_area| work_area.size),
            Some(LogicalSize::new(1512.0, 982.0))
        );
    }

    #[test]
    fn resize_clamps_window_size_to_work_area() {
        let size = clamp_window_size_to_work_area(
            LogicalSize::new(920.0, 720.0),
            LogicalSize::new(800.0, 640.0),
        );

        assert_eq!(size.width, 800.0);
        assert_eq!(size.height, 640.0);
    }

    #[test]
    fn visible_position_keeps_resized_window_inside_work_area() {
        let position = compute_visible_window_position(
            LogicalPosition::new(0.0, 24.0),
            LogicalSize::new(1440.0, 900.0),
            LogicalSize::new(920.0, 720.0),
            LogicalPosition::new(900.0, 260.0),
        );

        assert_eq!(position.x, 520.0);
        assert_eq!(position.y, 204.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn toggle_reveal_prefers_show_for_inactive_visible_panel() {
        use super::{
            macos_panel_toggle_reveal_reason, MacOsPanelState, PanelLevel,
            MACOS_LAUNCHER_PANEL_LEVEL,
        };

        let panel_state = MacOsPanelState {
            visible: true,
            key_window: false,
            occlusion_visible: true,
            app_active: false,
        };

        assert_eq!(
            macos_panel_toggle_reveal_reason(true, Some(panel_state)),
            Some("panel_not_key_window")
        );
        assert_eq!(MACOS_LAUNCHER_PANEL_LEVEL, PanelLevel::Status);
    }
}
