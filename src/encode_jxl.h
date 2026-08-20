// encode_jxl.h -- JPEG XL encoder entry point for XDR Snip v0.6.0 Phase 2.
//
// Wraps libjxl v0.12.0's C encoder API (JxlEncoder*) behind a small,
// dependency-light interface that mirrors the semantics of the Rust
// encode_image() dispatch in crates/snip-app/src/capture.rs for the other
// 7 formats, but is JXL-specific (see issue #7/#8: JXL becomes the default
// format because it is the only new-in-C++ format capable of carrying both
// SDR 8-bit AND HDR float pixel data through one codec).
//
// Pixel data contract (mirrors capture.rs's two pixel sources):
//   - SDR:  interleaved RGB8, 3 bytes/pixel, no alpha. Matches the
//     `rgb_pixels: &[u8]` (RGB8, tone-mapped) input every other v0.5.0
//     format encoder takes.
//   - HDR:  interleaved RGBA float, either 4x float16 or 4x float32 per
//     pixel (8 or 16 bytes/pixel respectively). This is new: v0.5.0's only
//     raw-HDR path (OpenEXR, HdrPixelData) carried R16G16B16A16Float
//     (half-precision, 8 bytes/px, 4 channels). JXL Phase 2 accepts that
//     representation directly via JXL_TYPE_FLOAT16, OR a float32 buffer
//     via JXL_TYPE_FLOAT if the caller already promoted to f32 (e.g. the
//     same rgba_f32 buffer capture.rs builds for encode_exr_hdr/_sdr).
//     preservesHdr() for Jxl (types.h) is what makes this path reachable.
//
// Error handling: NEVER returns a truncated/empty buffer on failure. Every
// libjxl call is checked; the first failure aborts the encode and returns
// a JxlEncodeResult with ok=false and a human-readable message describing
// which libjxl call failed and its JxlEncoderError code where available.

#pragma once

#include <cstdint>
#include <string>
#include <vector>

#include "types.h"

namespace snip {

// Pixel buffer sample type for encodeJxl's input. Mirrors a subset of
// libjxl's JxlDataType (jxl/types.h) -- only the types this encoder
// actually accepts are exposed here, so callers can't pass something the
// wrapper does not handle (e.g. UINT16).
enum class JxlPixelType {
    Uint8,    // SDR: interleaved RGB8, 3 bytes/pixel, no alpha.
    Float16,  // HDR: interleaved RGBA half-float, 8 bytes/pixel.
    Float32,  // HDR: interleaved RGBA float, 16 bytes/pixel.
};

// Describes the pixel buffer passed to encodeJxl().
struct JxlImageInput {
    const void* pixels = nullptr;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    JxlPixelType pixelType = JxlPixelType::Uint8;
    // Number of channels in `pixels`: 3 (RGB, only valid with Uint8) or 4
    // (RGBA, only valid with Float16/Float32). Enforced in encode_jxl.cpp.
    std::uint32_t numChannels = 3;
};

// Result of a JXL encode attempt. Never contains a partial/truncated buffer
// when ok == false -- bytes is guaranteed empty in that case.
struct JxlEncodeResult {
    bool ok = false;
    std::vector<std::uint8_t> bytes;
    // Populated when ok == false: which libjxl call failed and why. Always
    // human-readable; never empty when ok == false.
    std::string error;
};

// Encodes `input` to a JPEG XL bitstream using `options` (see types.h's
// JxlOptions: quality is a libjxl butteraugli DISTANCE, not 0-100 quality;
// effort is the JXL_ENC_FRAME_SETTING_EFFORT tier 1-9; lossless forces
// mathematically-lossless encoding regardless of quality/distance).
//
// Runs single-threaded (no JxlEncoderSetParallelRunner call, i.e. libjxl's
// built-in sequential default) -- this deliberately avoids linking
// jxl_threads, which does not compile on GCC/mingw in this toolchain
// (Phase 0 finding, issue #7 KNOWN TRAP). Screenshot-sized images do not
// need parallel encode to be interactive at effort<=7.
JxlEncodeResult encodeJxl(const JxlImageInput& input, const JxlOptions& options);

}  // namespace snip
