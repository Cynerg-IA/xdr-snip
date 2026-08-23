// clipboard.cpp -- implementation for clipboard.h. Direct port of
// crates/snip-app/src/clipboard.rs's RGB8->RGBA8 conversion; platform
// clipboard access is a declared seam (see clipboard.h file header).

#include "clipboard.h"

namespace snip {

std::vector<std::uint8_t> rgbToRgba(const std::vector<std::uint8_t>& rgbPixels,
                                     std::uint32_t width, std::uint32_t height) {
    const std::size_t pixelCount = static_cast<std::size_t>(width) * height;
    std::vector<std::uint8_t> rgba;
    rgba.reserve(pixelCount * 4);

    for (std::size_t i = 0; i < pixelCount; ++i) {
        const std::size_t off = i * 3;
        rgba.push_back(rgbPixels[off]);      // R
        rgba.push_back(rgbPixels[off + 1]);  // G
        rgba.push_back(rgbPixels[off + 2]);  // B
        rgba.push_back(255);                 // A (fully opaque)
    }
    return rgba;
}

#ifdef _WIN32
ClipboardResult Win32ClipboardPlatform::setImage(const std::vector<std::uint8_t>& /*rgbaPixels*/,
                                                  std::uint32_t /*width*/,
                                                  std::uint32_t /*height*/) {
    // Phase 3b TODO: OpenClipboard(nullptr) -> EmptyClipboard() -> pack a
    // BITMAPINFOHEADER + BGRA (or CF_DIBV5 for alpha) buffer ->
    // SetClipboardData(CF_DIB or CF_DIBV5, hGlobal) -> CloseClipboard(),
    // surfacing GetLastError() on any failed step. Deliberately
    // unimplemented in Phase 3a -- see clipboard.h file header.
    ClipboardResult result;
    result.ok = false;
    result.error =
        "Win32ClipboardPlatform::setImage: not yet implemented (Phase 3b TODO -- "
        "OpenClipboard/SetClipboardData wiring)";
    return result;
}
#endif  // _WIN32

ClipboardResult RecordingClipboardPlatform::setImage(const std::vector<std::uint8_t>& rgbaPixels,
                                                       std::uint32_t width, std::uint32_t height) {
    ++callCount;
    lastRgbaPixels = rgbaPixels;
    lastWidth = width;
    lastHeight = height;

    ClipboardResult result;
    if (simulateFailure) {
        result.ok = false;
        result.error = "RecordingClipboardPlatform: simulated clipboard-access failure";
        return result;
    }
    result.ok = true;
    return result;
}

ClipboardResult copyToClipboardPixels(const std::vector<std::uint8_t>& rgbPixels,
                                       std::uint32_t width, std::uint32_t height,
                                       ClipboardPlatform& platform) {
    const std::size_t pixelCount = static_cast<std::size_t>(width) * height;
    const std::size_t expectedRgbLen = pixelCount * 3;

    if (rgbPixels.size() < expectedRgbLen) {
        ClipboardResult result;
        result.ok = false;
        result.error = "RGB buffer too small: expected " + std::to_string(expectedRgbLen) +
                        " bytes for " + std::to_string(width) + "x" + std::to_string(height) +
                        ", got " + std::to_string(rgbPixels.size());
        return result;
    }

    std::vector<std::uint8_t> rgba = rgbToRgba(rgbPixels, width, height);
    return platform.setImage(rgba, width, height);
}

}  // namespace snip
