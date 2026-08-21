// encode_jxl.cpp -- implementation of the JPEG XL Phase 2 encoder.
//
// Ported logic vs the Rust source: v0.5.0's capture.rs has NO JXL path (JXL
// did not exist as a format), so there is nothing to "port" call-for-call
// here the way encode_png/encode_bmp/etc mirror an `image` crate call. What
// IS ported faithfully is the *pixel data contract*: the RGB8 buffer this
// function accepts for SDR is exactly the `rgb_pixels: &[u8]` (3 bytes/px,
// tone-mapped) buffer capture.rs's encode_image() passes to every other
// encoder, and the RGBA-float buffer this function accepts for HDR mirrors
// the same R16G16B16A16Float layout capture.rs's HdrPixelData carries for
// encode_exr_hdr (see capture.rs's f16-per-channel decode there). No
// invented pixel layout -- callers hand this function the same buffers the
// existing SDR/HDR pipelines already produce.
//
// libjxl API surface used (verified against headers at
// /work/libjxl/lib/include/jxl/{encode,types,codestream_header,
// color_encoding}.h in the xdrsnip-jxl-size:v2 toolchain image -- NOT
// guessed):
//   JxlEncoderCreate, JxlEncoderDestroy, JxlEncoderFrameSettingsCreate,
//   JxlEncoderUseContainer, JxlEncoderInitBasicInfo, JxlEncoderSetBasicInfo,
//   JxlEncoderSetColorEncoding, JxlColorEncodingSetToSRGB,
//   JxlEncoderSetFrameLossless, JxlEncoderSetFrameDistance,
//   JxlEncoderFrameSettingsSetOption(JXL_ENC_FRAME_SETTING_EFFORT, n),
//   JxlEncoderAddImageFrame, JxlEncoderCloseInput, JxlEncoderProcessOutput,
//   JxlEncoderGetError.

#include "encode_jxl.h"

#include <cstring>

#include <jxl/encode.h>
#include <jxl/types.h>

namespace snip {

namespace {

// Builds a human-readable message for a JXL_ENC_ERROR return, including the
// JxlEncoderError code when the encoder object is still valid.
std::string describeError(JxlEncoder* enc, const char* whichCall) {
    std::string msg = "libjxl call failed: ";
    msg += whichCall;
    if (enc != nullptr) {
        JxlEncoderError code = JxlEncoderGetError(enc);
        msg += " (JxlEncoderError=";
        msg += std::to_string(static_cast<int>(code));
        msg += ")";
    }
    return msg;
}

// RAII wrapper so every early-return error path still destroys the encoder.
// (JxlEncoderFrameSettings objects are owned by the encoder and destroyed
// with it -- no separate cleanup needed for those, per encode.h's doc
// comment on JxlEncoderFrameSettings.)
struct EncoderGuard {
    JxlEncoder* enc;
    explicit EncoderGuard(JxlEncoder* e) : enc(e) {}
    ~EncoderGuard() {
        if (enc != nullptr) {
            JxlEncoderDestroy(enc);
        }
    }
    EncoderGuard(const EncoderGuard&) = delete;
    EncoderGuard& operator=(const EncoderGuard&) = delete;
};

JxlEncodeResult fail(JxlEncoder* enc, const char* whichCall) {
    JxlEncodeResult r;
    r.ok = false;
    r.bytes.clear();
    r.error = describeError(enc, whichCall);
    return r;
}

JxlEncodeResult failMsg(std::string msg) {
    JxlEncodeResult r;
    r.ok = false;
    r.bytes.clear();
    r.error = std::move(msg);
    return r;
}

}  // namespace

JxlEncodeResult encodeJxl(const JxlImageInput& input, const JxlOptions& options) {
    // ---- Validate input shape before touching libjxl at all. ----
    if (input.pixels == nullptr || input.width == 0 || input.height == 0) {
        return failMsg("encodeJxl: empty or zero-sized input image");
    }
    const bool isHdr = (input.pixelType == JxlPixelType::Float16 ||
                         input.pixelType == JxlPixelType::Float32);
    if (isHdr && input.numChannels != 4) {
        return failMsg("encodeJxl: HDR (float) input must be 4-channel RGBA");
    }
    if (!isHdr && input.numChannels != 3) {
        return failMsg("encodeJxl: SDR (uint8) input must be 3-channel RGB");
    }

    // ---- JxlPixelFormat: describes the *input buffer* layout. ----
    JxlPixelFormat pixelFormat;
    std::memset(&pixelFormat, 0, sizeof(pixelFormat));
    pixelFormat.num_channels = input.numChannels;
    pixelFormat.endianness = JXL_NATIVE_ENDIAN;
    pixelFormat.align = 0;
    switch (input.pixelType) {
        case JxlPixelType::Uint8:
            pixelFormat.data_type = JXL_TYPE_UINT8;
            break;
        case JxlPixelType::Float16:
            pixelFormat.data_type = JXL_TYPE_FLOAT16;
            break;
        case JxlPixelType::Float32:
            pixelFormat.data_type = JXL_TYPE_FLOAT;
            break;
    }

    // bytes-per-pixel used to compute the input buffer size passed to
    // JxlEncoderAddImageFrame.
    std::size_t bytesPerSample;
    switch (input.pixelType) {
        case JxlPixelType::Uint8:
            bytesPerSample = 1;
            break;
        case JxlPixelType::Float16:
            bytesPerSample = 2;
            break;
        case JxlPixelType::Float32:
            bytesPerSample = 4;
            break;
    }
    const std::size_t bufferSize = static_cast<std::size_t>(input.width) *
                                    static_cast<std::size_t>(input.height) *
                                    static_cast<std::size_t>(input.numChannels) *
                                    bytesPerSample;

    // ---- Create encoder. ----
    JxlEncoder* enc = JxlEncoderCreate(nullptr /* default memory manager */);
    if (enc == nullptr) {
        return failMsg("encodeJxl: JxlEncoderCreate returned null");
    }
    EncoderGuard guard(enc);

    // No JxlEncoderSetParallelRunner call: deliberately single-threaded, see
    // encode_jxl.h doc comment (avoids linking jxl_threads, which fails to
    // compile on GCC/mingw per Phase 0 KNOWN TRAP in issue #7).

    // Emit the standard ISOBMFF container. This is required for the JXL
    // codestream to carry an explicit color profile box reliably across
    // decoders/viewers; without it libjxl still produces a valid naked
    // codestream (starts with the 2-byte marker 0xFF 0x0A) but downstream
    // tools that expect the "JXL " container signature (0x00 0x00 0x00 0x0C
    // 'J' 'X' 'L' ' ' 0x0D 0x0A 0x87 0x0A) would reject it. We standardize
    // on the container form for both SDR and HDR outputs.
    if (JxlEncoderUseContainer(enc, JXL_TRUE) != JXL_ENC_SUCCESS) {
        return fail(enc, "JxlEncoderUseContainer");
    }

    // ---- JxlBasicInfo. ----
    JxlBasicInfo basicInfo;
    JxlEncoderInitBasicInfo(&basicInfo);
    basicInfo.xsize = input.width;
    basicInfo.ysize = input.height;
    basicInfo.num_color_channels = 3;
    basicInfo.num_extra_channels = (input.numChannels == 4) ? 1 : 0;

    if (isHdr) {
        // HDR: float samples. exponent_bits_per_sample is the field that
        // silently corrupts HDR if left at 0 (== "unsigned integer" per the
        // header's own comment on JxlBasicInfo.exponent_bits_per_sample) --
        // this is the highest-risk part of the port per the task spec.
        if (input.pixelType == JxlPixelType::Float16) {
            basicInfo.bits_per_sample = 16;
            basicInfo.exponent_bits_per_sample = 5;  // IEEE-754 binary16.
        } else {
            basicInfo.bits_per_sample = 32;
            basicInfo.exponent_bits_per_sample = 8;  // IEEE-754 binary32.
        }
        basicInfo.alpha_bits = basicInfo.bits_per_sample;
        basicInfo.alpha_exponent_bits = basicInfo.exponent_bits_per_sample;
        basicInfo.alpha_premultiplied = JXL_FALSE;
        // HDR content is carried through untouched (linear scRGB-ish
        // float), matching OpenEXR's preservesHdr() contract in types.h:
        // do not let libjxl reinterpret/convert it against a target
        // display profile.
        basicInfo.uses_original_profile = JXL_TRUE;
    } else {
        basicInfo.bits_per_sample = 8;
        basicInfo.exponent_bits_per_sample = 0;  // unsigned integer samples.
        basicInfo.alpha_bits = (input.numChannels == 4) ? 8 : 0;
        basicInfo.alpha_exponent_bits = 0;
        basicInfo.alpha_premultiplied = JXL_FALSE;
        basicInfo.uses_original_profile = options.lossless ? JXL_TRUE : JXL_FALSE;
    }

    if (JxlEncoderSetBasicInfo(enc, &basicInfo) != JXL_ENC_SUCCESS) {
        return fail(enc, "JxlEncoderSetBasicInfo");
    }

    // ---- Color encoding. ----
    // sRGB primaries/white point apply to both SDR and HDR here: HDR input
    // in this codebase is scRGB-derived (see hdr_capture.rs / capture.rs
    // tone-map path), which uses sRGB/Rec.709 primaries with an extended
    // (linear, unclamped) range -- not PQ/HLG. So: sRGB primaries always,
    // transfer function is nonlinear sRGB for 8-bit and LINEAR for float,
    // matching encode.h's own documented default ("pixels are assumed to be
    // nonlinear sRGB for integer data types ... and linear sRGB for
    // floating point data types" when no color encoding is set at all --
    // we set it explicitly here rather than relying on that default so the
    // intent is not implicit).
    JxlColorEncoding colorEncoding;
    JxlColorEncodingSetToSRGB(&colorEncoding, /*is_gray=*/JXL_FALSE);
    if (isHdr) {
        colorEncoding.transfer_function = JXL_TRANSFER_FUNCTION_LINEAR;
    }
    if (JxlEncoderSetColorEncoding(enc, &colorEncoding) != JXL_ENC_SUCCESS) {
        return fail(enc, "JxlEncoderSetColorEncoding");
    }

    // ---- Frame settings: effort, distance/lossless. ----
    JxlEncoderFrameSettings* frameSettings = JxlEncoderFrameSettingsCreate(enc, nullptr);
    if (frameSettings == nullptr) {
        return fail(enc, "JxlEncoderFrameSettingsCreate");
    }

    if (JxlEncoderFrameSettingsSetOption(frameSettings, JXL_ENC_FRAME_SETTING_EFFORT,
                                          static_cast<int64_t>(options.effort)) != JXL_ENC_SUCCESS) {
        return fail(enc, "JxlEncoderFrameSettingsSetOption(EFFORT)");
    }

    if (options.lossless) {
        // lossless=true must call JxlEncoderSetFrameLossless AND set
        // distance 0, per the task spec -- libjxl's lossless path is driven
        // by the lossless flag, but distance is left in an unspecified
        // state otherwise, so pin it to 0.0 (mathematically lossless)
        // explicitly rather than relying on flag-implies-distance.
        if (JxlEncoderSetFrameLossless(frameSettings, JXL_TRUE) != JXL_ENC_SUCCESS) {
            return fail(enc, "JxlEncoderSetFrameLossless");
        }
        if (JxlEncoderSetFrameDistance(frameSettings, 0.0f) != JXL_ENC_SUCCESS) {
            return fail(enc, "JxlEncoderSetFrameDistance(lossless=0.0)");
        }
    } else {
        if (JxlEncoderSetFrameLossless(frameSettings, JXL_FALSE) != JXL_ENC_SUCCESS) {
            return fail(enc, "JxlEncoderSetFrameLossless");
        }
        if (JxlEncoderSetFrameDistance(frameSettings, options.quality) != JXL_ENC_SUCCESS) {
            return fail(enc, "JxlEncoderSetFrameDistance");
        }
    }

    // ---- Add the pixel data and close input (single frame, no animation). ----
    if (JxlEncoderAddImageFrame(frameSettings, &pixelFormat, input.pixels, bufferSize) !=
        JXL_ENC_SUCCESS) {
        return fail(enc, "JxlEncoderAddImageFrame");
    }
    JxlEncoderCloseInput(enc);

    // ---- Drain JxlEncoderProcessOutput into a growable buffer. ----
    std::vector<std::uint8_t> output;
    output.resize(64 * 1024);
    std::size_t totalWritten = 0;

    JxlEncoderStatus status;
    do {
        // JxlEncoderProcessOutput consumes (next_out, avail_out) from the
        // *current write position* -- give it the remainder of `output`
        // past what's already been written, growing the buffer whenever it
        // asks for more room (JXL_ENC_NEED_MORE_OUTPUT), per encode.h's
        // documented "*avail_out >= 32" contract.
        if (output.size() - totalWritten < 32) {
            output.resize(output.size() * 2);
        }
        std::uint8_t* nextOut = output.data() + totalWritten;
        std::size_t availOut = output.size() - totalWritten;
        std::uint8_t* nextOutCursor = nextOut;
        std::size_t availOutCursor = availOut;

        status = JxlEncoderProcessOutput(enc, &nextOutCursor, &availOutCursor);

        // The encoder advances nextOutCursor and decrements availOutCursor
        // as it writes; the delta is exactly how many bytes landed in this
        // call.
        std::size_t written = availOut - availOutCursor;
        totalWritten += written;

        if (status == JXL_ENC_ERROR) {
            return fail(enc, "JxlEncoderProcessOutput");
        }
        if (status == JXL_ENC_NEED_MORE_OUTPUT) {
            output.resize(output.size() * 2);
        }
    } while (status != JXL_ENC_SUCCESS);

    output.resize(totalWritten);

    if (output.empty()) {
        // Never emit a truncated/empty "success" -- treat this as a hard
        // failure per the task's error-handling requirement.
        return failMsg("encodeJxl: encoder reported success but produced 0 bytes");
    }

    JxlEncodeResult result;
    result.ok = true;
    result.bytes = std::move(output);
    return result;
}

}  // namespace snip
