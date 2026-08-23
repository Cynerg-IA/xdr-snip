// hotkey.h -- C++ port of crates/snip-app/src/hotkey.rs (v0.5.0 Rust
// source), for v0.6.0 Phase 3a.
//
// Rust used the `global-hotkey` crate, which internally wraps
// RegisterHotKey on Windows (and X11/other backends elsewhere) -- there is
// no C++ equivalent crate. Per the Phase 3a task split:
//   - parseKeyCode() / parseModifiers(): the key-name/modifier-name ->
//     platform-code translation tables are PURE LOGIC (no Win32/WinRT
//     calls) and are ported faithfully, 1:1 with hotkey.rs's match arms.
//   - The actual OS-level registration (Rust's GlobalHotKeyManager::new()
//     + manager.register(hotkey)) has NO portable equivalent. This is
//     wired behind an abstract PLATFORM SEAM (HotkeyPlatform) so:
//       * registerHotkey() ports the Rust function's CONTROL FLOW exactly
//         (parse key -> parse modifiers -> build descriptor -> call the
//         platform to register -> log/return), but delegates the actual
//         "make this a real OS hotkey" step to a HotkeyPlatform
//         implementation.
//       * #ifdef _WIN32: a real implementation (Win32Win32HotkeyPlatform)
//         is DECLARED here with its RegisterHotKey/UnregisterHotKey calls
//         left as Phase 3b TODOs (implementation body throws/returns a
//         "not yet implemented" error) -- the header shape is real, the
//         guts are not, and this is explicitly flagged, not silently
//         faked.
//       * Linux/other: a NullHotkeyPlatform test double is provided so the
//         portable logic (key/modifier parsing, code structure) can be
//         compiled and unit-tested on Linux CI without pretending a
//         non-Windows global hotkey actually works.

#pragma once

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

#include "types.h"

namespace snip {

// ======================== KEY / MODIFIER CODES ========================

// Mirrors global_hotkey::hotkey::Code's variants actually used by
// hotkey.rs's parse_key_code() match arms (function keys F1-F12, a
// curated set of special keys, letters A-Z, digits 0-9). This is a
// closed, explicit enum -- NOT a passthrough of any platform virtual-key
// header -- so parseKeyCode() stays platform-free. The exact identifiers
// mirror global-hotkey's `Code` enum member names (KeyA not just A, etc.)
// so a future platform implementation can map 1:1 to
// global-hotkey's own VK_* / X11 keysym tables if useful as a reference.
enum class KeyCode {
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    PrintScreen, ScrollLock, Pause, Insert, Delete, Home, End,
    PageUp, PageDown, Escape, Space, Tab, Enter, Backspace,
    KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL,
    KeyM, KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX,
    KeyY, KeyZ,
    Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
};

// Mirrors global_hotkey::hotkey::Modifiers as a bitmask. Only the four
// modifiers hotkey.rs's parse_modifiers() actually recognizes (Alt,
// Control, Shift, Super/Win/Meta) are represented.
enum HotkeyModifierBits : std::uint32_t {
    kModifierNone = 0,
    kModifierAlt = 1u << 0,
    kModifierControl = 1u << 1,
    kModifierShift = 1u << 2,
    kModifierSuper = 1u << 3,
};
using HotkeyModifiers = std::uint32_t;

// A parsed, platform-independent hotkey descriptor -- the C++ analogue of
// Rust's `HotKey` (global_hotkey::hotkey::HotKey), reduced to the fields
// this port actually needs (id/code/modifiers). Rust's HotKey::id() is a
// hash of (modifiers, code); this port assigns id at registration time via
// the platform implementation instead of reimplementing global-hotkey's
// internal hashing (out of scope -- no caller in this phase inspects a
// specific id value, only that ids returnd for different configs are
// distinguishable, which the platform seam is free to guarantee however it
// likes).
struct ParsedHotkey {
    KeyCode code = KeyCode::PrintScreen;
    HotkeyModifiers modifiers = kModifierNone;
};

// ======================== RESULT ========================

// Result of registerHotkey(). Matches the JxlEncodeResult /
// CaptureResult convention used elsewhere in this port (Result-struct, not
// exception) -- registerHotkey() is called once at startup on the hot
// "did the app come up correctly" path where the caller wants a value to
// check, not an exception to catch.
struct HotkeyResult {
    bool ok = false;
    std::uint32_t id = 0;  // Platform-assigned id, valid only when ok == true.
    // Human-readable description of what failed. Always non-empty when
    // ok == false; always empty when ok == true.
    std::string error;
};

// ======================== PURE LOGIC (ported 1:1 from hotkey.rs) ========================

// Translates a human-readable key name (from config, case-insensitive) to
// a KeyCode. Direct port of hotkey.rs's parse_key_code() match arms
// (including all name aliases: "printscreen"/"print_screen"/"prtsc",
// "esc"/"escape", "return"/"enter", etc). Returns false (leaving *out
// unmodified) for an unrecognized name, mirroring Rust's
// Err(SnipError::HotkeyRegistration(...)) path -- callers get the same
// "unknown key name: '<name>'" message via the `error` out-parameter.
bool parseKeyCode(const std::string& name, KeyCode* out, std::string* error);

// Translates a list of modifier name strings (case-insensitive) into a
// combined HotkeyModifiers bitmask. Direct port of hotkey.rs's
// parse_modifiers() (Alt, Ctrl/Control, Shift, Super/Win/Meta). Returns
// false (leaving *out unmodified) on the first unrecognized modifier name,
// mirroring Rust's early-return Err.
bool parseModifiers(const std::vector<std::string>& names, HotkeyModifiers* out,
                     std::string* error);

// ======================== PLATFORM SEAM ========================

// Abstract interface for the OS-level "make this key combination a real
// global hotkey" operation that Rust's `global-hotkey` crate performed via
// GlobalHotKeyManager::new() + manager.register(hotkey). There is no
// portable C++ equivalent (RegisterHotKey is Win32-only; X11/Wayland have
// their own, different mechanisms) -- this interface is the seam Phase 3b
// implements for real on Windows.
class HotkeyPlatform {
public:
    virtual ~HotkeyPlatform() = default;

    // Registers `hotkey` as a system-wide hotkey. Returns an id the caller
    // can later use to match incoming hotkey-pressed events (mirrors
    // Rust's HotKey::id()). Implementations must be idempotent-safe to
    // call once per process lifetime per hotkey (registerHotkey() below
    // calls this exactly once).
    virtual HotkeyResult registerGlobalHotkey(const ParsedHotkey& hotkey) = 0;
};

#ifdef _WIN32
// Real Windows implementation seam. NOT implemented in Phase 3a -- this
// class is DECLARED so Phase 3b can fill in registerGlobalHotkey() with
// the actual RegisterHotKey(HWND, id, fsModifiers, vk) call (translating
// KeyCode/HotkeyModifiers to VK_* / MOD_* constants) without changing this
// header's public shape. Calling it today returns ok=false with an
// explicit "not yet implemented" error -- it does NOT pretend to succeed.
// See hotkey.cpp for the stub body.
class Win32HotkeyPlatform : public HotkeyPlatform {
public:
    HotkeyResult registerGlobalHotkey(const ParsedHotkey& hotkey) override;
};
#endif  // _WIN32

// Test double for non-Windows builds (and for Phase 3a's own unit tests on
// any platform): records every registration attempt it receives instead of
// touching the OS, so tests can assert the portable logic in
// registerHotkey() below calls the platform seam with the correctly
// parsed key+modifiers, without requiring a real windowing system. This is
// NOT a "fake success" for production use -- see capture.h/tray.h's
// equivalent doubles for the same convention across this phase's three
// thin-wrapper files.
class RecordingHotkeyPlatform : public HotkeyPlatform {
public:
    HotkeyResult registerGlobalHotkey(const ParsedHotkey& hotkey) override;

    // Test inspection: every hotkey this double was asked to register, in
    // call order.
    std::vector<ParsedHotkey> registeredHotkeys;
    // When true, registerGlobalHotkey() returns ok=false (simulates a
    // platform-level registration failure, e.g. the combination is already
    // taken by another application).
    bool simulateFailure = false;
    std::uint32_t nextId = 1;
};

// ======================== PUBLIC API ========================

// Registers a global hotkey based on the user's configuration. Direct
// port of hotkey.rs's register_hotkey() control flow: parse key -> parse
// modifiers -> build descriptor -> register via `platform`. `platform`
// defaults to a Win32HotkeyPlatform on Windows builds (still Phase 3b
// TODO-bodied) or must be supplied explicitly (e.g.
// RecordingHotkeyPlatform) on non-Windows builds / in tests.
HotkeyResult registerHotkey(const HotkeyConfig& config, HotkeyPlatform& platform);

}  // namespace snip
