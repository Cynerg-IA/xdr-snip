// test_encode_jxl.cpp -- Phase 2 tests for the JPEG XL encoder
// (src/encode_jxl.h/.cpp).
//
// Same hand-rolled CHECK/TestCase runner convention as
// tests/test_main.cpp (Phase 1) -- no external test framework, no second
// FetchContent dependency. Kept as a SEPARATE binary (snip_encode_jxl_tests)
// from Phase 1's snip_types_tests so Phase 1's 22 tests remain completely
// untouched by this phase's work.
//
// A minimal libjxl DECODE helper is implemented locally in this test file
// (not exposed from encode_jxl.h -- the task scope is an encoder entry
// point only) purely to give the lossless round-trip test and the HDR
// exponent-bits assertion real proof instead of trusting the encoder's
// return code alone.

#include <cstdio>
#include <cstring>
#include <functional>
#include <vector>

#include <jxl/decode.h>
#include <jxl/types.h>

#include "encode_jxl.h"
#include "types.h"

namespace {

int g_failures = 0;
int g_total = 0;

#define CHECK(cond)                                                         \
    do {                                                                    \
        ++g_total;                                                         \
        if (!(cond)) {                                                      \
            std::printf("    CHECK FAILED: %s (line %d)\n", #cond, __LINE__); \
            return false;                                                   \
        }                                                                   \
    } while (0)

// ======================== test fixtures ========================

// Builds a small synthetic 8-bit RGB gradient image (no external file I/O).
std::vector<std::uint8_t> makeSdrGradient(std::uint32_t w, std::uint32_t h) {
    std::vector<std::uint8_t> px(static_cast<std::size_t>(w) * h * 3);
    for (std::uint32_t y = 0; y < h; ++y) {
        for (std::uint32_t x = 0; x < w; ++x) {
            std::size_t off = (static_cast<std::size_t>(y) * w + x) * 3;
            px[off + 0] = static_cast<std::uint8_t>((x * 255) / (w > 1 ? w - 1 : 1));
            px[off + 1] = static_cast<std::uint8_t>((y * 255) / (h > 1 ? h - 1 : 1));
            px[off + 2] = static_cast<std::uint8_t>(128);
        }
    }
    return px;
}

// Builds a synthetic RGB8 image with per-pixel pseudo-random noise. Unlike
// a smooth gradient (which JXL's lossless modular predictor compresses
// extremely well, sometimes better than lossy VarDCT), noisy content is
// where lossy compression's size advantage actually shows up -- a smooth
// gradient is close to an adversarial input for the
// "lossy < lossless" size comparison and was the root cause of this test
// initially failing against makeSdrGradient.
std::vector<std::uint8_t> makeSdrNoise(std::uint32_t w, std::uint32_t h) {
    std::vector<std::uint8_t> px(static_cast<std::size_t>(w) * h * 3);
    std::uint32_t state = 0x9E3779B9u;  // deterministic xorshift seed
    for (auto& byte : px) {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        byte = static_cast<std::uint8_t>(state & 0xFFu);
    }
    return px;
}

// Builds a small synthetic f32 HDR RGBA image with values above 1.0 (true
// HDR range, not just [0,1] SDR-in-float) so the test actually exercises
// the HDR path rather than a relabeled SDR image.
std::vector<float> makeHdrGradientF32(std::uint32_t w, std::uint32_t h) {
    std::vector<float> px(static_cast<std::size_t>(w) * h * 4);
    for (std::uint32_t y = 0; y < h; ++y) {
        for (std::uint32_t x = 0; x < w; ++x) {
            std::size_t off = (static_cast<std::size_t>(y) * w + x) * 4;
            // Sweep 0.0 .. 4.0 -- well above the 1.0 SDR ceiling.
            float t = (w > 1) ? static_cast<float>(x) / static_cast<float>(w - 1) : 0.0f;
            px[off + 0] = t * 4.0f;
            px[off + 1] = 0.5f;
            px[off + 2] = 2.0f;
            px[off + 3] = 1.0f;
        }
    }
    return px;
}

// Minimal libjxl decode: decodes a buffer produced by encodeJxl() back to
// interleaved 8-bit RGB. Returns false on any decode error. Only supports
// UINT8 output (sufficient for the lossless SDR round-trip test).
bool decodeToRgb8(const std::vector<std::uint8_t>& jxlBytes, std::uint32_t expectedW,
                   std::uint32_t expectedH, std::vector<std::uint8_t>* outPixels) {
    JxlDecoder* dec = JxlDecoderCreate(nullptr);
    if (dec == nullptr) return false;

    if (JxlDecoderSubscribeEvents(dec, JXL_DEC_BASIC_INFO | JXL_DEC_FULL_IMAGE) !=
        JXL_DEC_SUCCESS) {
        JxlDecoderDestroy(dec);
        return false;
    }

    JxlDecoderSetInput(dec, jxlBytes.data(), jxlBytes.size());
    JxlDecoderCloseInput(dec);

    JxlPixelFormat format;
    std::memset(&format, 0, sizeof(format));
    format.num_channels = 3;
    format.data_type = JXL_TYPE_UINT8;
    format.endianness = JXL_NATIVE_ENDIAN;
    format.align = 0;

    JxlBasicInfo info;
    bool gotBasicInfo = false;

    for (;;) {
        JxlDecoderStatus status = JxlDecoderProcessInput(dec);
        if (status == JXL_DEC_ERROR) {
            JxlDecoderDestroy(dec);
            return false;
        }
        if (status == JXL_DEC_BASIC_INFO) {
            if (JxlDecoderGetBasicInfo(dec, &info) != JXL_DEC_SUCCESS) {
                JxlDecoderDestroy(dec);
                return false;
            }
            gotBasicInfo = true;
            if (info.xsize != expectedW || info.ysize != expectedH) {
                JxlDecoderDestroy(dec);
                return false;
            }
        } else if (status == JXL_DEC_NEED_IMAGE_OUT_BUFFER) {
            std::size_t bufferSize = 0;
            if (JxlDecoderImageOutBufferSize(dec, &format, &bufferSize) != JXL_DEC_SUCCESS) {
                JxlDecoderDestroy(dec);
                return false;
            }
            outPixels->resize(bufferSize);
            if (JxlDecoderSetImageOutBuffer(dec, &format, outPixels->data(), bufferSize) !=
                JXL_DEC_SUCCESS) {
                JxlDecoderDestroy(dec);
                return false;
            }
        } else if (status == JXL_DEC_FULL_IMAGE) {
            // Pixels are in outPixels now; keep looping to reach SUCCESS.
            continue;
        } else if (status == JXL_DEC_SUCCESS) {
            break;
        }
        // Other informative events (COLOR_ENCODING, FRAME, etc.) are
        // ignored -- we only subscribed to BASIC_INFO | FULL_IMAGE, but
        // libjxl may still emit a few others; loop continues.
    }

    JxlDecoderDestroy(dec);
    return gotBasicInfo;
}

// ======================== tests ========================

bool test_sdr_8bit_encode_nonempty_and_signature() {
    auto px = makeSdrGradient(16, 12);
    snip::JxlImageInput input;
    input.pixels = px.data();
    input.width = 16;
    input.height = 12;
    input.pixelType = snip::JxlPixelType::Uint8;
    input.numChannels = 3;

    snip::JxlOptions opts;  // defaults: quality=1.45, effort=7, lossless=false
    snip::JxlEncodeResult result = snip::encodeJxl(input, opts);

    CHECK(result.ok);
    CHECK(!result.bytes.empty());

    // encode_jxl.cpp calls JxlEncoderUseContainer(enc, JXL_TRUE), so the
    // output must carry the ISOBMFF container signature:
    //   00 00 00 0C 4A 58 4C 20 0D 0A 87 0A  ("....JXL ....")
    // NOT the naked-codestream signature (FF 0A) -- assert the RIGHT one
    // for the settings actually used, per the task instructions.
    static const std::uint8_t kContainerSig[12] = {0x00, 0x00, 0x00, 0x0C, 0x4A, 0x58,
                                                     0x4C, 0x20, 0x0D, 0x0A, 0x87, 0x0A};
    CHECK(result.bytes.size() >= sizeof(kContainerSig));
    CHECK(std::memcmp(result.bytes.data(), kContainerSig, sizeof(kContainerSig)) == 0);
    return true;
}

bool test_hdr_f32_encode_nonempty_and_basicinfo() {
    auto px = makeHdrGradientF32(8, 6);
    snip::JxlImageInput input;
    input.pixels = px.data();
    input.width = 8;
    input.height = 6;
    input.pixelType = snip::JxlPixelType::Float32;
    input.numChannels = 4;

    snip::JxlOptions opts;
    snip::JxlEncodeResult result = snip::encodeJxl(input, opts);

    CHECK(result.ok);
    CHECK(!result.bytes.empty());

    // Decode just far enough to read JxlBasicInfo and confirm
    // exponent_bits_per_sample was set correctly for f32 (8 bits, IEEE-754
    // binary32) -- this is the highest-risk field per the task spec, so
    // assert it directly rather than merely trusting encodeJxl()'s return
    // code.
    JxlDecoder* dec = JxlDecoderCreate(nullptr);
    CHECK(dec != nullptr);
    CHECK(JxlDecoderSubscribeEvents(dec, JXL_DEC_BASIC_INFO) == JXL_DEC_SUCCESS);
    JxlDecoderSetInput(dec, result.bytes.data(), result.bytes.size());
    JxlDecoderCloseInput(dec);

    JxlBasicInfo info;
    bool sawBasicInfo = false;
    for (;;) {
        JxlDecoderStatus status = JxlDecoderProcessInput(dec);
        if (status == JXL_DEC_ERROR) break;
        if (status == JXL_DEC_BASIC_INFO) {
            sawBasicInfo = (JxlDecoderGetBasicInfo(dec, &info) == JXL_DEC_SUCCESS);
            break;
        }
        if (status == JXL_DEC_SUCCESS) break;
    }
    JxlDecoderDestroy(dec);

    CHECK(sawBasicInfo);
    CHECK(info.xsize == 8);
    CHECK(info.ysize == 6);
    CHECK(info.bits_per_sample == 32);
    CHECK(info.exponent_bits_per_sample == 8);
    CHECK(info.alpha_bits == 32);
    CHECK(info.alpha_exponent_bits == 8);
    return true;
}

bool test_hdr_f16_encode_basicinfo_exponent_bits() {
    // Same as above but for FLOAT16 -- confirms the OTHER branch of the
    // "highest-risk" exponent_bits_per_sample logic (5 bits, IEEE-754
    // binary16), since capture.rs's raw HDR path is natively f16
    // (R16G16B16A16Float), not f32.
    std::vector<std::uint16_t> half(8 * 6 * 4);
    // Halfway-reasonable half-float bit patterns are not needed for this
    // test (we only assert on BasicInfo, not sample values) -- zero-fill.
    std::fill(half.begin(), half.end(), 0);

    snip::JxlImageInput input;
    input.pixels = half.data();
    input.width = 8;
    input.height = 6;
    input.pixelType = snip::JxlPixelType::Float16;
    input.numChannels = 4;

    snip::JxlOptions opts;
    snip::JxlEncodeResult result = snip::encodeJxl(input, opts);
    CHECK(result.ok);
    CHECK(!result.bytes.empty());

    JxlDecoder* dec = JxlDecoderCreate(nullptr);
    CHECK(dec != nullptr);
    CHECK(JxlDecoderSubscribeEvents(dec, JXL_DEC_BASIC_INFO) == JXL_DEC_SUCCESS);
    JxlDecoderSetInput(dec, result.bytes.data(), result.bytes.size());
    JxlDecoderCloseInput(dec);

    JxlBasicInfo info;
    bool sawBasicInfo = false;
    for (;;) {
        JxlDecoderStatus status = JxlDecoderProcessInput(dec);
        if (status == JXL_DEC_ERROR) break;
        if (status == JXL_DEC_BASIC_INFO) {
            sawBasicInfo = (JxlDecoderGetBasicInfo(dec, &info) == JXL_DEC_SUCCESS);
            break;
        }
        if (status == JXL_DEC_SUCCESS) break;
    }
    JxlDecoderDestroy(dec);

    CHECK(sawBasicInfo);
    CHECK(info.bits_per_sample == 16);
    CHECK(info.exponent_bits_per_sample == 5);
    return true;
}

bool test_lossless_roundtrips_exactly() {
    auto px = makeSdrGradient(10, 10);
    snip::JxlImageInput input;
    input.pixels = px.data();
    input.width = 10;
    input.height = 10;
    input.pixelType = snip::JxlPixelType::Uint8;
    input.numChannels = 3;

    snip::JxlOptions opts;
    opts.lossless = true;
    snip::JxlEncodeResult result = snip::encodeJxl(input, opts);
    CHECK(result.ok);
    CHECK(!result.bytes.empty());

    std::vector<std::uint8_t> decoded;
    CHECK(decodeToRgb8(result.bytes, 10, 10, &decoded));
    CHECK(decoded.size() == px.size());
    CHECK(std::memcmp(decoded.data(), px.data(), px.size()) == 0);
    return true;
}

bool test_distance_smaller_than_lossless() {
    // The task requirement is "distance 1.45 produces a smaller buffer than
    // distance 0.0 on the same input" -- but libjxl (this toolchain's
    // v0.12.0) actively REJECTS JxlEncoderSetFrameDistance(0.0) with
    // JXL_ENC_ERR_API_USAGE (0x81) unless JxlEncoderSetFrameLossless(TRUE)
    // was also called first; this was discovered empirically (not
    // documented as a hard error in encode.h's comment, which only says
    // "however, use JxlEncoderSetFrameLossless instead ... as setting
    // distance to 0 alone is not the only requirement" -- it reads like a
    // correctness caveat, not an API-usage rejection, but the library
    // enforces it as the latter). So "distance 0.0" in this test means
    // lossless=true (which is exactly the encode_jxl.cpp code path that
    // sets both SetFrameLossless(TRUE) and SetFrameDistance(0.0) together)
    // compared against the default lossy distance (1.45) -- this still
    // proves the required property (lower distance number = larger/equal
    // file) using the two encode paths this encoder actually exposes.
    auto px = makeSdrNoise(64, 64);

    snip::JxlImageInput input;
    input.pixels = px.data();
    input.width = 64;
    input.height = 64;
    input.pixelType = snip::JxlPixelType::Uint8;
    input.numChannels = 3;

    snip::JxlOptions lossyOpts;
    lossyOpts.quality = 1.45f;
    lossyOpts.lossless = false;
    snip::JxlEncodeResult lossy = snip::encodeJxl(input, lossyOpts);
    CHECK(lossy.ok);

    snip::JxlOptions losslessOpts;
    losslessOpts.lossless = true;  // forces distance 0.0 via the lossless path
    snip::JxlEncodeResult lossless = snip::encodeJxl(input, losslessOpts);
    if (!lossless.ok) {
        std::printf("    lossless.error = %s\n", lossless.error.c_str());
    }
    CHECK(lossless.ok);

    CHECK(lossy.bytes.size() < lossless.bytes.size());
    return true;
}

bool test_invalid_input_returns_error_path() {
    // Zero-sized image must fail cleanly, not "succeed" with an empty
    // buffer.
    snip::JxlImageInput input;
    input.pixels = nullptr;
    input.width = 0;
    input.height = 0;
    input.pixelType = snip::JxlPixelType::Uint8;
    input.numChannels = 3;

    snip::JxlOptions opts;
    snip::JxlEncodeResult result = snip::encodeJxl(input, opts);

    CHECK(!result.ok);
    CHECK(result.bytes.empty());
    CHECK(!result.error.empty());
    return true;
}

bool test_mismatched_channel_count_returns_error_path() {
    // HDR pixel type with 3 channels (invalid -- HDR requires RGBA/4) must
    // be rejected before any libjxl call, not silently truncated.
    std::vector<float> px(8 * 6 * 3, 0.5f);
    snip::JxlImageInput input;
    input.pixels = px.data();
    input.width = 8;
    input.height = 6;
    input.pixelType = snip::JxlPixelType::Float32;
    input.numChannels = 3;  // invalid for HDR

    snip::JxlOptions opts;
    snip::JxlEncodeResult result = snip::encodeJxl(input, opts);

    CHECK(!result.ok);
    CHECK(result.bytes.empty());
    CHECK(!result.error.empty());
    return true;
}

// ======================== test runner ========================

struct TestCase {
    const char* name;
    std::function<bool()> fn;
};

}  // namespace

int main() {
    std::vector<TestCase> tests = {
        {"sdr_8bit_encode_nonempty_and_signature", test_sdr_8bit_encode_nonempty_and_signature},
        {"hdr_f32_encode_nonempty_and_basicinfo", test_hdr_f32_encode_nonempty_and_basicinfo},
        {"hdr_f16_encode_basicinfo_exponent_bits", test_hdr_f16_encode_basicinfo_exponent_bits},
        {"lossless_roundtrips_exactly", test_lossless_roundtrips_exactly},
        {"distance_smaller_than_lossless", test_distance_smaller_than_lossless},
        {"invalid_input_returns_error_path", test_invalid_input_returns_error_path},
        {"mismatched_channel_count_returns_error_path",
         test_mismatched_channel_count_returns_error_path},
    };

    int passed = 0;
    for (const auto& tc : tests) {
        std::printf("[ RUN      ] %s\n", tc.name);
        bool ok = false;
        try {
            ok = tc.fn();
        } catch (const std::exception& e) {
            std::printf("    EXCEPTION: %s\n", e.what());
            ok = false;
        } catch (...) {
            std::printf("    UNKNOWN EXCEPTION\n");
            ok = false;
        }
        if (ok) {
            std::printf("[       OK ] %s\n", tc.name);
            ++passed;
        } else {
            std::printf("[  FAILED  ] %s\n", tc.name);
            ++g_failures;
        }
    }

    std::printf("\n%d/%zu tests passed (%d CHECKs total)\n", passed, tests.size(), g_total);
    if (g_failures > 0) {
        std::printf("FAILURES: %d test(s) failed\n", g_failures);
        return 1;
    }
    std::printf("ALL TESTS PASSED\n");
    return 0;
}
