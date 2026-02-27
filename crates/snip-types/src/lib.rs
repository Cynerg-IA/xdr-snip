//! # snip-types
//!
//! Shared type definitions, configuration structs, and error types for XDR Snip.
//! This crate is dependency-free from platform APIs so it can be used in tests
//! and tooling without pulling in the Windows crate.

use serde::{Deserialize, Serialize};
use std::fmt;

// ======================== OUTPUT FORMAT ========================

/// Supported output image formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Jpeg,
    Png,
    #[serde(rename = "webp")]
    WebP,
    Avif,
    Tiff,
    Bmp,
    Qoi,
    #[serde(rename = "openexr")]
    OpenExr,
}

impl OutputFormat {
    /// File extension for this format (without leading dot).
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::WebP => "webp",
            Self::Avif => "avif",
            Self::Tiff => "tiff",
            Self::Bmp => "bmp",
            Self::Qoi => "qoi",
            Self::OpenExr => "exr",
        }
    }

    /// Human-readable display name for UI dropdowns.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::WebP => "WebP",
            Self::Avif => "AVIF",
            Self::Tiff => "TIFF",
            Self::Bmp => "BMP",
            Self::Qoi => "QOI",
            Self::OpenExr => "OpenEXR (HDR)",
        }
    }

    /// All supported formats, in display order.
    pub const ALL: &'static [OutputFormat] = &[
        Self::Jpeg,
        Self::Png,
        Self::WebP,
        Self::Avif,
        Self::Tiff,
        Self::Bmp,
        Self::Qoi,
        Self::OpenExr,
    ];

    /// Whether this format preserves HDR data (no tone mapping needed).
    pub fn preserves_hdr(&self) -> bool {
        matches!(self, Self::OpenExr)
    }
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Jpeg
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

// ======================== PER-FORMAT OPTIONS ========================

/// JPEG-specific encoding options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JpegOptions {
    /// Quality level (50-100). Higher = larger, sharper.
    #[serde(default = "default_jpeg_quality")]
    pub quality: u32,

    /// Chroma subsampling mode.
    #[serde(default)]
    pub chroma_subsampling: ChromaSubsampling,
}

impl Default for JpegOptions {
    fn default() -> Self {
        Self {
            quality: default_jpeg_quality(),
            chroma_subsampling: ChromaSubsampling::default(),
        }
    }
}

/// Chroma subsampling modes for JPEG encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChromaSubsampling {
    /// 4:4:4 — no subsampling, best quality, largest files.
    #[serde(rename = "4:4:4")]
    Full,
    /// 4:2:2 — horizontal subsampling (default, good balance).
    #[serde(rename = "4:2:2")]
    Half,
    /// 4:2:0 — horizontal + vertical subsampling, smallest files.
    #[serde(rename = "4:2:0")]
    Quarter,
}

impl Default for ChromaSubsampling {
    fn default() -> Self {
        Self::Half
    }
}

impl ChromaSubsampling {
    /// Human-readable label for the settings UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Full => "4:4:4 \u{2014} best quality",
            Self::Half => "4:2:2 \u{2014} balanced",
            Self::Quarter => "4:2:0 \u{2014} smallest",
        }
    }

    /// All subsampling options in display order.
    pub const ALL: &'static [ChromaSubsampling] = &[Self::Full, Self::Half, Self::Quarter];
}

/// PNG-specific encoding options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PngOptions {
    /// Compression level: 0 = fast, 6 = default, 9 = max.
    #[serde(default = "default_png_compression")]
    pub compression: u8,

    /// Filter strategy applied before compression.
    #[serde(default)]
    pub filter: PngFilter,
}

impl Default for PngOptions {
    fn default() -> Self {
        Self {
            compression: default_png_compression(),
            filter: PngFilter::default(),
        }
    }
}

/// PNG pre-compression filter strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PngFilter {
    Adaptive,
    None,
    Sub,
    Up,
    Average,
    Paeth,
}

impl Default for PngFilter {
    fn default() -> Self {
        Self::Adaptive
    }
}

impl PngFilter {
    /// Human-readable label for the settings UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Adaptive => "Adaptive (auto)",
            Self::None => "None",
            Self::Sub => "Sub",
            Self::Up => "Up",
            Self::Average => "Average",
            Self::Paeth => "Paeth",
        }
    }

    /// All filter options in display order.
    pub const ALL: &'static [PngFilter] = &[
        Self::Adaptive,
        Self::None,
        Self::Sub,
        Self::Up,
        Self::Average,
        Self::Paeth,
    ];
}

/// WebP-specific encoding options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebPOptions {
    /// Lossy vs lossless mode.
    #[serde(default)]
    pub lossless: bool,

    /// Quality (0-100). Only used in lossy mode.
    #[serde(default = "default_webp_quality")]
    pub quality: f32,
}

impl Default for WebPOptions {
    fn default() -> Self {
        Self {
            lossless: false,
            quality: default_webp_quality(),
        }
    }
}

/// AVIF-specific encoding options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvifOptions {
    /// Quality (1-100). Higher = better quality, larger.
    #[serde(default = "default_avif_quality")]
    pub quality: u8,

    /// Speed (4-10). Lower = slower but better compression.
    /// Speeds 1-3 are clamped to 4 (impractical for screenshots).
    #[serde(default = "default_avif_speed")]
    pub speed: u8,
}

impl Default for AvifOptions {
    fn default() -> Self {
        Self {
            quality: default_avif_quality(),
            speed: default_avif_speed(),
        }
    }
}

/// TIFF compression options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TiffCompression {
    None,
    Lzw,
    Deflate,
    Packbits,
}

impl Default for TiffCompression {
    fn default() -> Self {
        Self::Lzw
    }
}

impl TiffCompression {
    /// Human-readable label for the settings UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "None (uncompressed)",
            Self::Lzw => "LZW (default)",
            Self::Deflate => "Deflate/Zip",
            Self::Packbits => "PackBits",
        }
    }

    /// All compression options in display order.
    pub const ALL: &'static [TiffCompression] = &[
        Self::None,
        Self::Lzw,
        Self::Deflate,
        Self::Packbits,
    ];
}

/// TIFF-specific encoding options wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TiffOptions {
    /// Compression algorithm.
    #[serde(default)]
    pub compression: TiffCompression,
}

impl Default for TiffOptions {
    fn default() -> Self {
        Self {
            compression: TiffCompression::default(),
        }
    }
}

/// OpenEXR compression options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExrCompression {
    Uncompressed,
    Rle,
    Zip1,
    Zip16,
    Piz,
    Pxr24,
    B44,
    #[serde(rename = "b44a")]
    B44A,
}

impl Default for ExrCompression {
    fn default() -> Self {
        Self::Zip16
    }
}

impl ExrCompression {
    /// Human-readable label for the settings UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Uncompressed => "Uncompressed",
            Self::Rle => "RLE",
            Self::Zip1 => "ZIP (scanline)",
            Self::Zip16 => "ZIP16 (default)",
            Self::Piz => "PIZ (wavelet)",
            Self::Pxr24 => "PXR24 (lossy 24-bit)",
            Self::B44 => "B44 (lossy fixed-rate)",
            Self::B44A => "B44A (lossy adaptive)",
        }
    }

    /// All compression options in display order.
    pub const ALL: &'static [ExrCompression] = &[
        Self::Uncompressed,
        Self::Rle,
        Self::Zip1,
        Self::Zip16,
        Self::Piz,
        Self::Pxr24,
        Self::B44,
        Self::B44A,
    ];
}

/// OpenEXR-specific encoding options wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExrOptions {
    /// Compression algorithm.
    #[serde(default)]
    pub compression: ExrCompression,
}

impl Default for ExrOptions {
    fn default() -> Self {
        Self {
            compression: ExrCompression::default(),
        }
    }
}

/// All format-specific options, bundled together.
/// Every format's options are always present (with defaults) even when a
/// different format is selected — this preserves user choices when switching
/// formats in the settings dialog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatOptions {
    #[serde(default)]
    pub jpeg: JpegOptions,
    #[serde(default)]
    pub png: PngOptions,
    #[serde(default)]
    pub webp: WebPOptions,
    #[serde(default)]
    pub avif: AvifOptions,
    #[serde(default)]
    pub tiff: TiffOptions,
    #[serde(default)]
    pub exr: ExrOptions,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            jpeg: JpegOptions::default(),
            png: PngOptions::default(),
            webp: WebPOptions::default(),
            avif: AvifOptions::default(),
            tiff: TiffOptions::default(),
            exr: ExrOptions::default(),
        }
    }
}

// ======================== HDR PIXEL DATA ========================

/// Raw HDR pixel data for formats that preserve HDR (OpenEXR).
/// Carries the `R16G16B16A16Float` bytes before tone mapping.
pub struct HdrPixelData {
    /// Raw pixel bytes: 8 bytes per pixel (4 × f16: R, G, B, A).
    pub pixels: Vec<u8>,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
}

// ======================== CONFIGURATION ========================

/// Top-level application configuration loaded from `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Settings related to screenshot capture (format, quality, output path, naming).
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

/// Capture-related settings: output format, encoding options, directory, naming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    /// Output image format.
    #[serde(default)]
    pub format: OutputFormat,

    /// Per-format encoding options (all formats stored, active one used).
    #[serde(default)]
    pub format_options: FormatOptions,

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
            format: OutputFormat::default(),
            format_options: FormatOptions::default(),
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
fn default_jpeg_quality() -> u32 {
    85
}

/// Default PNG compression: 6 (the libpng default).
fn default_png_compression() -> u8 {
    6
}

/// Default WebP lossy quality.
fn default_webp_quality() -> f32 {
    80.0
}

/// Default AVIF quality.
fn default_avif_quality() -> u8 {
    80
}

/// Default AVIF speed (1=slowest/best, 10=fastest/worst).
fn default_avif_speed() -> u8 {
    4
}

/// Default save directory — `~/Pictures/XDR-Snips` for clean separation from
/// other screenshot tools.
fn default_save_dir() -> String {
    "~/Pictures/XDR-Snips".to_string()
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

/// Central error type for all XDR Snip operations.
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

// ======================== TESTS ========================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let cfg = Config::default();
        assert_eq!(cfg.capture.format, OutputFormat::Jpeg);
        assert_eq!(cfg.capture.format_options.jpeg.quality, 85);
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

    #[test]
    fn output_format_extensions() {
        assert_eq!(OutputFormat::Jpeg.extension(), "jpg");
        assert_eq!(OutputFormat::Png.extension(), "png");
        assert_eq!(OutputFormat::WebP.extension(), "webp");
        assert_eq!(OutputFormat::OpenExr.extension(), "exr");
    }

    #[test]
    fn output_format_hdr_preservation() {
        assert!(!OutputFormat::Jpeg.preserves_hdr());
        assert!(!OutputFormat::Png.preserves_hdr());
        assert!(OutputFormat::OpenExr.preserves_hdr());
    }

    #[test]
    fn format_options_roundtrip_toml() {
        let cfg = Config::default();
        let serialized = toml::to_string(&cfg).expect("serialize");
        let deserialized: Config = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.capture.format, OutputFormat::Jpeg);
        assert_eq!(deserialized.capture.format_options.jpeg.quality, 85);
    }

    #[test]
    fn legacy_config_loads_with_defaults() {
        // A legacy config with bare quality field — new format fields get defaults
        let legacy = r#"
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
"#;
        let cfg: Config = toml::from_str(legacy).expect("should parse legacy config");
        assert_eq!(cfg.capture.format, OutputFormat::Jpeg);
        assert_eq!(cfg.capture.format_options.jpeg.quality, 85);
    }
}
