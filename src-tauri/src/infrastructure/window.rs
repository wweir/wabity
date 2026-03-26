use anyhow::{Context, Result};
#[cfg(target_os = "macos")]
use std::sync::mpsc;
use std::time::Duration;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow, WindowEvent,
};
use tokio::time::sleep;

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSWindowOcclusionState};

use crate::{
    domain::execution::ExecutionResult,
    services::application::APPLICATION_CACHE_STALE_AFTER,
    state::{AppState, ShortcutRuntimeState},
};

const OCR_CAPTURED_TEXT_EVENT: &str = "ocr-captured-text";
const OCR_ERROR_EVENT: &str = "ocr-error";
const OCR_TRANSLATION_STARTED_EVENT: &str = "ocr-translation-started";
const OCR_TRANSLATION_RESULT_EVENT: &str = "ocr-translation-result";
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
struct OcrCapturedTextPayload {
    source_mode: ShortcutTranslationSourceMode,
    source_text: String,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutTranslationSourceMode {
    Ocr,
    Selection,
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
                let tauri_reported_focus = main_window(&launcher_app_handle)
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
                                    ?error,
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

                    let window = match main_window(&app_handle) {
                        Ok(window) => window,
                        Err(error) => {
                            tracing::warn!(
                                ?error,
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
                                ?error,
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
                        tracing::warn!(?error, "failed to auto-hide launcher window on blur");
                    }
                });
            }
            _ => {}
        }
    });

    Ok(())
}

pub fn toggle_main_window(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;
    let launcher_state = app.state::<ShortcutRuntimeState>();
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
                    ?error,
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

    prepare_main_window_for_show(&window)?;
    launcher_state.cancel_launcher_blur_auto_hide_confirmation();
    launcher_state.set_launcher_blur_auto_hide_enabled(true);
    launcher_state.arm_launcher_resize_reposition(LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD);
    launcher_state
        .arm_launcher_blur_auto_hide_suppression(LAUNCHER_SHOW_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD);
    set_default_window_position(&window)?;
    show_window(&window)?;
    order_main_window_front(&window)?;
    launcher_state.set_launcher_visible(true);
    Ok(())
}

pub fn show_main_window_with_ocr_text(app: &AppHandle, text: String) -> Result<()> {
    let window = main_window(app)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    window
        .emit(
            OCR_CAPTURED_TEXT_EVENT,
            OcrCapturedTextPayload {
                source_mode: ShortcutTranslationSourceMode::Ocr,
                source_text: text,
            },
        )
        .context("failed to emit OCR captured text event")?;
    prepare_main_window_for_show(&window)?;
    shortcut_state.cancel_launcher_blur_auto_hide_confirmation();
    shortcut_state.set_launcher_blur_auto_hide_enabled(true);
    shortcut_state.arm_launcher_resize_reposition(LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD);
    shortcut_state
        .arm_launcher_blur_auto_hide_suppression(LAUNCHER_SHOW_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD);
    set_default_window_position(&window)?;
    show_window(&window)?;
    order_main_window_front(&window)?;
    shortcut_state.set_launcher_visible(true);
    Ok(())
}

pub fn show_main_window_with_error(app: &AppHandle, error_message: &str) -> Result<()> {
    let window = main_window(app)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    window
        .emit(OCR_ERROR_EVENT, error_message)
        .context("failed to emit OCR error event")?;
    prepare_main_window_for_show(&window)?;
    shortcut_state.cancel_launcher_blur_auto_hide_confirmation();
    shortcut_state.set_launcher_blur_auto_hide_enabled(true);
    shortcut_state.arm_launcher_resize_reposition(LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD);
    shortcut_state
        .arm_launcher_blur_auto_hide_suppression(LAUNCHER_SHOW_BLUR_AUTO_HIDE_SUPPRESSION_PERIOD);
    set_default_window_position(&window)?;
    show_window(&window)?;
    order_main_window_front(&window)?;
    shortcut_state.set_launcher_visible(true);
    Ok(())
}

pub fn show_main_window_with_shortcut_translation_started(
    app: &AppHandle,
    source_mode: ShortcutTranslationSourceMode,
    source_text: String,
) -> Result<()> {
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
    set_default_window_position(&window)?;
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

pub fn hide_main_window(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    hide_launcher_window_statefully(&window, shortcut_state.inner(), "explicit_hide")
}

pub fn resize_main_window(app: &AppHandle, width: f64, height: f64) -> Result<()> {
    let window = main_window(app)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    let launcher_visible = shortcut_state.is_launcher_visible();
    let should_reposition_default = shortcut_state.should_reposition_launcher_on_resize();
    let work_area = resolve_window_work_area(&window)?;
    let requested_size = LogicalSize::new(
        width.max(MIN_WINDOW_DIMENSION),
        height.max(MIN_WINDOW_DIMENSION),
    );
    let next_size = work_area
        .map(|bounds| clamp_window_size_to_work_area(requested_size, bounds.size))
        .unwrap_or(requested_size);

    tracing::info!(
        window_label = window.label(),
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

    #[cfg(target_os = "macos")]
    {
        if has_macos_panel(&window) {
            run_macos_panel_on_main_thread(&window, move |panel| {
                panel.set_content_size(next_size.width, next_size.height);
            })?;
            if should_reposition_default {
                set_default_window_position(&window)?;
            } else {
                clamp_window_position_within_work_area(&window, work_area, next_size)?;
            }
            return Ok(());
        }
    }

    window
        .set_size(next_size)
        .context("failed to resize launcher window")?;
    if should_reposition_default {
        set_default_window_position(&window)?;
    } else {
        clamp_window_position_within_work_area(&window, work_area, next_size)?;
    }

    Ok(())
}

fn main_window(app: &AppHandle) -> Result<WebviewWindow> {
    app.get_webview_window("main")
        .context("main launcher window must exist")
}

fn apply_platform_window_behavior(window: &WebviewWindow) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        initialize_macos_panel(window)?;
    }

    Ok(())
}

fn set_default_window_position(window: &WebviewWindow) -> Result<()> {
    let monitor = resolve_target_monitor(window)?;

    let Some(monitor) = monitor else {
        return Ok(());
    };

    let scale_factor = monitor.scale_factor();
    let work_area = monitor.work_area();
    let work_area_position = work_area.position.to_logical::<f64>(scale_factor);
    let work_area_size = work_area.size.to_logical::<f64>(scale_factor);
    let window_size = window
        .inner_size()
        .context("failed to get launcher window size for default position")?
        .to_logical::<f64>(scale_factor);

    let position = compute_default_window_position(work_area_position, work_area_size, window_size);

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
    if window.app_handle().get_webview_panel("main").is_ok() {
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

        let window = match main_window(&app_handle) {
            Ok(window) => window,
            Err(error) => {
                tracing::warn!(
                    ?error,
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
                        ?error,
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
                            ?error,
                            window_label,
                            "failed to inspect macOS panel state after visibility reinforcement"
                        );
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    ?error,
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
        clamp_window_size_to_work_area, compute_default_window_position,
        compute_visible_window_position, LogicalPosition, LogicalSize, OcrCapturedTextPayload,
        OcrTranslationResultPayload, OcrTranslationStartedPayload, ShortcutTranslationSourceMode,
        LAUNCHER_VERTICAL_CENTER_RATIO,
    };
    use crate::domain::execution::ExecutionResult;
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
    fn ocr_captured_text_payload_uses_expected_wire_shape() {
        let payload = serde_json::to_value(OcrCapturedTextPayload {
            source_mode: ShortcutTranslationSourceMode::Ocr,
            source_text: "captured text".to_string(),
        })
        .expect("payload should serialize");

        assert_eq!(
            payload,
            json!({
                "sourceMode": "ocr",
                "sourceText": "captured text",
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
