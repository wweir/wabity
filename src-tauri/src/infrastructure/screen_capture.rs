use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::oneshot;

pub const SCREEN_CAPTURE_OVERLAY_WINDOW_LABEL: &str = "screenshot-capture-overlay";
const MIN_REGION_SIZE: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenCaptureBackend {
    ScreenCaptureKit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenCaptureMode {
    Region,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenCaptureRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone)]
pub struct ScreenCaptureResult {
    pub image_path: PathBuf,
    pub backend: ScreenCaptureBackend,
    pub mode: ScreenCaptureMode,
    pub rect: ScreenCaptureRect,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenCaptureRectPayload {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy)]
struct OverlayGeometry {
    position: LogicalPosition<f64>,
    size: LogicalSize<f64>,
}

enum RegionSelectionOutcome {
    Completed(ScreenCaptureRect),
    Cancelled,
}

type RegionSelectionSender = oneshot::Sender<RegionSelectionOutcome>;
type PendingRegionSelections = Arc<StdMutex<HashMap<String, RegionSelectionSender>>>;

fn pending_region_selections() -> &'static PendingRegionSelections {
    static PENDING_REGION_SELECTIONS: OnceLock<PendingRegionSelections> = OnceLock::new();
    PENDING_REGION_SELECTIONS.get_or_init(|| Arc::new(StdMutex::new(HashMap::new())))
}

pub async fn capture_user_selected_region(app: &AppHandle) -> Result<Option<ScreenCaptureResult>> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        bail!("interactive screenshot OCR is only implemented on macOS")
    }

    #[cfg(target_os = "macos")]
    {
        let geometry = overlay_geometry(app)?;
        let token = next_region_selection_token();
        let (tx, rx) = oneshot::channel();
        pending_region_selections()
            .lock()
            .expect("pending region selections mutex should not be poisoned")
            .insert(token.clone(), tx);

        match open_overlay_window(app, &token, geometry) {
            Ok(()) => {}
            Err(error) => {
                pending_region_selections()
                    .lock()
                    .expect("pending region selections mutex should not be poisoned")
                    .remove(&token);
                return Err(error);
            }
        };

        let outcome = rx
            .await
            .context("failed to receive screen capture region selection")?;
        close_overlay_window(app);

        let selected_rect = match outcome {
            RegionSelectionOutcome::Completed(rect) => rect,
            RegionSelectionOutcome::Cancelled => return Ok(None),
        };
        validate_region_rect(selected_rect)?;

        let rect = ScreenCaptureRect {
            x: geometry.position.x + selected_rect.x,
            y: geometry.position.y + selected_rect.y,
            width: selected_rect.width,
            height: selected_rect.height,
        };
        let image_path = capture_region_with_screen_capture_kit(rect).await?;

        Ok(Some(ScreenCaptureResult {
            image_path,
            backend: ScreenCaptureBackend::ScreenCaptureKit,
            mode: ScreenCaptureMode::Region,
            rect,
        }))
    }
}

pub async fn complete_region_selection(
    token: String,
    rect: ScreenCaptureRectPayload,
) -> Result<()> {
    let rect = ScreenCaptureRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    };
    validate_region_rect(rect)?;
    resolve_region_selection(token, RegionSelectionOutcome::Completed(rect)).await
}

pub async fn cancel_region_selection(token: String) -> Result<()> {
    resolve_region_selection(token, RegionSelectionOutcome::Cancelled).await
}

pub fn remove_screenshot_file(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        tracing::debug!(
            error = format_args!("{:#}", error),
            image_path = %path.display(),
            "failed to remove screenshot review image file"
        );
    }
}

async fn resolve_region_selection(token: String, outcome: RegionSelectionOutcome) -> Result<()> {
    let sender = pending_region_selections()
        .lock()
        .expect("pending region selections mutex should not be poisoned")
        .remove(token.trim())
        .ok_or_else(|| anyhow!("screen capture region selection token does not exist"))?;
    sender
        .send(outcome)
        .map_err(|_| anyhow!("screen capture region selection receiver is closed"))
}

fn validate_region_rect(rect: ScreenCaptureRect) -> Result<()> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
    {
        bail!("screen capture region contains non-finite coordinates");
    }
    if rect.width < MIN_REGION_SIZE || rect.height < MIN_REGION_SIZE {
        bail!("screen capture region is too small");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn overlay_geometry(app: &AppHandle) -> Result<OverlayGeometry> {
    let cursor = app
        .cursor_position()
        .context("failed to read cursor position for screen capture overlay")?;
    let monitor = app
        .monitor_from_point(cursor.x, cursor.y)
        .context("failed to resolve monitor under cursor for screen capture overlay")?
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or_else(|| anyhow!("no monitor available for screen capture overlay"))?;
    let scale = monitor.scale_factor();
    let position = monitor.position().to_logical::<f64>(scale);
    let size = monitor.size().to_logical::<f64>(scale);

    Ok(OverlayGeometry { position, size })
}

#[cfg(target_os = "macos")]
fn open_overlay_window(app: &AppHandle, token: &str, geometry: OverlayGeometry) -> Result<()> {
    close_overlay_window(app);

    let route = format!("index.html?token={token}");
    let window = WebviewWindowBuilder::new(
        app,
        SCREEN_CAPTURE_OVERLAY_WINDOW_LABEL,
        WebviewUrl::App(route.into()),
    )
    .title("Wabity Screen Capture")
    .position(geometry.position.x, geometry.position.y)
    .inner_size(geometry.size.width, geometry.size.height)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .skip_taskbar(true)
    .resizable(false)
    .focused(true)
    .shadow(false)
    .build()
    .context("failed to open screen capture overlay window")?;

    let cancel_token = token.to_string();
    window.on_window_event(move |event| {
        if matches!(
            event,
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
        ) {
            cancel_pending_region_selection(&cancel_token);
        }
    });
    Ok(())
}

fn cancel_pending_region_selection(token: &str) {
    let sender = pending_region_selections()
        .lock()
        .expect("pending region selections mutex should not be poisoned")
        .remove(token);
    if let Some(sender) = sender {
        let _ = sender.send(RegionSelectionOutcome::Cancelled);
    }
}

fn close_overlay_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SCREEN_CAPTURE_OVERLAY_WINDOW_LABEL) {
        if let Err(error) = window.destroy() {
            tracing::debug!(
                error = format_args!("{:#}", error),
                "failed to close screen capture overlay window"
            );
        }
    }
}

fn next_region_selection_token() -> String {
    static REGION_SELECTION_COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);

    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let sequence = REGION_SELECTION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("scr-{timestamp_ms}-{sequence}")
}

fn next_screenshot_path() -> Result<PathBuf> {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let directory = std::env::temp_dir()
        .join("wabity")
        .join("screenshot-review");
    std::fs::create_dir_all(&directory).with_context(|| {
        format!(
            "failed to create screenshot review temp directory: {}",
            directory.display()
        )
    })?;
    Ok(directory.join(format!("wabity-screenshot-review-{timestamp_ms}.png")))
}

#[cfg(target_os = "macos")]
async fn capture_region_with_screen_capture_kit(rect: ScreenCaptureRect) -> Result<PathBuf> {
    tokio::task::spawn_blocking(move || capture_region_with_screen_capture_kit_blocking(rect))
        .await
        .context("failed to join ScreenCaptureKit screenshot task")?
}

#[cfg(target_os = "macos")]
fn capture_region_with_screen_capture_kit_blocking(rect: ScreenCaptureRect) -> Result<PathBuf> {
    use std::time::Duration;

    use block2::RcBlock;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::NSError;
    use objc2_screen_capture_kit::{
        SCScreenshotConfiguration, SCScreenshotDynamicRange, SCScreenshotManager,
        SCScreenshotOutput,
    };

    let output_path = next_screenshot_path()?;
    let output_path_for_capture = output_path.clone();
    let config = unsafe { SCScreenshotConfiguration::new() };
    let cg_rect = CGRect::new(
        CGPoint::new(rect.x, rect.y),
        CGSize::new(rect.width, rect.height),
    );

    unsafe {
        config.setShowsCursor(false);
        config.setDynamicRange(SCScreenshotDynamicRange::SDR);
    }

    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let tx = StdMutex::new(Some(tx));
    let block = RcBlock::new(
        move |output: *mut SCScreenshotOutput, error: *mut NSError| {
            let result = if !error.is_null() {
                let description = unsafe { &*error }.localizedDescription().to_string();
                Err(format!("ScreenCaptureKit screenshot failed: {description}"))
            } else if output.is_null() {
                Err("ScreenCaptureKit returned no screenshot output".to_string())
            } else {
                let screenshot = unsafe { &*output };
                match unsafe { screenshot.sdrImage() }.or_else(|| unsafe { screenshot.hdrImage() })
                {
                    Some(image) => persist_png_image(&image, &output_path_for_capture)
                        .map_err(|error| format!("failed to write screenshot PNG: {error:#}")),
                    None => {
                        Err("ScreenCaptureKit returned no SDR/HDR image to persist".to_string())
                    }
                }
            };

            if let Some(sender) = tx.lock().ok().and_then(|mut guard| guard.take()) {
                let _ = sender.send(result);
            }
        },
    );

    unsafe {
        SCScreenshotManager::captureScreenshotWithRect_configuration_completionHandler(
            cg_rect,
            &config,
            Some(&block),
        );
    }

    rx.recv_timeout(Duration::from_secs(30))
        .context("failed to receive ScreenCaptureKit screenshot result")?
        .map_err(anyhow::Error::msg)?;

    let metadata = output_path.metadata().with_context(|| {
        format!(
            "ScreenCaptureKit did not write screenshot image: {}",
            output_path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        bail!("ScreenCaptureKit did not write screenshot image");
    }

    Ok(output_path)
}

#[cfg(target_os = "macos")]
fn persist_png_image(image: &objc2_core_graphics::CGImage, output_path: &Path) -> Result<()> {
    use objc2_core_foundation::CFMutableData;
    use objc2_image_io::CGImageDestination;
    use objc2_uniform_type_identifiers::UTTypePNG;

    let image_data = CFMutableData::new(None, 0)
        .ok_or_else(|| anyhow!("failed to allocate ImageIO screenshot buffer"))?;
    let png_type = unsafe { UTTypePNG.identifier() };
    let destination =
        unsafe { CGImageDestination::with_data(image_data.as_ref(), png_type.as_ref(), 1, None) }
            .ok_or_else(|| anyhow!("failed to create ImageIO PNG encoder for screenshot"))?;

    unsafe {
        destination.add_image(image, None);
    }
    if !unsafe { destination.finalize() } {
        bail!("ImageIO failed to encode screenshot PNG");
    }

    let png_bytes = image_data.to_vec();
    if png_bytes.is_empty() {
        bail!("ImageIO encoded an empty screenshot PNG");
    }
    std::fs::write(output_path, png_bytes).with_context(|| {
        format!(
            "failed to write screenshot PNG file: {}",
            output_path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{validate_region_rect, ScreenCaptureRect};

    #[test]
    fn region_rect_rejects_non_finite_coordinates() {
        assert!(validate_region_rect(ScreenCaptureRect {
            x: f64::NAN,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        })
        .is_err());
    }

    #[test]
    fn region_rect_rejects_empty_selection() {
        assert!(validate_region_rect(ScreenCaptureRect {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: 10.0,
        })
        .is_err());
    }

    #[test]
    fn region_rect_accepts_valid_selection() {
        validate_region_rect(ScreenCaptureRect {
            x: 1.0,
            y: 2.0,
            width: 120.0,
            height: 80.0,
        })
        .expect("valid region should be accepted");
    }
}
