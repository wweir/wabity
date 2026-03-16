use anyhow::{Context, Result};
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard};
#[cfg(target_os = "macos")]
use std::thread;
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use objc2_app_kit::NSPasteboard;

#[cfg(target_os = "macos")]
const COPY_SETTLE_DELAY_MS: u64 = 20;
#[cfg(target_os = "macos")]
const COPY_SETTLE_POLL_ATTEMPTS: usize = 5;

/// Get the currently selected text by simulating a copy operation.
/// This temporarily saves the clipboard, copies the selection, reads it,
/// then restores the original clipboard content.
pub fn get_selected_text() -> Result<Option<String>> {
    let mut clipboard = Clipboard::new().context("failed to access clipboard")?;

    // Save current clipboard content
    let original_text = clipboard.get_text().ok();
    let original_image = clipboard.get_image().ok();

    #[cfg(target_os = "macos")]
    let baseline_change_count = current_pasteboard_change_count();

    // Simulate copy command (Cmd+C on macOS, Ctrl+C on others)
    copy_selection()?;

    #[cfg(target_os = "macos")]
    let clipboard_updated_by_copy = clipboard_updated_by_copy(baseline_change_count);

    #[cfg(not(target_os = "macos"))]
    let clipboard_updated_by_copy = clipboard_updated_by_copy();

    let selection_result = read_selected_text_after_copy(
        &mut clipboard,
        original_text.as_deref(),
        clipboard_updated_by_copy,
    );

    // Restore original clipboard content
    if let Some(text) = original_text {
        let _ = clipboard.set_text(text);
    } else if let Some(image) = original_image {
        let _ = clipboard.set_image(image);
    }

    selection_result
}

fn read_selected_text_after_copy(
    clipboard: &mut Clipboard,
    original_text: Option<&str>,
    clipboard_updated_by_copy: bool,
) -> Result<Option<String>> {
    let selected_text = clipboard.get_text().ok();
    Ok(finalize_selected_text(
        selected_text,
        original_text,
        clipboard_updated_by_copy,
    ))
}

fn finalize_selected_text(
    selected_text: Option<String>,
    _previous_clipboard_text: Option<&str>,
    clipboard_updated_by_copy: bool,
) -> Option<String> {
    if !clipboard_updated_by_copy {
        return None;
    }

    let selected_text = selected_text.filter(|text| !text.trim().is_empty())?;

    #[cfg(not(target_os = "macos"))]
    if _previous_clipboard_text == Some(selected_text.as_str()) {
        return None;
    }

    Some(selected_text)
}

#[cfg(target_os = "macos")]
fn clipboard_updated_by_copy(baseline_change_count: isize) -> bool {
    for _ in 0..COPY_SETTLE_POLL_ATTEMPTS {
        if current_pasteboard_change_count() != baseline_change_count {
            return true;
        }

        thread::sleep(Duration::from_millis(COPY_SETTLE_DELAY_MS));
    }

    current_pasteboard_change_count() != baseline_change_count
}

#[cfg(not(target_os = "macos"))]
fn clipboard_updated_by_copy() -> bool {
    // Non-macOS platforms currently lack a stable clipboard change counter in this codebase.
    // Reading the clipboard is still useful, but identical text cannot be distinguished here.
    true
}

#[cfg(target_os = "macos")]
fn current_pasteboard_change_count() -> isize {
    NSPasteboard::generalPasteboard().changeCount()
}

#[cfg(target_os = "macos")]
fn copy_selection() -> Result<()> {
    let mut enigo =
        Enigo::new(&enigo::Settings::default()).context("failed to create enigo instance")?;

    // Simulate Command+C (Meta+C)
    enigo.key(Key::Meta, Direction::Press)?;
    enigo.key(Key::Unicode('c'), Direction::Press)?;
    enigo.key(Key::Unicode('c'), Direction::Release)?;
    enigo.key(Key::Meta, Direction::Release)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::finalize_selected_text;

    #[test]
    fn returns_none_when_copy_did_not_update_clipboard() {
        assert_eq!(
            finalize_selected_text(
                Some("stale clipboard".to_string()),
                Some("stale clipboard"),
                false
            ),
            None
        );
    }

    #[test]
    fn returns_text_when_copy_updated_clipboard() {
        assert_eq!(
            finalize_selected_text(Some("selected text".to_string()), Some("clipboard"), true),
            Some("selected text".to_string())
        );
    }

    #[test]
    fn filters_out_blank_selection() {
        assert_eq!(
            finalize_selected_text(Some("   ".to_string()), None, true),
            None
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn copy_selection() -> Result<()> {
    let mut enigo =
        Enigo::new(&enigo::Settings::default()).context("failed to create enigo instance")?;

    // Simulate Ctrl+C
    enigo.key(Key::Control, Direction::Press)?;
    enigo.key(Key::Unicode('c'), Direction::Press)?;
    enigo.key(Key::Unicode('c'), Direction::Release)?;
    enigo.key(Key::Control, Direction::Release)?;

    Ok(())
}
