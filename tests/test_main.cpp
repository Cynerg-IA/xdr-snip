// test_main.cpp -- hand-rolled test runner for snip_types / snip config.
//
// Replaces the Rust #[cfg(test)] mod tests in snip-types/src/lib.rs and
// snip-app/src/config.rs (issue #8: "the Rust tests are the only existing
// safety net; C++ equivalents must exist before Phase 2"). Each test is a
// plain function returning bool; main() runs them all, prints PASS/FAIL per
// test, and returns nonzero on any failure so `ctest` reports failure
// correctly.
//
// No external test framework dependency -- avoids a second FetchContent
// beyond toml++, per issue #8's explicit "zero extra network-fetch risk"
// guidance.

#include <cstdio>
#include <fstream>
#include <functional>
#include <sstream>
#include <string>
#include <vector>

#include "config.h"
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

// ======================== types.h tests (port of snip-types tests) ========================

bool test_default_config_is_valid() {
    snip::Config cfg;
    // C++ behavior change vs Rust: default format is Jxl, not Jpeg.
    CHECK(cfg.capture.format == snip::OutputFormat::Jxl);
    CHECK(cfg.capture.formatOptions.jpeg.quality == 85);
    CHECK(cfg.behavior.copyToClipboard);
    CHECK(cfg.behavior.saveToFile);
    CHECK(cfg.behavior.showNotification);
    CHECK(cfg.hotkey.key == "PrintScreen");
    CHECK(cfg.hotkey.modifiers.empty());
    return true;
}

bool test_resize_config_defaults() {
    snip::Config cfg;
    CHECK(!cfg.capture.resize.enabled);
    CHECK(cfg.capture.resize.maxWidth == 2048);
    CHECK(cfg.capture.resize.maxHeight == 2048);
    CHECK(!cfg.capture.resize.keepOriginal);
    return true;
}

bool test_resize_config_serialization_roundtrip() {
    snip::Config cfg;
    cfg.capture.resize.enabled = true;
    cfg.capture.resize.maxWidth = 3840;
    cfg.capture.resize.maxHeight = 2160;
    std::string serialized = snip::serializeConfig(cfg);
    snip::Config deserialized = snip::parseConfig(serialized);
    CHECK(deserialized.capture.resize.enabled);
    CHECK(deserialized.capture.resize.maxWidth == 3840);
    CHECK(deserialized.capture.resize.maxHeight == 2160);
    return true;
}

bool test_region_display() {
    snip::Region r{100, 200, 800, 600};
    CHECK(r.toString() == "800x600+100+200");
    return true;
}

bool test_output_format_extensions() {
    // Required test #2: extension() correct for all 8 formats.
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

bool test_output_format_display_names() {
    CHECK(std::string(snip::displayName(snip::OutputFormat::Jxl)) == "JPEG XL");
    CHECK(std::string(snip::displayName(snip::OutputFormat::Jpeg)) == "JPEG");
    CHECK(std::string(snip::displayName(snip::OutputFormat::Png)) == "PNG");
    CHECK(std::string(snip::displayName(snip::OutputFormat::WebP)) == "WebP");
    CHECK(std::string(snip::displayName(snip::OutputFormat::Tiff)) == "TIFF");
    CHECK(std::string(snip::displayName(snip::OutputFormat::Bmp)) == "BMP");
    CHECK(std::string(snip::displayName(snip::OutputFormat::Qoi)) == "QOI");
    CHECK(std::string(snip::displayName(snip::OutputFormat::OpenExr)) == "OpenEXR (HDR)");
    return true;
}

bool test_output_format_all_order() {
    const auto& all = snip::allOutputFormats();
    CHECK(all.size() == 8);
    CHECK(all.front() == snip::OutputFormat::Jxl);  // Jxl must be FIRST
    return true;
}

bool test_output_format_hdr_preservation() {
    // Required test #3: preservesHdr() true for Jxl + OpenExr only.
    CHECK(snip::preservesHdr(snip::OutputFormat::Jxl));
    CHECK(snip::preservesHdr(snip::OutputFormat::OpenExr));
    CHECK(!snip::preservesHdr(snip::OutputFormat::Jpeg));
    CHECK(!snip::preservesHdr(snip::OutputFormat::Png));
    CHECK(!snip::preservesHdr(snip::OutputFormat::WebP));
    CHECK(!snip::preservesHdr(snip::OutputFormat::Tiff));
    CHECK(!snip::preservesHdr(snip::OutputFormat::Bmp));
    CHECK(!snip::preservesHdr(snip::OutputFormat::Qoi));
    return true;
}

bool test_default_format_is_jxl() {
    // Required test #4: default format is Jxl.
    CHECK(snip::defaultFormat() == snip::OutputFormat::Jxl);
    return true;
}

bool test_format_options_roundtrip_toml() {
    // Required test #1: round-trip Config -> serialize -> parse -> equal
    // (field-by-field, since Config has no operator== in this port).
    snip::Config cfg;
    std::string serialized = snip::serializeConfig(cfg);
    snip::Config deserialized = snip::parseConfig(serialized);
    CHECK(deserialized.capture.format == snip::OutputFormat::Jxl);
    CHECK(deserialized.capture.formatOptions.jpeg.quality == 85);
    CHECK(deserialized.capture.formatOptions.jpeg.chromaSubsampling ==
          snip::ChromaSubsampling::Half);
    CHECK(deserialized.capture.formatOptions.png.compression == 6);
    CHECK(deserialized.capture.formatOptions.png.filter == snip::PngFilter::Adaptive);
    CHECK(deserialized.capture.formatOptions.webp.lossless == false);
    CHECK(deserialized.capture.formatOptions.webp.quality == 80.0f);
    CHECK(deserialized.capture.formatOptions.tiff.compression ==
          snip::TiffCompression::Lzw);
    CHECK(deserialized.capture.formatOptions.exr.compression ==
          snip::ExrCompression::Zip16);
    CHECK(deserialized.capture.formatOptions.jxl.lossless ==
          cfg.capture.formatOptions.jxl.lossless);
    CHECK(deserialized.capture.saveDir == cfg.capture.saveDir);
    CHECK(deserialized.capture.filenamePattern == cfg.capture.filenamePattern);
    CHECK(deserialized.hotkey.key == cfg.hotkey.key);
    CHECK(deserialized.hotkey.modifiers == cfg.hotkey.modifiers);
    CHECK(deserialized.behavior.copyToClipboard == cfg.behavior.copyToClipboard);
    CHECK(deserialized.behavior.saveToFile == cfg.behavior.saveToFile);
    CHECK(deserialized.behavior.showNotification == cfg.behavior.showNotification);
    return true;
}

bool test_legacy_config_loads_with_defaults() {
    // Config missing format/format_options entirely -> defaults apply.
    // NOTE: mirrors the Rust legacy_config_loads_with_defaults test, adapted
    // for the Jxl default (was Jpeg in Rust).
    const std::string legacy = R"TOML(
[capture]
save_dir = "~/Pictures/XDR-Snips"
filename_pattern = "screenshot_{timestamp}"

[hotkey]
key = "PrintScreen"
modifiers = []

[behavior]
copy_to_clipboard = true
save_to_file = true
show_notification = true
)TOML";
    snip::Config cfg = snip::parseConfig(legacy);
    CHECK(cfg.capture.format == snip::OutputFormat::Jxl);
    CHECK(cfg.capture.formatOptions.jpeg.quality == 85);
    return true;
}

// ======================== required-test checklist items 5-7 ========================

bool test_v05x_format_jpeg_parses_as_jpeg() {
    // Required test #5: legacy v0.5.x config with format = "jpeg" parses
    // and yields Jpeg.
    const std::string v05x = R"TOML(
[capture]
format = "jpeg"
save_dir = "~/Pictures/XDR-Snips"
filename_pattern = "screenshot_{timestamp}"

[capture.format_options.jpeg]
quality = 85
chroma_subsampling = "4:2:2"

[hotkey]
key = "PrintScreen"
modifiers = []

[behavior]
copy_to_clipboard = true
save_to_file = true
show_notification = true
)TOML";
    snip::Config cfg = snip::parseConfig(v05x);
    CHECK(cfg.capture.format == snip::OutputFormat::Jpeg);
    return true;
}

bool test_unknown_format_falls_back_to_default_no_throw() {
    // Required test #6: unknown/unparseable format string -> default (Jxl),
    // no throw/crash.
    const std::string badFormat = R"TOML(
[capture]
format = "avif"
)TOML";
    snip::Config cfg = snip::parseConfig(badFormat);  // must not throw
    CHECK(cfg.capture.format == snip::OutputFormat::Jxl);

    const std::string garbageFormat = R"TOML(
[capture]
format = "totally-not-a-format"
)TOML";
    snip::Config cfg2 = snip::parseConfig(garbageFormat);  // must not throw
    CHECK(cfg2.capture.format == snip::OutputFormat::Jxl);
    return true;
}

bool test_resize_defaults_required_check() {
    // Required test #7: resize defaults disabled, 2048, 2048,
    // keepOriginal=false.
    snip::Config cfg;
    CHECK(cfg.capture.resize.enabled == false);
    CHECK(cfg.capture.resize.maxWidth == 2048);
    CHECK(cfg.capture.resize.maxHeight == 2048);
    CHECK(cfg.capture.resize.keepOriginal == false);
    return true;
}

// ======================== v0.6.0 phase-1 config-fix tests (issue #8/#9) ========================

bool test_duplicate_key_last_wins() {
    // toml++ rejects duplicate keys outright; v0.5.x shipped configs with a
    // duplicated keep_original, so we pre-dedupe with last-wins semantics
    // (matching Rust's toml crate) before handing text to toml::parse.
    // Assert the VALUE, not just no-throw -- a no-throw-only test would
    // still pass under an (incorrect) first-wins implementation.
    const std::string dup = "[capture.resize]\nenabled = false\nkeep_original = false\nkeep_original = true\n";
    snip::Config cfg = snip::parseConfig(dup);  // must not throw
    CHECK(cfg.capture.resize.keepOriginal == true);
    return true;
}

bool test_real_repo_config_toml_parses() {
    // The repo's actual shipped config.toml must parse without throwing --
    // guards against regressions in the deduped file or the parser itself.
    std::ifstream in("config.toml", std::ios::binary);
    CHECK(static_cast<bool>(in));
    std::ostringstream buf;
    buf << in.rdbuf();
    snip::Config cfg = snip::parseConfig(buf.str());  // must not throw
    (void)cfg;
    return true;
}

bool test_jxl_quality_is_85_equivalent() {
    // Default JXL distance must be 1.45, the libjxl JPEG-quality-85
    // equivalent per issue #8 (d = 0.1 + (100-q)*0.09).
    snip::JxlOptions opts;
    CHECK(opts.quality == 1.45f);
    return true;
}

// ======================== config.h tests (port of snip-app config.rs tests) ========================

bool test_expand_tilde_no_tilde() {
    std::string p = snip::expandTilde("C:/some/path");
    CHECK(p == "C:/some/path");
    return true;
}

bool test_expand_tilde_with_tilde() {
    std::string p = snip::expandTilde("~/Pictures/Screenshots");
    CHECK(!p.empty() && p[0] != '~');
    return true;
}

bool test_generate_filename_replaces_timestamp() {
    std::string name = snip::generateFilename("shot_{timestamp}_end");
    CHECK(name.find("{timestamp}") == std::string::npos);
    CHECK(name.rfind("shot_", 0) == 0);
    CHECK(name.size() >= 4 && name.compare(name.size() - 4, 4, "_end") == 0);
    return true;
}

bool test_migrate_legacy_config_detects_bare_quality() {
    const std::string legacy = R"TOML(
[capture]
quality = 92
save_dir = "~/Pictures/XDR-Snips"
filename_pattern = "screenshot_{timestamp}"

[hotkey]
key = "PrintScreen"
modifiers = []

[behavior]
copy_to_clipboard = true
save_to_file = true
show_notification = true
)TOML";
    auto migrated = snip::migrateLegacyConfig(legacy);
    CHECK(migrated.has_value());

    const std::string& newToml = *migrated;
    CHECK(newToml.find("format = \"jpeg\"") != std::string::npos);
    CHECK(newToml.find("[capture.format_options.jpeg]") != std::string::npos ||
          newToml.find("format_options") != std::string::npos);

    snip::Config cfg = snip::parseConfig(newToml);
    CHECK(cfg.capture.format == snip::OutputFormat::Jpeg);
    CHECK(cfg.capture.formatOptions.jpeg.quality == 92);
    return true;
}

bool test_migrate_legacy_config_skips_new_format() {
    const std::string newConfig = R"TOML(
[capture]
format = "png"
save_dir = "~/Pictures/XDR-Snips"

[capture.format_options.png]
compression = 6
filter = "adaptive"
)TOML";
    auto migrated = snip::migrateLegacyConfig(newConfig);
    CHECK(!migrated.has_value());
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
        {"default_config_is_valid", test_default_config_is_valid},
        {"resize_config_defaults", test_resize_config_defaults},
        {"resize_config_serialization_roundtrip", test_resize_config_serialization_roundtrip},
        {"region_display", test_region_display},
        {"output_format_extensions", test_output_format_extensions},
        {"output_format_display_names", test_output_format_display_names},
        {"output_format_all_order", test_output_format_all_order},
        {"output_format_hdr_preservation", test_output_format_hdr_preservation},
        {"default_format_is_jxl", test_default_format_is_jxl},
        {"format_options_roundtrip_toml", test_format_options_roundtrip_toml},
        {"legacy_config_loads_with_defaults", test_legacy_config_loads_with_defaults},
        {"v05x_format_jpeg_parses_as_jpeg", test_v05x_format_jpeg_parses_as_jpeg},
        {"unknown_format_falls_back_to_default_no_throw",
         test_unknown_format_falls_back_to_default_no_throw},
        {"resize_defaults_required_check", test_resize_defaults_required_check},
        {"expand_tilde_no_tilde", test_expand_tilde_no_tilde},
        {"expand_tilde_with_tilde", test_expand_tilde_with_tilde},
        {"generate_filename_replaces_timestamp", test_generate_filename_replaces_timestamp},
        {"migrate_legacy_config_detects_bare_quality",
         test_migrate_legacy_config_detects_bare_quality},
        {"migrate_legacy_config_skips_new_format", test_migrate_legacy_config_skips_new_format},
        {"duplicate_key_last_wins", test_duplicate_key_last_wins},
        {"real_repo_config_toml_parses", test_real_repo_config_toml_parses},
        {"jxl_quality_is_85_equivalent", test_jxl_quality_is_85_equivalent},
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
