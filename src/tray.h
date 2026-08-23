// tray.h -- C++ port of crates/snip-app/src/tray.rs (v0.5.0 Rust source),
// for v0.6.0 Phase 3a.
//
// Rust used the `tray-icon` crate, which internally wraps
// Shell_NotifyIcon + a native context menu on Windows. There is no C++
// equivalent crate. Per the Phase 3a task split:
//   - generateSnipIconRgba() / fillRect(): the 32x32 crop-mark icon pixel
//     generation is PURE LOGIC (no OS calls, just an RGBA buffer) and is
//     ported faithfully, 1:1 with tray.rs's create_snip_icon()/fill_rect().
//   - The actual OS tray icon + native menu creation (Rust's
//     TrayIconBuilder::build(), Menu::new()/append(), Icon::from_rgba())
//     has NO portable equivalent. This is wired behind an abstract
//     PLATFORM SEAM (TrayPlatform) so:
//       * createTray() ports the Rust function's CONTROL FLOW exactly
//         (build menu item id list -> generate the icon RGBA buffer ->
//         hand off to the platform to actually create the tray icon +
//         menu), but delegates the OS-level work to a TrayPlatform
//         implementation.
//       * #ifdef _WIN32: a real implementation (Win32TrayPlatform) is
//         DECLARED here with its Shell_NotifyIcon/menu-creation calls left
//         as a Phase 3b TODO (stub body returns a "not yet implemented"
//         error) -- flagged explicitly, not silently faked.
//       * Linux/other: a RecordingTrayPlatform test double is provided so
//         the portable icon generation and menu-id-list construction can
//         be compiled and unit-tested on Linux CI without a real system
//         tray.

#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace snip {

// ======================== MENU IDS ========================

// IDs for each tray menu item, returned by createTray(). Direct port of
// Rust's TrayMenuIds. In Rust these were opaque strings assigned by the
// tray-icon crate's MenuItem::new()/id(); in this port they are assigned
// by the TrayPlatform implementation at creation time (see TrayPlatform
// below) since there is no portable menu-item-id source.
struct TrayMenuIds {
    std::string screenshot;
    std::string openFolder;
    std::string settings;
    std::string quit;
};

// ======================== RESULT ========================

// Result of createTray(). Matches the JxlEncodeResult / CaptureResult /
// HotkeyResult / ClipboardResult convention used elsewhere in this port.
struct TrayResult {
    bool ok = false;
    TrayMenuIds ids;  // Valid only when ok == true.
    // Human-readable description of what failed. Always non-empty when
    // ok == false; always empty when ok == true.
    std::string error;
};

// ======================== PURE LOGIC (ported 1:1 from tray.rs) ========================

// Width and height of the tray icon in pixels. Same constant as Rust's
// ICON_SIZE.
extern const std::uint32_t kTrayIconSize;

// Fills a rectangle in an RGBA pixel buffer. Direct port of tray.rs's
// fill_rect() -- same bounds check (px/py < stride), same per-pixel byte
// layout (4 bytes/pixel, RGBA order).
void fillRect(std::vector<std::uint8_t>* rgba, std::uint32_t stride, std::uint32_t x,
              std::uint32_t y, std::uint32_t w, std::uint32_t h, std::uint8_t r, std::uint8_t g,
              std::uint8_t b, std::uint8_t a);

// Generates a kTrayIconSize x kTrayIconSize RGBA pixel buffer containing
// four crop-mark corner brackets (crop handles) on a transparent
// background -- direct port of tray.rs's create_snip_icon() (same arm
// length=9, thickness=2, margin=4 constants; same white-on-transparent
// color choice; same eight fillRect() calls for the four L-shaped
// brackets). Exposed here (Rust kept it private to tray.rs, returning a
// crate-internal `Icon` object) as a raw RGBA byte buffer so Phase 3a's
// tests can pin the pixel contract directly, and so a Phase 3b
// Win32TrayPlatform can pass the same buffer straight into
// CreateIconFromResourceEx / a 32-bit DIB without this function needing to
// know about HICON at all.
std::vector<std::uint8_t> generateSnipIconRgba();

// ======================== PLATFORM SEAM ========================

// Abstract interface for the OS-level "create a system tray icon with this
// context menu" operation that Rust's `tray-icon` crate performed via
// TrayIconBuilder::build() (backed by Shell_NotifyIcon on Windows). There
// is no portable C++ equivalent -- this interface is the seam Phase 3b
// implements for real on Windows.
class TrayPlatform {
public:
    virtual ~TrayPlatform() = default;

    // Creates the tray icon + context menu (layout: "Take Screenshot",
    // separator, "Open Folder", "Settings", separator, "Quit" -- same
    // order as tray.rs's create_tray()) using `iconRgba` (kTrayIconSize x
    // kTrayIconSize, 4 bytes/pixel) as the icon image and `tooltip` as the
    // hover tooltip text. Returns the assigned TrayMenuIds on success.
    virtual TrayResult createTrayIcon(const std::vector<std::uint8_t>& iconRgba,
                                       const std::string& tooltip) = 0;
};

#ifdef _WIN32
// Real Windows implementation seam. NOT implemented in Phase 3a -- this
// class is DECLARED so Phase 3b can fill in createTrayIcon() with the
// actual Shell_NotifyIcon(NIM_ADD, ...) + CreatePopupMenu()/
// AppendMenu()/TrackPopupMenu() sequence (RGBA -> HICON conversion via
// CreateIconFromResourceEx is also Phase 3b's job) without changing this
// header's public shape. Calling it today returns ok=false with an
// explicit "not yet implemented" error -- it does NOT pretend to succeed.
// See tray.cpp for the stub body.
class Win32TrayPlatform : public TrayPlatform {
public:
    TrayResult createTrayIcon(const std::vector<std::uint8_t>& iconRgba,
                               const std::string& tooltip) override;
};
#endif  // _WIN32

// Test double for non-Windows builds (and for Phase 3a's own unit tests on
// any platform): records the icon buffer + tooltip it was asked to create
// a tray for, and returns synthetic-but-distinguishable menu ids, instead
// of touching a real system tray. This is NOT a "fake working tray" for
// production use -- see hotkey.h/clipboard.h's equivalent doubles for the
// same convention across this phase's three thin-wrapper files.
class RecordingTrayPlatform : public TrayPlatform {
public:
    TrayResult createTrayIcon(const std::vector<std::uint8_t>& iconRgba,
                               const std::string& tooltip) override;

    // Test inspection.
    std::vector<std::uint8_t> lastIconRgba;
    std::string lastTooltip;
    int callCount = 0;
    // When true, createTrayIcon() returns ok=false (simulates a
    // platform-level tray-creation failure).
    bool simulateFailure = false;
};

// ======================== PUBLIC API ========================

// Creates the system tray icon and its context menu. Direct port of
// tray.rs's create_tray() control flow: generate the crop-mark icon RGBA
// buffer -> hand off to `platform` to actually build the tray icon + menu
// with the fixed tooltip text "XDR Snip — Press PrintScreen to capture"
// (same string as Rust). `platform` must be supplied by the caller (e.g. a
// Win32TrayPlatform on Windows builds once Phase 3b lands, or a
// RecordingTrayPlatform in tests) -- there is no default, for the same
// reason copyToClipboardPixels() takes an explicit platform parameter (see
// clipboard.h).
TrayResult createTray(TrayPlatform& platform);

}  // namespace snip
