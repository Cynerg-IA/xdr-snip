// types.cpp -- implementation for types.h. See types.h for the full
// behavior-change rationale (Jxl addition, preservesHdr, defaultFormat).

#include "types.h"

#include <sstream>

namespace snip {

const char* extension(OutputFormat fmt) {
    switch (fmt) {
        case OutputFormat::Jxl:     return "jxl";
        case OutputFormat::Jpeg:    return "jpg";
        case OutputFormat::Png:     return "png";
        case OutputFormat::WebP:    return "webp";
        case OutputFormat::Tiff:    return "tiff";
        case OutputFormat::Bmp:     return "bmp";
        case OutputFormat::Qoi:     return "qoi";
        case OutputFormat::OpenExr: return "exr";
    }
    return "";
}

const char* displayName(OutputFormat fmt) {
    switch (fmt) {
        case OutputFormat::Jxl:     return "JPEG XL";
        case OutputFormat::Jpeg:    return "JPEG";
        case OutputFormat::Png:     return "PNG";
        case OutputFormat::WebP:    return "WebP";
        case OutputFormat::Tiff:    return "TIFF";
        case OutputFormat::Bmp:     return "BMP";
        case OutputFormat::Qoi:     return "QOI";
        case OutputFormat::OpenExr: return "OpenEXR (HDR)";
    }
    return "";
}

bool preservesHdr(OutputFormat fmt) {
    return fmt == OutputFormat::Jxl || fmt == OutputFormat::OpenExr;
}

OutputFormat defaultFormat() {
    return OutputFormat::Jxl;
}

const char* toWireString(OutputFormat fmt) {
    switch (fmt) {
        case OutputFormat::Jxl:     return "jxl";
        case OutputFormat::Jpeg:    return "jpeg";
        case OutputFormat::Png:     return "png";
        case OutputFormat::WebP:    return "webp";
        case OutputFormat::Tiff:    return "tiff";
        case OutputFormat::Bmp:     return "bmp";
        case OutputFormat::Qoi:     return "qoi";
        case OutputFormat::OpenExr: return "openexr";
    }
    return "";
}

bool fromWireString(const std::string& s, OutputFormat* out) {
    if (s == "jxl")     { *out = OutputFormat::Jxl;     return true; }
    if (s == "jpeg")    { *out = OutputFormat::Jpeg;    return true; }
    if (s == "png")     { *out = OutputFormat::Png;     return true; }
    if (s == "webp")    { *out = OutputFormat::WebP;    return true; }
    if (s == "tiff")    { *out = OutputFormat::Tiff;    return true; }
    if (s == "bmp")     { *out = OutputFormat::Bmp;     return true; }
    if (s == "qoi")     { *out = OutputFormat::Qoi;     return true; }
    if (s == "openexr") { *out = OutputFormat::OpenExr; return true; }
    return false;
}

const char* toWireString(ChromaSubsampling v) {
    switch (v) {
        case ChromaSubsampling::Full:    return "4:4:4";
        case ChromaSubsampling::Half:    return "4:2:2";
        case ChromaSubsampling::Quarter: return "4:2:0";
    }
    return "";
}

bool fromWireString(const std::string& s, ChromaSubsampling* out) {
    if (s == "4:4:4") { *out = ChromaSubsampling::Full;    return true; }
    if (s == "4:2:2") { *out = ChromaSubsampling::Half;    return true; }
    if (s == "4:2:0") { *out = ChromaSubsampling::Quarter; return true; }
    return false;
}

const char* toWireString(PngFilter v) {
    switch (v) {
        case PngFilter::Adaptive: return "adaptive";
        case PngFilter::None:     return "none";
        case PngFilter::Sub:      return "sub";
        case PngFilter::Up:       return "up";
        case PngFilter::Average:  return "average";
        case PngFilter::Paeth:    return "paeth";
    }
    return "";
}

bool fromWireString(const std::string& s, PngFilter* out) {
    if (s == "adaptive") { *out = PngFilter::Adaptive; return true; }
    if (s == "none")     { *out = PngFilter::None;     return true; }
    if (s == "sub")      { *out = PngFilter::Sub;      return true; }
    if (s == "up")       { *out = PngFilter::Up;       return true; }
    if (s == "average")  { *out = PngFilter::Average;  return true; }
    if (s == "paeth")    { *out = PngFilter::Paeth;    return true; }
    return false;
}

const char* toWireString(TiffCompression v) {
    switch (v) {
        case TiffCompression::None:     return "none";
        case TiffCompression::Lzw:      return "lzw";
        case TiffCompression::Deflate:  return "deflate";
        case TiffCompression::Packbits: return "packbits";
    }
    return "";
}

bool fromWireString(const std::string& s, TiffCompression* out) {
    if (s == "none")     { *out = TiffCompression::None;     return true; }
    if (s == "lzw")      { *out = TiffCompression::Lzw;      return true; }
    if (s == "deflate")  { *out = TiffCompression::Deflate;  return true; }
    if (s == "packbits") { *out = TiffCompression::Packbits; return true; }
    return false;
}

const char* toWireString(ExrCompression v) {
    switch (v) {
        case ExrCompression::Uncompressed: return "uncompressed";
        case ExrCompression::Rle:          return "rle";
        case ExrCompression::Zip1:         return "zip1";
        case ExrCompression::Zip16:        return "zip16";
        case ExrCompression::Piz:          return "piz";
        case ExrCompression::Pxr24:        return "pxr24";
        case ExrCompression::B44:          return "b44";
        case ExrCompression::B44A:         return "b44a";
    }
    return "";
}

bool fromWireString(const std::string& s, ExrCompression* out) {
    if (s == "uncompressed") { *out = ExrCompression::Uncompressed; return true; }
    if (s == "rle")          { *out = ExrCompression::Rle;          return true; }
    if (s == "zip1")         { *out = ExrCompression::Zip1;         return true; }
    if (s == "zip16")        { *out = ExrCompression::Zip16;        return true; }
    if (s == "piz")          { *out = ExrCompression::Piz;          return true; }
    if (s == "pxr24")        { *out = ExrCompression::Pxr24;        return true; }
    if (s == "b44")          { *out = ExrCompression::B44;          return true; }
    if (s == "b44a")         { *out = ExrCompression::B44A;         return true; }
    return false;
}

std::string Region::toString() const {
    std::ostringstream oss;
    oss << w << "x" << h << "+" << x << "+" << y;
    return oss.str();
}

std::ostream& operator<<(std::ostream& os, const Region& r) {
    return os << r.toString();
}

}  // namespace snip
