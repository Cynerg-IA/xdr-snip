// config.h -- C++ port of crates/snip-app/src/config.rs (v0.5.0 Rust
// source).
//
// Loads/saves config.toml, same schema and key names as the Rust version.
// See config.cpp for the backward-compat contract (unknown format string ->
// default format with a logged warning, never a throw/crash) and the
// legacy pre-v0.4 bare-`quality` migration.
//
// Error signaling: this phase has no Win32/subprocess/clipboard code yet, so
// a full SnipError-equivalent (as in the Rust source) is out of scope. We
// use a small ConfigError exception type (see below) for genuine I/O/parse
// failures in load/save, EXCEPT for the specific "unknown format string"
// path, which per the spec must NEVER throw -- it falls back to
// OutputFormat::defaultFormat() with a logged warning instead.

#pragma once

#include <chrono>
#include <filesystem>
#include <optional>
#include <stdexcept>
#include <string>

#include "types.h"

namespace snip {

// Thrown by loadConfig/saveConfig on genuine I/O or TOML-syntax failures
// (e.g. the file exists but is not valid TOML at all, or the directory
// cannot be created/written). This is a deliberate, documented substitute
// for the Rust SnipError::Config(String) variant -- narrower in scope since
// no other SnipError variants (hotkey/overlay/clipboard/etc) exist yet in
// this C++ port.
class ConfigError : public std::runtime_error {
public:
    explicit ConfigError(const std::string& msg) : std::runtime_error(msg) {}
};

// ======================== PLATFORM SEAM ========================
//
// Platform-specific config directory resolution, isolated behind this one
// function so Phase 4/5 Windows work is a clean swap-in. Rust used
// dirs::config_dir() (-> %APPDATA% on Windows). On Windows this should
// resolve via SHGetKnownFolderPath(FOLDERID_RoamingAppData); on Linux/CI
// (this phase's only buildable/testable target) it falls back to
// $XDG_CONFIG_HOME or $HOME/.config. See config.cpp for the actual
// implementation, clearly bounded under #ifdef _WIN32 / #else.
//
// Returns std::nullopt if no base config directory could be resolved at all
// (e.g. no HOME and no XDG_CONFIG_HOME on Linux) -- callers should treat
// this the same way Rust treated dirs::config_dir() returning None.
std::optional<std::filesystem::path> platformConfigDir();

// ======================== CORE CONFIG API ========================

// Resolves the config directory (platformConfigDir() + "xdr-snip"),
// creating it if needed. Throws ConfigError if the base directory cannot be
// resolved or created.
std::filesystem::path resolveConfigDir();

// Returns the path to the config file (<config dir>/config.toml). Throws
// ConfigError under the same conditions as resolveConfigDir().
std::filesystem::path configFilePath();

// Loads the application configuration from <config dir>/config.toml.
//
// If the file does not exist, a default config is written to disk and
// returned. Detects legacy configs (bare `quality` field under [capture])
// and migrates them to the new format-options structure, re-saving
// non-fatally on migration. An unknown/unparseable `format` string falls
// back to OutputFormat::defaultFormat() (Jxl) with a warning logged to
// stderr -- this path NEVER throws for a bad format value specifically.
//
// Throws ConfigError for other genuine I/O/parse failures (unreadable
// file, directory creation failure, TOML that fails to parse as a table at
// all).
Config loadConfig();

// Saves the given configuration to <config dir>/config.toml, overwriting
// any existing file. Throws ConfigError on I/O failure.
void saveConfig(const Config& config);

// ======================== HELPERS (exposed for testing) ========================

// Expands a leading "~/" in a path string to the user's home directory.
// Non-"~/"-prefixed strings (including a bare "~" with no trailing slash)
// are returned unchanged -- this exactly matches the Rust expand_tilde,
// which only strips a literal "~/" prefix. If no home directory can be
// resolved, logs a warning to stderr and returns the path as-is (still
// containing the literal "~/").
std::string expandTilde(const std::string& path);

// Generates a filename from a pattern string, replacing the literal
// substring "{timestamp}" with the current local time formatted as
// YYYYMMDD_HHmmss. Simple substring replace, not full templating -- matches
// Rust's generate_filename exactly.
std::string generateFilename(const std::string& pattern);

// Serializes a Config to a pretty TOML string. Exposed for round-trip
// testing.
std::string serializeConfig(const Config& config);

// Parses a TOML string into a Config. Missing keys/sections at every level
// fall back to field-level defaults (mirroring Rust's #[serde(default)]
// behavior applied recursively). An unknown/unparseable `format` value
// falls back to OutputFormat::defaultFormat() with a warning logged to
// stderr -- NEVER throws for that specific case.
//
// Throws ConfigError if the input is not syntactically valid TOML at all,
// or the top level is not a table.
Config parseConfig(const std::string& tomlText);

// Detects and migrates a pre-v0.4 legacy config (bare `quality = N` under
// [capture], no `format` key present). Returns the migrated TOML string if
// migration was needed, or std::nullopt if the input is not legacy-shaped
// (mirrors Rust's migrate_legacy_config Option<String> return).
std::optional<std::string> migrateLegacyConfig(const std::string& tomlText);

}  // namespace snip
