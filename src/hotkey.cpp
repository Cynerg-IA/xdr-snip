// hotkey.cpp -- implementation for hotkey.h. Direct port of
// crates/snip-app/src/hotkey.rs's parsing logic; platform registration is
// a declared seam (see hotkey.h file header).

#include "hotkey.h"

#include <algorithm>
#include <cctype>

namespace snip {

namespace {

std::string toLowerAscii(const std::string& s) {
    std::string out = s;
    std::transform(out.begin(), out.end(), out.begin(),
                    [](unsigned char c) { return static_cast<char>(std::tolower(c)); });
    return out;
}

}  // namespace

bool parseKeyCode(const std::string& name, KeyCode* out, std::string* error) {
    const std::string lower = toLowerAscii(name);

    // Function keys.
    if (lower == "f1") { *out = KeyCode::F1; return true; }
    if (lower == "f2") { *out = KeyCode::F2; return true; }
    if (lower == "f3") { *out = KeyCode::F3; return true; }
    if (lower == "f4") { *out = KeyCode::F4; return true; }
    if (lower == "f5") { *out = KeyCode::F5; return true; }
    if (lower == "f6") { *out = KeyCode::F6; return true; }
    if (lower == "f7") { *out = KeyCode::F7; return true; }
    if (lower == "f8") { *out = KeyCode::F8; return true; }
    if (lower == "f9") { *out = KeyCode::F9; return true; }
    if (lower == "f10") { *out = KeyCode::F10; return true; }
    if (lower == "f11") { *out = KeyCode::F11; return true; }
    if (lower == "f12") { *out = KeyCode::F12; return true; }

    // Special keys.
    if (lower == "printscreen" || lower == "print_screen" || lower == "prtsc") {
        *out = KeyCode::PrintScreen;
        return true;
    }
    if (lower == "scrolllock" || lower == "scroll_lock") { *out = KeyCode::ScrollLock; return true; }
    if (lower == "pause") { *out = KeyCode::Pause; return true; }
    if (lower == "insert") { *out = KeyCode::Insert; return true; }
    if (lower == "delete") { *out = KeyCode::Delete; return true; }
    if (lower == "home") { *out = KeyCode::Home; return true; }
    if (lower == "end") { *out = KeyCode::End; return true; }
    if (lower == "pageup" || lower == "page_up") { *out = KeyCode::PageUp; return true; }
    if (lower == "pagedown" || lower == "page_down") { *out = KeyCode::PageDown; return true; }
    if (lower == "escape" || lower == "esc") { *out = KeyCode::Escape; return true; }
    if (lower == "space") { *out = KeyCode::Space; return true; }
    if (lower == "tab") { *out = KeyCode::Tab; return true; }
    if (lower == "enter" || lower == "return") { *out = KeyCode::Enter; return true; }
    if (lower == "backspace") { *out = KeyCode::Backspace; return true; }

    // Letters.
    if (lower == "a") { *out = KeyCode::KeyA; return true; }
    if (lower == "b") { *out = KeyCode::KeyB; return true; }
    if (lower == "c") { *out = KeyCode::KeyC; return true; }
    if (lower == "d") { *out = KeyCode::KeyD; return true; }
    if (lower == "e") { *out = KeyCode::KeyE; return true; }
    if (lower == "f") { *out = KeyCode::KeyF; return true; }
    if (lower == "g") { *out = KeyCode::KeyG; return true; }
    if (lower == "h") { *out = KeyCode::KeyH; return true; }
    if (lower == "i") { *out = KeyCode::KeyI; return true; }
    if (lower == "j") { *out = KeyCode::KeyJ; return true; }
    if (lower == "k") { *out = KeyCode::KeyK; return true; }
    if (lower == "l") { *out = KeyCode::KeyL; return true; }
    if (lower == "m") { *out = KeyCode::KeyM; return true; }
    if (lower == "n") { *out = KeyCode::KeyN; return true; }
    if (lower == "o") { *out = KeyCode::KeyO; return true; }
    if (lower == "p") { *out = KeyCode::KeyP; return true; }
    if (lower == "q") { *out = KeyCode::KeyQ; return true; }
    if (lower == "r") { *out = KeyCode::KeyR; return true; }
    if (lower == "s") { *out = KeyCode::KeyS; return true; }
    if (lower == "t") { *out = KeyCode::KeyT; return true; }
    if (lower == "u") { *out = KeyCode::KeyU; return true; }
    if (lower == "v") { *out = KeyCode::KeyV; return true; }
    if (lower == "w") { *out = KeyCode::KeyW; return true; }
    if (lower == "x") { *out = KeyCode::KeyX; return true; }
    if (lower == "y") { *out = KeyCode::KeyY; return true; }
    if (lower == "z") { *out = KeyCode::KeyZ; return true; }

    // Digits.
    if (lower == "0") { *out = KeyCode::Digit0; return true; }
    if (lower == "1") { *out = KeyCode::Digit1; return true; }
    if (lower == "2") { *out = KeyCode::Digit2; return true; }
    if (lower == "3") { *out = KeyCode::Digit3; return true; }
    if (lower == "4") { *out = KeyCode::Digit4; return true; }
    if (lower == "5") { *out = KeyCode::Digit5; return true; }
    if (lower == "6") { *out = KeyCode::Digit6; return true; }
    if (lower == "7") { *out = KeyCode::Digit7; return true; }
    if (lower == "8") { *out = KeyCode::Digit8; return true; }
    if (lower == "9") { *out = KeyCode::Digit9; return true; }

    if (error != nullptr) {
        *error = "unknown key name: '" + name + "'";
    }
    return false;
}

bool parseModifiers(const std::vector<std::string>& names, HotkeyModifiers* out,
                     std::string* error) {
    HotkeyModifiers mods = kModifierNone;

    for (const std::string& name : names) {
        const std::string lower = toLowerAscii(name);
        if (lower == "alt") {
            mods |= kModifierAlt;
        } else if (lower == "ctrl" || lower == "control") {
            mods |= kModifierControl;
        } else if (lower == "shift") {
            mods |= kModifierShift;
        } else if (lower == "super" || lower == "win" || lower == "meta") {
            mods |= kModifierSuper;
        } else {
            if (error != nullptr) {
                *error = "unknown modifier: '" + name + "'";
            }
            return false;
        }
    }

    *out = mods;
    return true;
}

#ifdef _WIN32
HotkeyResult Win32HotkeyPlatform::registerGlobalHotkey(const ParsedHotkey& /*hotkey*/) {
    // Phase 3b TODO: translate KeyCode -> VK_* and HotkeyModifiers ->
    // MOD_ALT|MOD_CONTROL|MOD_SHIFT|MOD_WIN, then call
    // RegisterHotKey(nullptr, id, fsModifiers | MOD_NOREPEAT, vk) and
    // surface GetLastError() on failure (e.g. ERROR_HOTKEY_ALREADY_REGISTERED).
    // Deliberately unimplemented in Phase 3a -- see hotkey.h file header.
    HotkeyResult result;
    result.ok = false;
    result.error =
        "Win32HotkeyPlatform::registerGlobalHotkey: not yet implemented (Phase 3b TODO -- "
        "RegisterHotKey wiring)";
    return result;
}
#endif  // _WIN32

HotkeyResult RecordingHotkeyPlatform::registerGlobalHotkey(const ParsedHotkey& hotkey) {
    registeredHotkeys.push_back(hotkey);

    HotkeyResult result;
    if (simulateFailure) {
        result.ok = false;
        result.error = "RecordingHotkeyPlatform: simulated registration failure";
        return result;
    }
    result.ok = true;
    result.id = nextId++;
    return result;
}

HotkeyResult registerHotkey(const HotkeyConfig& config, HotkeyPlatform& platform) {
    KeyCode code;
    std::string codeError;
    if (!parseKeyCode(config.key, &code, &codeError)) {
        HotkeyResult result;
        result.ok = false;
        result.error = codeError;
        return result;
    }

    HotkeyModifiers modifiers;
    std::string modError;
    if (!parseModifiers(config.modifiers, &modifiers, &modError)) {
        HotkeyResult result;
        result.ok = false;
        result.error = modError;
        return result;
    }

    ParsedHotkey hotkey;
    hotkey.code = code;
    hotkey.modifiers = modifiers;

    return platform.registerGlobalHotkey(hotkey);
}

}  // namespace snip
