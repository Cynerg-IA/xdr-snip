//! Screenshot capture via Windows.Graphics.Capture API (pure Rust).
//!
//! Uses WinRT Graphics Capture to grab frames in R16G16B16A16Float (HDR),
//! tone maps via Extended Reinhard, and encodes to JPEG — all in-process.
//! No subprocess, no C# dependency, single exe.

use std::path::Path;
use std::time::Instant;

use half::f16;
use rayon::prelude::*;
use snip_types::{Region, SnipError};
use tracing::{debug, info};

use windows::core::Interface;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::Imaging::SoftwareBitmap;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HMONITOR, MONITORINFO,
};
use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

// ======================== CONSTANTS ========================

/// Maximum wait time for a capture frame (milliseconds).
const FRAME_TIMEOUT_MS: u64 = 5000;

/// Rec.709 luminance coefficients for HDR tone mapping.
const LUM_R: f32 = 0.2126;
const LUM_G: f32 = 0.7152;
const LUM_B: f32 = 0.0722;

/// sRGB linear-to-gamma threshold (IEC 61966-2-1).
const SRGB_THRESHOLD: f32 = 0.0031308;

// ======================== PUBLIC API ========================

/// Captures a screen region using Windows.Graphics.Capture, tone maps HDR data,
/// and encodes as JPEG.
///
/// # Arguments
/// * `region` — monitor-relative rectangle to capture.
/// * `monitor` — 0-based monitor index.
/// * `quality` — JPEG quality (1-100).
/// * `output` — destination file path for the JPEG.
pub fn capture_region(
    region: &Region,
    monitor: u32,
    quality: u32,
    output: &Path,
) -> Result<(), SnipError> {
    info!(
        "capture_region: region={}, monitor={}, quality={}, output={}",
        region, monitor, quality, output.display()
    );

    let start = Instant::now();

    // Step 1: Enumerate monitors to find the target HMONITOR
    let monitors = enumerate_monitors();
    debug!("capture_region: found {} monitor(s)", monitors.len());

    if monitor as usize >= monitors.len() {
        return Err(SnipError::CaptureProcess(format!(
            "monitor index {} out of range (have {} monitors)",
            monitor, monitors.len()
        )));
    }

    let target = &monitors[monitor as usize];
    debug!(
        "capture_region: target monitor bounds={}x{} at ({},{})",
        target.width, target.height, target.left, target.top
    );

    // Step 2: Create D3D11 device
    debug!("capture_region: creating D3D11 device");
    let d3d_device = create_d3d11_device()?;

    // Step 3: Create GraphicsCaptureItem from HMONITOR
    debug!("capture_region: creating capture item for monitor");
    let capture_item = create_capture_item(target.hmonitor)?;
    let item_size = capture_item.Size().map_err(|e| {
        SnipError::CaptureProcess(format!("failed to get capture item size: {}", e))
    })?;

    debug!(
        "capture_region: capture item size={}x{}",
        item_size.Width, item_size.Height
    );

    // Step 4: Create frame pool with HDR pixel format
    debug!("capture_region: creating frame pool (R16G16B16A16Float)");
    let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &d3d_device,
        DirectXPixelFormat::R16G16B16A16Float,
        1,
        item_size,
    )
    .map_err(|e| SnipError::CaptureProcess(format!("failed to create frame pool: {}", e)))?;

    // Step 5: Create and start capture session
    let session = frame_pool
        .CreateCaptureSession(&capture_item)
        .map_err(|e| SnipError::CaptureProcess(format!("failed to create session: {}", e)))?;

    // Disable cursor and border for clean screenshots
    let _ = session.SetIsCursorCaptureEnabled(false);
    let _ = session.SetIsBorderRequired(false);

    debug!("capture_region: starting capture session");
    session
        .StartCapture()
        .map_err(|e| SnipError::CaptureProcess(format!("failed to start capture: {}", e)))?;

    // Step 6: Wait for a frame
    let frame_start = Instant::now();
    let mut frame = None;

    while frame_start.elapsed().as_millis() < FRAME_TIMEOUT_MS as u128 {
        if let Ok(f) = frame_pool.TryGetNextFrame() {
            frame = Some(f);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Stop session immediately — we only need one frame
    session.Close().ok();

    let frame = frame.ok_or_else(|| {
        SnipError::CaptureFailed(format!(
            "timed out waiting for frame ({}ms)",
            FRAME_TIMEOUT_MS
        ))
    })?;

    debug!("capture_region: frame captured");

    // Step 7: Convert frame surface to SoftwareBitmap
    let surface = frame
        .Surface()
        .map_err(|e| SnipError::CaptureFailed(format!("failed to get surface: {}", e)))?;

    let bitmap = SoftwareBitmap::CreateCopyFromSurfaceAsync(&surface)
        .map_err(|e| SnipError::CaptureFailed(format!("CreateCopyFromSurfaceAsync failed: {}", e)))?
        .get()
        .map_err(|e| SnipError::CaptureFailed(format!("async bitmap copy failed: {}", e)))?;

    // Done with the frame
    frame.Close().ok();
    frame_pool.Close().ok();

    let bmp_w = bitmap.PixelWidth().unwrap_or(0) as usize;
    let bmp_h = bitmap.PixelHeight().unwrap_or(0) as usize;
    let pixel_format = bitmap.BitmapPixelFormat().ok();

    debug!(
        "capture_region: bitmap {}x{}, format={:?}",
        bmp_w, bmp_h, pixel_format
    );

    // Step 8: Read pixel data from bitmap
    let raw_pixels = read_bitmap_pixels(&bitmap, bmp_w, bmp_h)?;

    let is_hdr = matches!(
        pixel_format,
        Some(windows::Graphics::Imaging::BitmapPixelFormat::Rgba16)
            | Some(windows::Graphics::Imaging::BitmapPixelFormat::Gray16)
    );

    debug!(
        "capture_region: read {} bytes, is_hdr={}",
        raw_pixels.len(),
        is_hdr
    );

    // Step 9: Crop to region
    let crop_x = region.x as usize;
    let crop_y = region.y as usize;
    let crop_w = region.w as usize;
    let crop_h = region.h as usize;

    let bgra_pixels = if is_hdr {
        // HDR path: crop from half-float data, tone map to BGRA8
        let bytes_per_pixel = 8usize; // R16G16B16A16Float = 8 bytes/pixel
        let src_stride = bmp_w * bytes_per_pixel;

        let cropped_hdr = crop_pixel_data(&raw_pixels, src_stride, bytes_per_pixel, crop_x, crop_y, crop_w, crop_h);
        debug!("capture_region: tone mapping {}x{} HDR region", crop_w, crop_h);
        tone_map_hdr(&cropped_hdr, crop_w, crop_h)
    } else {
        // SDR path: already BGRA8 (4 bytes/pixel)
        let bytes_per_pixel = 4usize;
        let src_stride = bmp_w * bytes_per_pixel;

        crop_pixel_data(&raw_pixels, src_stride, bytes_per_pixel, crop_x, crop_y, crop_w, crop_h)
    };

    debug!(
        "capture_region: {} bytes of BGRA8 pixel data for {}x{}",
        bgra_pixels.len(),
        crop_w,
        crop_h
    );

    // Step 10: Convert BGRA to RGB and encode as JPEG
    let rgb_pixels = bgra_to_rgb(&bgra_pixels, crop_w, crop_h);

    debug!("capture_region: encoding JPEG quality={}", quality);
    encode_jpeg(&rgb_pixels, crop_w as u32, crop_h as u32, quality, output)?;

    let file_size = output.metadata().map(|m| m.len()).unwrap_or(0);
    let elapsed = start.elapsed();

    info!(
        "capture_region: complete — {}x{}, {} bytes, {:.1}ms",
        crop_w, crop_h, file_size, elapsed.as_secs_f64() * 1000.0
    );

    Ok(())
}

// ======================== D3D11 DEVICE ========================

/// Creates a WinRT IDirect3DDevice backed by a D3D11 hardware device.
fn create_d3d11_device() -> Result<windows::Graphics::DirectX::Direct3D11::IDirect3DDevice, SnipError> {
    // Create native D3D11 device
    let mut d3d_device = None;

    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            windows::Win32::Foundation::HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d_device),
            None,
            None,
        )
    }
    .map_err(|e| SnipError::CaptureProcess(format!("D3D11CreateDevice failed: {}", e)))?;

    let d3d_device = d3d_device
        .ok_or_else(|| SnipError::CaptureProcess("D3D11CreateDevice returned null".into()))?;

    // QI for IDXGIDevice
    let dxgi_device: windows::Win32::Graphics::Dxgi::IDXGIDevice =
        d3d_device.cast().map_err(|e| {
            SnipError::CaptureProcess(format!("cast to IDXGIDevice failed: {}", e))
        })?;

    // Wrap as WinRT IDirect3DDevice
    let winrt_device = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
        .map_err(|e| {
            SnipError::CaptureProcess(format!(
                "CreateDirect3D11DeviceFromDXGIDevice failed: {}",
                e
            ))
        })?;

    winrt_device.cast().map_err(|e| {
        SnipError::CaptureProcess(format!("cast to IDirect3DDevice failed: {}", e))
    })
}

// ======================== CAPTURE ITEM ========================

/// Creates a GraphicsCaptureItem from an HMONITOR handle via COM interop.
fn create_capture_item(hmonitor: HMONITOR) -> Result<GraphicsCaptureItem, SnipError> {
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(
            |e| {
                SnipError::CaptureProcess(format!(
                    "failed to get IGraphicsCaptureItemInterop: {}",
                    e
                ))
            },
        )?;

    let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(hmonitor) }.map_err(|e| {
        SnipError::CaptureProcess(format!("CreateForMonitor failed: {}", e))
    })?;

    Ok(item)
}

// ======================== MONITOR ENUMERATION ========================

/// Information about a display monitor.
struct MonitorInfo {
    hmonitor: HMONITOR,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

/// Enumerates all display monitors, sorted: primary first, then left-to-right.
fn enumerate_monitors() -> Vec<MonitorInfo> {
    let mut monitors: Vec<MonitorInfo> = Vec::new();
    let monitors_ptr = &mut monitors as *mut Vec<MonitorInfo>;

    unsafe extern "system" fn enum_proc(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<MonitorInfo>);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };

        if GetMonitorInfoW(hmon, &mut info).as_bool() {
            list.push(MonitorInfo {
                hmonitor: hmon,
                left: info.rcMonitor.left,
                top: info.rcMonitor.top,
                width: info.rcMonitor.right - info.rcMonitor.left,
                height: info.rcMonitor.bottom - info.rcMonitor.top,
            });
        }

        BOOL(1)
    }

    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(monitors_ptr as isize),
        );
    }

    // Sort: primary (0,0) first, then left-to-right
    monitors.sort_by(|a, b| {
        let a_primary = a.left == 0 && a.top == 0;
        let b_primary = b.left == 0 && b.top == 0;
        if a_primary != b_primary {
            return if a_primary {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        a.left.cmp(&b.left).then(a.top.cmp(&b.top))
    });

    monitors
}

// ======================== PIXEL DATA ========================

/// Reads raw pixel bytes from a SoftwareBitmap via BitmapBuffer.
fn read_bitmap_pixels(
    bitmap: &SoftwareBitmap,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, SnipError> {
    use windows::Graphics::Imaging::BitmapBufferAccessMode;

    let buffer = bitmap
        .LockBuffer(BitmapBufferAccessMode::Read)
        .map_err(|e| SnipError::CaptureFailed(format!("LockBuffer failed: {}", e)))?;

    let reference = buffer
        .CreateReference()
        .map_err(|e| SnipError::CaptureFailed(format!("CreateReference failed: {}", e)))?;

    // Get the plane description to know the actual stride
    let desc = buffer
        .GetPlaneDescription(0)
        .map_err(|e| SnipError::CaptureFailed(format!("GetPlaneDescription failed: {}", e)))?;

    let actual_stride = desc.Stride as usize;

    // Get raw byte pointer via IMemoryBufferByteAccess
    let byte_access: windows::Win32::System::WinRT::IMemoryBufferByteAccess =
        reference.cast().map_err(|e| {
            SnipError::CaptureFailed(format!("cast to IMemoryBufferByteAccess failed: {}", e))
        })?;

    let mut data_ptr: *mut u8 = std::ptr::null_mut();
    let mut capacity: u32 = 0;

    unsafe {
        byte_access
            .GetBuffer(&mut data_ptr, &mut capacity)
            .map_err(|e| SnipError::CaptureFailed(format!("GetBuffer failed: {}", e)))?;
    }

    if data_ptr.is_null() || capacity == 0 {
        return Err(SnipError::CaptureFailed(
            "GetBuffer returned null or zero capacity".into(),
        ));
    }

    // Determine bytes per pixel from the bitmap format
    let pixel_format = bitmap.BitmapPixelFormat().ok();
    let bpp: usize = match pixel_format {
        Some(windows::Graphics::Imaging::BitmapPixelFormat::Rgba16) => 8,
        Some(windows::Graphics::Imaging::BitmapPixelFormat::Gray16) => 8,
        _ => 4, // Bgra8 or other
    };

    let expected_stride = width * bpp;

    // Copy data row by row (actual_stride may differ from expected_stride)
    let mut pixels = vec![0u8; height * expected_stride];

    for y in 0..height {
        let src_offset = y * actual_stride;
        let dst_offset = y * expected_stride;
        let copy_len = expected_stride.min(actual_stride);

        if src_offset + copy_len <= capacity as usize {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data_ptr.add(src_offset),
                    pixels[dst_offset..].as_mut_ptr(),
                    copy_len,
                );
            }
        }
    }

    debug!(
        "read_bitmap_pixels: read {}x{} ({} bytes, stride={}, bpp={})",
        width, height, pixels.len(), actual_stride, bpp
    );

    Ok(pixels)
}

/// Crops pixel data from a larger buffer given coordinates and bytes-per-pixel.
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

// ======================== TONE MAPPING ========================

/// Tone maps R16G16B16A16Float (HDR) pixel data to BGRA8 (SDR).
///
/// Uses Extended Reinhard (luminance-preserving) with sRGB gamma encoding.
/// Processes scanlines in parallel via rayon.
fn tone_map_hdr(half_pixels: &[u8], width: usize, height: usize) -> Vec<u8> {
    let src_stride = width * 8; // 8 bytes per pixel (4 x f16)
    let dst_stride = width * 4; // 4 bytes per pixel (BGRA8)
    let mut output = vec![0u8; height * dst_stride];

    // Process scanlines in parallel
    output
        .par_chunks_mut(dst_stride)
        .enumerate()
        .for_each(|(y, dst_row)| {
            let src_offset = y * src_stride;
            let src_row = &half_pixels[src_offset..src_offset + src_stride];

            for x in 0..width {
                let px_offset = x * 8;
                let out_offset = x * 4;

                // Decode 4 half-floats (R, G, B, A)
                let r_bits = u16::from_le_bytes([src_row[px_offset], src_row[px_offset + 1]]);
                let g_bits = u16::from_le_bytes([src_row[px_offset + 2], src_row[px_offset + 3]]);
                let b_bits = u16::from_le_bytes([src_row[px_offset + 4], src_row[px_offset + 5]]);
                let a_bits = u16::from_le_bytes([src_row[px_offset + 6], src_row[px_offset + 7]]);

                let r = f16::from_bits(r_bits).to_f32();
                let g = f16::from_bits(g_bits).to_f32();
                let b = f16::from_bits(b_bits).to_f32();
                let a = f16::from_bits(a_bits).to_f32();

                // Rec.709 luminance
                let lum = LUM_R * r + LUM_G * g + LUM_B * b;

                if lum <= 0.0 {
                    // Zero/negative luminance — output black, fully opaque
                    dst_row[out_offset] = 0;     // B
                    dst_row[out_offset + 1] = 0; // G
                    dst_row[out_offset + 2] = 0; // R
                    dst_row[out_offset + 3] = 255; // A
                } else {
                    // Extended Reinhard: L_mapped = L / (1 + L)
                    let lum_mapped = lum / (1.0 + lum);
                    let scale = lum_mapped / lum;

                    let r_mapped = linear_to_srgb(clamp01(r * scale));
                    let g_mapped = linear_to_srgb(clamp01(g * scale));
                    let b_mapped = linear_to_srgb(clamp01(b * scale));

                    // Store as BGRA
                    dst_row[out_offset] = float_to_byte(b_mapped);
                    dst_row[out_offset + 1] = float_to_byte(g_mapped);
                    dst_row[out_offset + 2] = float_to_byte(r_mapped);
                    dst_row[out_offset + 3] = float_to_byte(clamp01(a));
                }
            }
        });

    output
}

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

/// Quantizes a [0, 1] float to u8 with rounding.
#[inline]
fn float_to_byte(v: f32) -> u8 {
    (v * 255.0 + 0.5) as u8
}

// ======================== PIXEL FORMAT CONVERSION ========================

/// Converts BGRA8 pixel data to RGB8 for JPEG encoding.
fn bgra_to_rgb(bgra: &[u8], width: usize, height: usize) -> Vec<u8> {
    let pixel_count = width * height;
    let mut rgb = Vec::with_capacity(pixel_count * 3);

    for i in 0..pixel_count {
        let offset = i * 4;
        rgb.push(bgra[offset + 2]); // R (from BGRA offset 2)
        rgb.push(bgra[offset + 1]); // G (from BGRA offset 1)
        rgb.push(bgra[offset]);     // B (from BGRA offset 0)
    }

    rgb
}

// ======================== JPEG ENCODING ========================

/// Encodes RGB8 pixel data as a JPEG file using the `image` crate.
fn encode_jpeg(
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
