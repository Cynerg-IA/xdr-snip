# Hacker News Post Draft

**Title:** Show HN: XDR Snip – Lightweight screenshot tool for Windows 11 (Rust, JPEG output)

**URL:** https://github.com/db-cynerg-ia/xdr-snip

**First comment (post immediately after submitting):**

Hi HN! I built this because Windows clipboard sends uncompressed bitmaps — a 1080p screenshot is 6MB+, which crashes Claude Code conversations and bloats everything you paste into.

XDR Snip freezes the screen with a GDI BitBlt snapshot, lets you drag-select a region, then encodes to JPEG (200-800KB). The output comes from the frozen snapshot, so what you see during selection is exactly what you get.

Single ~11MB Rust binary, no installer, no .NET. Uses the `windows` crate for Win32/GDI, `image` for JPEG encoding, `arboard` for clipboard. About 2000 lines across 8 modules.

The frozen overlay approach was surprisingly tricky — you need to BitBlt the entire virtual screen, double-buffer the selection rendering, then extract pixels from the original snapshot (not the display buffer) via GetDIBits. Happy to discuss the Win32 integration details.
