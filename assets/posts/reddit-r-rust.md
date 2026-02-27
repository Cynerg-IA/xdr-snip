# r/rust Post Draft

**Title:** I built a lightweight screenshot tool in pure Rust — frozen screen overlay, JPEG output, single exe

**Body:**

I got frustrated with Windows 11's clipboard sending 6MB+ uncompressed bitmaps every time I pasted a screenshot — it was crashing my Claude Code conversations. So I built **XDR Snip**, a tiny screenshot tool that captures to JPEG instead.

**How it works:**
- Press Print Screen → the entire screen freezes instantly (GDI BitBlt snapshot)
- Drag to select a region — selected area shows at full brightness, rest is dimmed
- Release → JPEG saved to disk + copied to clipboard
- Preview popup shows thumbnail, dimensions, and file size

**What makes it different:**
- Output is JPEG (200-800KB) instead of uncompressed bitmap (6MB+)
- The capture comes from the frozen snapshot — what you selected is exactly what you get, even if the screen changes underneath
- Single ~11MB exe, no installer, no .NET, no dependencies
- Per-monitor DPI v2 aware (works correctly on mixed-DPI setups)

**Tech stack:** Pure Rust using the `windows` crate for Win32/GDI APIs, `image` for JPEG encoding, `arboard` for clipboard, `tray-icon` for system tray. No unsafe abstractions needed beyond the Win32 FFI.

The whole thing is about 2000 lines of Rust across 8 modules.

MIT licensed: https://github.com/db-cynerg-ia/xdr-snip

Happy to answer questions about the Win32/GDI integration in Rust!
