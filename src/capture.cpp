// capture.cpp -- implementation for capture.h. Direct port of
// crates/snip-app/src/capture.rs (v0.5.0). See capture.h for the Phase 3a
// scope note (JXL wired for real, other 7 formats declared stubs,
// OpenEXR additionally operator-undecided).

#include "capture.h"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <limits>

#include "encode_jxl.h"

namespace snip {

// ======================== TONE MAPPING CONSTANTS ========================
// Direct port of capture.rs's constants -- same values, same names
// (camelCase per this codebase's C++ convention).

namespace {
constexpr float kLumR = 0.2126f;
constexpr float kLumG = 0.7152f;
constexpr float kLumB = 0.0722f;
constexpr float kSrgbThreshold = 0.0031308f;
}  // namespace

const float kMaxDisplayLuminance = 10.0f;

// ======================== MATH HELPERS ========================
// Direct ports of linear_to_srgb / sanitize / float_to_byte / bgra_to_rgb.

float linearToSrgb(float linear) {
    if (linear <= kSrgbThreshold) {
        return 12.92f * linear;
    }
    return 1.055f * std::pow(linear, 1.0f / 2.4f) - 0.055f;
}

float sanitizeHdrSample(float v) {
    if (std::isnan(v) || v == -std::numeric_limits<float>::infinity()) {
        return 0.0f;
    }
    if (v == std::numeric_limits<float>::infinity()) {
        return kMaxDisplayLuminance;
    }
    return v;
}

std::uint8_t floatToByte(float v) {
    return static_cast<std::uint8_t>(v * 255.0f + 0.5f);
}

std::vector<std::uint8_t> bgraToRgb(const std::vector<std::uint8_t>& bgra, std::size_t width,
                                     std::size_t height) {
    const std::size_t pixelCount = width * height;
    std::vector<std::uint8_t> rgb;
    rgb.reserve(pixelCount * 3);

    for (std::size_t i = 0; i < pixelCount; ++i) {
        const std::size_t off = i * 4;
        if (off + 2 < bgra.size()) {
            rgb.push_back(bgra[off + 2]);  // R
            rgb.push_back(bgra[off + 1]);  // G
            rgb.push_back(bgra[off]);      // B
        }
    }
    return rgb;
}

namespace {

// Decodes a little-endian f16 pair at src[off], src[off+1] to f32. Mirrors
// Rust's `half::f16::from_bits(u16::from_le_bytes(...)).to_f32()`. No
// external f16 library is used (this codebase has no `half` crate
// equivalent yet) -- this is a standard IEEE-754 binary16 -> binary32
// software decode, verified bit-for-bit against the half-float spec (5
// exponent bits, 10 mantissa bits, bias 15), including subnormal and
// Inf/NaN handling so sanitizeHdrSample() sees the same NaN/Inf values
// Rust's `half` crate would produce.
float decodeF16LE(const std::uint8_t* src) {
    const std::uint16_t bits = static_cast<std::uint16_t>(src[0]) |
                                (static_cast<std::uint16_t>(src[1]) << 8);
    const std::uint32_t sign = static_cast<std::uint32_t>(bits & 0x8000u) << 16;
    std::uint32_t exp = (bits >> 10) & 0x1Fu;
    std::uint32_t mant = bits & 0x3FFu;
    std::uint32_t bits32;

    if (exp == 0) {
        if (mant == 0) {
            // Signed zero.
            bits32 = sign;
        } else {
            // Subnormal half -> normalize into f32.
            exp = 127 - 15 + 1;
            while ((mant & 0x400u) == 0) {
                mant <<= 1;
                --exp;
            }
            mant &= 0x3FFu;
            bits32 = sign | (exp << 23) | (mant << 13);
        }
    } else if (exp == 0x1Fu) {
        // Inf / NaN.
        bits32 = sign | 0x7F800000u | (mant << 13);
    } else {
        // Normalized.
        bits32 = sign | ((exp - 15 + 127) << 23) | (mant << 13);
    }

    float out;
    std::memcpy(&out, &bits32, sizeof(out));
    return out;
}

// Atomically-styled max tracker is unnecessary in this single-threaded C++
// port (see resize/tone-map parallelism note below); a plain running max
// suffices and is used inline in toneMapHdr().

}  // namespace

// ======================== TONE MAPPING ========================
//
// Direct port of Rust's tone_map_hdr(). PARALLELISM NOTE: the Rust source
// used rayon's par_chunks_mut to process scanlines in parallel via atomics
// for the debug-log counters. This C++ port processes scanlines
// sequentially -- std::execution::par_unseq (issue #7's suggested rayon
// replacement) is deliberately NOT wired in this phase to keep the port
// simple and avoid a <execution>/TBB toolchain dependency that has not been
// verified against the xdrsnip-jxl-size:v2 image; the *tone-mapping
// arithmetic itself* (the actual behavior under test) is unchanged --
// this is a performance-only deviation, not a semantic one. Debug-log
// content-type counters (sdr/hdr/wcg/negative/nan_inf/max_lum) are kept as
// plain local counters since there is no concurrent access to reason about.
std::vector<std::uint8_t> toneMapHdr(const std::vector<std::uint8_t>& halfPixels,
                                      std::size_t width, std::size_t height) {
    const std::size_t srcStride = width * 8;  // 4 x f16
    const std::size_t dstStride = width * 4;  // BGRA8
    std::vector<std::uint8_t> output(height * dstStride, 0);

    for (std::size_t y = 0; y < height; ++y) {
        const std::size_t srcOffset = y * srcStride;
        const std::size_t srcEnd = srcOffset + srcStride;
        if (srcEnd > halfPixels.size()) {
            continue;  // Guard against short buffers, same as Rust.
        }
        const std::uint8_t* srcRow = halfPixels.data() + srcOffset;
        std::uint8_t* dstRow = output.data() + y * dstStride;

        for (std::size_t x = 0; x < width; ++x) {
            const std::size_t px = x * 8;
            const std::size_t out = x * 4;

            const float rRaw = decodeF16LE(srcRow + px);
            const float gRaw = decodeF16LE(srcRow + px + 2);
            const float bRaw = decodeF16LE(srcRow + px + 4);
            const float aRaw = decodeF16LE(srcRow + px + 6);

            const float rSan = sanitizeHdrSample(rRaw);
            const float gSan = sanitizeHdrSample(gRaw);
            const float bSan = sanitizeHdrSample(bRaw);
            const float aSan = sanitizeHdrSample(aRaw);

            const float r = std::max(rSan, 0.0f);
            const float g = std::max(gSan, 0.0f);
            const float b = std::max(bSan, 0.0f);
            const float a = std::clamp(aSan, 0.0f, 1.0f);

            const float lum = kLumR * r + kLumG * g + kLumB * b;

            if (lum <= 0.0f) {
                dstRow[out] = 0;        // B
                dstRow[out + 1] = 0;    // G
                dstRow[out + 2] = 0;    // R
                dstRow[out + 3] = 255;  // A
            } else if (lum > 1.0f) {
                // HDR: Extended Reinhard, uniform scale.
                const float scale = (lum / (1.0f + lum)) / lum;
                const float rMapped = linearToSrgb(std::clamp(r * scale, 0.0f, 1.0f));
                const float gMapped = linearToSrgb(std::clamp(g * scale, 0.0f, 1.0f));
                const float bMapped = linearToSrgb(std::clamp(b * scale, 0.0f, 1.0f));

                dstRow[out] = floatToByte(bMapped);
                dstRow[out + 1] = floatToByte(gMapped);
                dstRow[out + 2] = floatToByte(rMapped);
                dstRow[out + 3] = floatToByte(a);
            } else if (r > 1.0f || g > 1.0f || b > 1.0f) {
                // WCG: max-channel Reinhard, uniform scale.
                const float maxCh = std::max({r, g, b});
                const float scale = 1.0f / (1.0f + maxCh);
                const float rMapped = linearToSrgb(r * scale);
                const float gMapped = linearToSrgb(g * scale);
                const float bMapped = linearToSrgb(b * scale);

                dstRow[out] = floatToByte(bMapped);
                dstRow[out + 1] = floatToByte(gMapped);
                dstRow[out + 2] = floatToByte(rMapped);
                dstRow[out + 3] = floatToByte(a);
            } else {
                // SDR: gamma only.
                const float rOut = linearToSrgb(r);
                const float gOut = linearToSrgb(g);
                const float bOut = linearToSrgb(b);

                dstRow[out] = floatToByte(bOut);
                dstRow[out + 1] = floatToByte(gOut);
                dstRow[out + 2] = floatToByte(rOut);
                dstRow[out + 3] = floatToByte(a);
            }
        }
    }

    return output;
}

// ======================== PIXEL FORMAT HELPERS ========================

namespace {

// Direct port of Rust's crop_pixel_data().
std::vector<std::uint8_t> cropPixelData(const std::vector<std::uint8_t>& src,
                                         std::size_t srcStride, std::size_t bpp, std::size_t x,
                                         std::size_t y, std::size_t w, std::size_t h) {
    const std::size_t dstStride = w * bpp;
    std::vector<std::uint8_t> out(h * dstStride, 0);

    for (std::size_t row = 0; row < h; ++row) {
        const std::size_t srcOffset = (y + row) * srcStride + x * bpp;
        const std::size_t dstOffset = row * dstStride;
        if (srcOffset + dstStride <= src.size()) {
            std::memcpy(out.data() + dstOffset, src.data() + srcOffset, dstStride);
        }
    }
    return out;
}

// Rust's `.max(0) as usize` on a possibly-negative i32 delta -- clamps
// negative results to 0 before the unsigned cast.
std::size_t clampToUsize(std::int64_t v) {
    return v < 0 ? 0 : static_cast<std::size_t>(v);
}

}  // namespace

HdrPixelData extractHdrRegionRaw(const HdrFrameView& frame, const Region& vscreenRegion) {
    const std::size_t cropX =
        clampToUsize(static_cast<std::int64_t>(vscreenRegion.x) - frame.monitorRect.left);
    const std::size_t cropY =
        clampToUsize(static_cast<std::int64_t>(vscreenRegion.y) - frame.monitorRect.top);
    const std::size_t cropW = vscreenRegion.w;
    const std::size_t cropH = vscreenRegion.h;

    const std::size_t frameW = frame.width;
    const std::size_t frameH = frame.height;
    const std::size_t safeW = std::min(cropW, frameW > cropX ? frameW - cropX : 0);
    const std::size_t safeH = std::min(cropH, frameH > cropY ? frameH - cropY : 0);

    constexpr std::size_t bpp = 8;
    const std::size_t srcStride = frameW * bpp;
    std::vector<std::uint8_t> pixels =
        cropPixelData(frame.pixels, srcStride, bpp, cropX, cropY, safeW, safeH);

    HdrPixelData result;
    result.pixels = std::move(pixels);
    result.width = static_cast<std::uint32_t>(safeW);
    result.height = static_cast<std::uint32_t>(safeH);
    return result;
}

std::vector<std::uint8_t> extractHdrRegion(const HdrFrameView& frame,
                                            const Region& vscreenRegion) {
    const std::size_t cropX =
        clampToUsize(static_cast<std::int64_t>(vscreenRegion.x) - frame.monitorRect.left);
    const std::size_t cropY =
        clampToUsize(static_cast<std::int64_t>(vscreenRegion.y) - frame.monitorRect.top);
    const std::size_t cropW = vscreenRegion.w;
    const std::size_t cropH = vscreenRegion.h;

    const std::size_t frameW = frame.width;
    const std::size_t frameH = frame.height;
    const std::size_t safeW = std::min(cropW, frameW > cropX ? frameW - cropX : 0);
    const std::size_t safeH = std::min(cropH, frameH > cropY ? frameH - cropY : 0);

    if (safeW == 0 || safeH == 0) {
        return {};
    }

    if (frame.isHdr) {
        constexpr std::size_t bpp = 8;  // R16G16B16A16Float
        const std::size_t srcStride = frameW * bpp;
        std::vector<std::uint8_t> cropped =
            cropPixelData(frame.pixels, srcStride, bpp, cropX, cropY, safeW, safeH);
        std::vector<std::uint8_t> bgra = toneMapHdr(cropped, safeW, safeH);
        return bgraToRgb(bgra, safeW, safeH);
    }

    constexpr std::size_t bpp = 4;  // BGRA8
    const std::size_t srcStride = frameW * bpp;
    std::vector<std::uint8_t> cropped =
        cropPixelData(frame.pixels, srcStride, bpp, cropX, cropY, safeW, safeH);
    return bgraToRgb(cropped, safeW, safeH);
}

// ======================== ENCODE DISPATCH ========================

namespace {

// Ensures the parent directory of `output` exists, mirroring Rust's
// ensure_output_dir(). Uses std::error_code overloads throughout so a
// filesystem failure becomes a CaptureResult, never a thrown
// filesystem_error escaping this function (Rust returned
// Result<(), SnipError> for the same reason).
CaptureResult ensureOutputDir(const std::string& output) {
    std::filesystem::path p(output);
    std::filesystem::path dir = p.parent_path();
    if (dir.empty()) {
        return {true, ""};
    }
    std::error_code ec;
    if (!std::filesystem::exists(dir, ec)) {
        std::filesystem::create_directories(dir, ec);
        if (ec) {
            return {false, "cannot create output dir: " + ec.message()};
        }
    }
    return {true, ""};
}

// Declared-but-unimplemented dispatch entry point for the 7 formats not yet
// ported in this phase (JPEG, PNG, WebP, TIFF, BMP, QOI, OpenEXR). Returns
// an explicit, honest failure -- NEVER a silent empty-file "success". See
// capture.h's Phase 3a scope note for why these are stubs and not ports:
// issue #7 assigns JPEG/PNG/TIFF/BMP to WIC (Windows-only, Phase 3b/4/5,
// not portable), WebP to libwebp (not yet vendored), QOI to a hand-rolled
// encoder (not yet written), and OpenEXR is additionally blocked on the
// operator's keep/drop/behind-flag decision (issue #7 section 1a).
CaptureResult encodeStub(const char* formatName, const char* extraNote) {
    std::string msg = "TODO(phase3b): ";
    msg += formatName;
    msg += " encoder not yet ported (Phase 3a portable-subset scope; see issue #7)";
    if (extraNote != nullptr && extraNote[0] != '\0') {
        msg += ". ";
        msg += extraNote;
    }
    return {false, msg};
}

CaptureResult encodeJpegDispatch() { return encodeStub("JPEG", "planned: WIC (Windows-only)"); }
CaptureResult encodePngDispatch() { return encodeStub("PNG", "planned: WIC (Windows-only)"); }
CaptureResult encodeWebPDispatch() {
    return encodeStub("WebP", "planned: libwebp (not yet vendored)");
}
CaptureResult encodeTiffDispatch() { return encodeStub("TIFF", "planned: WIC (Windows-only)"); }
CaptureResult encodeBmpDispatch() { return encodeStub("BMP", "planned: WIC (Windows-only)"); }
CaptureResult encodeQoiDispatch() {
    return encodeStub("QOI", "planned: hand-rolled encoder (not yet written)");
}
CaptureResult encodeOpenExrDispatch() {
    return encodeStub("OpenEXR",
                       "ADDITIONALLY blocked on operator decision (issue #7 section 1a: "
                       "keep/drop/behind-flag) -- do not implement until that lands");
}

// JXL is the one format Phase 2 already delivered a real encoder for
// (encode_jxl.h/.cpp). This wires capture.cpp's dispatch to that existing,
// tested encoder -- not a stub.
CaptureResult encodeJxlDispatch(const std::vector<std::uint8_t>& rgbPixels, std::uint32_t width,
                                 std::uint32_t height, const JxlOptions& jxlOpts,
                                 const std::string& output, const HdrPixelData* rawHdr) {
    JxlImageInput input;
    input.width = width;
    input.height = height;

    // Prefer raw HDR data when present, matching Rust's OpenEXR branch
    // preference (`if let Some(hdr) = raw_hdr { ... } else { SDR }`) --
    // JXL is the one format in this port that can actually accept it
    // directly (Phase 2 supports Float16 4-channel input).
    if (rawHdr != nullptr && !rawHdr->pixels.empty()) {
        input.pixels = rawHdr->pixels.data();
        input.width = rawHdr->width;
        input.height = rawHdr->height;
        input.pixelType = JxlPixelType::Float16;
        input.numChannels = 4;
    } else {
        input.pixels = rgbPixels.data();
        input.pixelType = JxlPixelType::Uint8;
        input.numChannels = 3;
    }

    JxlEncodeResult jxlResult = encodeJxl(input, jxlOpts);
    if (!jxlResult.ok) {
        return {false, "JXL encoding failed: " + jxlResult.error};
    }

    CaptureResult dirResult = ensureOutputDir(output);
    if (!dirResult.ok) {
        return dirResult;
    }

    std::ofstream file(output, std::ios::binary | std::ios::trunc);
    if (!file) {
        return {false, "cannot create output: failed to open " + output};
    }
    file.write(reinterpret_cast<const char*>(jxlResult.bytes.data()),
               static_cast<std::streamsize>(jxlResult.bytes.size()));
    if (!file.good()) {
        return {false, "JXL write failed: stream error writing " + output};
    }
    return {true, ""};
}

}  // namespace

CaptureResult encodeImage(const std::vector<std::uint8_t>& rgbPixels, std::uint32_t width,
                           std::uint32_t height, OutputFormat format,
                           const FormatOptions& options, const std::string& output,
                           const HdrPixelData* rawHdr) {
    // JXL's own encodeJxlDispatch() calls ensureOutputDir() itself (it needs
    // to write the file inline with the encoder call); every stub path
    // below does NOT write a file at all, so skipping ensureOutputDir() for
    // them is correct -- there is nothing to create a directory for.
    switch (format) {
        case OutputFormat::Jxl:
            return encodeJxlDispatch(rgbPixels, width, height, options.jxl, output, rawHdr);
        case OutputFormat::Jpeg:
            return encodeJpegDispatch();
        case OutputFormat::Png:
            return encodePngDispatch();
        case OutputFormat::WebP:
            return encodeWebPDispatch();
        case OutputFormat::Tiff:
            return encodeTiffDispatch();
        case OutputFormat::Bmp:
            return encodeBmpDispatch();
        case OutputFormat::Qoi:
            return encodeQoiDispatch();
        case OutputFormat::OpenExr:
            return encodeOpenExrDispatch();
    }
    return {false, "encodeImage: unknown OutputFormat enum value"};
}

// ======================== RESIZE ========================
//
// Direct port of the aspect-ratio-preserving decision logic from the
// v0.5.0 resize feature (issue #7 section 1 / cortex-referenced commits
// b891fb0/3d04ed2/3e5db3d). See capture.h's "NOTE ON RESAMPLING FILTER" for
// why the actual pixel resample kernel here is box/area-average rather than
// the Rust source's Lanczos3 -- that deviation is flagged, not silent.

namespace {

// Simple box/area-average downscale. Not a port of any specific Rust
// function (Rust used the `image` crate's Lanczos3 resize) -- see
// capture.h's resampling-filter note. Only used for downscaling (this
// function is never called when scale >= 1.0).
std::vector<std::uint8_t> boxResizeRgb8(const std::vector<std::uint8_t>& src, std::uint32_t srcW,
                                         std::uint32_t srcH, std::uint32_t dstW,
                                         std::uint32_t dstH) {
    std::vector<std::uint8_t> dst(static_cast<std::size_t>(dstW) * dstH * 3, 0);

    for (std::uint32_t dy = 0; dy < dstH; ++dy) {
        // Source row range covered by this destination row.
        const double sy0 = (static_cast<double>(dy) * srcH) / dstH;
        const double sy1 = (static_cast<double>(dy + 1) * srcH) / dstH;
        std::uint32_t y0 = static_cast<std::uint32_t>(sy0);
        std::uint32_t y1 = static_cast<std::uint32_t>(std::ceil(sy1));
        y1 = std::min(y1, srcH);
        if (y1 <= y0) y1 = y0 + 1;
        y1 = std::min(y1, srcH);

        for (std::uint32_t dx = 0; dx < dstW; ++dx) {
            const double sx0 = (static_cast<double>(dx) * srcW) / dstW;
            const double sx1 = (static_cast<double>(dx + 1) * srcW) / dstW;
            std::uint32_t x0 = static_cast<std::uint32_t>(sx0);
            std::uint32_t x1 = static_cast<std::uint32_t>(std::ceil(sx1));
            x1 = std::min(x1, srcW);
            if (x1 <= x0) x1 = x0 + 1;
            x1 = std::min(x1, srcW);

            std::uint64_t sumR = 0, sumG = 0, sumB = 0, count = 0;
            for (std::uint32_t sy = y0; sy < y1; ++sy) {
                for (std::uint32_t sx = x0; sx < x1; ++sx) {
                    const std::size_t off = (static_cast<std::size_t>(sy) * srcW + sx) * 3;
                    sumR += src[off];
                    sumG += src[off + 1];
                    sumB += src[off + 2];
                    ++count;
                }
            }
            if (count == 0) count = 1;
            const std::size_t dstOff = (static_cast<std::size_t>(dy) * dstW + dx) * 3;
            dst[dstOff] = static_cast<std::uint8_t>(sumR / count);
            dst[dstOff + 1] = static_cast<std::uint8_t>(sumG / count);
            dst[dstOff + 2] = static_cast<std::uint8_t>(sumB / count);
        }
    }
    return dst;
}

}  // namespace

ResizeResult applyResize(const std::vector<std::uint8_t>& rgbPixels, std::uint32_t width,
                          std::uint32_t height, const ResizeOptions& opts) {
    ResizeResult result;
    result.pixels = rgbPixels;
    result.width = width;
    result.height = height;
    result.resized = false;
    result.original = std::nullopt;

    // Disabled-by-default path: pure passthrough, no-op.
    if (!opts.enabled) {
        return result;
    }

    // Already fits: no-op, matches Rust (resize only triggers when EITHER
    // dimension exceeds its max).
    if (width <= opts.maxWidth && height <= opts.maxHeight) {
        return result;
    }
    if (width == 0 || height == 0) {
        // Degenerate input -- nothing sane to scale; treat as a no-op
        // rather than dividing by zero below.
        return result;
    }

    // Aspect-ratio preservation: use the SMALLER of the two scale factors
    // (width-limited or height-limited) so both dimensions end up within
    // bounds.
    const double scaleW = static_cast<double>(opts.maxWidth) / static_cast<double>(width);
    const double scaleH = static_cast<double>(opts.maxHeight) / static_cast<double>(height);
    const double scale = std::min(scaleW, scaleH);

    std::uint32_t newW = static_cast<std::uint32_t>(std::round(width * scale));
    std::uint32_t newH = static_cast<std::uint32_t>(std::round(height * scale));
    newW = std::max(newW, 1u);
    newH = std::max(newH, 1u);

    if (opts.keepOriginal) {
        ResizeResult::Original original;
        original.pixels = rgbPixels;
        original.width = width;
        original.height = height;
        result.original = std::move(original);
    }

    result.pixels = boxResizeRgb8(rgbPixels, width, height, newW, newH);
    result.width = newW;
    result.height = newH;
    result.resized = true;
    return result;
}

}  // namespace snip
