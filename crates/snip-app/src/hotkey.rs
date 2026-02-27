//! Global hotkey registration using the `global-hotkey` crate.
//!
//! Translates user-friendly key names from the config into platform key codes,
//! registers the combination, and returns the manager + hotkey handle so the
//! caller can listen for events and clean up on exit.

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::GlobalHotKeyManager;
use snip_types::{HotkeyConfig, SnipError};
use tracing::{debug, info};

/// Registers a global hotkey based on the user's configuration.
///
/// Returns the [`GlobalHotKeyManager`] (must be kept alive) and the [`HotKey`]
/// descriptor so the caller can match incoming events by ID.
pub fn register_hotkey(
    config: &HotkeyConfig,
) -> Result<(GlobalHotKeyManager, HotKey), SnipError> {
    debug!(
        "register_hotkey: key={}, modifiers={:?}",
        config.key, config.modifiers
    );

    let code = parse_key_code(&config.key)?;
    let modifiers = parse_modifiers(&config.modifiers)?;

    let hotkey = HotKey::new(Some(modifiers), code);

    debug!(
        "register_hotkey: parsed hotkey id={}, code={:?}, mods={:?}",
        hotkey.id(),
        code,
        modifiers
    );

    let manager = GlobalHotKeyManager::new().map_err(|e| {
        SnipError::HotkeyRegistration(format!("failed to create hotkey manager: {}", e))
    })?;

    manager.register(hotkey).map_err(|e| {
        SnipError::HotkeyRegistration(format!(
            "failed to register hotkey '{}': {}",
            config.key, e
        ))
    })?;

    info!(
        "register_hotkey: registered global hotkey — key={}, modifiers={:?}, id={}",
        config.key,
        config.modifiers,
        hotkey.id()
    );

    Ok((manager, hotkey))
}

/// Translates a human-readable key name (from config) to a [`Code`] enum variant.
///
/// Supports common key names: letters, digits, function keys, PrintScreen, etc.
fn parse_key_code(name: &str) -> Result<Code, SnipError> {
    let code = match name.to_lowercase().as_str() {
        // Function keys
        "f1" => Code::F1,
        "f2" => Code::F2,
        "f3" => Code::F3,
        "f4" => Code::F4,
        "f5" => Code::F5,
        "f6" => Code::F6,
        "f7" => Code::F7,
        "f8" => Code::F8,
        "f9" => Code::F9,
        "f10" => Code::F10,
        "f11" => Code::F11,
        "f12" => Code::F12,

        // Special keys
        "printscreen" | "print_screen" | "prtsc" => Code::PrintScreen,
        "scrolllock" | "scroll_lock" => Code::ScrollLock,
        "pause" => Code::Pause,
        "insert" => Code::Insert,
        "delete" => Code::Delete,
        "home" => Code::Home,
        "end" => Code::End,
        "pageup" | "page_up" => Code::PageUp,
        "pagedown" | "page_down" => Code::PageDown,
        "escape" | "esc" => Code::Escape,
        "space" => Code::Space,
        "tab" => Code::Tab,
        "enter" | "return" => Code::Enter,
        "backspace" => Code::Backspace,

        // Letters
        "a" => Code::KeyA,
        "b" => Code::KeyB,
        "c" => Code::KeyC,
        "d" => Code::KeyD,
        "e" => Code::KeyE,
        "f" => Code::KeyF,
        "g" => Code::KeyG,
        "h" => Code::KeyH,
        "i" => Code::KeyI,
        "j" => Code::KeyJ,
        "k" => Code::KeyK,
        "l" => Code::KeyL,
        "m" => Code::KeyM,
        "n" => Code::KeyN,
        "o" => Code::KeyO,
        "p" => Code::KeyP,
        "q" => Code::KeyQ,
        "r" => Code::KeyR,
        "s" => Code::KeyS,
        "t" => Code::KeyT,
        "u" => Code::KeyU,
        "v" => Code::KeyV,
        "w" => Code::KeyW,
        "x" => Code::KeyX,
        "y" => Code::KeyY,
        "z" => Code::KeyZ,

        // Digits
        "0" => Code::Digit0,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        "3" => Code::Digit3,
        "4" => Code::Digit4,
        "5" => Code::Digit5,
        "6" => Code::Digit6,
        "7" => Code::Digit7,
        "8" => Code::Digit8,
        "9" => Code::Digit9,

        other => {
            return Err(SnipError::HotkeyRegistration(format!(
                "unknown key name: '{}'",
                other
            )));
        }
    };

    debug!("parse_key_code: '{}' -> {:?}", name, code);
    Ok(code)
}

/// Translates a list of modifier name strings into a combined [`Modifiers`] bitflag.
///
/// Supported names (case-insensitive): Alt, Ctrl/Control, Shift, Super/Win/Meta.
fn parse_modifiers(names: &[String]) -> Result<Modifiers, SnipError> {
    let mut mods = Modifiers::empty();

    for name in names {
        let modifier = match name.to_lowercase().as_str() {
            "alt" => Modifiers::ALT,
            "ctrl" | "control" => Modifiers::CONTROL,
            "shift" => Modifiers::SHIFT,
            "super" | "win" | "meta" => Modifiers::SUPER,
            other => {
                return Err(SnipError::HotkeyRegistration(format!(
                    "unknown modifier: '{}'",
                    other
                )));
            }
        };
        mods |= modifier;
    }

    debug!("parse_modifiers: {:?} -> {:?}", names, mods);
    Ok(mods)
}
