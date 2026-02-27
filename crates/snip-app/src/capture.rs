//! JPEG encoding and HDR tone mapping for captured pixel data.
//!
//! Handles two pixel sources:
//! - **GDI fallback:** RGB8 pixels from the frozen overlay snapshot.
//! - **WinRT HDR:** `R16G16B16A16Float` or `BGRA8` from `hdr_capture`.
//!
//! HDR data is tone-mapped via Extended Reinhard (luminance-preserving)
//! with sRGB gamma encoding before JPEG encoding.

use std::path::Path;

use half::f16;
use rayon::prelude::*;
use snip_types::{Region, SnipError};
use tracing::{debug, info};

use crate::hdr_capture::HdrFrame;

// ======================== TONE MAPPING CONSTANTS ========================

/// Rec.709 luminance coefficients.
const LUM_R: f32 = 0.2126;
const LUM_G: f32 = 0.7152;
const LUM_B: f32 = 0.0722;

/// sRGB linear-to-gamma threshold (IEC 61966-2-1).
const SRGB_THRESHOLD: f32 = 0.0031308;

// ======================== PUBLIC API ========================

/// Encodes RGB8 pixel data as a JPEG file using the `image` crate.
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
        width, height, quality, output.display()
    );

    Ok(())
}

/// Extracts an RGB8 pixel buffer from an `HdrFrame` for a given region.
///
/// The `vscreen_region` is in virtual-screen coordinates. This function
/// translates it to the frame's monitor-relative coordinates, crops the
/// HDR data, tone-maps if needed, and returns RGB8 pixels ready for JPEG.
pub fn extract_hdr_region(frame: &HdrFrame, vscreen_region: &Region) -> Vec<u8> {
    // Translate virtual-screen coords to frame-relative coords
    let crop_x = (vscreen_region.x - frame.monitor_rect.left).max(0) as usize;
    let crop_y = (vscreen_region.y - frame.monitor_rect.top).max(0) as usize;
    let crop_w = vscreen_region.w as usize;
    let crop_h = vscreen_region.h as usize;

    debug!(
        "extract_hdr_region: crop ({},{}) {}x{} from {}x{} frame (hdr={})",
        crop_x, crop_y, crop_w, crop_h, frame.width, frame.height, frame.is_hdr
    );

    // Bounds check
    let frame_w = frame.width as usize;
    let frame_h = frame.height as usize;
    let safe_w = crop_w.min(frame_w.saturating_sub(crop_x));
    let safe_h = crop_h.min(frame_h.saturating_sub(crop_y));

    if safe_w == 0 || safe_h == 0 {
        info!("extract_hdr_region: region outside frame bounds, returning empty");
        return Vec::new();
    }

    if frame.is_hdr {
        // HDR: crop f16 data → tone map → BGRA8 → RGB8
        let bpp = 8usize; // R16G16B16A16Float
        let src_stride = frame_w * bpp;
        let cropped = crop_pixel_data(&frame.pixels, src_stride, bpp, crop_x, crop_y, safe_w, safe_h);

        debug!("extract_hdr_region: tone mapping {}x{} HDR pixels", safe_w, safe_h);
        let bgra = tone_map_hdr(&cropped, safe_w, safe_h);
        bgra_to_rgb(&bgra, safe_w, safe_h)
    } else {
        // SDR: crop BGRA8 → RGB8
        let bpp = 4usize;
        let src_stride = frame_w * bpp;
        let cropped = crop_pixel_data(&frame.pixels, src_stride, bpp, crop_x, crop_y, safe_w, safe_h);
        bgra_to_rgb(&cropped, safe_w, safe_h)
    }
}

// ======================== TONE MAPPING ========================

/// Tone maps `R16G16B16A16Float` (HDR) pixel data to `BGRA8` (SDR).
///
/// Uses Extended Reinhard (luminance-preserving) with sRGB gamma encoding.
/// Processes scanlines in parallel via rayon for performance.
///
/// SDR content (values in [0,1]) passes through nearly unchanged — the
/// Reinhard curve is close to identity for small inputs.
fn tone_map_hdr(half_pixels: &[u8], width: usize, height: usize) -> Vec<u8> {
    let src_stride = width * 8; // 4 × f16
    let dst_stride = width * 4; // BGRA8
    let mut output = vec![0u8; height * dst_stride];

    output
        .par_chunks_mut(dst_stride)
        .enumerate()
        .for_each(|(y, dst_row)| {
            let src_offset = y * src_stride;
            let src_end = src_offset + src_stride;

            // Guard against short buffers
            if src_end > half_pixels.len() {
                return;
            }
            let src_row = &half_pixels[src_offset..src_end];

            for x in 0..width {
                let px = x * 8;
                let out = x * 4;

                // Decode 4 half-floats: R, G, B, A
                let r = f16::from_bits(u16::from_le_bytes([src_row[px], src_row[px + 1]])).to_f32();
                let g = f16::from_bits(u16::from_le_bytes([src_row[px + 2], src_row[px + 3]])).to_f32();
                let b = f16::from_bits(u16::from_le_bytes([src_row[px + 4], src_row[px + 5]])).to_f32();
                let a = f16::from_bits(u16::from_le_bytes([src_row[px + 6], src_row[px + 7]])).to_f32();

                // Rec.709 luminance
                let lum = LUM_R * r + LUM_G * g + LUM_B * b;

                if lum <= 0.0 {
                    // Zero/negative luminance → black, fully opaque
                    dst_row[out] = 0;       // B
                    dst_row[out + 1] = 0;   // G
                    dst_row[out + 2] = 0;   // R
                    dst_row[out + 3] = 255; // A
                } else {
                    // Extended Reinhard: L_mapped = L / (1 + L)
                    let scale = (lum / (1.0 + lum)) / lum;

                    let r_mapped = linear_to_srgb(clamp01(r * scale));
                    let g_mapped = linear_to_srgb(clamp01(g * scale));
                    let b_mapped = linear_to_srgb(clamp01(b * scale));

                    // Store as BGRA
                    dst_row[out] = float_to_byte(b_mapped);
                    dst_row[out + 1] = float_to_byte(g_mapped);
                    dst_row[out + 2] = float_to_byte(r_mapped);
                    dst_row[out + 3] = float_to_byte(clamp01(a));
                }
            }
        });

    output
}

// ======================== PIXEL FORMAT HELPERS ========================

/// Crops pixel data from a larger buffer to the given rectangle.
fn crop_pixel_data(
    src: &[u8],
    src_stride: usize,
    bpp: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
) -> Vec<u8> {
    let dst_stride = w * bpp;
    let mut out = vec![0u8; h * dst_stride];

    for row in 0..h {
        let src_offset = (y + row) * src_stride + x * bpp;
        let dst_offset = row * dst_stride;

        if src_offset + dst_stride <= src.len() {
            out[dst_offset..dst_offset + dst_stride]
                .copy_from_slice(&src[src_offset..src_offset + dst_stride]);
        }
    }

    out
}

/// Converts BGRA8 pixel data to RGB8 for JPEG encoding.
fn bgra_to_rgb(bgra: &[u8], width: usize, height: usize) -> Vec<u8> {
    let pixel_count = width * height;
    let mut rgb = Vec::with_capacity(pixel_count * 3);

    for i in 0..pixel_count {
        let off = i * 4;
        if off + 2 < bgra.len() {
            rgb.push(bgra[off + 2]); // R
            rgb.push(bgra[off + 1]); // G
            rgb.push(bgra[off]);     // B
        }
    }

    rgb
}

// ======================== MATH HELPERS ========================

/// sRGB transfer function: linear → gamma-encoded (IEC 61966-2-1).
#[inline]
fn linear_to_srgb(linear: f32) -> f32 {
    if linear <= SRGB_THRESHOLD {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// Clamps a float to [0, 1], treating NaN as 0.
#[inline]
fn clamp01(v: f32) -> f32 {
    if v.is_nan() || v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

/// Quantizes a [0, 1] float to `u8` with rounding.
#[inline]
fn float_to_byte(v: f32) -> u8 {
    (v * 255.0 + 0.5) as u8
}

// ======================== TESTS ========================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_passthrough_black() {
        assert_eq!(linear_to_srgb(0.0), 0.0);
    }

    #[test]
    fn srgb_passthrough_white() {
        let result = linear_to_srgb(1.0);
        assert!((result - 1.0).abs() < 0.001);
    }

    #[test]
    fn clamp01_nan_is_zero() {
        assert_eq!(clamp01(f32::NAN), 0.0);
    }

    #[test]
    fn clamp01_negative_is_zero() {
        assert_eq!(clamp01(-1.0), 0.0);
    }

    #[test]
    fn clamp01_above_one_is_one() {
        assert_eq!(clamp01(2.5), 1.0);
    }

    #[test]
    fn float_to_byte_extremes() {
        assert_eq!(float_to_byte(0.0), 0);
        assert_eq!(float_to_byte(1.0), 255);
    }

    #[test]
    fn bgra_to_rgb_basic() {
        // One BGRA pixel: B=10, G=20, R=30, A=255
        let bgra = vec![10, 20, 30, 255];
        let rgb = bgra_to_rgb(&bgra, 1, 1);
        assert_eq!(rgb, vec![30, 20, 10]); // RGB order
    }
}
