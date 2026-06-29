# Color Space Handling Map

Audit of how each source color space flows through the capture pipeline — from
frame capture through tone mapping to final file encoding.

## Tone-Mapping Pipeline Overview

```
WinRT Graphics Capture  —>  HdrFrame (R16G16B16A16Float, scRGB linear)  —>  tone_map_hdr()
                                                                                   |
    GDI fallback (BitBlt) —>  Selection.pixels_rgb (BGRA8 / RGB8)           —>  extract_hdr_region / encode
```

`tone_map_hdr()` (capture.rs): input is scRGB f16 per channel, D65 white point, linear transfer.
Intended output is sRGB-encoded BGRA8 (byte, nonlinear gamma 2.2).

The GDI fallback path (overlay.rs `extract_selection_rgb`) captures via BitBlt → GetDIBits,
which yields device-dependent 32-bit BGRA with no ICC profile attached. These bytes are
assumed sRGB-compatible by the encoders below with no color-space conversion.

---

## Handling Matrix

| Source space         | Detection                         | Mapping path                                    | Output format                | Verdict                                                      |
|----------------------|-----------------------------------|-------------------------------------------------|------------------------------|--------------------------------------------------------------|
| **sRGB**             | WinRT frame f16: all channels in [0,1], L≤1  → **SDR branch**  | Linear → sRGB transfer only (linear_to_srgb)     | Any (exr/jpg/png/…)          | **OK** — input and output share the same space.             |
| **Display P3**       | WinRT frame f16: channels > 1.0, L≤1  → **WCG branch**  | Reinhard compression on max channel → uniform scale → sRGB gamma | Any (exr/jpg/png/…)          | **OK** — hue-preserving, no hard clipping, output within sRGB bounds. |
| **Rec.2020 /**       | WinRT frame f16: channels up to ~4.0  → **HDR branch** (L>1)  | Extended Reinhard luminance scale → uniform scale → sRGB gamma | Any (exr/jpg/png/…)  | **OK** — scales high luminance down; uniformly preserves hue. |
| **HDR10 (1000 nits)**| Same as Rec.2020 and above      | Same as Rec.2020 above                          | Any (exr/jpg/png/…)          | **OK** — identical path, same reasoning.                    |
| **scRGB (full)**     | WinRT frame f16: any channel > 1.0 OR L>1 | Branch 2 (HDR) or 3 (WCG)                     | Any (exr/jpg/png/…)          | **OK** — branches are selective and correct for any value.  |
| **scRGB (negative)**| WinRT frame f16: r<0 OR g<0 OR b<0 → sanitized to 0 | Removed via `.max(0.0)` before classification  | N/A                          | **OK** — negatives are driver artifacts, not chromaticity.  |
| **scRGB (NaN/Inf)**  | WinRT frame f16: NAN or ±Inf       | `sanitize()` replaces NAN→0, +Inf→10.0, -Inf→0 | N/A                          | **OK** — sanitized before classification.                   |
| **GDI fallback**     | WinRT frame unavailable (empty map, or .is_hdr==false) | **No tone mapping**. Direct BGRA8→RGB8 pass-through (overlay.rs) | Any (exr/jpg/png/…)          | **Minor gap noted** — no ICC profile or color-space tagging. |

## GDI Fallback Gap Detail

When WinRT capture is unavailable (D3D11 init failure, older OS, permissions), the fallback path is:

1. `capture_screen_snapshot()` (overlay.rs:237) — BitBlt virtual screen into a memory DC (32-bit device RGBA).
2. User selects a region.
3. `extract_selection_rgb()` (overlay.rs:335) — cropped via BitBlt into temp bitmap, read back via GetDIBits with BI_RGB.
4. BGRA→RGB conversion, returned as untagged RGB8 bytes.

**Gaps in the GDI path:**

1. **No ICC profile in output.** All encoders write raw RGB8 with no color profile blob.
   The PNG encoder (`image` crate's `PngEncoder::write_image`) does NOT emit an sRGB ancillary chunk.
   GDI BitBlt on Windows 10+ returns pixels in the *monitor's native color space* (often Display P3
   or Rec.2020 on HDR-capable displays), but the output file has no profile to indicate this.
   Color-managed viewers will assume sRGB, potentially mis-representing wide-gamut content.

2. **Mixed-color-space multi-monitor captures.** If a screenshot crosses from an sRGB monitor to a P3
   monitor, DWM compositor blends the two spaces uniformly, and the BitBlt snapshot captures the blended
   result in whatever space DWM chose (typically a composition-safe device space). No per-monitor
   color-space tag exists in the output.

3. **No HDR from GDI.** BitBlt cannot capture HDR (float/10-bit) data. Content on an HDR display
   captured via GDI falls back to SDR tone-map output (the OS compositor's HDR → SDR mapping),
   which is already a lossy conversion before GDI even touches it.

These gaps are **constraints of the GDI API** and cannot be fixed purely in Rust code.
A solution would require capturing via the DWM Desktop Duplication API or DXGI pivotal
method with color-profile awareness — a separate feature, not a tone-map fix.

## Working branches (capture.rs:tone_map_hdr)

| Branch             | Condition                          | Math                                           |
|--------------------|------------------------------------|------------------------------------------------|
| Black              | `lum <= 0.0`                       | (0, 0, 0, 255) — fully opaque black.           |
| HDR                | `lum > 1.0`                        | scale = (L/(1+L))/L; r'=r*scale; linear_to_srgb(r'). |
| WCG                | `lum <= 1.0` AND any channel > 1.0 | scale = 1/(1+max_ch); r'=r*scale; linear_to_srgb(r'). |
| SDR                | `lum <= 1.0` AND all channels ≤ 1.0 | r' = linear_to_srgb(r)  (pass-through).        |

## Line numbers reference

- `tone_map_hdr`:         capture.rs:495
- HDR branch:             capture.rs:566
- WCG branch:             capture.rs:582
- SDR branch:             capture.rs:600
- Sanitize (NAN/Inf):     capture.rs:696
- Linear → sRGB gamma:    capture.rs:685
- GDI BitBlt capture:     overlay.rs:237
- GDI GetDIBits extract:  overlay.rs:335
