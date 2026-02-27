# r/windows Post Draft

**Title:** Made a lightweight Snipping Tool alternative that outputs JPEG instead of huge bitmaps

**Body:**

Windows 11's screenshot tools (Snipping Tool, Win+Shift+S) copy uncompressed bitmaps to the clipboard — a 1080p screenshot becomes 6MB+. If you paste these into web apps, chat tools, or AI assistants, they're unnecessarily huge.

I built **XDR Snip** to solve this:

- Press Print Screen → screen freezes → drag to select region → release
- Saves a JPEG (typically 200-800KB instead of 6MB) to clipboard and file
- Single .exe, no installer, no dependencies
- System tray with settings (quality slider, save directory)
- Works correctly on multi-monitor and mixed-DPI setups

**Important setup step:** You need to disable Windows' Print Screen binding first:
Settings → Accessibility → Keyboard → toggle OFF "Use the Print Screen key to open screen capture"

Free, open source (MIT): https://github.com/db-cynerg-ia/xdr-snip/releases

Just download `xdr-snip.exe` and run it. ~11MB, portable.
