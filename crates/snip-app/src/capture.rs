//! JPEG encoding for captured pixel data.
//!
//! The overlay extracts RGB pixels from the frozen GDI screen snapshot.
//! This module encodes those pixels as a JPEG file on disk.
//!
//! The previous WinRT Graphics Capture pipeline (HDR tone mapping, D3D11,
//! SoftwareBitmap) has been removed — the frozen overlay approach guarantees
//! the output matches what the user saw during selection. The WinRT code is
//! preserved in git history if HDR capture is needed in the future.

use std::path::Path;

use snip_types::SnipError;
use tracing::debug;

// ======================== PUBLIC API ========================

/// Encodes RGB8 pixel data as a JPEG file using the `image` crate.
///
/// Called by `main.rs` with pixels extracted from the frozen overlay snapshot.
///
/// # Arguments
/// * `rgb_pixels` — row-major RGB8 pixel data (3 bytes/pixel).
/// * `width` — image width in pixels.
/// * `height` — image height in pixels.
/// * `quality` — JPEG quality (50-100).
/// * `output` — destination file path for the JPEG.
pub fn encode_jpeg(
    rgb_pixels: &[u8],
    width: u32,
    height: u32,
    quality: u32,
    output: &Path,
) -> Result<(), SnipError> {
    use image::codecs::jpeg::JpegEncoder;
    use std::fs::File;
    use std::io::BufWriter;

    // Ensure output directory exists
    if let Some(dir) = output.parent() {
        if !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|e| {
                SnipError::CaptureFailed(format!("cannot create output dir: {}", e))
            })?;
        }
    }

    let file = File::create(output).map_err(|e| {
        SnipError::CaptureFailed(format!("cannot create output file: {}", e))
    })?;

    let writer = BufWriter::new(file);
    let mut encoder = JpegEncoder::new_with_quality(writer, quality as u8);

    encoder
        .encode(rgb_pixels, width, height, image::ExtendedColorType::Rgb8)
        .map_err(|e| SnipError::CaptureFailed(format!("JPEG encoding failed: {}", e)))?;

    debug!(
        "encode_jpeg: wrote {}x{} JPEG (quality={}) to {}",
        width,
        height,
        quality,
        output.display()
    );

    Ok(())
}
