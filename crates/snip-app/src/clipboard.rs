//! Clipboard operations — copies a captured JPEG to the system clipboard as
//! an RGBA image so it can be pasted into any application.

use std::path::Path;
use std::time::Instant;

use arboard::Clipboard;
use image::ImageReader;
use snip_types::SnipError;
use tracing::{debug, error, info};

/// Copies a JPEG file to the system clipboard as an image.
///
/// Loads the JPEG from disk, decodes it to RGBA, and places the pixel data
/// on the clipboard via the `arboard` crate.
///
/// # Errors
/// Returns [`SnipError::Clipboard`] if the image cannot be loaded or the
/// clipboard cannot be accessed.
pub fn copy_to_clipboard(jpeg_path: &Path) -> Result<(), SnipError> {
    info!(
        "copy_to_clipboard: loading image from {}",
        jpeg_path.display()
    );

    let start = Instant::now();

    // Load and decode the JPEG
    let img = ImageReader::open(jpeg_path)
        .map_err(|e| {
            SnipError::Clipboard(format!(
                "failed to open {}: {}",
                jpeg_path.display(),
                e
            ))
        })?
        .decode()
        .map_err(|e| {
            SnipError::Clipboard(format!(
                "failed to decode {}: {}",
                jpeg_path.display(),
                e
            ))
        })?;

    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    let pixels = rgba.into_raw();

    debug!(
        "copy_to_clipboard: decoded {}x{} image ({} bytes), took {:.1}ms",
        width,
        height,
        pixels.len(),
        start.elapsed().as_secs_f64() * 1000.0
    );

    // Open the system clipboard and set the image
    let mut clipboard = Clipboard::new().map_err(|e| {
        SnipError::Clipboard(format!("failed to open clipboard: {}", e))
    })?;

    let img_data = arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: pixels.into(),
    };

    clipboard.set_image(img_data).map_err(|e| {
        error!(
            "copy_to_clipboard: set_image failed: {}",
            e
        );
        SnipError::Clipboard(format!("failed to set clipboard image: {}", e))
    })?;

    info!(
        "copy_to_clipboard: copied {}x{} image to clipboard in {:.1}ms",
        width,
        height,
        start.elapsed().as_secs_f64() * 1000.0
    );

    Ok(())
}
