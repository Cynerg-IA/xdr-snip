# XDR Snip

Lightweight screenshot tool for Windows 11. Select a region on a frozen screen, get a small JPEG — ready to paste into Claude, browsers, or any app.

## Why

- **Windows clipboard sends uncompressed bitmaps** — a 1080p screenshot becomes 6MB+ when pasted, crashing Claude Code conversations
- **XDR Snip outputs JPEG** — captures from a frozen screen overlay, encodes to JPEG at ~200-800KB
- **What you select is what you get** — the output matches exactly what you saw during selection, even if the underlying screen changes

## Features

- **Print Screen** → fullscreen frozen overlay → drag to select region
- **Frozen capture** — screen freezes instantly via GDI BitBlt; pixels are extracted from the snapshot, not a live re-capture
- **JPEG output** — configurable quality (default 85%), typically 200-800KB for a full screen
- **Clipboard + file** — copies to clipboard and saves to `~/Pictures/XDR-Snips/`
- **System tray** — right-click for Take Screenshot, Open Folder, Settings, Quit
- **Settings window** — change save path and JPEG quality without editing config files
- **Capture preview** — popup with thumbnail, dimensions, file size, clipboard status (auto-closes after 5s, click to open file)
- **Single exe** — ~11MB, no installer, no dependencies, no .NET runtime
- **DPI-aware** — per-monitor DPI v2, correct coordinates on mixed-DPI setups

## Screenshot

```
Press PrintScreen → screen freezes with dark overlay → drag region → release
```

The selected region appears at full brightness with a cyan border. Escape or right-click to cancel.

## Install

Download `xdr-snip.exe` from [Releases](https://github.com/db-cynerg-ia/xdr-snip/releases) and run it. No installation needed.

> **Important: Unbind Windows Snipping Tool first!**
>
> Windows 11 intercepts Print Screen before any app can see it. You must disable this:
>
> **Settings → Accessibility → Keyboard → toggle OFF "Use the Print Screen key to open screen capture"**
>
> Without this step, XDR Snip will never receive the hotkey.

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

Single Rust binary using the `windows` crate for Win32/GDI APIs:

| Module | Role |
|--------|------|
| `main.rs` | DPI setup, message loop, hotkey + tray event dispatch |
| `overlay.rs` | Frozen-screen overlay (GDI BitBlt) with double-buffered region selection + pixel extraction |
| `capture.rs` | JPEG encoder — encodes RGB pixels from overlay snapshot |
| `clipboard.rs` | Decode JPEG → set clipboard image via `arboard` |
| `preview.rs` | Capture preview popup — thumbnail + info text (click to open, auto-closes after 5s) |
| `settings.rs` | GUI settings dialog — save path + quality slider with size estimates |
| `tray.rs` | System tray icon + context menu via `tray-icon` crate |
| `config.rs` | TOML config load/validate from `%APPDATA%` |

### Capture Pipeline

1. User presses Print Screen → overlay captures entire virtual screen via `BitBlt` (GDI)
2. Screen freezes: dimmed version shown as background, selected region at full brightness
3. User releases mouse → overlay extracts selected region's pixels from frozen snapshot via `GetDIBits`
4. BGRA→RGB conversion, JPEG encode via `image` crate
5. File saved + clipboard set + preview popup shown

## Release History

### v0.2.0 — Frozen overlay capture (2026-02-27)

- **Single executable** — pure Rust, no C# subprocess, no .NET dependency
- **Frozen screen overlay** — BitBlt snapshot with double-buffered rendering, zero flicker
- **Frozen capture** — output comes from the overlay snapshot, not a live re-capture (fixes content mismatch bug)
- **Capture preview popup** — thumbnail + info, click to open file, right-click to dismiss, 5s auto-close
- **Settings dialog** — save path + quality slider (50-100) with file size estimates
- **System tray** — Take Screenshot, Open Folder, Settings, Quit
- Escape / right-click to cancel selection
- Quality range clamped to 50-100 (below 50 = visible artifacts)
- Removed WinRT/D3D11/HDR dependencies (half, rayon) — lighter binary

### v0.1.0 — Initial release (2026-02-27)

- Region selection overlay
- HDR capture via Windows.Graphics.Capture + Extended Reinhard tone mapping
- JPEG output + clipboard
- C# capture subprocess (later removed in v0.2.0)

## License

MIT
