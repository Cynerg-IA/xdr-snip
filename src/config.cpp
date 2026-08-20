// config.cpp -- implementation for config.h. Port of
// crates/snip-app/src/config.rs.

#include "config.h"

#include <toml++/toml.h>

#include <cstdlib>
#include <ctime>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

namespace snip {

namespace {

constexpr const char* kAppDirName = "xdr-snip";
constexpr const char* kConfigFileName = "config.toml";

// toml++ rejects duplicate keys (TOML-spec correct), but Rust's toml crate
// tolerates them with last-wins. v0.5.x shipped a config.toml with a
// duplicated `keep_original`, so every existing user install would abort on
// upgrade. Pre-pass the text to keep only the LAST definition of each
// (section, key) so those configs still load.
std::string dedupeLastWins(const std::string& in) {
    std::istringstream is(in);
    std::string line;
    std::vector<std::string> out;
    std::string section;
    std::vector<std::pair<std::string, size_t>> seen;
    while (std::getline(is, line)) {
        std::string trimmed = line;
        size_t b = trimmed.find_first_not_of(" \t");
        if (b == std::string::npos || trimmed[b] == '#') { out.push_back(line); continue; }
        if (trimmed[b] == '[') { section = trimmed.substr(b); out.push_back(line); continue; }
        size_t eq = std::string::npos;
        bool inS = false, inD = false;
        for (size_t i = b; i < trimmed.size(); ++i) {
            char c = trimmed[i];
            if (c == '\'' && !inD) inS = !inS;
            else if (c == '"' && !inS) inD = !inD;
            else if (c == '=' && !inS && !inD) { eq = i; break; }
        }
        if (eq == std::string::npos) { out.push_back(line); continue; }
        std::string key = trimmed.substr(b, eq - b);
        size_t ke = key.find_last_not_of(" \t");
        if (ke != std::string::npos) key = key.substr(0, ke + 1);
        std::string full = section + "|" + key;
        bool dup = false;
        for (auto& p : seen) {
            if (p.first == full) { out[p.second] = line; dup = true; break; }
        }
        if (dup) continue;
        seen.push_back({full, out.size()});
        out.push_back(line);
    }
    std::string r;
    for (auto& l : out) { r += l; r += '\n'; }
    return r;
}

// [xdr-snip] warning prefix -- documented substitute for Rust's
// tracing::warn!() macro. No logging framework is pulled in for this phase;
// a prefixed stderr line is adequate per issue #8.
void logWarning(const std::string& msg) {
    std::cerr << "[xdr-snip] warning: " << msg << "\n";
}

// ---- TOML <-> OutputFormat, with the "unknown format -> default + warn,
// never throw" backward-compat contract lives here (used both by the raw
// top-level parse path and by nested table parsing). ----
OutputFormat parseFormatValue(const toml::node* node) {
    if (node == nullptr) {
        return defaultFormat();
    }
    const auto* strNode = node->as_string();
    if (strNode == nullptr) {
        logWarning("capture.format is not a string, falling back to default format");
        return defaultFormat();
    }
    OutputFormat fmt;
    if (!fromWireString(strNode->get(), &fmt)) {
        logWarning("unknown output format '" + strNode->get() +
                   "', falling back to default format");
        return defaultFormat();
    }
    return fmt;
}

// ---- generic per-field helpers mirroring #[serde(default = "...")] ----

template <typename T>
T getOr(const toml::table& tbl, std::string_view key, T fallback) {
    if (auto v = tbl[key].value<T>()) {
        return *v;
    }
    return fallback;
}

std::string getOr(const toml::table& tbl, std::string_view key, const char* fallback) {
    if (auto v = tbl[key].value<std::string>()) {
        return *v;
    }
    return fallback;
}

template <typename EnumT>
EnumT getEnumOr(const toml::table& tbl, std::string_view key, EnumT fallback) {
    if (auto v = tbl[key].value<std::string>()) {
        EnumT parsed;
        if (fromWireString(*v, &parsed)) {
            return parsed;
        }
        logWarning(std::string("unknown value for '") + std::string(key) +
                   "', using default");
    }
    return fallback;
}

const toml::table* getSubtable(const toml::table& tbl, std::string_view key) {
    if (auto* node = tbl.get(key)) {
        return node->as_table();
    }
    return nullptr;
}

JpegOptions parseJpegOptions(const toml::table* tbl) {
    JpegOptions opts;
    if (tbl == nullptr) return opts;
    opts.quality = getOr<int64_t>(*tbl, "quality", opts.quality);
    opts.chromaSubsampling =
        getEnumOr(*tbl, "chroma_subsampling", opts.chromaSubsampling);
    return opts;
}

PngOptions parsePngOptions(const toml::table* tbl) {
    PngOptions opts;
    if (tbl == nullptr) return opts;
    opts.compression = static_cast<std::uint8_t>(
        getOr<int64_t>(*tbl, "compression", opts.compression));
    opts.filter = getEnumOr(*tbl, "filter", opts.filter);
    return opts;
}

WebPOptions parseWebPOptions(const toml::table* tbl) {
    WebPOptions opts;
    if (tbl == nullptr) return opts;
    opts.lossless = getOr<bool>(*tbl, "lossless", opts.lossless);
    opts.quality = getOr<double>(*tbl, "quality", opts.quality);
    return opts;
}

TiffOptions parseTiffOptions(const toml::table* tbl) {
    TiffOptions opts;
    if (tbl == nullptr) return opts;
    opts.compression = getEnumOr(*tbl, "compression", opts.compression);
    return opts;
}

ExrOptions parseExrOptions(const toml::table* tbl) {
    ExrOptions opts;
    if (tbl == nullptr) return opts;
    opts.compression = getEnumOr(*tbl, "compression", opts.compression);
    return opts;
}

JxlOptions parseJxlOptions(const toml::table* tbl) {
    JxlOptions opts;
    if (tbl == nullptr) return opts;
    opts.quality = getOr<double>(*tbl, "quality", opts.quality);
    opts.effort = static_cast<int>(getOr<int64_t>(*tbl, "effort", opts.effort));
    opts.lossless = getOr<bool>(*tbl, "lossless", opts.lossless);
    return opts;
}

FormatOptions parseFormatOptions(const toml::table* tbl) {
    FormatOptions opts;
    if (tbl == nullptr) return opts;
    opts.jpeg = parseJpegOptions(getSubtable(*tbl, "jpeg"));
    opts.png = parsePngOptions(getSubtable(*tbl, "png"));
    opts.webp = parseWebPOptions(getSubtable(*tbl, "webp"));
    opts.tiff = parseTiffOptions(getSubtable(*tbl, "tiff"));
    opts.exr = parseExrOptions(getSubtable(*tbl, "exr"));
    opts.jxl = parseJxlOptions(getSubtable(*tbl, "jxl"));
    return opts;
}

ResizeOptions parseResizeOptions(const toml::table* tbl) {
    ResizeOptions opts;
    if (tbl == nullptr) return opts;
    opts.enabled = getOr<bool>(*tbl, "enabled", opts.enabled);
    opts.maxWidth = static_cast<std::uint32_t>(
        getOr<int64_t>(*tbl, "max_width", opts.maxWidth));
    opts.maxHeight = static_cast<std::uint32_t>(
        getOr<int64_t>(*tbl, "max_height", opts.maxHeight));
    opts.keepOriginal = getOr<bool>(*tbl, "keep_original", opts.keepOriginal);
    return opts;
}

CaptureConfig parseCaptureConfig(const toml::table* tbl) {
    CaptureConfig cfg;
    if (tbl == nullptr) return cfg;
    cfg.format = parseFormatValue(tbl->get("format"));
    cfg.formatOptions = parseFormatOptions(getSubtable(*tbl, "format_options"));
    cfg.saveDir = getOr(*tbl, "save_dir", cfg.saveDir.c_str());
    cfg.filenamePattern =
        getOr(*tbl, "filename_pattern", cfg.filenamePattern.c_str());
    cfg.resize = parseResizeOptions(getSubtable(*tbl, "resize"));
    return cfg;
}

HotkeyConfig parseHotkeyConfig(const toml::table* tbl) {
    HotkeyConfig cfg;
    if (tbl == nullptr) return cfg;
    cfg.key = getOr(*tbl, "key", cfg.key.c_str());
    if (auto* arr = tbl->get("modifiers"); arr != nullptr && arr->is_array()) {
        cfg.modifiers.clear();
        for (auto&& elem : *arr->as_array()) {
            if (auto s = elem.value<std::string>()) {
                cfg.modifiers.push_back(*s);
            }
        }
    }
    return cfg;
}

BehaviorConfig parseBehaviorConfig(const toml::table* tbl) {
    BehaviorConfig cfg;
    if (tbl == nullptr) return cfg;
    cfg.copyToClipboard = getOr<bool>(*tbl, "copy_to_clipboard", cfg.copyToClipboard);
    cfg.saveToFile = getOr<bool>(*tbl, "save_to_file", cfg.saveToFile);
    cfg.showNotification =
        getOr<bool>(*tbl, "show_notification", cfg.showNotification);
    return cfg;
}

// ---- serialization (Config -> toml::table) ----

toml::table serializeJpegOptions(const JpegOptions& o) {
    return toml::table{
        {"quality", static_cast<int64_t>(o.quality)},
        {"chroma_subsampling", toWireString(o.chromaSubsampling)},
    };
}

toml::table serializePngOptions(const PngOptions& o) {
    return toml::table{
        {"compression", static_cast<int64_t>(o.compression)},
        {"filter", toWireString(o.filter)},
    };
}

toml::table serializeWebPOptions(const WebPOptions& o) {
    return toml::table{
        {"lossless", o.lossless},
        {"quality", static_cast<double>(o.quality)},
    };
}

toml::table serializeTiffOptions(const TiffOptions& o) {
    return toml::table{
        {"compression", toWireString(o.compression)},
    };
}

toml::table serializeExrOptions(const ExrOptions& o) {
    return toml::table{
        {"compression", toWireString(o.compression)},
    };
}

toml::table serializeJxlOptions(const JxlOptions& o) {
    return toml::table{
        {"quality", static_cast<double>(o.quality)},
        {"effort", static_cast<int64_t>(o.effort)},
        {"lossless", o.lossless},
    };
}

toml::table serializeFormatOptions(const FormatOptions& o) {
    return toml::table{
        {"jpeg", serializeJpegOptions(o.jpeg)},
        {"png", serializePngOptions(o.png)},
        {"webp", serializeWebPOptions(o.webp)},
        {"tiff", serializeTiffOptions(o.tiff)},
        {"exr", serializeExrOptions(o.exr)},
        {"jxl", serializeJxlOptions(o.jxl)},
    };
}

toml::table serializeResizeOptions(const ResizeOptions& o) {
    return toml::table{
        {"enabled", o.enabled},
        {"max_width", static_cast<int64_t>(o.maxWidth)},
        {"max_height", static_cast<int64_t>(o.maxHeight)},
        {"keep_original", o.keepOriginal},
    };
}

toml::table serializeCaptureConfig(const CaptureConfig& c) {
    return toml::table{
        {"format", toWireString(c.format)},
        {"format_options", serializeFormatOptions(c.formatOptions)},
        {"save_dir", c.saveDir},
        {"filename_pattern", c.filenamePattern},
        {"resize", serializeResizeOptions(c.resize)},
    };
}

toml::table serializeHotkeyConfig(const HotkeyConfig& h) {
    toml::array mods;
    for (const auto& m : h.modifiers) {
        mods.push_back(m);
    }
    return toml::table{
        {"key", h.key},
        {"modifiers", mods},
    };
}

toml::table serializeBehaviorConfig(const BehaviorConfig& b) {
    return toml::table{
        {"copy_to_clipboard", b.copyToClipboard},
        {"save_to_file", b.saveToFile},
        {"show_notification", b.showNotification},
    };
}

// Formats a toml::table to a string using double-quoted (non-literal)
// strings and no sub-table indentation, matching the textual style of
// Rust's `toml` crate output (toml++'s default formatter prefers
// single-quoted literal strings and indents nested tables, which would
// still round-trip correctly but diverges cosmetically from what a v0.5.x
// user's config.toml previously looked like -- keeping the visual format
// stable minimizes diff noise in migrated/re-saved files).
std::string formatTomlTable(const toml::table& tbl) {
    constexpr toml::format_flags flags = toml::toml_formatter::default_flags &
                                          ~toml::format_flags::allow_literal_strings;
    std::ostringstream oss;
    oss << toml::toml_formatter{tbl, flags};
    return oss.str();
}

}  // namespace

// ======================== PLATFORM SEAM ========================

#ifdef _WIN32

// NOTE: this branch is not compiled/tested in this (Linux) phase, but is
// written and correctly guarded so Phase 4/5 Windows work is a clean
// swap-in of the actual SHGetKnownFolderPath call.
#include <shlobj.h>
#include <windows.h>

std::optional<std::filesystem::path> platformConfigDir() {
    PWSTR path = nullptr;
    if (SUCCEEDED(SHGetKnownFolderPath(FOLDERID_RoamingAppData, 0, nullptr, &path))) {
        std::filesystem::path result(path);
        CoTaskMemFree(path);
        return result;
    }
    if (path != nullptr) {
        CoTaskMemFree(path);
    }
    return std::nullopt;
}

#else  // Linux/CI fallback -- mirrors the `dirs` crate's Linux behavior
       // closely enough for dev/CI purposes ($XDG_CONFIG_HOME or
       // $HOME/.config). The shipped product only targets Windows, so this
       // branch exists purely to make the code buildable/testable outside
       // Windows.

std::optional<std::filesystem::path> platformConfigDir() {
    if (const char* xdg = std::getenv("XDG_CONFIG_HOME"); xdg != nullptr && xdg[0] != '\0') {
        return std::filesystem::path(xdg);
    }
    if (const char* home = std::getenv("HOME"); home != nullptr && home[0] != '\0') {
        return std::filesystem::path(home) / ".config";
    }
    return std::nullopt;
}

#endif

// ======================== CORE CONFIG API ========================

std::filesystem::path resolveConfigDir() {
    auto base = platformConfigDir();
    if (!base.has_value()) {
        throw ConfigError("cannot determine platform config directory");
    }
    std::filesystem::path dir = *base / kAppDirName;

    std::error_code ec;
    if (!std::filesystem::exists(dir, ec)) {
        std::filesystem::create_directories(dir, ec);
        if (ec) {
            throw ConfigError("failed to create config dir " + dir.string() + ": " +
                               ec.message());
        }
    }
    return dir;
}

std::filesystem::path configFilePath() {
    return resolveConfigDir() / kConfigFileName;
}

std::string expandTilde(const std::string& path) {
    constexpr const char* kPrefix = "~/";
    if (path.rfind(kPrefix, 0) == 0) {  // starts_with, C++17-friendly
        std::string rest = path.substr(2);
        const char* home = std::getenv("HOME");
#ifdef _WIN32
        // On Windows HOME is not reliably set; USERPROFILE is the
        // equivalent. This mirrors dirs::home_dir()'s intent without
        // pulling in a platform path-resolution library here -- Phase 4/5
        // Windows work may want to route this through the same
        // SHGetKnownFolderPath-based helper as platformConfigDir.
        if (home == nullptr || home[0] == '\0') {
            home = std::getenv("USERPROFILE");
        }
#endif
        if (home != nullptr && home[0] != '\0') {
            std::filesystem::path result = std::filesystem::path(home) / rest;
            return result.generic_string();
        }
        logWarning("expandTilde: could not resolve home directory, returning path as-is");
        return path;
    }
    return path;
}

std::string generateFilename(const std::string& pattern) {
    std::time_t t = std::time(nullptr);
    std::tm localTm{};
#ifdef _WIN32
    localtime_s(&localTm, &t);
#else
    localtime_r(&t, &localTm);
#endif
    char buf[32];
    std::strftime(buf, sizeof(buf), "%Y%m%d_%H%M%S", &localTm);
    std::string timestamp(buf);

    std::string result = pattern;
    const std::string placeholder = "{timestamp}";
    auto pos = result.find(placeholder);
    if (pos != std::string::npos) {
        result.replace(pos, placeholder.size(), timestamp);
    }
    return result;
}

std::string serializeConfig(const Config& config) {
    toml::table root{
        {"capture", serializeCaptureConfig(config.capture)},
        {"hotkey", serializeHotkeyConfig(config.hotkey)},
        {"behavior", serializeBehaviorConfig(config.behavior)},
    };
    return formatTomlTable(root);
}

Config parseConfig(const std::string& tomlText) {
    toml::table root;
    try {
        root = toml::parse(dedupeLastWins(tomlText));
    } catch (const toml::parse_error& e) {
        throw ConfigError(std::string("failed to parse config TOML: ") +
                           e.description().data());
    }

    Config cfg;
    cfg.capture = parseCaptureConfig(getSubtable(root, "capture"));
    cfg.hotkey = parseHotkeyConfig(getSubtable(root, "hotkey"));
    cfg.behavior = parseBehaviorConfig(getSubtable(root, "behavior"));
    return cfg;
}

std::optional<std::string> migrateLegacyConfig(const std::string& tomlText) {
    toml::table root;
    try {
        root = toml::parse(dedupeLastWins(tomlText));
    } catch (const toml::parse_error&) {
        return std::nullopt;
    }

    auto* capture = root.get("capture") ? root.get("capture")->as_table() : nullptr;
    if (capture == nullptr) {
        return std::nullopt;
    }

    bool hasBareQuality = capture->contains("quality");
    bool hasFormat = capture->contains("format");
    if (!hasBareQuality || hasFormat) {
        return std::nullopt;
    }

    auto qualityNode = capture->get("quality");
    auto qualityVal = qualityNode ? qualityNode->value<int64_t>() : std::nullopt;
    if (!qualityVal.has_value()) {
        return std::nullopt;
    }
    int64_t oldQuality = *qualityVal;

    capture->erase("quality");
    capture->insert_or_assign("format", std::string("jpeg"));

    toml::table jpegOpts{
        {"quality", oldQuality},
        {"chroma_subsampling", std::string("4:2:2")},
    };
    toml::table formatOptions{
        {"jpeg", jpegOpts},
    };
    capture->insert_or_assign("format_options", formatOptions);

    return formatTomlTable(root);
}

namespace {

// Mirrors Rust's handle_removed_format: a raw substring check/replace for
// the one known-dead value (avif) that predates the general
// unknown-format-falls-back-to-default contract implemented in
// parseFormatValue/parseConfig. Kept for historical/documentation parity
// with the Rust source; parseConfig's own fallback logic makes this
// redundant in practice (an "avif" value simply falls through the
// fromWireString lookup and defaults to Jxl with a warning), but retaining
// the explicit check preserves the exact log message a v0.5.x user would
// have seen, easing the diagnostic transition.
std::optional<std::string> handleRemovedFormat(const std::string& raw) {
    const std::string needle = "format = \"avif\"";
    auto pos = raw.find(needle);
    if (pos == std::string::npos) {
        return std::nullopt;
    }
    logWarning("'avif' format is no longer supported, falling back to jpeg");
    std::string result = raw;
    result.replace(pos, needle.size(), "format = \"jpeg\"");
    return result;
}

}  // namespace

Config loadConfig() {
    std::filesystem::path path = configFilePath();

    if (!std::filesystem::exists(path)) {
        Config def;
        saveConfig(def);
        return def;
    }

    std::ifstream in(path, std::ios::binary);
    if (!in) {
        throw ConfigError("failed to read " + path.string());
    }
    std::ostringstream buf;
    buf << in.rdbuf();
    std::string raw = buf.str();

    std::optional<std::string> removedFormat = handleRemovedFormat(raw);
    std::optional<std::string> migrated = migrateLegacyConfig(raw);
    std::optional<std::string> merged = removedFormat.has_value() ? removedFormat : migrated;
    const std::string& parseStr = merged.has_value() ? *merged : raw;

    Config config = parseConfig(parseStr);

    if (migrated.has_value()) {
        try {
            saveConfig(config);
        } catch (const ConfigError& e) {
            logWarning(std::string("failed to re-save migrated config: ") + e.what());
            // Non-fatal -- config was already parsed successfully in memory.
        }
    }

    return config;
}

void saveConfig(const Config& config) {
    std::filesystem::path path = configFilePath();
    std::string text = serializeConfig(config);
    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    if (!out) {
        throw ConfigError("failed to write config to " + path.string());
    }
    out << text;
    if (!out) {
        throw ConfigError("failed to write config to " + path.string());
    }
}

}  // namespace snip
