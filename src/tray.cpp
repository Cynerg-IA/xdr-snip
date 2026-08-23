// tray.cpp -- implementation for tray.h. Direct port of
// crates/snip-app/src/tray.rs's icon-generation logic; platform tray/menu
// creation is a declared seam (see tray.h file header).

#include "tray.h"

namespace snip {

const std::uint32_t kTrayIconSize = 32;

void fillRect(std::vector<std::uint8_t>* rgba, std::uint32_t stride, std::uint32_t x,
              std::uint32_t y, std::uint32_t w, std::uint32_t h, std::uint8_t r, std::uint8_t g,
              std::uint8_t b, std::uint8_t a) {
    for (std::uint32_t dy = 0; dy < h; ++dy) {
        for (std::uint32_t dx = 0; dx < w; ++dx) {
            const std::uint32_t px = x + dx;
            const std::uint32_t py = y + dy;
            if (px < stride && py < stride) {
                const std::size_t idx = (static_cast<std::size_t>(py) * stride + px) * 4;
                (*rgba)[idx] = r;
                (*rgba)[idx + 1] = g;
                (*rgba)[idx + 2] = b;
                (*rgba)[idx + 3] = a;
            }
        }
    }
}

std::vector<std::uint8_t> generateSnipIconRgba() {
    const std::uint32_t size = kTrayIconSize;
    std::vector<std::uint8_t> rgba(static_cast<std::size_t>(size) * size * 4, 0);

    // White (#FFFFFF) corner brackets on transparent background. White
    // works on both dark (Win11 default) and light taskbars.
    const std::uint8_t r = 255, g = 255, b = 255, a = 255;

    const std::uint32_t arm = 9;     // length of each bracket arm in pixels
    const std::uint32_t thick = 2;   // line thickness
    const std::uint32_t margin = 4;  // inset from edge
    const std::uint32_t far = size - margin;  // far edge coordinate

    // Top-left bracket: horizontal + vertical.
    fillRect(&rgba, size, margin, margin, arm, thick, r, g, b, a);
    fillRect(&rgba, size, margin, margin, thick, arm, r, g, b, a);

    // Top-right bracket: horizontal + vertical.
    fillRect(&rgba, size, far - arm, margin, arm, thick, r, g, b, a);
    fillRect(&rgba, size, far - thick, margin, thick, arm, r, g, b, a);

    // Bottom-left bracket: horizontal + vertical.
    fillRect(&rgba, size, margin, far - thick, arm, thick, r, g, b, a);
    fillRect(&rgba, size, margin, far - arm, thick, arm, r, g, b, a);

    // Bottom-right bracket: horizontal + vertical.
    fillRect(&rgba, size, far - arm, far - thick, arm, thick, r, g, b, a);
    fillRect(&rgba, size, far - thick, far - arm, thick, arm, r, g, b, a);

    return rgba;
}

#ifdef _WIN32
TrayResult Win32TrayPlatform::createTrayIcon(const std::vector<std::uint8_t>& /*iconRgba*/,
                                              const std::string& /*tooltip*/) {
    // Phase 3b TODO: CreateIconFromResourceEx(iconRgba as a 32bpp DIB) ->
    // build NOTIFYICONDATA -> Shell_NotifyIcon(NIM_ADD, ...) ->
    // CreatePopupMenu()/AppendMenu() for the six-item layout (Take
    // Screenshot / separator / Open Folder / Settings / separator / Quit)
    // -> return real HMENU-item-id-derived TrayMenuIds. Deliberately
    // unimplemented in Phase 3a -- see tray.h file header.
    TrayResult result;
    result.ok = false;
    result.error =
        "Win32TrayPlatform::createTrayIcon: not yet implemented (Phase 3b TODO -- "
        "Shell_NotifyIcon/menu wiring)";
    return result;
}
#endif  // _WIN32

TrayResult RecordingTrayPlatform::createTrayIcon(const std::vector<std::uint8_t>& iconRgba,
                                                  const std::string& tooltip) {
    ++callCount;
    lastIconRgba = iconRgba;
    lastTooltip = tooltip;

    TrayResult result;
    if (simulateFailure) {
        result.ok = false;
        result.error = "RecordingTrayPlatform: simulated tray-creation failure";
        return result;
    }
    result.ok = true;
    result.ids.screenshot = "screenshot";
    result.ids.openFolder = "open_folder";
    result.ids.settings = "settings";
    result.ids.quit = "quit";
    return result;
}

TrayResult createTray(TrayPlatform& platform) {
    std::vector<std::uint8_t> iconRgba = generateSnipIconRgba();
    return platform.createTrayIcon(iconRgba, "XDR Snip \xe2\x80\x94 Press PrintScreen to capture");
}

}  // namespace snip
