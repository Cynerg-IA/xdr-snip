// capture.h -- C++ port of crates/snip-app/src/capture.rs (v0.5.0 Rust
// source), for v0.6.0 Phase 3a (issue #7, portable-subset tranche).
//
// Handles two pixel sources (mirrors the Rust doc comment exactly):
//   - GDI fallback: RGB8 pixels from the frozen overlay snapshot.
//   - WinRT HDR: R16G16B16A16Float (f16 RGBA) from hdr_capture (Phase 3b,
//     not yet ported -- this phase's HDR entry points accept already-
//     extracted pixel buffers, they do not talk to D3D11/WinRT themselves).
//
// HDR data is tone-mapped via Extended Reinhard (luminance-preserving) with
// sRGB gamma encoding. Wide Color Gamut (WCG) content -- where individual
// channels exceed sRGB but luminance is SDR -- uses max-channel Reinhard to
// compress into gamut while preserving hue. This is a byte-for-byte port of
// capture.rs's tone_map_hdr() math (same constants, same four-branch
// classification, same sanitize()/linear_to_srgb() helpers) -- see
// capture.cpp for the port notes on each function.
//
// ======================== PHASE 3a SCOPE NOTE ========================
// capture.rs dispatches encode_image() to 7 format-specific encoders (JPEG,
// PNG, WebP, TIFF, BMP, QOI, OpenEXR) plus the new-in-C++ JXL path (Phase
// 2, already landed: encode_jxl.h). Per the v0.6.0 issue #7 scope and the
// explicit Phase 3a instructions:
//   - JXL is wired for real (calls encodeJxl() from encode_jxl.h).
//   - The other 7 formats are DECLARED dispatch entry points only --
//     encodeImage() recognizes all 8 OutputFormat values and routes to a
//     named stub function per format, each of which returns a
//     CaptureResult with ok=false and an explicit "TODO(phase3b): <format>
//     encoder not yet ported" error message. They do NOT silently succeed
//     with an empty/wrong buffer -- see capture.cpp's encodeStub().
//   - OpenEXR is additionally UNDECIDED at the operator level (issue #7
//     section 1a: keep/drop/behind-flag not yet resolved) -- its dispatch
//     entry point (encodeOpenExrHdr/encodeOpenExrSdr) is declared but the
//     ExrCompression-specific work is explicitly out of scope until that
//     decision lands. It uses the same TODO(phase3b) stub convention as the
//     other 6, plus a comment noting the additional open decision.
//
// This mirrors the "declare the seam, don't invent behavior" instruction:
// callers can compile and link against the full 8-format dispatch surface
// today; only JXL actually produces bytes.

#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

#include "types.h"

namespace snip {

// ======================== HDR PIXEL SOURCE (Phase 3b seam) ========================

// Mirrors Rust's HdrPixelData (snip_types): raw R16G16B16A16Float (f16 RGBA,
// 8 bytes/pixel) pixel data for a cropped region, used by the OpenEXR HDR
// path to preserve HDR without tone mapping. Populated by hdr_capture.rs's
// port (Phase 3b, out of scope here) -- this phase only consumes the struct,
// it does not produce it from a live D3D11/WinRT capture.
struct HdrPixelData {
    std::vector<std::uint8_t> pixels;  // f16 RGBA, 8 bytes/pixel, row-major.
    std::uint32_t width = 0;
    std::uint32_t height = 0;
};

// A monitor's virtual-screen rectangle, in the same coordinate space as
// Region (types.h). Mirrors the subset of Rust's Win32 RECT that
// extract_hdr_region{,_raw}() actually reads (left/top only -- right/bottom
// are not used by the crop math, so they are intentionally omitted here
// rather than pulled in from a Win32 header this portable file must not
// depend on).
struct MonitorRect {
    std::int32_t left = 0;
    std::int32_t top = 0;
};

// Mirrors the subset of Rust's HdrFrame (hdr_capture.rs, Phase 3b) that
// capture.rs's extract_hdr_region{,_raw}() functions actually read: the
// pixel buffer, its dimensions, the source monitor's rect, and whether it
// is HDR (f16 R16G16B16A16Float) or SDR (BGRA8) data. Phase 3b will define
// the real HdrFrame (which additionally owns D3D11/WinRT capture-session
// state); this reduced struct is the exact read-only view capture.cpp
// needs and is what a Phase 3b HdrFrame should be convertible to/populate.
struct HdrFrameView {
    std::vector<std::uint8_t> pixels;  // f16 RGBA (is_hdr) or BGRA8, row-major.
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    MonitorRect monitorRect;
    bool isHdr = false;
};

// ======================== ENCODE RESULT ========================

// Result of an encode_image() (or per-format stub) call. Mirrors the
// Result<(), SnipError> contract from Rust: ok=true means the file was
// written to `output`; ok=false means nothing usable was written and
// `error` explains why. Matches the JxlEncodeResult convention introduced
// in Phase 2 (encode_jxl.h) rather than throwing, since encodeImage() sits
// on a hot capture-to-clipboard/disk path where the caller wants to check
// a return value, not unwind a C++ exception.
struct CaptureResult {
    bool ok = false;
    // Human-readable description of what failed. Always non-empty when
    // ok == false; always empty when ok == true.
    std::string error;
};

// ======================== PUBLIC API ========================

// Encodes pixel data in the configured format and writes to the output
// path. Mirrors Rust's encode_image() dispatch exactly: for most formats,
// `rgbPixels` is tone-mapped RGB8 data (3 bytes/pixel, row-major). For
// OpenEXR with `rawHdr` present, the raw f16 pixel data would be written
// directly (Phase 3b/section-1a-dependent; currently a declared stub -- see
// file header). JXL additionally accepts the raw HDR buffer directly
// (Phase 2 already supports float16/float32 input) when present.
//
// Ensures the parent directory of `output` exists (mkdir -p semantics),
// exactly like Rust's ensure_output_dir().
CaptureResult encodeImage(const std::vector<std::uint8_t>& rgbPixels, std::uint32_t width,
                           std::uint32_t height, OutputFormat format,
                           const FormatOptions& options, const std::string& output,
                           const HdrPixelData* rawHdr);

// Extracts raw HDR pixel data (R16G16B16A16Float) for a selected region.
// Used by OpenEXR to preserve HDR without tone mapping. Direct port of
// Rust's extract_hdr_region_raw() crop arithmetic.
HdrPixelData extractHdrRegionRaw(const HdrFrameView& frame, const Region& vscreenRegion);

// Extracts an RGB8 pixel buffer from an HdrFrameView for a given region.
// `vscreenRegion` is in virtual-screen coordinates; this function
// translates it to the frame's monitor-relative coordinates, crops the HDR
// or SDR data, tone-maps if needed (HDR only), and returns RGB8 pixels
// ready for encoding. Direct port of Rust's extract_hdr_region().
std::vector<std::uint8_t> extractHdrRegion(const HdrFrameView& frame, const Region& vscreenRegion);

// ======================== TONE MAPPING (exposed for testing) ========================

// Tone maps R16G16B16A16Float (scRGB) pixel data to BGRA8 (sRGB). Direct
// port of Rust's tone_map_hdr() -- same four-branch classification
// (zero/negative luminance -> black; HDR lum>1.0 -> Extended Reinhard;
// WCG lum<=1.0 but any channel>1.0 -> max-channel Reinhard; SDR -> gamma
// only), same NaN/Inf sanitization, same Rec.709 luminance coefficients.
// Exposed here (Rust kept it private to capture.rs) so Phase 3a's tests can
// pin the pixel-buffer contract directly instead of only indirectly via
// extractHdrRegion().
std::vector<std::uint8_t> toneMapHdr(const std::vector<std::uint8_t>& halfPixels,
                                      std::size_t width, std::size_t height);

// sRGB transfer function: linear -> gamma-encoded (IEC 61966-2-1). Exposed
// for testing; direct port of Rust's linear_to_srgb().
float linearToSrgb(float linear);

// Sanitizes an f32 decoded from f16: NaN -> 0, +Inf -> kMaxDisplayLuminance,
// -Inf -> 0. Does NOT clamp to [0,1] -- callers handle range based on
// context. Direct port of Rust's sanitize().
float sanitizeHdrSample(float v);

// Quantizes a [0, 1] float to uint8_t with rounding. Direct port of Rust's
// float_to_byte().
std::uint8_t floatToByte(float v);

// Converts BGRA8 pixel data to RGB8 for image encoding. Direct port of
// Rust's bgra_to_rgb().
std::vector<std::uint8_t> bgraToRgb(const std::vector<std::uint8_t>& bgra, std::size_t width,
                                     std::size_t height);

// Maximum displayable luminance in scRGB linear (~1000 nits). Values above
// this are typically driver/compositor artifacts. Same constant as Rust's
// MAX_DISPLAY_LUMINANCE.
extern const float kMaxDisplayLuminance;

// ======================== RESIZE ========================

// Result of applyResize(): the (possibly downscaled) pixel buffer plus its
// new dimensions, and -- when the resize spec's keepOriginal flag is set
// AND a resize actually occurred -- the untouched original buffer/
// dimensions so the caller can additionally write the "_full" sibling
// file. Mirrors the v0.5.0 resize feature (issue #7 section 1, "auto-resize
// ... optional _full original"): when disabled (ResizeOptions::enabled ==
// false), this is a no-op passthrough and original is never populated.
struct ResizeResult {
    std::vector<std::uint8_t> pixels;  // RGB8, 3 bytes/pixel.
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    struct Original {
        std::vector<std::uint8_t> pixels;
        std::uint32_t width = 0;
        std::uint32_t height = 0;
    };
    // Populated only when a resize actually happened AND keepOriginal was
    // requested. std::nullopt otherwise (disabled, no resize needed because
    // the image already fit, or keepOriginal == false).
    std::optional<Original> original;
    // True iff pixels/width/height differ from the input (i.e. a resize
    // was actually performed). False for the disabled-by-default path and
    // for images that already fit within maxWidth/maxHeight.
    bool resized = false;
};

// Applies the auto-resize policy described by `opts` to an RGB8 pixel
// buffer. Aspect ratio is always preserved (the smaller of the two scale
// factors -- width-limited or height-limited -- is used). When opts.enabled
// is false, or the image already fits within maxWidth/maxHeight, this
// returns the input unchanged with resized=false and original=std::nullopt
// (the disabled/no-op path).
//
// NOTE ON RESAMPLING FILTER: this port implements the ASPECT-RATIO MATH and
// the keepOriginal "_full" sibling-file contract, which are the pieces
// capture.rs (crate boundary for this phase) actually owns. The v0.5.0
// Rust implementation applied a Lanczos3 resample via the `image` crate's
// imageops::resize; this port uses a simple box/area-average downscale
// filter instead (the `image` crate has no C++ 1:1 equivalent and vendoring
// one is out of Phase 3a scope) -- FLAGGED explicitly per the "no silent
// approximation" instruction. Phase 3b/later should either vendor a
// Lanczos3 implementation or accept box-filter quality as a deliberate
// tradeoff; the resize DECISION LOGIC (whether/how much to scale, aspect
// preservation, the _full sibling contract) is a faithful, tested port
// regardless of which resampling kernel is used.
ResizeResult applyResize(const std::vector<std::uint8_t>& rgbPixels, std::uint32_t width,
                          std::uint32_t height, const ResizeOptions& opts);

}  // namespace snip
