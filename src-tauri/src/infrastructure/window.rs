use anyhow::{Context, Result};
use std::time::Duration;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow, WindowEvent,
};

use crate::{
    domain::execution::ExecutionResult,
    services::application::APPLICATION_CACHE_STALE_AFTER,
    state::{AppState, ShortcutRuntimeState},
};

const SELECTED_TEXT_EVENT: &str = "selected-text";
const OCR_ERROR_EVENT: &str = "ocr-error";
const OCR_TRANSLATION_STARTED_EVENT: &str = "ocr-translation-started";
const OCR_TRANSLATION_RESULT_EVENT: &str = "ocr-translation-result";
const LAUNCHER_VERTICAL_CENTER_RATIO: f64 = 0.382;
const LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD: Duration = Duration::from_millis(250);
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
    let owned_window = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            if launcher_state.is_transient_window_interaction_active() {
                tracing::debug!("skipping launcher auto-hide during transient window interaction");
                return;
            }

            if let Err(error) = hide_window(&owned_window) {
                tracing::warn!(?error, "failed to auto-hide launcher window on blur");
            } else {
                launcher_state.set_launcher_visible(false);
                launcher_state.clear_launcher_resize_reposition();
            }
        }
    });

    Ok(())
}

pub fn toggle_main_window(app: &AppHandle, selected_text: Option<String>) -> Result<()> {
    let window = main_window(app)?;
    let launcher_state = app.state::<ShortcutRuntimeState>();

    if launcher_state.is_launcher_visible() {
        hide_main_window(app)?;
        return Ok(());
    }

    // Emit the selected text event before showing the window
    if let Some(text) = selected_text {
        emit_selected_text(&window, text)?;
    }

    prepare_main_window_for_show(&window)?;
    launcher_state.arm_launcher_resize_reposition(LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD);
    set_default_window_position(&window)?;
    show_window(&window)?;
    order_main_window_front(&window)?;
    launcher_state.set_launcher_visible(true);
    Ok(())
}

pub fn show_main_window_with_text(app: &AppHandle, text: String) -> Result<()> {
    let window = main_window(app)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    emit_selected_text(&window, text)?;
    prepare_main_window_for_show(&window)?;
    shortcut_state.arm_launcher_resize_reposition(LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD);
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
    shortcut_state.arm_launcher_resize_reposition(LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD);
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
    shortcut_state.arm_launcher_resize_reposition(LAUNCHER_SHOW_RESIZE_REPOSITION_GRACE_PERIOD);
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
    hide_window(&window)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    shortcut_state.set_launcher_visible(false);
    shortcut_state.clear_launcher_resize_reposition();
    Ok(())
}

pub fn resize_main_window(app: &AppHandle, width: f64, height: f64) -> Result<()> {
    let window = main_window(app)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    let should_reposition_default = shortcut_state.should_reposition_launcher_on_resize();
    let work_area = resolve_window_work_area(&window)?;
    let requested_size = LogicalSize::new(
        width.max(MIN_WINDOW_DIMENSION),
        height.max(MIN_WINDOW_DIMENSION),
    );
    let next_size = work_area
        .map(|bounds| clamp_window_size_to_work_area(requested_size, bounds.size))
        .unwrap_or(requested_size);

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

fn emit_selected_text(window: &WebviewWindow, text: String) -> Result<()> {
    window
        .emit(SELECTED_TEXT_EVENT, text)
        .context("failed to emit selected-text event")
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
fn initialize_macos_panel(window: &WebviewWindow) -> Result<()> {
    if window.app_handle().get_webview_panel("main").is_ok() {
        return Ok(());
    }

    let panel = window
        .to_panel::<LauncherPanel>()
        .context("failed to convert launcher window to NSPanel")?;

    panel.set_level(PanelLevel::Status.value());
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
    panel.set_movable_by_window_background(true);

    Ok(())
}

fn show_window(window: &WebviewWindow) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if has_macos_panel(window) {
            run_macos_panel_on_main_thread(window, |panel| {
                panel.show_and_make_key();
            })?;
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

#[cfg(target_os = "macos")]
fn has_macos_panel(window: &WebviewWindow) -> bool {
    window
        .app_handle()
        .get_webview_panel(window.label())
        .is_ok()
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
        compute_visible_window_position, LogicalPosition, LogicalSize, OcrTranslationResultPayload,
        OcrTranslationStartedPayload, ShortcutTranslationSourceMode,
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
}
