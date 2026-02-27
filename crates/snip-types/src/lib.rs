//! # snip-types
//!
//! Shared type definitions, configuration structs, and error types for HDR Snip.
//! This crate is dependency-free from platform APIs so it can be used in tests
//! and tooling without pulling in the Windows crate.

use serde::{Deserialize, Serialize};
use std::fmt;

// ======================== CONFIGURATION ========================

/// Top-level application configuration loaded from `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Settings related to screenshot capture (quality, output path, naming).
    #[serde(default)]
    pub capture: CaptureConfig,

    /// Global hotkey binding configuration.
    #[serde(default)]
    pub hotkey: HotkeyConfig,

    /// Runtime behavior flags (clipboard, file save, notifications).
    #[serde(default)]
    pub behavior: BehaviorConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            capture: CaptureConfig::default(),
            hotkey: HotkeyConfig::default(),
            behavior: BehaviorConfig::default(),
        }
    }
}

/// Capture-related settings: JPEG quality, output directory, filename template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    /// JPEG quality level (1-100). Higher values produce larger, sharper files.
    #[serde(default = "default_quality")]
    pub quality: u32,

    /// Directory where screenshots are saved. Supports `~` for home directory.
    #[serde(default = "default_save_dir")]
    pub save_dir: String,

    /// Filename template. `{timestamp}` is replaced with `YYYYMMDD_HHmmss`.
    #[serde(default = "default_filename_pattern")]
    pub filename_pattern: String,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            quality: default_quality(),
            save_dir: default_save_dir(),
            filename_pattern: default_filename_pattern(),
        }
    }
}

/// Hotkey binding: which key and which modifier keys trigger a capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    /// Primary key name (e.g. "PrintScreen", "S", "F12").
    #[serde(default = "default_hotkey_key")]
    pub key: String,

    /// Modifier keys held alongside the primary key (e.g. ["Alt", "Shift"]).
    #[serde(default)]
    pub modifiers: Vec<String>,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            key: default_hotkey_key(),
            modifiers: Vec::new(),
        }
    }
}

/// Runtime behavior toggles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorConfig {
    /// Whether to copy the captured image to the system clipboard.
    #[serde(default = "default_true")]
    pub copy_to_clipboard: bool,

    /// Whether to persist the captured image as a file on disk.
    #[serde(default = "default_true")]
    pub save_to_file: bool,

    /// Whether to display a toast/balloon notification after capture.
    #[serde(default = "default_true")]
    pub show_notification: bool,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            copy_to_clipboard: true,
            save_to_file: true,
            show_notification: true,
        }
    }
}

// ======================== SERDE DEFAULTS ========================

/// Default JPEG quality: 85 is a good balance of size and sharpness.
fn default_quality() -> u32 {
    85
}

/// Default save directory using tilde notation for portability.
fn default_save_dir() -> String {
    "~/Pictures/Screenshots".to_string()
}

/// Default filename pattern with timestamp placeholder.
fn default_filename_pattern() -> String {
    "screenshot_{timestamp}".to_string()
}

/// Default hotkey trigger key.
fn default_hotkey_key() -> String {
    "PrintScreen".to_string()
}

/// Returns `true` — used as serde default for boolean flags.
fn default_true() -> bool {
    true
}

// ======================== GEOMETRY ========================

/// A rectangular screen region in pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// X coordinate of the top-left corner (can be negative on multi-monitor).
    pub x: i32,
    /// Y coordinate of the top-left corner (can be negative on multi-monitor).
    pub y: i32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}+{}+{}", self.w, self.h, self.x, self.y)
    }
}

// ======================== ERRORS ========================

/// Central error type for all HDR Snip operations.
#[derive(Debug, thiserror::Error)]
pub enum SnipError {
    /// Configuration loading or parsing failure.
    #[error("config error: {0}")]
    Config(String),

    /// Failed to register the global hotkey.
    #[error("hotkey registration error: {0}")]
    HotkeyRegistration(String),

    /// Overlay window creation or interaction failure.
    #[error("overlay error: {0}")]
    Overlay(String),

    /// Failed to launch or communicate with the capture subprocess.
    #[error("capture process error: {0}")]
    CaptureProcess(String),

    /// The capture subprocess ran but produced no usable output.
    #[error("capture failed: {0}")]
    CaptureFailed(String),

    /// Clipboard operation failure.
    #[error("clipboard error: {0}")]
    Clipboard(String),

    /// Notification delivery failure.
    #[error("notification error: {0}")]
    Notification(String),

    /// Underlying I/O error (file system, pipes, etc.).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let cfg = Config::default();
        assert_eq!(cfg.capture.quality, 85);
        assert!(cfg.behavior.copy_to_clipboard);
        assert!(cfg.behavior.save_to_file);
        assert!(cfg.behavior.show_notification);
        assert_eq!(cfg.hotkey.key, "PrintScreen");
        assert!(cfg.hotkey.modifiers.is_empty());
    }

    #[test]
    fn region_display() {
        let r = Region {
            x: 100,
            y: 200,
            w: 800,
            h: 600,
        };
        assert_eq!(r.to_string(), "800x600+100+200");
    }
}
