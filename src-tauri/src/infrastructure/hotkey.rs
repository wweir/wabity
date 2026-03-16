use anyhow::{Context, Result};
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

/// Parse a shortcut string like "Cmd+Shift+Space" or "Ctrl+Alt+A" into a Shortcut
pub fn parse_shortcut(shortcut_str: &str) -> Option<Shortcut> {
    let parts: Vec<&str> = shortcut_str.split('+').collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers = Modifiers::empty();
    let mut key_code = None;

    for part in parts {
        let part = part.trim();
        match part {
            "Cmd" | "Super" | "Meta" | "Command" => {
                #[cfg(target_os = "macos")]
                {
                    modifiers |= Modifiers::SUPER;
                }
                #[cfg(not(target_os = "macos"))]
                {
                    modifiers |= Modifiers::CONTROL;
                }
            }
            "Ctrl" | "Control" => modifiers |= Modifiers::CONTROL,
            "Alt" | "Option" => modifiers |= Modifiers::ALT,
            "Shift" => modifiers |= Modifiers::SHIFT,
            key => {
                key_code = parse_key_code(key);
            }
        }
    }

    key_code.map(|code| {
        if modifiers.is_empty() {
            Shortcut::new(None, code)
        } else {
            Shortcut::new(Some(modifiers), code)
        }
    })
}

fn parse_key_code(key: &str) -> Option<Code> {
    let key = key.to_uppercase();
    match key.as_str() {
        "SPACE" | " " => Some(Code::Space),
        "ENTER" | "RETURN" => Some(Code::Enter),
        "ESC" | "ESCAPE" => Some(Code::Escape),
        "TAB" => Some(Code::Tab),
        "BACKSPACE" => Some(Code::Backspace),
        "DELETE" | "DEL" => Some(Code::Delete),
        "UP" | "ARROWUP" => Some(Code::ArrowUp),
        "DOWN" | "ARROWDOWN" => Some(Code::ArrowDown),
        "LEFT" | "ARROWLEFT" => Some(Code::ArrowLeft),
        "RIGHT" | "ARROWRIGHT" => Some(Code::ArrowRight),
        "HOME" => Some(Code::Home),
        "END" => Some(Code::End),
        "PAGEUP" => Some(Code::PageUp),
        "PAGEDOWN" => Some(Code::PageDown),
        "F1" => Some(Code::F1),
        "F2" => Some(Code::F2),
        "F3" => Some(Code::F3),
        "F4" => Some(Code::F4),
        "F5" => Some(Code::F5),
        "F6" => Some(Code::F6),
        "F7" => Some(Code::F7),
        "F8" => Some(Code::F8),
        "F9" => Some(Code::F9),
        "F10" => Some(Code::F10),
        "F11" => Some(Code::F11),
        "F12" => Some(Code::F12),
        "0" => Some(Code::Digit0),
        "1" => Some(Code::Digit1),
        "2" => Some(Code::Digit2),
        "3" => Some(Code::Digit3),
        "4" => Some(Code::Digit4),
        "5" => Some(Code::Digit5),
        "6" => Some(Code::Digit6),
        "7" => Some(Code::Digit7),
        "8" => Some(Code::Digit8),
        "9" => Some(Code::Digit9),
        "A" => Some(Code::KeyA),
        "B" => Some(Code::KeyB),
        "C" => Some(Code::KeyC),
        "D" => Some(Code::KeyD),
        "E" => Some(Code::KeyE),
        "F" => Some(Code::KeyF),
        "G" => Some(Code::KeyG),
        "H" => Some(Code::KeyH),
        "I" => Some(Code::KeyI),
        "J" => Some(Code::KeyJ),
        "K" => Some(Code::KeyK),
        "L" => Some(Code::KeyL),
        "M" => Some(Code::KeyM),
        "N" => Some(Code::KeyN),
        "O" => Some(Code::KeyO),
        "P" => Some(Code::KeyP),
        "Q" => Some(Code::KeyQ),
        "R" => Some(Code::KeyR),
        "S" => Some(Code::KeyS),
        "T" => Some(Code::KeyT),
        "U" => Some(Code::KeyU),
        "V" => Some(Code::KeyV),
        "W" => Some(Code::KeyW),
        "X" => Some(Code::KeyX),
        "Y" => Some(Code::KeyY),
        "Z" => Some(Code::KeyZ),
        "." | "PERIOD" => Some(Code::Period),
        "," | "COMMA" => Some(Code::Comma),
        "/" | "SLASH" => Some(Code::Slash),
        ";" | "SEMICOLON" => Some(Code::Semicolon),
        "'" | "QUOTE" => Some(Code::Quote),
        "[" | "BRACKETLEFT" => Some(Code::BracketLeft),
        "]" | "BRACKETRIGHT" => Some(Code::BracketRight),
        "\\" | "BACKSLASH" => Some(Code::Backslash),
        "-" | "MINUS" => Some(Code::Minus),
        "=" | "EQUAL" => Some(Code::Equal),
        "`" | "BACKQUOTE" => Some(Code::Backquote),
        _ => None,
    }
}

pub fn register_shortcut(app: &AppHandle, shortcut: Shortcut) -> Result<()> {
    app.global_shortcut()
        .register(shortcut)
        .with_context(|| format!("failed to register launcher shortcut: {shortcut}"))?;
    Ok(())
}

pub fn unregister_shortcut(app: &AppHandle, shortcut: Shortcut) -> Result<()> {
    app.global_shortcut()
        .unregister(shortcut)
        .with_context(|| format!("failed to unregister launcher shortcut: {shortcut}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_shortcut;

    #[test]
    fn parse_shortcut_rejects_modifier_only_value() {
        assert!(parse_shortcut("Shift+Shift").is_none());
        assert!(parse_shortcut("Cmd").is_none());
    }

    #[test]
    fn parse_shortcut_accepts_modifier_with_key() {
        assert!(parse_shortcut("Cmd+Shift+Space").is_some());
        assert!(parse_shortcut("Ctrl+K").is_some());
    }
}
