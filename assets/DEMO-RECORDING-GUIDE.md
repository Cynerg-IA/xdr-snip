# Demo GIF Recording Guide

## Install a GIF recorder

```powershell
winget install NickeManarin.ScreenToGif
```

Or download from https://www.screentogif.com/

## Recording steps

1. **Open ScreenToGif** → click "Recorder"
2. **Set recording area** to ~800x500px (keeps GIF under 5MB)
3. **Set FPS** to 10 (lower = smaller file, still smooth enough)
4. **Prepare:** Have XDR Snip running in the tray, with something interesting on screen (code editor, browser, etc.)

### Capture sequence (~8 seconds total)

1. Start recording
2. Press **Print Screen** — screen freezes with dark overlay
3. **Drag** to select a region (show the cyan border and bright selection)
4. **Release** — preview popup appears with thumbnail + file info
5. Wait 1-2 seconds showing the preview
6. Stop recording

## Post-processing in ScreenToGif

1. **Editor** → trim any dead frames at start/end
2. **Resize** to 800px wide if larger
3. **Save as** → GIF → `assets/demo.gif`
4. Target: under 3MB (GitHub displays GIFs inline up to 10MB, but smaller = faster load)

## Add to README

After saving the GIF, the README already has a placeholder. Replace:

```markdown
<!-- Demo GIF: see assets/DEMO-RECORDING-GUIDE.md -->
```

with:

```markdown
![XDR Snip Demo](assets/demo.gif)
```

Then commit and push.
