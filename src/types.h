// types.h -- C++ port of crates/snip-types/src/lib.rs (v0.5.0 Rust source).
//
// Shared type definitions and configuration structs for XDR Snip. This
// header is dependency-free from platform APIs (no Win32, no D3D11) so it
// can be built and tested on any platform, including Linux CI.
//
// Behavior changes vs the Rust source (both intentional, spec-mandated by
// issue #8, NOT bugs):
//   1. OutputFormat gains a new first-class value: Jxl (JPEG XL). It is
//      FIRST in ALL[] so it drives the settings-UI combo box order.
//   2. OutputFormat::preservesHdr() returns true for BOTH Jxl and OpenExr
//      (Rust only had OpenExr). JPEG XL accepts f16/f32 pixel input per the
//      libjxl encoder API (verified against libjxl headers in Phase 0), so
//      it can carry HDR data without lossy tone-mapping the way JPEG/PNG/etc
//      would require.
//   3. OutputFormat::defaultFormat() is now Jxl (Rust default was Jpeg).
//
// Everything else mirrors v0.5.0 Rust semantics exactly -- no other
// invented or "improved" behavior.

#pragma once

#include <cstdint>
#include <ostream>
#include <string>
#include <vector>

namespace snip {

// ======================== OUTPUT FORMAT ========================

// Supported output image formats. Jxl is intentionally first: it is the new
// default and drives UI ordering (see ALL below).
enum class OutputFormat {
    Jxl,
    Jpeg,
    Png,
    WebP,
    Tiff,
    Bmp,
    Qoi,
    OpenExr,
};

// All supported formats, in display/UI order. Jxl first per issue #8.
inline const std::vector<OutputFormat>& allOutputFormats() {
    static const std::vector<OutputFormat> kAll = {
        OutputFormat::Jxl,   OutputFormat::Jpeg, OutputFormat::Png,
        OutputFormat::WebP,  OutputFormat::Tiff, OutputFormat::Bmp,
        OutputFormat::Qoi,   OutputFormat::OpenExr,
    };
    return kAll;
}

// File extension for this format (without leading dot).
const char* extension(OutputFormat fmt);

// Human-readable display name for UI dropdowns.
const char* displayName(OutputFormat fmt);

// Whether this format preserves HDR data (no tone mapping needed).
// true for Jxl and OpenExr only -- see file-header comment for rationale.
bool preservesHdr(OutputFormat fmt);

// Default output format: Jxl (NEW vs Rust's Jpeg default; see file header).
OutputFormat defaultFormat();

// The lowercase wire-format string used for TOML serialization (matches the
// Rust `#[serde(rename_all = "lowercase")]` enum-level attribute, with
// WebP -> "webp" and OpenExr -> "openexr" explicit renames preserved).
const char* toWireString(OutputFormat fmt);

// Parses a wire-format string (as produced by toWireString) into an
// OutputFormat. Returns false if the string does not match any known
// format; *out is left unmodified in that case. Callers that need the
// "unknown format -> default with warning" backward-compat contract should
// use config.h's format-parsing helpers, not this function directly.
bool fromWireString(const std::string& s, OutputFormat* out);

// ======================== PER-FORMAT OPTIONS ========================

// Chroma subsampling modes for JPEG encoding.
enum class ChromaSubsampling {
    Full,     // 4:4:4 -- no subsampling, best quality, largest files.
    Half,     // 4:2:2 -- horizontal subsampling (default, good balance).
    Quarter,  // 4:2:0 -- horizontal + vertical subsampling, smallest files.
};

const char* toWireString(ChromaSubsampling v);
bool fromWireString(const std::string& s, ChromaSubsampling* out);

struct JpegOptions {
    // Quality level (50-100). Higher = larger, sharper. Rust default: 85.
    std::uint32_t quality = 85;
    // Rust default: Half (4:2:2).
    ChromaSubsampling chromaSubsampling = ChromaSubsampling::Half;
};

// PNG pre-compression filter strategies.
enum class PngFilter {
    Adaptive,  // default
    None,
    Sub,
    Up,
    Average,
    Paeth,
};

const char* toWireString(PngFilter v);
bool fromWireString(const std::string& s, PngFilter* out);

struct PngOptions {
    // Compression level: 0 = fast, 6 = default (libpng default), 9 = max.
    std::uint8_t compression = 6;
    PngFilter filter = PngFilter::Adaptive;
};

struct WebPOptions {
    // Lossy vs lossless mode. Rust default: false.
    bool lossless = false;
    // Quality (0-100). Only used in lossy mode. Rust default: 80.0.
    float quality = 80.0f;
};

// TIFF compression options.
enum class TiffCompression {
    None,
    Lzw,  // default
    Deflate,
    Packbits,
};

const char* toWireString(TiffCompression v);
bool fromWireString(const std::string& s, TiffCompression* out);

struct TiffOptions {
    TiffCompression compression = TiffCompression::Lzw;
};

// OpenEXR compression options. B44A has an explicit lowercase-with-suffix
// wire rename ("b44a") mirroring the Rust `#[serde(rename = "b44a")]`.
enum class ExrCompression {
    Uncompressed,
    Rle,
    Zip1,
    Zip16,  // default
    Piz,
    Pxr24,
    B44,
    B44A,
};

const char* toWireString(ExrCompression v);
bool fromWireString(const std::string& s, ExrCompression* out);

struct ExrOptions {
    ExrCompression compression = ExrCompression::Zip16;
};

// JXL-specific encoding options. NEW in C++ -- no Rust precedent since JXL
// did not exist as a format in v0.5.0. Defaults chosen deliberately (flagged
// for review per issue #8):
//   - quality = 1.0f: this is a libjxl "distance" value (butteraugli-ish
//     perceptual distance), NOT a 0-100 quality percentage. Distance is
//     inverted vs quality: 0.0 = mathematically lossless, ~1.0 is the
//     libjxl-documented "visually lossless" sweet spot broadly recognized
//     as comparable to a high JPEG quality (~90-ish equivalent), and it is
//     libjxl's own default distance when none is specified via the CLI
//     (cjxl). The issue spec asked for "~85-equivalent"; 1.0 distance is
//     the closest well-documented, libjxl-native anchor point rather than
//     an invented number, and errs slightly higher quality than 85 to be
//     a safe, uncontroversial default for a screenshot tool where text
//     legibility matters.
//   - effort = 7: libjxl's own default encoder effort (1=fastest/worst,
//     9=slowest/best; 7 is documented as the cjxl default "squirrel"
//     preset), a reasonable speed/size tradeoff for interactive screenshot
//     capture.
//   - lossless = false: NEW field, no Rust precedent. Defaults to false to
//     match the lossy-by-default behavior of every other format in this
//     codebase (Jpeg, WebP all default lossless=false/quality-based).
struct JxlOptions {
    float quality = 1.0f;  // libjxl "distance"; see comment above.
    int effort = 7;
    bool lossless = false;
};

// All format-specific options, bundled together. Every format's options are
// always present (with defaults) even when a different format is selected --
// this preserves user choices when switching formats in the settings
// dialog. Matches Rust's FormatOptions fields exactly (jpeg, png, webp,
// tiff, exr) PLUS a new jxl member (6th field). Bmp and Qoi are optionless
// formats in both Rust and C++ -- do NOT add bmp/qoi options structs, that
// would not match the Rust source.
struct FormatOptions {
    JpegOptions jpeg;
    PngOptions png;
    WebPOptions webp;
    TiffOptions tiff;
    ExrOptions exr;
    JxlOptions jxl;
};

// ======================== CONFIGURATION ========================

// Auto-resize (downscale) options applied after capture.
//
// When enabled, captures wider than max_width or taller than max_height are
// scaled down proportionally so both dimensions fit within the limits.
struct ResizeOptions {
    bool enabled = false;
    std::uint32_t maxWidth = 2048;
    std::uint32_t maxHeight = 2048;
    // When enabled is true AND the capture exceeds the max limits, the
    // saved file and clipboard stay the reduced version, but an additional
    // sibling file with a "_full" suffix is also written containing the
    // untouched original capture at full resolution.
    bool keepOriginal = false;
};

// Capture-related settings: output format, encoding options, directory,
// naming.
struct CaptureConfig {
    OutputFormat format = OutputFormat::Jxl;  // see defaultFormat() above.
    FormatOptions formatOptions;
    // Directory where screenshots are saved. Supports '~' for home
    // directory (see config.h's expandTilde).
    std::string saveDir = "~/Pictures/XDR-Snips";
    // Filename template. "{timestamp}" is replaced with YYYYMMDD_HHmmss.
    std::string filenamePattern = "screenshot_{timestamp}";
    ResizeOptions resize;
};

// Global hotkey binding: which key and which modifier keys trigger a
// capture.
struct HotkeyConfig {
    std::string key = "PrintScreen";
    std::vector<std::string> modifiers;  // default empty
};

// Runtime behavior toggles.
struct BehaviorConfig {
    bool copyToClipboard = true;
    bool saveToFile = true;
    bool showNotification = true;
};

// Top-level application configuration loaded from config.toml.
struct Config {
    CaptureConfig capture;
    HotkeyConfig hotkey;
    BehaviorConfig behavior;
};

// ======================== GEOMETRY ========================

// A rectangular screen region in pixel coordinates.
struct Region {
    std::int32_t x = 0;  // can be negative on multi-monitor setups
    std::int32_t y = 0;
    std::uint32_t w = 0;
    std::uint32_t h = 0;

    // Matches Rust's Display impl: "{w}x{h}+{x}+{y}"
    std::string toString() const;
};

std::ostream& operator<<(std::ostream& os, const Region& r);

}  // namespace snip
