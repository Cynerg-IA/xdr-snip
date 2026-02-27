//! Clipboard operations — copies captured pixels to the system clipboard as
//! an RGBA image so it can be pasted into any application.
//!
//! Works directly with raw RGB8 pixels — no format-specific decoding needed.

use std::time::Instant;

use arboard::Clipboard;
use snip_types::SnipError;
use tracing::{debug, error, info};

/// Copies raw RGB8 pixels to the system clipboard as an image.
///
/// Converts RGB8 → RGBA8 (fully opaque) and places the pixel data on the
/// clipboard via the `arboard` crate. This is format-agnostic — works for
/// any output format since we use the raw capture pixels directly.
///
/// # Arguments
/// * `rgb_pixels` — row-major RGB8 pixel data (3 bytes/pixel).
/// * `width` — image width in pixels.
/// * `height` — image height in pixels.
///
/// # Errors
/// Returns [`SnipError::Clipboard`] if the clipboard cannot be accessed.
pub fn copy_to_clipboard_pixels(
    rgb_pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<(), SnipError> {
    info!(
        "copy_to_clipboard_pixels: copying {}x{} image ({} bytes RGB)",
        width, height, rgb_pixels.len()
    );

    let start = Instant::now();

    // Convert RGB8 → RGBA8 (fully opaque alpha)
    let pixel_count = (width as usize) * (height as usize);
    let expected_rgb_len = pixel_count * 3;

    if rgb_pixels.len() < expected_rgb_len {
        return Err(SnipError::Clipboard(format!(
            "RGB buffer too small: expected {} bytes for {}x{}, got {}",
            expected_rgb_len, width, height, rgb_pixels.len()
        )));
    }

    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for i in 0..pixel_count {
        let off = i * 3;
        rgba.push(rgb_pixels[off]);     // R
        rgba.push(rgb_pixels[off + 1]); // G
        rgba.push(rgb_pixels[off + 2]); // B
        rgba.push(255);                 // A (fully opaque)
    }

    debug!(
        "copy_to_clipboard_pixels: RGB→RGBA conversion took {:.1}ms ({} → {} bytes)",
        start.elapsed().as_secs_f64() * 1000.0,
        rgb_pixels.len(),
        rgba.len()
    );

    // Open the system clipboard and set the image
    let mut clipboard = Clipboard::new().map_err(|e| {
        SnipError::Clipboard(format!("failed to open clipboard: {}", e))
    })?;

    let img_data = arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: rgba.into(),
    };

    clipboard.set_image(img_data).map_err(|e| {
        error!(
            "copy_to_clipboard_pixels: set_image failed: {}",
            e
        );
        SnipError::Clipboard(format!("failed to set clipboard image: {}", e))
    })?;

    info!(
        "copy_to_clipboard_pixels: copied {}x{} image to clipboard in {:.1}ms",
        width,
        height,
        start.elapsed().as_secs_f64() * 1000.0
    );

    Ok(())
}
