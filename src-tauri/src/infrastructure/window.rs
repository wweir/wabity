use anyhow::{Context, Result};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow, WindowEvent,
};

use crate::{
    services::application::APPLICATION_CACHE_STALE_AFTER,
    state::{AppState, ShortcutRuntimeState},
};

const SELECTED_TEXT_EVENT: &str = "selected-text";
const OCR_ERROR_EVENT: &str = "ocr-error";
const LAUNCHER_VERTICAL_CENTER_RATIO: f64 = 0.382;

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
    set_default_window_position(&window)?;
    show_window(&window)?;
    order_main_window_front(&window)?;
    launcher_state.mark_launcher_shown();
    launcher_state.set_launcher_visible(true);
    Ok(())
}

pub fn show_main_window_with_text(app: &AppHandle, text: String) -> Result<()> {
    let window = main_window(app)?;
    emit_selected_text(&window, text)?;
    prepare_main_window_for_show(&window)?;
    set_default_window_position(&window)?;
    show_window(&window)?;
    order_main_window_front(&window)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    shortcut_state.mark_launcher_shown();
    shortcut_state.set_launcher_visible(true);
    Ok(())
}

pub fn show_main_window_with_error(app: &AppHandle, error_message: &str) -> Result<()> {
    let window = main_window(app)?;
    window
        .emit(OCR_ERROR_EVENT, error_message)
        .context("failed to emit OCR error event")?;
    prepare_main_window_for_show(&window)?;
    set_default_window_position(&window)?;
    show_window(&window)?;
    order_main_window_front(&window)?;
    let shortcut_state = app.state::<ShortcutRuntimeState>();
    shortcut_state.mark_launcher_shown();
    shortcut_state.set_launcher_visible(true);
    Ok(())
}

pub fn hide_main_window(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;
    hide_window(&window)?;
    app.state::<ShortcutRuntimeState>()
        .set_launcher_visible(false);
    Ok(())
}

pub fn resize_main_window(app: &AppHandle, width: f64, height: f64) -> Result<()> {
    let window = main_window(app)?;
    let should_reposition_default = !app
        .state::<ShortcutRuntimeState>()
        .has_launcher_been_shown();

    #[cfg(target_os = "macos")]
    {
        if has_macos_panel(&window) {
            run_macos_panel_on_main_thread(&window, move |panel| {
                panel.set_content_size(width, height);
            })?;
            if should_reposition_default {
                set_default_window_position(&window)?;
            }
            return Ok(());
        }
    }

    window
        .set_size(LogicalSize::new(width, height))
        .context("failed to resize launcher window")?;
    if should_reposition_default {
        set_default_window_position(&window)?;
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
        compute_default_window_position, LogicalPosition, LogicalSize,
        LAUNCHER_VERTICAL_CENTER_RATIO,
    };

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
}
