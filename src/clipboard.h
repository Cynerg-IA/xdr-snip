// clipboard.h -- C++ port of crates/snip-app/src/clipboard.rs (v0.5.0
// Rust source), for v0.6.0 Phase 3a.
//
// Rust used the `arboard` crate for clipboard access, which internally
// wraps the Win32 clipboard API (OpenClipboard/SetClipboardData/CF_DIB
// etc. on Windows). There is no C++ equivalent crate. Per the Phase 3a
// task split:
//   - rgbToRgba(): the RGB8 -> RGBA8 (opaque alpha) pixel conversion is
//     PURE LOGIC (no OS calls) and is ported faithfully, 1:1 with
//     clipboard.rs's copy_to_clipboard_pixels() conversion loop.
//   - The actual "put these bytes on the system clipboard" step (Rust's
//     Clipboard::new() + clipboard.set_image(img_data)) has NO portable
//     equivalent. This is wired behind an abstract PLATFORM SEAM
//     (ClipboardPlatform) so:
//       * copyToClipboardPixels() ports the Rust function's CONTROL FLOW
//         exactly (validate buffer size -> convert RGB8->RGBA8 -> call the
//         platform to set the image), but delegates the actual OS
//         clipboard write to a ClipboardPlatform implementation.
//       * #ifdef _WIN32: a real implementation (Win32ClipboardPlatform) is
//         DECLARED here with its OpenClipboard/SetClipboardData(CF_DIB)
//         call left as a Phase 3b TODO (the stub body returns a "not yet
//         implemented" error) -- flagged explicitly, not silently faked.
//       * Linux/other: a RecordingClipboardPlatform test double is
//         provided so the portable RGB8->RGBA8 conversion and the
//         validate/convert/dispatch control flow can be compiled and
//         unit-tested on Linux CI without a real clipboard.

#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace snip {

// ======================== RESULT ========================

// Result of copyToClipboardPixels(). Matches the JxlEncodeResult /
// CaptureResult / HotkeyResult convention used elsewhere in this port.
struct ClipboardResult {
    bool ok = false;
    // Human-readable description of what failed. Always non-empty when
    // ok == false; always empty when ok == true.
    std::string error;
};

// ======================== PURE LOGIC (ported 1:1 from clipboard.rs) ========================

// Converts RGB8 (3 bytes/pixel) to RGBA8 (4 bytes/pixel, fully opaque
// alpha = 255). Direct port of clipboard.rs's inline conversion loop
// inside copy_to_clipboard_pixels(). Exposed separately here (Rust kept it
// inline) so Phase 3a's tests can pin the conversion contract directly.
//
// Precondition: rgbPixels.size() >= width * height * 3 (callers should use
// copyToClipboardPixels()'s built-in size validation rather than calling
// this directly with unchecked input).
std::vector<std::uint8_t> rgbToRgba(const std::vector<std::uint8_t>& rgbPixels,
                                     std::uint32_t width, std::uint32_t height);

// ======================== PLATFORM SEAM ========================

// Abstract interface for the OS-level "put this RGBA image on the system
// clipboard" operation that Rust's `arboard` crate performed via
// Clipboard::new() + clipboard.set_image(). There is no portable C++
// equivalent (the Win32 clipboard API, X11 selections, and Wayland
// data-control protocols are all different mechanisms) -- this interface
// is the seam Phase 3b implements for real on Windows.
class ClipboardPlatform {
public:
    virtual ~ClipboardPlatform() = default;

    // Sets the system clipboard's image content to `rgbaPixels`
    // (width*height*4 bytes, RGBA8, row-major, opaque alpha already
    // applied by rgbToRgba()).
    virtual ClipboardResult setImage(const std::vector<std::uint8_t>& rgbaPixels,
                                      std::uint32_t width, std::uint32_t height) = 0;
};

#ifdef _WIN32
// Real Windows implementation seam. NOT implemented in Phase 3a -- this
// class is DECLARED so Phase 3b can fill in setImage() with the actual
// OpenClipboard/EmptyClipboard/SetClipboardData(CF_DIB, ...)/CloseClipboard
// sequence (RGBA -> BGRA + BITMAPINFOHEADER packing is also Phase 3b's
// job, since it is Win32-DIB-specific, not part of the portable
// rgbToRgba() contract) without changing this header's public shape.
// Calling it today returns ok=false with an explicit "not yet implemented"
// error -- it does NOT pretend to succeed. See clipboard.cpp for the stub
// body.
class Win32ClipboardPlatform : public ClipboardPlatform {
public:
    ClipboardResult setImage(const std::vector<std::uint8_t>& rgbaPixels, std::uint32_t width,
                              std::uint32_t height) override;
};
#endif  // _WIN32

// Test double for non-Windows builds (and for Phase 3a's own unit tests on
// any platform): records every setImage() call it receives instead of
// touching the OS clipboard, so tests can assert copyToClipboardPixels()
// calls the platform seam with the correctly converted RGBA buffer and
// dimensions, without requiring a real display/clipboard server. This is
// NOT a "fake success" for production use -- see hotkey.h/tray.h's
// equivalent doubles for the same convention across this phase's three
// thin-wrapper files.
class RecordingClipboardPlatform : public ClipboardPlatform {
public:
    ClipboardResult setImage(const std::vector<std::uint8_t>& rgbaPixels, std::uint32_t width,
                              std::uint32_t height) override;

    // Test inspection: the most recent (and, in this phase's usage
    // pattern, only) image this double was asked to set.
    std::vector<std::uint8_t> lastRgbaPixels;
    std::uint32_t lastWidth = 0;
    std::uint32_t lastHeight = 0;
    int callCount = 0;
    // When true, setImage() returns ok=false (simulates a platform-level
    // clipboard-access failure, e.g. another process holds the clipboard
    // open).
    bool simulateFailure = false;
};

// ======================== PUBLIC API ========================

// Copies raw RGB8 pixels to the system clipboard as an image. Direct port
// of clipboard.rs's copy_to_clipboard_pixels() control flow: validate the
// input buffer is large enough for width*height*3 bytes -> convert RGB8 ->
// RGBA8 (opaque) -> hand off to `platform` to actually set the clipboard.
// `platform` must be supplied by the caller (e.g. a Win32ClipboardPlatform
// on Windows builds once Phase 3b lands, or a RecordingClipboardPlatform
// in tests) -- there is no default, since silently defaulting to a no-op
// double in production code would be exactly the kind of "fake a working
// clipboard" this phase is instructed not to do.
ClipboardResult copyToClipboardPixels(const std::vector<std::uint8_t>& rgbPixels,
                                       std::uint32_t width, std::uint32_t height,
                                       ClipboardPlatform& platform);

}  // namespace snip
