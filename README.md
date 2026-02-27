# XDR Snip

[![Windows 11](https://img.shields.io/badge/Windows-11-0078D4?logo=windows11)](https://github.com/db-cynerg-ia/xdr-snip)
[![Rust](https://img.shields.io/badge/Rust-stable-DEA584?logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/db-cynerg-ia/xdr-snip)](https://github.com/db-cynerg-ia/xdr-snip/releases)

<p align="center">
  <img src="assets/social-preview.png" alt="XDR Snip" width="720">
</p>

Lightweight screenshot tool for Windows 11. Select a region on a frozen screen, get a small JPEG — ready to paste into Claude, browsers, or any app.

## Why

- **Windows clipboard sends uncompressed bitmaps** — a 1080p screenshot becomes 6MB+ when pasted, crashing Claude Code conversations
- **XDR Snip outputs JPEG** — captures from a frozen screen overlay, encodes to JPEG at ~200-800KB
- **What you select is what you get** — the output matches exactly what you saw during selection, even if the underlying screen changes

## Features

- **Print Screen** → fullscreen frozen overlay → drag to select region
- **HDR + SDR support** — WinRT Graphics Capture with Extended Reinhard tone mapping; automatic fallback to GDI for compatibility
- **Frozen capture** — screen freezes instantly; pixels are extracted from the snapshot, not a live re-capture
- **JPEG output** — configurable quality (default 85%), typically 200-800KB for a full screen
- **Clipboard + file** — copies to clipboard and saves to `~/Pictures/XDR-Snips/`
- **System tray** — right-click for Take Screenshot, Open Folder, Settings, Quit
- **Settings window** — change save path and JPEG quality without editing config files
- **Capture preview** — popup with thumbnail, dimensions, file size, clipboard status (auto-closes after 5s, click to open file)
- **Single exe** — ~11MB, no installer, no dependencies, no .NET runtime
- **DPI-aware** — per-monitor DPI v2, correct coordinates on mixed-DPI setups

## Demo

<!-- Demo GIF: see assets/DEMO-RECORDING-GUIDE.md -->
> **Press PrintScreen** → screen freezes with dark overlay → **drag region** → release

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

Single Rust binary using the `windows` crate for Win32/GDI and WinRT APIs:

| Module | Role |
|--------|------|
| `main.rs` | DPI setup, message loop, hotkey + tray event dispatch, dual-capture orchestration |
| `hdr_capture.rs` | WinRT Graphics Capture — per-monitor HDR frame acquisition (D3D11, R16G16B16A16Float) |
| `overlay.rs` | Frozen-screen overlay (GDI BitBlt) with double-buffered region selection |
| `capture.rs` | HDR tone mapping (Extended Reinhard) + JPEG encoder |
| `clipboard.rs` | Decode JPEG → set clipboard image via `arboard` |
| `preview.rs` | Capture preview popup — thumbnail + info text (click to open, auto-closes after 5s) |
| `settings.rs` | GUI settings dialog — save path + quality slider with size estimates |
| `tray.rs` | System tray icon + context menu via `tray-icon` crate |
| `config.rs` | TOML config load/validate from `%APPDATA%` |

### Capture Pipeline

1. User presses Print Screen
2. **WinRT** captures each monitor as `R16G16B16A16Float` (preserves HDR data)
3. **GDI** `BitBlt` captures the virtual screen for the frozen overlay display
4. Screen freezes: dimmed version shown as background, selected region at full brightness
5. User releases mouse → identifies target monitor
6. If WinRT frame available: crop + Extended Reinhard tone map → RGB8. Otherwise: GDI fallback via `GetDIBits`
7. JPEG encode via `image` crate → file saved + clipboard set + preview popup

## Release History

### v0.3.0 — HDR capture re-integration (2026-02-27)

- **HDR + SDR support** — WinRT Graphics Capture with `R16G16B16A16Float` pixel format, Extended Reinhard tone mapping
- **Dual-capture architecture** — WinRT captures HDR frames per-monitor, GDI provides the frozen overlay display
- **Automatic fallback** — if WinRT capture fails (permissions, driver), falls back to GDI-only (v0.2.0 behavior)
- **SDR passthrough** — Reinhard curve is near-identity for [0,1] values; SDR content is unaffected
- New module: `hdr_capture.rs` (D3D11 device, WinRT frame pool, IMemoryBufferByteAccess pixel reading)
- Expanded `capture.rs` with tone mapping, HDR region extraction, pixel format conversion

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
