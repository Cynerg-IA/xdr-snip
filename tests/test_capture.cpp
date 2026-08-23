// test_capture.cpp -- Phase 3a tests for capture.h/.cpp, tray.h/.cpp,
// hotkey.h/.cpp, clipboard.h/.cpp.
//
// Same hand-rolled CHECK/TestCase runner convention as tests/test_main.cpp
// (Phase 1) and tests/test_encode_jxl.cpp (Phase 2) -- no external test
// framework. Kept as a SEPARATE binary (snip_capture_tests) so Phase 1's
// 22 tests and Phase 2's 7 tests remain completely untouched.

#include <cmath>
#include <cstdio>
#include <cstring>
#include <functional>
#include <limits>
#include <vector>

#include "capture.h"
#include "clipboard.h"
#include "hotkey.h"
#include "tray.h"
#include "types.h"

namespace {

int g_failures = 0;
int g_total = 0;

#define CHECK(cond)                                                             \
    do {                                                                        \
        ++g_total;                                                              \
        if (!(cond)) {                                                          \
            std::printf("    CHECK FAILED: %s (line %d)\n", #cond, __LINE__);   \
            return false;                                                       \
        }                                                                       \
    } while (0)

// ======================== fixtures ========================

// Encodes an f32 value as little-endian IEEE-754 binary16 (f16) bytes,
// appended to `out`. Used to build synthetic HDR pixel buffers without
// depending on any external half-float library (mirrors the encode side
// of capture.cpp's decodeF16LE()).
void appendF16LE(std::vector<std::uint8_t>* out, float value) {
    std::uint32_t bits32;
    std::memcpy(&bits32, &value, sizeof(bits32));
    const std::uint32_t sign = (bits32 >> 16) & 0x8000u;
    std::int32_t exp = static_cast<std::int32_t>((bits32 >> 23) & 0xFFu) - 127 + 15;
    std::uint32_t mant = bits32 & 0x7FFFFFu;
    std::uint16_t half;

    if (exp <= 0) {
        half = static_cast<std::uint16_t>(sign);  // Flush to zero (sufficient for these tests).
    } else if (exp >= 0x1F) {
        // Inf (mant==0) or NaN (mant!=0) -- preserve "is this a NaN" via the
        // mantissa's top bit instead of always flattening to Inf. Without
        // this, encoding std::nanf("") here would silently produce +Inf
        // half-bits (0x7C00), which decodeF16LE() in capture.cpp correctly
        // treats as +Infinity, not NaN -- masking the NaN-specific
        // sanitizeHdrSample() code path this test exists to exercise.
        const std::uint16_t nanBit = (mant != 0) ? 0x0200u : 0u;
        half = static_cast<std::uint16_t>(sign | 0x7C00u | nanBit);
    } else {
        half = static_cast<std::uint16_t>(sign | (static_cast<std::uint32_t>(exp) << 10) |
                                           (mant >> 13));
    }
    out->push_back(static_cast<std::uint8_t>(half & 0xFF));
    out->push_back(static_cast<std::uint8_t>((half >> 8) & 0xFF));
}

// Builds a 1x1 HDR pixel buffer (R16G16B16A16Float, 8 bytes) with the given
// RGBA f32 values.
std::vector<std::uint8_t> makeHdrPixel(float r, float g, float b, float a) {
    std::vector<std::uint8_t> px;
    appendF16LE(&px, r);
    appendF16LE(&px, g);
    appendF16LE(&px, b);
    appendF16LE(&px, a);
    return px;
}

// ======================== tone-mapping pixel-buffer contract tests ========================
//
// Pins the SDR/HDR pixel-buffer contracts encode_jxl.h documents: RGB8 is
// 3 bytes/px, RGBA f16 is 8 bytes/px, RGBA f32 is 16 bytes/px. capture.cpp
// consumes the f16 (8 bytes/px) contract; these tests exercise it via
// toneMapHdr() and confirm the four-branch classification from
// capture.rs's tone_map_hdr() doc comment.

bool test_tonemap_zero_luminance_is_opaque_black() {
    std::vector<std::uint8_t> px = makeHdrPixel(0.0f, 0.0f, 0.0f, 1.0f);
    std::vector<std::uint8_t> bgra = snip::toneMapHdr(px, 1, 1);
    CHECK(bgra.size() == 4);
    CHECK(bgra[0] == 0);    // B
    CHECK(bgra[1] == 0);    // G
    CHECK(bgra[2] == 0);    // R
    CHECK(bgra[3] == 255);  // A fully opaque
    return true;
}

bool test_tonemap_sdr_range_gamma_only() {
    // R=G=B=1.0 (white), luminance = 1.0 exactly -- SDR branch is lum<=1.0
    // AND all channels <=1.0, so this must NOT go through the WCG/HDR
    // path.
    std::vector<std::uint8_t> px = makeHdrPixel(1.0f, 1.0f, 1.0f, 1.0f);
    std::vector<std::uint8_t> bgra = snip::toneMapHdr(px, 1, 1);
    CHECK(bgra.size() == 4);
    // linear_to_srgb(1.0) ~= 1.0 -> float_to_byte ~= 255.
    CHECK(bgra[0] == 255);
    CHECK(bgra[1] == 255);
    CHECK(bgra[2] == 255);
    CHECK(bgra[3] == 255);
    return true;
}

bool test_tonemap_hdr_range_compresses_below_255() {
    // Luminance > 1.0 must take the Extended Reinhard HDR branch and
    // compress toward but never reach 255 for a very bright pixel.
    std::vector<std::uint8_t> px = makeHdrPixel(50.0f, 50.0f, 50.0f, 1.0f);
    std::vector<std::uint8_t> bgra = snip::toneMapHdr(px, 1, 1);
    CHECK(bgra.size() == 4);
    CHECK(bgra[2] < 255);  // R channel: compressed, not clipped-white.
    CHECK(bgra[2] > 200);  // Still near-white (high luminance).
    return true;
}

bool test_tonemap_wcg_range_compresses_but_preserves_hue() {
    // R=1.2 (>1.0, WCG), G=0.6, B=0.0 -- luminance = 0.2126*1.2 + 0.7152*0.6
    // = 0.255 + 0.429 = 0.684, which is <=1.0, so this must take the WCG
    // max-channel-Reinhard branch, not the HDR branch.
    std::vector<std::uint8_t> px = makeHdrPixel(1.2f, 0.6f, 0.0f, 1.0f);
    std::vector<std::uint8_t> bgra = snip::toneMapHdr(px, 1, 1);
    CHECK(bgra.size() == 4);
    CHECK(bgra[2] > 0);    // R channel present.
    CHECK(bgra[2] < 255);  // Compressed into gamut, not hard-clamped.
    CHECK(bgra[1] > 0);    // G channel present (hue preserved, not zeroed).
    CHECK(bgra[0] == 0);   // B channel was exactly 0 -> stays 0.
    return true;
}

bool test_tonemap_nan_sanitized_to_zero() {
    std::vector<std::uint8_t> px = makeHdrPixel(std::nanf(""), 0.0f, 0.0f, 1.0f);
    std::vector<std::uint8_t> bgra = snip::toneMapHdr(px, 1, 1);
    CHECK(bgra.size() == 4);
    // NaN -> sanitize -> 0.0, all channels 0 -> zero luminance -> opaque black.
    CHECK(bgra[0] == 0);
    CHECK(bgra[1] == 0);
    CHECK(bgra[2] == 0);
    CHECK(bgra[3] == 255);
    return true;
}

bool test_sanitize_helper_contract() {
    CHECK(snip::sanitizeHdrSample(std::nanf("")) == 0.0f);
    CHECK(snip::sanitizeHdrSample(std::numeric_limits<float>::infinity()) ==
          snip::kMaxDisplayLuminance);
    CHECK(snip::sanitizeHdrSample(-std::numeric_limits<float>::infinity()) == 0.0f);
    CHECK(snip::sanitizeHdrSample(0.5f) == 0.5f);
    CHECK(snip::sanitizeHdrSample(-0.5f) == -0.5f);  // Negatives pass through -- caller clamps.
    return true;
}

bool test_linear_to_srgb_endpoints() {
    CHECK(snip::linearToSrgb(0.0f) == 0.0f);
    const float white = snip::linearToSrgb(1.0f);
    CHECK(std::fabs(white - 1.0f) < 0.001f);
    return true;
}

bool test_float_to_byte_extremes() {
    CHECK(snip::floatToByte(0.0f) == 0);
    CHECK(snip::floatToByte(1.0f) == 255);
    return true;
}

bool test_bgra_to_rgb_channel_order() {
    std::vector<std::uint8_t> bgra = {10, 20, 30, 255};  // B=10 G=20 R=30 A=255
    std::vector<std::uint8_t> rgb = snip::bgraToRgb(bgra, 1, 1);
    CHECK(rgb.size() == 3);
    CHECK(rgb[0] == 30);  // R
    CHECK(rgb[1] == 20);  // G
    CHECK(rgb[2] == 10);  // B
    return true;
}

// SDR RGB8 contract: 3 bytes/px, as encode_jxl.h documents for the
// existing SDR encode path -- confirmed by round-tripping bgraToRgb() over
// a multi-pixel buffer and checking the resulting length.
bool test_sdr_rgb8_contract_3_bytes_per_pixel() {
    std::vector<std::uint8_t> bgra(4 * 4 * 4, 0);  // 4x4 image, BGRA8.
    std::vector<std::uint8_t> rgb = snip::bgraToRgb(bgra, 4, 4);
    CHECK(rgb.size() == static_cast<std::size_t>(4 * 4 * 3));
    return true;
}

// RGBA f16 contract: 8 bytes/px -- confirmed via makeHdrPixel()'s buffer
// size and toneMapHdr()'s expected src_stride computation (width * 8).
bool test_hdr_f16_contract_8_bytes_per_pixel() {
    std::vector<std::uint8_t> px = makeHdrPixel(0.1f, 0.2f, 0.3f, 1.0f);
    CHECK(px.size() == 8);
    return true;
}

// ======================== extractHdrRegion / extractHdrRegionRaw ========================

bool test_extract_hdr_region_sdr_path() {
    // 2x2 BGRA8 frame, not HDR -- extractHdrRegion should just crop +
    // bgra_to_rgb, no tone mapping.
    snip::HdrFrameView frame;
    frame.width = 2;
    frame.height = 2;
    frame.isHdr = false;
    frame.monitorRect = {0, 0};
    // Row-major BGRA8: (0,0)=B10G20R30A255, others zero.
    frame.pixels = {10, 20, 30, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0};

    snip::Region region{0, 0, 1, 1};
    std::vector<std::uint8_t> rgb = snip::extractHdrRegion(frame, region);
    CHECK(rgb.size() == 3);
    CHECK(rgb[0] == 30);  // R
    CHECK(rgb[1] == 20);  // G
    CHECK(rgb[2] == 10);  // B
    return true;
}

bool test_extract_hdr_region_out_of_bounds_returns_empty() {
    snip::HdrFrameView frame;
    frame.width = 2;
    frame.height = 2;
    frame.isHdr = false;
    frame.monitorRect = {0, 0};
    frame.pixels.assign(2 * 2 * 4, 0);

    // Region entirely outside the frame.
    snip::Region region{100, 100, 10, 10};
    std::vector<std::uint8_t> rgb = snip::extractHdrRegion(frame, region);
    CHECK(rgb.empty());
    return true;
}

bool test_extract_hdr_region_raw_crops_f16_data() {
    snip::HdrFrameView frame;
    frame.width = 2;
    frame.height = 1;
    frame.isHdr = true;
    frame.monitorRect = {5, 5};  // vscreen -> frame-relative translation.
    std::vector<std::uint8_t> px0 = makeHdrPixel(0.1f, 0.2f, 0.3f, 1.0f);
    std::vector<std::uint8_t> px1 = makeHdrPixel(0.4f, 0.5f, 0.6f, 1.0f);
    frame.pixels.insert(frame.pixels.end(), px0.begin(), px0.end());
    frame.pixels.insert(frame.pixels.end(), px1.begin(), px1.end());

    // vscreen region at (6,5), 1x1 -> frame-relative (1,0) -> should crop px1.
    snip::Region region{6, 5, 1, 1};
    snip::HdrPixelData result = snip::extractHdrRegionRaw(frame, region);
    CHECK(result.width == 1);
    CHECK(result.height == 1);
    CHECK(result.pixels.size() == 8);
    CHECK(result.pixels == px1);
    return true;
}

// ======================== resize: aspect ratio, _full sibling, disabled path ========================

bool test_resize_disabled_is_passthrough() {
    snip::ResizeOptions opts;
    opts.enabled = false;
    opts.maxWidth = 10;
    opts.maxHeight = 10;

    std::vector<std::uint8_t> px(100 * 100 * 3, 42);
    snip::ResizeResult result = snip::applyResize(px, 100, 100, opts);

    CHECK(!result.resized);
    CHECK(result.width == 100);
    CHECK(result.height == 100);
    CHECK(result.pixels.size() == px.size());
    CHECK(!result.original.has_value());
    return true;
}

bool test_resize_already_fits_is_noop() {
    snip::ResizeOptions opts;
    opts.enabled = true;
    opts.maxWidth = 2048;
    opts.maxHeight = 2048;

    std::vector<std::uint8_t> px(100 * 50 * 3, 7);
    snip::ResizeResult result = snip::applyResize(px, 100, 50, opts);

    CHECK(!result.resized);
    CHECK(result.width == 100);
    CHECK(result.height == 50);
    CHECK(!result.original.has_value());
    return true;
}

bool test_resize_preserves_aspect_ratio_width_limited() {
    // 4000x2000 (2:1) capped at 2048x2048 -> width-limited (scale=2048/4000
    // = 0.512), height would become 2000*0.512=1024, well within 2048.
    snip::ResizeOptions opts;
    opts.enabled = true;
    opts.maxWidth = 2048;
    opts.maxHeight = 2048;

    std::vector<std::uint8_t> px(4000 * 2000 * 3, 0);
    snip::ResizeResult result = snip::applyResize(px, 4000, 2000, opts);

    CHECK(result.resized);
    CHECK(result.width == 2048);
    // Aspect ratio 2:1 preserved -> height should be ~1024.
    CHECK(result.height >= 1020 && result.height <= 1028);
    return true;
}

bool test_resize_preserves_aspect_ratio_height_limited() {
    // 2000x4000 (1:2) capped at 2048x2048 -> height-limited.
    snip::ResizeOptions opts;
    opts.enabled = true;
    opts.maxWidth = 2048;
    opts.maxHeight = 2048;

    std::vector<std::uint8_t> px(2000 * 4000 * 3, 0);
    snip::ResizeResult result = snip::applyResize(px, 2000, 4000, opts);

    CHECK(result.resized);
    CHECK(result.height == 2048);
    CHECK(result.width >= 1020 && result.width <= 1028);
    return true;
}

bool test_resize_keep_original_produces_full_sibling_data() {
    snip::ResizeOptions opts;
    opts.enabled = true;
    opts.maxWidth = 100;
    opts.maxHeight = 100;
    opts.keepOriginal = true;

    std::vector<std::uint8_t> px(400 * 400 * 3, 99);
    snip::ResizeResult result = snip::applyResize(px, 400, 400, opts);

    CHECK(result.resized);
    CHECK(result.width == 100);
    CHECK(result.height == 100);
    CHECK(result.original.has_value());
    CHECK(result.original->width == 400);
    CHECK(result.original->height == 400);
    CHECK(result.original->pixels.size() == px.size());
    CHECK(result.original->pixels == px);  // Untouched original, byte-for-byte.
    return true;
}

bool test_resize_no_keep_original_omits_full_sibling() {
    snip::ResizeOptions opts;
    opts.enabled = true;
    opts.maxWidth = 100;
    opts.maxHeight = 100;
    opts.keepOriginal = false;

    std::vector<std::uint8_t> px(400 * 400 * 3, 1);
    snip::ResizeResult result = snip::applyResize(px, 400, 400, opts);

    CHECK(result.resized);
    CHECK(!result.original.has_value());
    return true;
}

// ======================== filename/dispatch: extension per OutputFormat ========================

bool test_extension_for_all_8_formats() {
    CHECK(std::string(snip::extension(snip::OutputFormat::Jxl)) == "jxl");
    CHECK(std::string(snip::extension(snip::OutputFormat::Jpeg)) == "jpg");
    CHECK(std::string(snip::extension(snip::OutputFormat::Png)) == "png");
    CHECK(std::string(snip::extension(snip::OutputFormat::WebP)) == "webp");
    CHECK(std::string(snip::extension(snip::OutputFormat::Tiff)) == "tiff");
    CHECK(std::string(snip::extension(snip::OutputFormat::Bmp)) == "bmp");
    CHECK(std::string(snip::extension(snip::OutputFormat::Qoi)) == "qoi");
    CHECK(std::string(snip::extension(snip::OutputFormat::OpenExr)) == "exr");
    return true;
}

// encodeImage() dispatch: JXL is real (Phase 2), the other 7 must return an
// explicit ok=false with a non-empty TODO(phase3b) error -- never a silent
// "success" with no file written.
bool test_dispatch_jxl_is_real_not_stub() {
    std::vector<std::uint8_t> rgb(4 * 4 * 3, 128);
    snip::FormatOptions opts;
    snip::CaptureResult result =
        snip::encodeImage(rgb, 4, 4, snip::OutputFormat::Jxl, opts, "/tmp/snip_test_dispatch.jxl",
                           nullptr);
    CHECK(result.ok);
    CHECK(result.error.empty());
    return true;
}

bool test_dispatch_other_7_formats_are_explicit_stubs() {
    std::vector<std::uint8_t> rgb(2 * 2 * 3, 0);
    snip::FormatOptions opts;
    const snip::OutputFormat stubFormats[] = {
        snip::OutputFormat::Jpeg, snip::OutputFormat::Png,  snip::OutputFormat::WebP,
        snip::OutputFormat::Tiff, snip::OutputFormat::Bmp,  snip::OutputFormat::Qoi,
        snip::OutputFormat::OpenExr,
    };
    for (snip::OutputFormat fmt : stubFormats) {
        snip::CaptureResult result =
            snip::encodeImage(rgb, 2, 2, fmt, opts, "/tmp/snip_test_stub_out", nullptr);
        CHECK(!result.ok);
        CHECK(!result.error.empty());
        CHECK(result.error.find("TODO(phase3b)") != std::string::npos);
    }
    return true;
}

bool test_dispatch_openexr_stub_flags_operator_decision() {
    std::vector<std::uint8_t> rgb(1 * 1 * 3, 0);
    snip::FormatOptions opts;
    snip::CaptureResult result =
        snip::encodeImage(rgb, 1, 1, snip::OutputFormat::OpenExr, opts, "/tmp/snip_test_exr",
                           nullptr);
    CHECK(!result.ok);
    // OpenEXR's stub message must additionally mention the operator
    // decision block (issue #7 section 1a), not just the generic
    // TODO(phase3b) text every other stub has.
    CHECK(result.error.find("operator decision") != std::string::npos);
    return true;
}

// ======================== platform seams: tray/hotkey/clipboard ========================

bool test_hotkey_registration_uses_configured_key_and_modifiers() {
    snip::HotkeyConfig config;
    config.key = "F8";
    config.modifiers = {"ctrl", "shift"};

    snip::RecordingHotkeyPlatform platform;
    snip::HotkeyResult result = snip::registerHotkey(config, platform);

    CHECK(result.ok);
    CHECK(platform.registeredHotkeys.size() == 1);
    CHECK(platform.registeredHotkeys[0].code == snip::KeyCode::F8);
    CHECK((platform.registeredHotkeys[0].modifiers & snip::kModifierControl) != 0);
    CHECK((platform.registeredHotkeys[0].modifiers & snip::kModifierShift) != 0);
    CHECK((platform.registeredHotkeys[0].modifiers & snip::kModifierAlt) == 0);
    return true;
}

bool test_hotkey_default_config_registers_printscreen_no_modifiers() {
    snip::HotkeyConfig config;  // defaults: key="PrintScreen", modifiers={}
    snip::RecordingHotkeyPlatform platform;
    snip::HotkeyResult result = snip::registerHotkey(config, platform);

    CHECK(result.ok);
    CHECK(platform.registeredHotkeys.size() == 1);
    CHECK(platform.registeredHotkeys[0].code == snip::KeyCode::PrintScreen);
    CHECK(platform.registeredHotkeys[0].modifiers == snip::kModifierNone);
    return true;
}

bool test_hotkey_unknown_key_name_fails_before_touching_platform() {
    snip::HotkeyConfig config;
    config.key = "NotARealKey";

    snip::RecordingHotkeyPlatform platform;
    snip::HotkeyResult result = snip::registerHotkey(config, platform);

    CHECK(!result.ok);
    CHECK(result.error.find("unknown key name") != std::string::npos);
    // Must fail during parsing, before ever calling the platform.
    CHECK(platform.registeredHotkeys.empty());
    return true;
}

bool test_hotkey_unknown_modifier_fails_before_touching_platform() {
    snip::HotkeyConfig config;
    config.key = "a";
    config.modifiers = {"hyper"};  // not a recognized modifier

    snip::RecordingHotkeyPlatform platform;
    snip::HotkeyResult result = snip::registerHotkey(config, platform);

    CHECK(!result.ok);
    CHECK(result.error.find("unknown modifier") != std::string::npos);
    CHECK(platform.registeredHotkeys.empty());
    return true;
}

bool test_hotkey_platform_failure_propagates() {
    snip::HotkeyConfig config;
    config.key = "a";

    snip::RecordingHotkeyPlatform platform;
    platform.simulateFailure = true;
    snip::HotkeyResult result = snip::registerHotkey(config, platform);

    CHECK(!result.ok);
    CHECK(!result.error.empty());
    return true;
}

bool test_clipboard_converts_and_invokes_platform() {
    std::vector<std::uint8_t> rgb = {10, 20, 30, 40, 50, 60};  // 2 pixels RGB8.
    snip::RecordingClipboardPlatform platform;
    snip::ClipboardResult result = snip::copyToClipboardPixels(rgb, 2, 1, platform);

    CHECK(result.ok);
    CHECK(platform.callCount == 1);
    CHECK(platform.lastWidth == 2);
    CHECK(platform.lastHeight == 1);
    CHECK(platform.lastRgbaPixels.size() == 8);
    CHECK(platform.lastRgbaPixels[0] == 10);
    CHECK(platform.lastRgbaPixels[1] == 20);
    CHECK(platform.lastRgbaPixels[2] == 30);
    CHECK(platform.lastRgbaPixels[3] == 255);  // Opaque alpha injected.
    CHECK(platform.lastRgbaPixels[4] == 40);
    CHECK(platform.lastRgbaPixels[7] == 255);
    return true;
}

bool test_clipboard_undersized_buffer_rejected_before_platform_call() {
    std::vector<std::uint8_t> rgb = {1, 2, 3};  // Only 1 pixel's worth.
    snip::RecordingClipboardPlatform platform;
    // Claim a 2x1 image (needs 6 bytes) with only 3 bytes supplied.
    snip::ClipboardResult result = snip::copyToClipboardPixels(rgb, 2, 1, platform);

    CHECK(!result.ok);
    CHECK(result.error.find("too small") != std::string::npos);
    CHECK(platform.callCount == 0);  // Must fail before ever calling the platform.
    return true;
}

bool test_clipboard_platform_failure_propagates() {
    std::vector<std::uint8_t> rgb = {1, 2, 3};
    snip::RecordingClipboardPlatform platform;
    platform.simulateFailure = true;
    snip::ClipboardResult result = snip::copyToClipboardPixels(rgb, 1, 1, platform);

    CHECK(!result.ok);
    CHECK(!result.error.empty());
    return true;
}

bool test_tray_creation_invokes_platform_with_generated_icon() {
    snip::RecordingTrayPlatform platform;
    snip::TrayResult result = snip::createTray(platform);

    CHECK(result.ok);
    CHECK(platform.callCount == 1);
    CHECK(platform.lastIconRgba.size() ==
          static_cast<std::size_t>(snip::kTrayIconSize) * snip::kTrayIconSize * 4);
    CHECK(!platform.lastTooltip.empty());
    CHECK(!result.ids.screenshot.empty());
    CHECK(!result.ids.openFolder.empty());
    CHECK(!result.ids.settings.empty());
    CHECK(!result.ids.quit.empty());
    return true;
}

bool test_tray_platform_failure_propagates() {
    snip::RecordingTrayPlatform platform;
    platform.simulateFailure = true;
    snip::TrayResult result = snip::createTray(platform);

    CHECK(!result.ok);
    CHECK(!result.error.empty());
    return true;
}

// Pins the icon-generation pixel contract: 32x32 RGBA, corner brackets are
// non-transparent (alpha=255) at the expected margin offset, center stays
// fully transparent.
bool test_generate_snip_icon_has_corner_brackets() {
    std::vector<std::uint8_t> rgba = snip::generateSnipIconRgba();
    CHECK(rgba.size() == static_cast<std::size_t>(snip::kTrayIconSize) * snip::kTrayIconSize * 4);

    // Top-left bracket pixel at (4,4) (the margin offset) should be opaque white.
    const std::size_t idx = (4 * snip::kTrayIconSize + 4) * 4;
    CHECK(rgba[idx] == 255);      // R
    CHECK(rgba[idx + 1] == 255);  // G
    CHECK(rgba[idx + 2] == 255);  // B
    CHECK(rgba[idx + 3] == 255);  // A

    // Center of the icon should remain transparent (no bracket there).
    const std::size_t centerIdx = (16 * snip::kTrayIconSize + 16) * 4;
    CHECK(rgba[centerIdx + 3] == 0);
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
        {"tonemap_zero_luminance_is_opaque_black", test_tonemap_zero_luminance_is_opaque_black},
        {"tonemap_sdr_range_gamma_only", test_tonemap_sdr_range_gamma_only},
        {"tonemap_hdr_range_compresses_below_255", test_tonemap_hdr_range_compresses_below_255},
        {"tonemap_wcg_range_compresses_but_preserves_hue",
         test_tonemap_wcg_range_compresses_but_preserves_hue},
        {"tonemap_nan_sanitized_to_zero", test_tonemap_nan_sanitized_to_zero},
        {"sanitize_helper_contract", test_sanitize_helper_contract},
        {"linear_to_srgb_endpoints", test_linear_to_srgb_endpoints},
        {"float_to_byte_extremes", test_float_to_byte_extremes},
        {"bgra_to_rgb_channel_order", test_bgra_to_rgb_channel_order},
        {"sdr_rgb8_contract_3_bytes_per_pixel", test_sdr_rgb8_contract_3_bytes_per_pixel},
        {"hdr_f16_contract_8_bytes_per_pixel", test_hdr_f16_contract_8_bytes_per_pixel},
        {"extract_hdr_region_sdr_path", test_extract_hdr_region_sdr_path},
        {"extract_hdr_region_out_of_bounds_returns_empty",
         test_extract_hdr_region_out_of_bounds_returns_empty},
        {"extract_hdr_region_raw_crops_f16_data", test_extract_hdr_region_raw_crops_f16_data},
        {"resize_disabled_is_passthrough", test_resize_disabled_is_passthrough},
        {"resize_already_fits_is_noop", test_resize_already_fits_is_noop},
        {"resize_preserves_aspect_ratio_width_limited",
         test_resize_preserves_aspect_ratio_width_limited},
        {"resize_preserves_aspect_ratio_height_limited",
         test_resize_preserves_aspect_ratio_height_limited},
        {"resize_keep_original_produces_full_sibling_data",
         test_resize_keep_original_produces_full_sibling_data},
        {"resize_no_keep_original_omits_full_sibling",
         test_resize_no_keep_original_omits_full_sibling},
        {"extension_for_all_8_formats", test_extension_for_all_8_formats},
        {"dispatch_jxl_is_real_not_stub", test_dispatch_jxl_is_real_not_stub},
        {"dispatch_other_7_formats_are_explicit_stubs",
         test_dispatch_other_7_formats_are_explicit_stubs},
        {"dispatch_openexr_stub_flags_operator_decision",
         test_dispatch_openexr_stub_flags_operator_decision},
        {"hotkey_registration_uses_configured_key_and_modifiers",
         test_hotkey_registration_uses_configured_key_and_modifiers},
        {"hotkey_default_config_registers_printscreen_no_modifiers",
         test_hotkey_default_config_registers_printscreen_no_modifiers},
        {"hotkey_unknown_key_name_fails_before_touching_platform",
         test_hotkey_unknown_key_name_fails_before_touching_platform},
        {"hotkey_unknown_modifier_fails_before_touching_platform",
         test_hotkey_unknown_modifier_fails_before_touching_platform},
        {"hotkey_platform_failure_propagates", test_hotkey_platform_failure_propagates},
        {"clipboard_converts_and_invokes_platform", test_clipboard_converts_and_invokes_platform},
        {"clipboard_undersized_buffer_rejected_before_platform_call",
         test_clipboard_undersized_buffer_rejected_before_platform_call},
        {"clipboard_platform_failure_propagates", test_clipboard_platform_failure_propagates},
        {"tray_creation_invokes_platform_with_generated_icon",
         test_tray_creation_invokes_platform_with_generated_icon},
        {"tray_platform_failure_propagates", test_tray_platform_failure_propagates},
        {"generate_snip_icon_has_corner_brackets", test_generate_snip_icon_has_corner_brackets},
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
