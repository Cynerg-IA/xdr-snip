//! Configuration loading, legacy migration, and path utilities.
//!
//! Reads `config.toml` from `%APPDATA%/xdr-snip/`, creating it with defaults
//! if the file or directory does not yet exist. Detects legacy config files
//! (pre-v0.4 with bare `quality` field) and migrates them to the new format.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;
use snip_types::{Config, OutputFormat, SnipError};
use tracing::{debug, info, warn};

/// Name of the config directory under `%APPDATA%`.
const APP_DIR_NAME: &str = "xdr-snip";

/// Config file name inside the app directory.
const CONFIG_FILE_NAME: &str = "config.toml";

/// Loads the application configuration from `%APPDATA%/xdr-snip/config.toml`.
///
/// If the file does not exist, a default config is written to disk and returned.
/// Detects legacy configs (bare `quality` field under `[capture]`) and migrates
/// them to the new format-options structure.
///
/// Returns [`SnipError::Config`] if the file exists but cannot be parsed.
pub fn load_config() -> Result<Config, SnipError> {
    debug!("load_config: resolving config path");

    let config_dir = resolve_config_dir()?;
    let config_path = config_dir.join(CONFIG_FILE_NAME);

    debug!("load_config: config_path={}", config_path.display());

    if !config_path.exists() {
        info!(
            "load_config: config file not found, creating default at {}",
            config_path.display()
        );
        let default = Config::default();
        write_config(&config_path, &default)?;
        return Ok(default);
    }

    let raw = fs::read_to_string(&config_path).map_err(|e| {
        SnipError::Config(format!(
            "failed to read {}: {}",
            config_path.display(),
            e
        ))
    })?;

    debug!(
        "load_config: read {} bytes from config file",
        raw.len()
    );

    // Check for legacy config before parsing into the new struct,
    // and remove deprecated formats (e.g. avif).
    let removed_format = handle_removed_format(&raw);
    let migrated = migrate_legacy_config(&raw);
    let parse_str = removed_format
        .or(migrated.clone())
        .as_deref()
        .unwrap_or(&raw);

    let config: Config = toml::from_str(parse_str).map_err(|e| {
        SnipError::Config(format!(
            "failed to parse {}: {}",
            config_path.display(),
            e
        ))
    })?;

    // If we migrated, re-save in the new format
    if migrated.is_some() {
        info!("load_config: legacy config migrated, re-saving in new format");
        if let Err(e) = write_config(&config_path, &config) {
            warn!("load_config: failed to re-save migrated config: {}", e);
            // Non-fatal — config was parsed successfully
        }
    }

    info!(
        "load_config: loaded config — format={}, save_dir={}, hotkey={}",
        config.capture.format, config.capture.save_dir, config.hotkey.key
    );

    Ok(config)
}

/// Validates loaded config and handles removed/deprecated formats.
///
/// If the config specifies a format that no longer exists (e.g. after
/// removing AVIF support), replaces it with `jpeg` before parsing.
fn handle_removed_format(raw: &str) -> Option<String> {
    if !raw.contains("format = \"avif\"") {
        return None;
    }
    warn!("load_config: 'avif' format is no longer supported, falling back to jpeg");
    let result = raw.replace("format = \"avif\"", "format = \"jpeg\"");
    debug!("handle_removed_format: replaced avif with jpeg in config");
    Some(result)
}

/// Detects and migrates a pre-v0.4 legacy config.///
/// Legacy configs have a bare `quality = N` under `[capture]` but no `format`
/// or `format_options` fields. This function:
/// 1. Parses as raw TOML table to check for the legacy field.
/// 2. Reads the old quality value.
/// 3. Removes the bare `quality` key.
/// 4. Adds `format = "jpeg"` and `format_options.jpeg.quality = N`.
///
/// Returns `Some(new_toml_string)` if migration was needed, `None` otherwise.
fn migrate_legacy_config(raw: &str) -> Option<String> {
    let table: toml::Value = toml::from_str(raw).ok()?;
    let capture = table.get("capture")?.as_table()?;

    // Legacy indicator: has bare `quality` but no `format` field
    let has_bare_quality = capture.contains_key("quality");
    let has_format = capture.contains_key("format");

    if !has_bare_quality || has_format {
        debug!("migrate_legacy_config: no migration needed (bare_quality={}, has_format={})",
            has_bare_quality, has_format);
        return None;
    }

    let old_quality = capture.get("quality")?.as_integer()? as u32;
    info!(
        "migrate_legacy_config: detected legacy config with quality={}, migrating",
        old_quality
    );

    // Build the new config from the parsed table, overriding capture section
    let mut new_table = table.as_table()?.clone();
    let new_capture = new_table.get_mut("capture")?.as_table_mut()?;

    // Remove the bare quality field
    new_capture.remove("quality");

    // Add format = "jpeg"
    new_capture.insert(
        "format".to_string(),
        toml::Value::String("jpeg".to_string()),
    );

    // Add format_options with the migrated quality
    let mut jpeg_opts = toml::map::Map::new();
    jpeg_opts.insert(
        "quality".to_string(),
        toml::Value::Integer(old_quality as i64),
    );
    jpeg_opts.insert(
        "chroma_subsampling".to_string(),
        toml::Value::String("4:2:2".to_string()),
    );

    let mut format_options = toml::map::Map::new();
    format_options.insert("jpeg".to_string(), toml::Value::Table(jpeg_opts));

    new_capture.insert(
        "format_options".to_string(),
        toml::Value::Table(format_options),
    );

    let migrated = toml::to_string_pretty(&toml::Value::Table(new_table)).ok()?;
    debug!(
        "migrate_legacy_config: migration complete — quality {} → format_options.jpeg.quality",
        old_quality
    );

    Some(migrated)
}

/// Expands a leading `~` in a path string to the user's home directory.
///
/// Non-tilde paths are returned as-is.
///
/// # Examples
/// ```ignore
/// let p = expand_tilde("~/Pictures/Screenshots");
/// // => C:\Users\Alice\Pictures\Screenshots  (on Windows)
/// ```
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        // dirs::home_dir() returns the user's home directory
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
        warn!("expand_tilde: could not resolve home directory, returning path as-is");
    }
    PathBuf::from(path)
}

/// Generates a filename from a pattern string, replacing `{timestamp}` with
/// the current local time formatted as `YYYYMMDD_HHmmss`.
///
/// # Examples
/// ```ignore
/// let name = generate_filename("screenshot_{timestamp}");
/// // => "screenshot_20260227_143022"
/// ```
pub fn generate_filename(pattern: &str) -> String {
    let now = Local::now();
    let timestamp = now.format("%Y%m%d_%H%M%S").to_string();
    let result = pattern.replace("{timestamp}", &timestamp);
    debug!(
        "generate_filename: pattern={} -> result={}",
        pattern, result
    );
    result
}

/// Returns the path to the config file (`%APPDATA%/xdr-snip/config.toml`).
///
/// Used by the tray "Settings" menu item to open the file in the default editor.
pub fn config_file_path() -> Result<PathBuf, SnipError> {
    let dir = resolve_config_dir()?;
    Ok(dir.join(CONFIG_FILE_NAME))
}

/// Saves the given configuration to `%APPDATA%/xdr-snip/config.toml`.
///
/// Overwrites the existing file. Used by the settings dialog to persist changes.
pub fn save_config(config: &Config) -> Result<(), SnipError> {
    let path = config_file_path()?;
    debug!("save_config: writing to {}", path.display());
    write_config(&path, config)?;
    info!("save_config: config saved successfully");
    Ok(())
}

// ======================== INTERNAL HELPERS ========================

/// Resolves the config directory (`%APPDATA%/xdr-snip`), creating it if needed.
fn resolve_config_dir() -> Result<PathBuf, SnipError> {
    let base = dirs::config_dir().ok_or_else(|| {
        SnipError::Config("cannot determine %APPDATA% directory".to_string())
    })?;

    let dir = base.join(APP_DIR_NAME);

    if !dir.exists() {
        debug!(
            "resolve_config_dir: creating directory {}",
            dir.display()
        );
        fs::create_dir_all(&dir).map_err(|e| {
            SnipError::Config(format!(
                "failed to create config dir {}: {}",
                dir.display(),
                e
            ))
        })?;
    }

    Ok(dir)
}

/// Serializes config to TOML and writes it to `path`.
fn write_config(path: &Path, config: &Config) -> Result<(), SnipError> {
    let toml_str = toml::to_string_pretty(config).map_err(|e| {
        SnipError::Config(format!("failed to serialize config: {}", e))
    })?;

    fs::write(path, &toml_str).map_err(|e| {
        SnipError::Config(format!(
            "failed to write config to {}: {}",
            path.display(),
            e
        ))
    })?;

    info!(
        "write_config: wrote {} bytes to {}",
        toml_str.len(),
        path.display()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use snip_types::OutputFormat;

    #[test]
    fn expand_tilde_no_tilde() {
        let p = expand_tilde("C:/some/path");
        assert_eq!(p, PathBuf::from("C:/some/path"));
    }

    #[test]
    fn expand_tilde_with_tilde() {
        let p = expand_tilde("~/Pictures/Screenshots");
        // Should not start with ~ anymore (unless home_dir fails)
        assert!(!p.to_string_lossy().starts_with('~'));
    }

    #[test]
    fn generate_filename_replaces_timestamp() {
        let name = generate_filename("shot_{timestamp}_end");
        // Should not contain the literal placeholder anymore
        assert!(!name.contains("{timestamp}"));
        assert!(name.starts_with("shot_"));
        assert!(name.ends_with("_end"));
    }

    #[test]
    fn migrate_legacy_config_detects_bare_quality() {
        let legacy = r#"
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
"#;
        let migrated = migrate_legacy_config(legacy);
        assert!(migrated.is_some(), "should detect legacy config");

        let new_toml = migrated.unwrap();
        // New format field should be present
        assert!(new_toml.contains("format = \"jpeg\""), "format field should be added");
        // Quality should be under format_options.jpeg section
        assert!(new_toml.contains("[capture.format_options.jpeg]"), "jpeg options section should exist");

        // Parse and verify the migration produced correct values
        let cfg: Config = toml::from_str(&new_toml).expect("migrated config should parse");
        assert_eq!(cfg.capture.format, OutputFormat::Jpeg);
        assert_eq!(cfg.capture.format_options.jpeg.quality, 92);
        // Verify bare quality is not in the capture section (it's under format_options.jpeg)
        let reparsed: toml::Value = toml::from_str(&new_toml).unwrap();
        let capture = reparsed.get("capture").unwrap().as_table().unwrap();
        assert!(!capture.contains_key("quality"), "bare quality should be removed from [capture]");
    }

    #[test]
    fn migrate_legacy_config_skips_new_format() {
        let new_config = r#"
[capture]
format = "png"
save_dir = "~/Pictures/XDR-Snips"

[capture.format_options.png]
compression = 6
filter = "adaptive"
"#;
        let migrated = migrate_legacy_config(new_config);
        assert!(migrated.is_none(), "should not migrate new-format config");
    }
}
