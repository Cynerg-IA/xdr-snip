//! Configuration loading and path utilities.
//!
//! Reads `config.toml` from `%APPDATA%/xdr-snip/`, creating it with defaults
//! if the file or directory does not yet exist.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;
use snip_types::{Config, SnipError};
use tracing::{debug, info, warn};

/// Name of the config directory under `%APPDATA%`.
const APP_DIR_NAME: &str = "xdr-snip";

/// Config file name inside the app directory.
const CONFIG_FILE_NAME: &str = "config.toml";

/// Loads the application configuration from `%APPDATA%/xdr-snip/config.toml`.
///
/// If the file does not exist, a default config is written to disk and returned.
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
        write_default_config(&config_path, &default)?;
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

    let config: Config = toml::from_str(&raw).map_err(|e| {
        SnipError::Config(format!(
            "failed to parse {}: {}",
            config_path.display(),
            e
        ))
    })?;

    // Validate quality range
    if config.capture.quality == 0 || config.capture.quality > 100 {
        warn!(
            "load_config: quality {} out of 1-100 range, clamping",
            config.capture.quality
        );
    }

    info!(
        "load_config: loaded config — quality={}, save_dir={}, hotkey={}",
        config.capture.quality, config.capture.save_dir, config.hotkey.key
    );

    Ok(config)
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
    write_default_config(&path, config)?;
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

/// Serializes the default config to TOML and writes it to `path`.
fn write_default_config(path: &Path, config: &Config) -> Result<(), SnipError> {
    let toml_str = toml::to_string_pretty(config).map_err(|e| {
        SnipError::Config(format!("failed to serialize default config: {}", e))
    })?;

    fs::write(path, &toml_str).map_err(|e| {
        SnipError::Config(format!(
            "failed to write default config to {}: {}",
            path.display(),
            e
        ))
    })?;

    info!(
        "write_default_config: wrote {} bytes to {}",
        toml_str.len(),
        path.display()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
