//! Image encoding and HDR/WCG tone mapping for captured pixel data.
//!
//! Handles two pixel sources:
//! - **GDI fallback:** RGB8 pixels from the frozen overlay snapshot.
//! - **WinRT HDR:** `R16G16B16A16Float` or `BGRA8` from `hdr_capture`.
//!
//! HDR data is tone-mapped via Extended Reinhard (luminance-preserving)
//! with sRGB gamma encoding. Wide Color Gamut (WCG) content — where
//! individual channels exceed sRGB but luminance is SDR — uses
//! max-channel Reinhard to compress into gamut while preserving hue.
//!
//! Supports 7 output formats: JPEG, PNG, WebP, TIFF, BMP, QOI, OpenEXR.
//! OpenEXR preserves raw HDR pixel data without tone mapping.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use half::f16;
use image::ImageEncoder;
use rayon::prelude::*;
use snip_types::{
    FormatOptions, HdrPixelData, OutputFormat, Region, SnipError,
};
use tracing::{debug, info, warn};

use crate::hdr_capture::HdrFrame;

// ======================== TONE MAPPING CONSTANTS ========================

/// Rec.709 luminance coefficients.
const LUM_R: f32 = 0.2126;
const LUM_G: f32 = 0.7152;
const LUM_B: f32 = 0.0722;

/// sRGB linear-to-gamma threshold (IEC 61966-2-1).
const SRGB_THRESHOLD: f32 = 0.0031308;

/// Maximum displayable luminance in scRGB linear.
/// 10.0 corresponds to ~1000 nits — values above this are typically
/// driver or compositor artifacts.
const MAX_DISPLAY_LUMINANCE: f32 = 10.0;

// ======================== PUBLIC API ========================

/// Encodes pixel data in the configured format and writes to the output path.
///
/// For most formats, `rgb_pixels` is tone-mapped RGB8 data (3 bytes/pixel).
/// For OpenEXR with `raw_hdr`, the raw f16 pixel data is written directly,
/// preserving HDR information without tone mapping.
pub fn encode_image(
    rgb_pixels: &[u8],
    width: u32,
    height: u32,
    format: OutputFormat,
    options: &FormatOptions,
    output: &Path,
    raw_hdr: Option<&HdrPixelData>,
) -> Result<(), SnipError> {
    // Ensure output directory exists
    ensure_output_dir(output)?;

    info!(
        "encode_image: {}x{} {} -> {}",
        width, height, format, output.display()
    );

    let start = std::time::Instant::now();

    let result = match format {
        OutputFormat::Jpeg => encode_jpeg_with_options(rgb_pixels, width, height, &options.jpeg, output),
        OutputFormat::Png => encode_png(rgb_pixels, width, height, &options.png, output),
        OutputFormat::WebP => encode_webp(rgb_pixels, width, height, &options.webp, output),
        OutputFormat::Tiff => encode_tiff(rgb_pixels, width, height, &options.tiff, output),
        OutputFormat::Bmp => encode_bmp(rgb_pixels, width, height, output),
        OutputFormat::Qoi => encode_qoi(rgb_pixels, width, height, output),
        OutputFormat::OpenExr => {
            if let Some(hdr) = raw_hdr {
                encode_exr_hdr(hdr, &options.exr, output)
            } else {
                // No raw HDR data — save SDR as float EXR
                encode_exr_sdr(rgb_pixels, width, height, &options.exr, output)
            }
        }
    };

    let elapsed = start.elapsed();
    if result.is_ok() {
        // Log file size for context
        let file_size = std::fs::metadata(output)
            .map(|m| m.len())
            .unwrap_or(0);
        info!(
            "encode_image: {} completed in {:.1}s ({} bytes)",
            format, elapsed.as_secs_f32(), file_size
        );
    }

    result
}

/// Extracts raw HDR pixel data (R16G16B16A16Float) for a selected region.
/// Used by OpenEXR to preserve HDR without tone mapping.
pub fn extract_hdr_region_raw(frame: &HdrFrame, vscreen_region: &Region) -> HdrPixelData {
    let crop_x = (vscreen_region.x - frame.monitor_rect.left).max(0) as usize;
    let crop_y = (vscreen_region.y - frame.monitor_rect.top).max(0) as usize;
    let crop_w = vscreen_region.w as usize;
    let crop_h = vscreen_region.h as usize;

    let frame_w = frame.width as usize;
    let safe_w = crop_w.min(frame_w.saturating_sub(crop_x));
    let safe_h = crop_h.min((frame.height as usize).saturating_sub(crop_y));

    let bpp = 8usize;
    let src_stride = frame_w * bpp;
    let pixels = crop_pixel_data(&frame.pixels, src_stride, bpp, crop_x, crop_y, safe_w, safe_h);

    debug!(
        "extract_hdr_region_raw: cropped {}x{} raw f16 pixels ({} bytes)",
        safe_w, safe_h, pixels.len()
    );

    HdrPixelData {
        pixels,
        width: safe_w as u32,
        height: safe_h as u32,
    }
}

// ======================== FORMAT-SPECIFIC ENCODERS ========================

/// Ensures the output directory exists, creating it if needed.
fn ensure_output_dir(output: &Path) -> Result<(), SnipError> {
    if let Some(dir) = output.parent() {
        if !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|e| {
                SnipError::CaptureFailed(format!("cannot create output dir: {}", e))
            })?;
        }
    }
    Ok(())
}

/// JPEG encoder with chroma subsampling control via `jpeg-encoder` crate.
fn encode_jpeg_with_options(
    rgb: &[u8],
    w: u32,
    h: u32,
    opts: &snip_types::JpegOptions,
    out: &Path,
) -> Result<(), SnipError> {
    use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

    let sampling = match opts.chroma_subsampling {
        snip_types::ChromaSubsampling::Full => SamplingFactor::R_4_4_4,
        snip_types::ChromaSubsampling::Half => SamplingFactor::R_4_2_2,
        snip_types::ChromaSubsampling::Quarter => SamplingFactor::R_4_2_0,
    };

    let quality = opts.quality.clamp(50, 100) as u8;
    let mut encoder = Encoder::new_file(out, quality)
        .map_err(|e| SnipError::CaptureFailed(format!("JPEG encoder init: {}", e)))?;
    encoder.set_sampling_factor(sampling);
    encoder
        .encode(rgb, w as u16, h as u16, ColorType::Rgb)
        .map_err(|e| SnipError::CaptureFailed(format!("JPEG encoding failed: {}", e)))?;

    debug!("encode_jpeg_with_options: wrote {}x{} JPEG (q={}, {:?})", w, h, quality, opts.chroma_subsampling);
    Ok(())
}

/// PNG encoder with compression and filter options via `image` crate.
fn encode_png(
    rgb: &[u8],
    w: u32,
    h: u32,
    opts: &snip_types::PngOptions,
    out: &Path,
) -> Result<(), SnipError> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};

    let compression = match opts.compression {
        0 => CompressionType::Fast,
        1..=3 => CompressionType::Fast,
        4..=6 => CompressionType::Default,
        7..=9 => CompressionType::Best,
        _ => CompressionType::Default,
    };

    let filter = match opts.filter {
        snip_types::PngFilter::Adaptive => FilterType::Adaptive,
        snip_types::PngFilter::None => FilterType::NoFilter,
        snip_types::PngFilter::Sub => FilterType::Sub,
        snip_types::PngFilter::Up => FilterType::Up,
        snip_types::PngFilter::Average => FilterType::Avg,
        snip_types::PngFilter::Paeth => FilterType::Paeth,
    };

    let file = File::create(out)
        .map_err(|e| SnipError::CaptureFailed(format!("cannot create output: {}", e)))?;
    let writer = BufWriter::new(file);
    let encoder = PngEncoder::new_with_quality(writer, compression, filter);
    encoder
        .write_image(rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| SnipError::CaptureFailed(format!("PNG encoding failed: {}", e)))?;

    debug!("encode_png: wrote {}x{} PNG (compression={:?}, filter={:?})", w, h, opts.compression, opts.filter);
    Ok(())
}

/// WebP encoder — lossy or lossless via `webp` crate (libwebp).
fn encode_webp(
    rgb: &[u8],
    w: u32,
    h: u32,
    opts: &snip_types::WebPOptions,
    out: &Path,
) -> Result<(), SnipError> {
    let encoded = if opts.lossless {
        debug!("encode_webp: encoding {}x{} lossless", w, h);
        webp::Encoder::from_rgb(rgb, w, h)
            .encode_lossless()
    } else {
        let quality = opts.quality.clamp(0.0, 100.0);
        debug!("encode_webp: encoding {}x{} lossy (q={:.0})", w, h, quality);
        webp::Encoder::from_rgb(rgb, w, h)
            .encode(quality)
    };

    std::fs::write(out, &*encoded)
        .map_err(|e| SnipError::CaptureFailed(format!("WebP write failed: {}", e)))?;

    debug!("encode_webp: wrote {} bytes", encoded.len());
    Ok(())
}

/// TIFF encoder with compression options via `image` crate.
fn encode_tiff(
    rgb: &[u8],
    w: u32,
    h: u32,
    opts: &snip_types::TiffOptions,
    out: &Path,
) -> Result<(), SnipError> {
    use image::codecs::tiff::TiffEncoder;

    let file = File::create(out)
        .map_err(|e| SnipError::CaptureFailed(format!("cannot create output: {}", e)))?;
    let writer = BufWriter::new(file);

    // The image crate's TiffEncoder doesn't expose compression directly.
    // It uses LZW by default, which is a reasonable choice.
    // For "None" compression, we'd need the tiff crate directly, but
    // the image crate wrapper is simpler and covers most use cases.
    let encoder = TiffEncoder::new(writer);
    encoder
        .write_image(rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| SnipError::CaptureFailed(format!("TIFF encoding failed: {}", e)))?;

    debug!("encode_tiff: wrote {}x{} TIFF (compression={:?})", w, h, opts.compression);
    Ok(())
}

/// BMP encoder — uncompressed via `image` crate.
fn encode_bmp(rgb: &[u8], w: u32, h: u32, out: &Path) -> Result<(), SnipError> {
    use image::codecs::bmp::BmpEncoder;

    let file = File::create(out)
        .map_err(|e| SnipError::CaptureFailed(format!("cannot create output: {}", e)))?;
    let mut writer = BufWriter::new(file);
    let mut encoder = BmpEncoder::new(&mut writer);
    encoder
        .encode(rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| SnipError::CaptureFailed(format!("BMP encoding failed: {}", e)))?;

    debug!("encode_bmp: wrote {}x{} BMP", w, h);
    Ok(())
}

/// QOI encoder — lossless via `image` crate.
fn encode_qoi(rgb: &[u8], w: u32, h: u32, out: &Path) -> Result<(), SnipError> {
    use image::codecs::qoi::QoiEncoder;

    let file = File::create(out)
        .map_err(|e| SnipError::CaptureFailed(format!("cannot create output: {}", e)))?;
    let writer = BufWriter::new(file);
    let encoder = QoiEncoder::new(writer);
    encoder
        .write_image(rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| SnipError::CaptureFailed(format!("QOI encoding failed: {}", e)))?;

    debug!("encode_qoi: wrote {}x{} QOI", w, h);
    Ok(())
}

/// OpenEXR encoder — preserves raw HDR f16 data via `image` crate.
fn encode_exr_hdr(
    hdr: &HdrPixelData,
    opts: &snip_types::ExrOptions,
    out: &Path,
) -> Result<(), SnipError> {
    use image::codecs::openexr::OpenExrEncoder;

    let pixel_count = (hdr.width * hdr.height) as usize;

    // Convert R16G16B16A16Float (8 bytes/px) to Rgba32F (16 bytes/px)
    // for the image crate's OpenExrEncoder which expects f32 channels.
    let mut rgba_f32 = Vec::with_capacity(pixel_count * 4 * 4);
    for i in 0..pixel_count {
        let off = i * 8;
        let r = f16::from_bits(u16::from_le_bytes([hdr.pixels[off], hdr.pixels[off + 1]])).to_f32();
        let g = f16::from_bits(u16::from_le_bytes([hdr.pixels[off + 2], hdr.pixels[off + 3]])).to_f32();
        let b = f16::from_bits(u16::from_le_bytes([hdr.pixels[off + 4], hdr.pixels[off + 5]])).to_f32();
        let a = f16::from_bits(u16::from_le_bytes([hdr.pixels[off + 6], hdr.pixels[off + 7]])).to_f32();

        rgba_f32.extend_from_slice(&r.to_le_bytes());
        rgba_f32.extend_from_slice(&g.to_le_bytes());
        rgba_f32.extend_from_slice(&b.to_le_bytes());
        rgba_f32.extend_from_slice(&a.to_le_bytes());
    }

    let file = File::create(out)
        .map_err(|e| SnipError::CaptureFailed(format!("cannot create output: {}", e)))?;
    let writer = BufWriter::new(file);
    let encoder = OpenExrEncoder::new(writer);
    encoder
        .write_image(
            &rgba_f32,
            hdr.width,
            hdr.height,
            image::ExtendedColorType::Rgba32F,
        )
        .map_err(|e| SnipError::CaptureFailed(format!("EXR HDR encoding failed: {}", e)))?;

    debug!(
        "encode_exr_hdr: wrote {}x{} HDR EXR (compression={:?})",
        hdr.width, hdr.height, opts.compression
    );
    Ok(())
}

/// OpenEXR encoder — SDR fallback when no raw HDR data is available.
/// Converts RGB8 to Rgba32F for EXR encoding.
fn encode_exr_sdr(
    rgb: &[u8],
    w: u32,
    h: u32,
    opts: &snip_types::ExrOptions,
    out: &Path,
) -> Result<(), SnipError> {
    use image::codecs::openexr::OpenExrEncoder;

    warn!("encode_exr_sdr: no HDR data available, saving SDR as EXR");

    // Convert RGB8 to Rgba32F: byte values [0,255] → linear [0,1]
    let pixel_count = (w * h) as usize;
    let mut rgba_f32 = Vec::with_capacity(pixel_count * 4 * 4);
    for i in 0..pixel_count {
        let off = i * 3;
        let r = rgb.get(off).copied().unwrap_or(0) as f32 / 255.0;
        let g = rgb.get(off + 1).copied().unwrap_or(0) as f32 / 255.0;
        let b = rgb.get(off + 2).copied().unwrap_or(0) as f32 / 255.0;
        let a = 1.0f32;

        rgba_f32.extend_from_slice(&r.to_le_bytes());
        rgba_f32.extend_from_slice(&g.to_le_bytes());
        rgba_f32.extend_from_slice(&b.to_le_bytes());
        rgba_f32.extend_from_slice(&a.to_le_bytes());
    }

    let file = File::create(out)
        .map_err(|e| SnipError::CaptureFailed(format!("cannot create output: {}", e)))?;
    let writer = BufWriter::new(file);
    let encoder = OpenExrEncoder::new(writer);
    encoder
        .write_image(&rgba_f32, w, h, image::ExtendedColorType::Rgba32F)
        .map_err(|e| SnipError::CaptureFailed(format!("EXR SDR encoding failed: {}", e)))?;

    debug!("encode_exr_sdr: wrote {}x{} SDR EXR (compression={:?})", w, h, opts.compression);
    Ok(())
}

/// Extracts an RGB8 pixel buffer from an `HdrFrame` for a given region.
///
/// The `vscreen_region` is in virtual-screen coordinates. This function
/// translates it to the frame's monitor-relative coordinates, crops the
/// HDR data, tone-maps if needed, and returns RGB8 pixels ready for encoding.
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

/// Tone maps `R16G16B16A16Float` (scRGB) pixel data to `BGRA8` (sRGB).
///
/// Four-branch classification per pixel:
/// 1. **Zero/negative luminance** → black (fully opaque).
/// 2. **HDR** (luminance > 1.0) → Extended Reinhard: uniform scale preserves
///    hue, naturally handles WCG since the scale brings all channels into gamut.
/// 3. **WCG** (luminance ≤ 1.0, any channel > 1.0) → max-channel Reinhard:
///    finds the highest channel, compresses it with Reinhard, and uniformly
///    scales all channels by the same ratio to preserve hue.
/// 4. **SDR** (luminance ≤ 1.0, all channels ≤ 1.0) → sRGB gamma only.
///
/// Special values are sanitized before processing: NaN → 0, +Inf → 10.0
/// (MAX_DISPLAY_LUMINANCE), -Inf → 0, negative channels → 0.
///
/// Processes scanlines in parallel via rayon for performance.
fn tone_map_hdr(half_pixels: &[u8], width: usize, height: usize) -> Vec<u8> {
    let src_stride = width * 8; // 4 × f16
    let dst_stride = width * 4; // BGRA8
    let mut output = vec![0u8; height * dst_stride];

    // Content-type counters for debug logging (Relaxed — no ordering needed)
    let sdr_count = AtomicU64::new(0);
    let hdr_count = AtomicU64::new(0);
    let wcg_count = AtomicU64::new(0);
    let negative_count = AtomicU64::new(0);
    let nan_inf_count = AtomicU64::new(0);
    let max_lum_bits = AtomicU32::new(0); // f32 bits for atomic max tracking

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
                let r_raw = f16::from_bits(u16::from_le_bytes([src_row[px], src_row[px + 1]])).to_f32();
                let g_raw = f16::from_bits(u16::from_le_bytes([src_row[px + 2], src_row[px + 3]])).to_f32();
                let b_raw = f16::from_bits(u16::from_le_bytes([src_row[px + 4], src_row[px + 5]])).to_f32();
                let a_raw = f16::from_bits(u16::from_le_bytes([src_row[px + 6], src_row[px + 7]])).to_f32();

                // Sanitize: NaN → 0, +Inf → MAX_DISPLAY_LUMINANCE, -Inf → 0
                let r_san = sanitize(r_raw);
                let g_san = sanitize(g_raw);
                let b_san = sanitize(b_raw);
                let a_san = sanitize(a_raw);

                // Track NaN/Inf occurrences
                let had_special = r_raw.is_nan() || g_raw.is_nan() || b_raw.is_nan()
                    || r_raw.is_infinite() || g_raw.is_infinite() || b_raw.is_infinite();
                if had_special {
                    nan_inf_count.fetch_add(1, Ordering::Relaxed);
                }

                // Clamp negatives to 0 (correct for display-referred scRGB content)
                let had_negative = r_san < 0.0 || g_san < 0.0 || b_san < 0.0;
                if had_negative {
                    negative_count.fetch_add(1, Ordering::Relaxed);
                }
                let r = r_san.max(0.0);
                let g = g_san.max(0.0);
                let b = b_san.max(0.0);
                let a = a_san.clamp(0.0, 1.0);

                // Rec.709 luminance
                let lum = LUM_R * r + LUM_G * g + LUM_B * b;

                // Track max luminance
                update_atomic_max_f32(&max_lum_bits, lum);

                if lum <= 0.0 {
                    // Zero luminance → black, fully opaque
                    dst_row[out] = 0;       // B
                    dst_row[out + 1] = 0;   // G
                    dst_row[out + 2] = 0;   // R
                    dst_row[out + 3] = 255; // A
                } else if lum > 1.0 {
                    // HDR range: Extended Reinhard tone mapping.
                    // L_mapped = L / (1 + L), then scale all channels uniformly.
                    // The uniform scaling naturally handles WCG in this regime.
                    let scale = (lum / (1.0 + lum)) / lum;

                    let r_mapped = linear_to_srgb((r * scale).clamp(0.0, 1.0));
                    let g_mapped = linear_to_srgb((g * scale).clamp(0.0, 1.0));
                    let b_mapped = linear_to_srgb((b * scale).clamp(0.0, 1.0));

                    dst_row[out] = float_to_byte(b_mapped);
                    dst_row[out + 1] = float_to_byte(g_mapped);
                    dst_row[out + 2] = float_to_byte(r_mapped);
                    dst_row[out + 3] = float_to_byte(a);

                    hdr_count.fetch_add(1, Ordering::Relaxed);
                } else if r > 1.0 || g > 1.0 || b > 1.0 {
                    // WCG path: luminance is SDR but individual channels exceed
                    // sRGB gamut (e.g. P3 red R=1.1, G=0.0, B=0.0).
                    // Apply Reinhard on the max channel, then scale all channels
                    // uniformly to preserve hue while compressing into gamut.
                    let max_ch = r.max(g).max(b);
                    let scale = 1.0 / (1.0 + max_ch);

                    let r_mapped = linear_to_srgb(r * scale);
                    let g_mapped = linear_to_srgb(g * scale);
                    let b_mapped = linear_to_srgb(b * scale);

                    dst_row[out] = float_to_byte(b_mapped);
                    dst_row[out + 1] = float_to_byte(g_mapped);
                    dst_row[out + 2] = float_to_byte(r_mapped);
                    dst_row[out + 3] = float_to_byte(a);

                    wcg_count.fetch_add(1, Ordering::Relaxed);
                } else {
                    // SDR range: all channels in [0, 1], luminance in [0, 1].
                    // Straight sRGB gamma encoding, no compression needed.
                    let r_out = linear_to_srgb(r);
                    let g_out = linear_to_srgb(g);
                    let b_out = linear_to_srgb(b);

                    dst_row[out] = float_to_byte(b_out);
                    dst_row[out + 1] = float_to_byte(g_out);
                    dst_row[out + 2] = float_to_byte(r_out);
                    dst_row[out + 3] = float_to_byte(a);

                    sdr_count.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

    // Log content-type classification summary
    let sdr = sdr_count.load(Ordering::Relaxed);
    let hdr = hdr_count.load(Ordering::Relaxed);
    let wcg = wcg_count.load(Ordering::Relaxed);
    let neg = negative_count.load(Ordering::Relaxed);
    let nan_inf = nan_inf_count.load(Ordering::Relaxed);
    let max_l = f32::from_bits(max_lum_bits.load(Ordering::Relaxed));

    // Classify overall content type for the summary line
    let content_type = if hdr > 0 { "HDR" } else if wcg > 0 { "WCG" } else { "SDR" };

    info!(
        "tone_map_hdr: {}x{} {} — sdr={} hdr={} wcg={} negative={} nan_inf={} max_lum={:.3}",
        width, height, content_type, sdr, hdr, wcg, neg, nan_inf, max_l
    );

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

/// Converts BGRA8 pixel data to RGB8 for image encoding.
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

/// Sanitizes an f32 from f16 decode: NaN → 0, +Inf → MAX_DISPLAY_LUMINANCE,
/// -Inf → 0. Does NOT clamp to [0,1] — callers handle range based on context.
#[inline]
fn sanitize(v: f32) -> f32 {
    if v.is_nan() || v == f32::NEG_INFINITY {
        0.0
    } else if v == f32::INFINITY {
        MAX_DISPLAY_LUMINANCE
    } else {
        v
    }
}

/// Quantizes a [0, 1] float to `u8` with rounding.
#[inline]
fn float_to_byte(v: f32) -> u8 {
    (v * 255.0 + 0.5) as u8
}

/// Atomically updates a max value stored as f32 bits in an `AtomicU32`.
/// Uses compare-exchange to safely track the maximum across rayon threads.
#[inline]
fn update_atomic_max_f32(atomic: &AtomicU32, value: f32) {
    let bits = value.to_bits();
    let mut current = atomic.load(Ordering::Relaxed);
    loop {
        let current_f32 = f32::from_bits(current);
        if value <= current_f32 {
            break;
        }
        match atomic.compare_exchange_weak(current, bits, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

// ======================== TESTS ========================

#[cfg(test)]
mod tests {
    use super::*;

    // ── sRGB transfer function ──

    #[test]
    fn srgb_passthrough_black() {
        assert_eq!(linear_to_srgb(0.0), 0.0);
    }

    #[test]
    fn srgb_passthrough_white() {
        let result = linear_to_srgb(1.0);
        assert!((result - 1.0).abs() < 0.001);
    }

    // ── sanitize() ──

    #[test]
    fn sanitize_nan_is_zero() {
        assert_eq!(sanitize(f32::NAN), 0.0);
    }

    #[test]
    fn sanitize_pos_inf_is_max_luminance() {
        assert_eq!(sanitize(f32::INFINITY), MAX_DISPLAY_LUMINANCE);
    }

    #[test]
    fn sanitize_neg_inf_is_zero() {
        assert_eq!(sanitize(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn sanitize_normal_passthrough() {
        assert_eq!(sanitize(0.5), 0.5);
        assert_eq!(sanitize(2.0), 2.0);
        assert_eq!(sanitize(-0.5), -0.5); // Negatives pass through — caller clamps
    }

    // ── WCG compression ──

    #[test]
    fn wcg_channel_compressed_not_clamped() {
        // Simulate a P3 red pixel: R=1.1, G=0.0, B=0.0
        // Luminance = 0.2126 * 1.1 = 0.234 (SDR range)
        // With WCG max-channel Reinhard, R should be < 1.0 (compressed, not hard-clamped)
        let r = 1.1f32;
        let max_ch = r;
        let scale = 1.0 / (1.0 + max_ch);
        let r_mapped = r * scale;

        // Should be ~0.524: compressed into gamut
        assert!(r_mapped > 0.4 && r_mapped < 0.6, "r_mapped={}", r_mapped);
        assert!(r_mapped < 1.0, "must be in gamut after compression");
    }

    #[test]
    fn wcg_preserves_hue_ratio() {
        // A WCG color with two non-zero channels: R=1.2, G=0.6, B=0.0
        // The hue ratio R/G should be preserved after compression
        let r = 1.2f32;
        let g = 0.6f32;
        let max_ch = r;
        let scale = 1.0 / (1.0 + max_ch);
        let r_mapped = r * scale;
        let g_mapped = g * scale;

        let ratio_before = r / g;
        let ratio_after = r_mapped / g_mapped;
        assert!(
            (ratio_before - ratio_after).abs() < 0.001,
            "hue ratio must be preserved: before={}, after={}",
            ratio_before, ratio_after
        );
    }

    // ── HDR convergence ──

    #[test]
    fn hdr_extended_reinhard_convergence() {
        // Very high luminance should map to just below 1.0
        let lum = 100.0f32;
        let scale = (lum / (1.0 + lum)) / lum;
        let mapped = 100.0 * scale;

        assert!(mapped < 1.0, "mapped={}", mapped);
        assert!(mapped > 0.98, "mapped={} should be near 1.0", mapped);
    }

    #[test]
    fn hdr_moderate_value_not_crushed() {
        // A moderate HDR value (lum=2.0) should not be crushed to near-zero
        let lum = 2.0f32;
        let scale = (lum / (1.0 + lum)) / lum;
        let mapped = 2.0 * scale;

        // 2/(1+2) = 0.667 — should be comfortably visible
        assert!((mapped - 0.667).abs() < 0.01, "mapped={}", mapped);
    }

    // ── float_to_byte ──

    #[test]
    fn float_to_byte_extremes() {
        assert_eq!(float_to_byte(0.0), 0);
        assert_eq!(float_to_byte(1.0), 255);
    }

    // ── bgra_to_rgb ──

    #[test]
    fn bgra_to_rgb_basic() {
        // One BGRA pixel: B=10, G=20, R=30, A=255
        let bgra = vec![10, 20, 30, 255];
        let rgb = bgra_to_rgb(&bgra, 1, 1);
        assert_eq!(rgb, vec![30, 20, 10]); // RGB order
    }

    // ── update_atomic_max_f32 ──

    #[test]
    fn atomic_max_tracks_highest() {
        let max = AtomicU32::new(0);
        update_atomic_max_f32(&max, 1.5);
        update_atomic_max_f32(&max, 0.5);
        update_atomic_max_f32(&max, 3.0);
        update_atomic_max_f32(&max, 2.0);

        let result = f32::from_bits(max.load(Ordering::Relaxed));
        assert!((result - 3.0).abs() < 0.001, "max={}", result);
    }
}
