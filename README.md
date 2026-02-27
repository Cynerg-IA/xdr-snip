# XDR Snip

Lightweight HDR-aware screenshot tool for Windows 11. Select a region, capture it with correct HDR-to-SDR tone mapping, get a small JPEG — ready to paste into Claude, browsers, or any app.

## Why

- **Windows clipboard sends uncompressed bitmaps** — a 1080p screenshot becomes 6MB+ when pasted, crashing Claude Code conversations
- **Snipping Tool breaks HDR content** — blown-out highlights, wrong colors
- **XDR Snip fixes both** — captures via `Windows.Graphics.Capture` with `R16G16B16A16Float`, applies Extended Reinhard tone mapping, outputs JPEG at ~200-800KB

## Features

- **Print Screen** → fullscreen frozen overlay → drag to select region
- **HDR tone mapping** — Extended Reinhard (luminance-preserving), correct on both HDR and SDR content
- **JPEG output** — configurable quality (default 85%), typically 200-800KB for a full screen
- **Clipboard + file** — copies to clipboard and saves to `~/Pictures/XDR-Snips/`
- **System tray** — right-click for Take Screenshot, Open Folder, Settings, Quit
- **Settings window** — change save path and JPEG quality without editing config files
- **Capture preview** — popup with thumbnail, dimensions, file size, clipboard status (auto-closes after 4s)
- **Single exe** — ~11MB, no installer, no dependencies, no .NET runtime
- **DPI-aware** — per-monitor DPI v2, correct coordinates on mixed-DPI setups

## Screenshot

```
Press PrintScreen → screen freezes with dark overlay → drag region → release
```

The selected region appears at full brightness with a cyan border. Escape or right-click to cancel.

## Install

Download `xdr-snip.exe` from [Releases](https://github.com/db-cynerg-ia/xdr-snip/releases) and run it. No installation needed.

> **⚠️ Important: Unbind Windows Snipping Tool first!**
>
> Windows 11 intercepts Print Screen before any app can see it. You must disable this:
>
> **Settings → Accessibility → Keyboard → toggle OFF "Use the Print Screen key to open screen capture"**
>
> Without this step, XDR Snip will never receive the hotkey.

### Prerequisites

- Windows 10 1903+ (Windows.Graphics.Capture API)

## Build from source

```powershell
# Requires Rust toolchain (rustup.rs)
cargo build --release
# Output: target/release/xdr-snip.exe
```

## Configuration

Config file: `%APPDATA%\xdr-snip\config.toml` (created on first run with defaults)

```toml
[capture]
quality = 85                          # JPEG quality 50-100 (recommended: 85)
save_dir = "~/Pictures/XDR-Snips"     # Output directory
filename_pattern = "screenshot_{timestamp}"

[hotkey]
key = "PrintScreen"                   # Trigger key
modifiers = []                        # Optional: ["Alt"], ["Ctrl", "Shift"], etc.

[behavior]
copy_to_clipboard = true
save_to_file = true
show_notification = true
```

## Architecture

Single Rust binary using the `windows` crate for all Win32 and WinRT APIs:

| Module | Role |
|--------|------|
| `main.rs` | DPI setup, message loop, hotkey + tray event dispatch |
| `overlay.rs` | Frozen-screen overlay with double-buffered region selection |
| `capture.rs` | `Windows.Graphics.Capture` → D3D11 → HDR tone map → JPEG encode |
| `clipboard.rs` | Decode JPEG → set `CF_DIB` via `arboard` |
| `preview.rs` | Capture preview popup — thumbnail + info text (auto-closes after 4s) |
| `settings.rs` | GUI settings dialog — save path + quality slider |
| `tray.rs` | System tray icon + context menu via `tray-icon` crate |
| `config.rs` | TOML config load/validate from `%APPDATA%` |

### HDR Tone Mapping Pipeline

1. Capture frame in `R16G16B16A16Float` (preserves full HDR range)
2. Read pixel data via `IMemoryBufferByteAccess`
3. Decode `f16` → `f32` per channel
4. Compute luminance: `L = 0.2126R + 0.7152G + 0.0722B`
5. Extended Reinhard: `L' = L / (1 + L)` — SDR values (<1.0) pass through nearly unchanged
6. Scale channels by `L'/L`, apply sRGB gamma
7. Parallel scanline processing via `rayon`

## License

MIT
