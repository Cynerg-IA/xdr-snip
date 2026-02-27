//! WinRT Graphics Capture — per-monitor HDR frame acquisition.
//!
//! Captures each monitor via `Windows.Graphics.Capture` with
//! `R16G16B16A16Float` pixel format to preserve HDR data. Returns a map
//! of `HMONITOR → HdrFrame` for the caller to crop + tone-map after the
//! user selects a region.
//!
//! If any step fails (permissions, driver, older OS), the public API
//! returns an empty map and the caller falls back to the GDI path.

use std::collections::HashMap;
use std::mem;
use std::time::Instant;

use snip_types::SnipError;
use tracing::{debug, info, warn};
use windows::core::Interface;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::Imaging::{BitmapBufferAccessMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Win32::Foundation::{BOOL, HMODULE, LPARAM, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

// ======================== CONSTANTS ========================

/// Maximum wait time for a single capture frame.
const FRAME_TIMEOUT_MS: u64 = 2000;

// ======================== PUBLIC TYPES ========================

/// Raw pixel data captured from a single monitor via WinRT Graphics Capture.
pub struct HdrFrame {
    /// Raw pixel bytes — `R16G16B16A16Float` (8 bytes/px) when HDR,
    /// `BGRA8` (4 bytes/px) when SDR.
    pub pixels: Vec<u8>,

    /// Frame width in pixels.
    pub width: u32,

    /// Frame height in pixels.
    pub height: u32,

    /// `true` if pixels are `R16G16B16A16Float` (f16 per channel).
    pub is_hdr: bool,

    /// Monitor bounds in virtual-screen coordinates.
    pub monitor_rect: RECT,
}

// ======================== PUBLIC API ========================

/// Captures all monitors via WinRT Graphics Capture.
///
/// Returns a map from raw `HMONITOR` value (as `isize`) to the captured
/// `HdrFrame`. On any critical failure (D3D11 init, etc.) returns an empty
/// map — the caller should fall back to GDI.
pub fn capture_all_monitors() -> HashMap<isize, HdrFrame> {
    info!("capture_all_monitors: starting WinRT capture");
    let start = Instant::now();

    let monitors = enumerate_monitors();
    if monitors.is_empty() {
        warn!("capture_all_monitors: no monitors found");
        return HashMap::new();
    }
    debug!(
        "capture_all_monitors: found {} monitor(s)",
        monitors.len()
    );

    // Create the D3D11 device once — shared across all monitors
    let d3d_device = match create_d3d11_device() {
        Ok(dev) => dev,
        Err(e) => {
            warn!("capture_all_monitors: D3D11 init failed, falling back to GDI: {}", e);
            return HashMap::new();
        }
    };

    let mut frames = HashMap::new();

    for mon in &monitors {
        let hmon_val = mon.hmonitor.0 as isize;
        debug!(
            "capture_all_monitors: capturing monitor at ({},{}) {}x{}",
            mon.left, mon.top, mon.width, mon.height
        );

        match capture_single_monitor(&d3d_device, mon) {
            Ok(frame) => {
                debug!(
                    "capture_all_monitors: got {}x{} frame (hdr={})",
                    frame.width, frame.height, frame.is_hdr
                );
                frames.insert(hmon_val, frame);
            }
            Err(e) => {
                warn!(
                    "capture_all_monitors: failed for monitor at ({},{}): {}",
                    mon.left, mon.top, e
                );
                // Continue — other monitors may succeed
            }
        }
    }

    let elapsed = start.elapsed();
    info!(
        "capture_all_monitors: captured {}/{} monitors in {:.1}ms",
        frames.len(),
        monitors.len(),
        elapsed.as_secs_f64() * 1000.0
    );

    frames
}

// ======================== SINGLE MONITOR CAPTURE ========================

/// Captures a single monitor's frame via WinRT Graphics Capture.
fn capture_single_monitor(
    d3d_device: &windows::Graphics::DirectX::Direct3D11::IDirect3DDevice,
    monitor: &MonitorInfo,
) -> Result<HdrFrame, SnipError> {
    // Create GraphicsCaptureItem from HMONITOR
    let capture_item = create_capture_item(monitor.hmonitor)?;
    let item_size = capture_item.Size().map_err(|e| {
        SnipError::CaptureFailed(format!("failed to get capture item size: {}", e))
    })?;

    debug!(
        "capture_single_monitor: item size={}x{}",
        item_size.Width, item_size.Height
    );

    // Create frame pool with HDR pixel format
    let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        d3d_device,
        DirectXPixelFormat::R16G16B16A16Float,
        1,
        item_size,
    )
    .map_err(|e| SnipError::CaptureFailed(format!("CreateFreeThreaded failed: {}", e)))?;

    // Create and start capture session
    let session = frame_pool
        .CreateCaptureSession(&capture_item)
        .map_err(|e| SnipError::CaptureFailed(format!("CreateCaptureSession failed: {}", e)))?;

    // Clean capture: no cursor, no yellow border
    let _ = session.SetIsCursorCaptureEnabled(false);
    let _ = session.SetIsBorderRequired(false);

    session
        .StartCapture()
        .map_err(|e| SnipError::CaptureFailed(format!("StartCapture failed: {}", e)))?;

    // Poll for the first frame
    let poll_start = Instant::now();
    let mut frame = None;

    while poll_start.elapsed().as_millis() < FRAME_TIMEOUT_MS as u128 {
        if let Ok(f) = frame_pool.TryGetNextFrame() {
            frame = Some(f);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Stop session — we only need one frame
    session.Close().ok();

    let frame = frame.ok_or_else(|| {
        SnipError::CaptureFailed(format!(
            "timed out waiting for frame ({}ms)",
            FRAME_TIMEOUT_MS
        ))
    })?;

    // Convert frame surface to SoftwareBitmap for CPU access
    let surface = frame
        .Surface()
        .map_err(|e| SnipError::CaptureFailed(format!("Surface() failed: {}", e)))?;

    let bitmap = SoftwareBitmap::CreateCopyFromSurfaceAsync(&surface)
        .map_err(|e| SnipError::CaptureFailed(format!("CreateCopyFromSurfaceAsync: {}", e)))?
        .get()
        .map_err(|e| SnipError::CaptureFailed(format!("async bitmap copy failed: {}", e)))?;

    // Release GPU resources promptly
    frame.Close().ok();
    frame_pool.Close().ok();

    let bmp_w = bitmap.PixelWidth().unwrap_or(0) as u32;
    let bmp_h = bitmap.PixelHeight().unwrap_or(0) as u32;
    let pixel_format = bitmap.BitmapPixelFormat().ok();

    debug!(
        "capture_single_monitor: bitmap {}x{}, format={:?}",
        bmp_w, bmp_h, pixel_format
    );

    // Read raw pixels
    let raw_pixels = read_bitmap_pixels(&bitmap, bmp_w as usize, bmp_h as usize)?;

    let is_hdr = matches!(
        pixel_format,
        Some(BitmapPixelFormat::Rgba16) | Some(BitmapPixelFormat::Gray16)
    );

    let rect = RECT {
        left: monitor.left,
        top: monitor.top,
        right: monitor.left + monitor.width,
        bottom: monitor.top + monitor.height,
    };

    Ok(HdrFrame {
        pixels: raw_pixels,
        width: bmp_w,
        height: bmp_h,
        is_hdr,
        monitor_rect: rect,
    })
}

// ======================== D3D11 DEVICE ========================

/// Creates a WinRT `IDirect3DDevice` backed by a D3D11 hardware device.
fn create_d3d11_device(
) -> Result<windows::Graphics::DirectX::Direct3D11::IDirect3DDevice, SnipError> {
    let mut d3d_device = None;

    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d_device),
            None,
            None,
        )
    }
    .map_err(|e| SnipError::CaptureFailed(format!("D3D11CreateDevice failed: {}", e)))?;

    let d3d_device = d3d_device
        .ok_or_else(|| SnipError::CaptureFailed("D3D11CreateDevice returned null".into()))?;

    // QueryInterface for IDXGIDevice
    let dxgi_device: windows::Win32::Graphics::Dxgi::IDXGIDevice =
        d3d_device.cast().map_err(|e| {
            SnipError::CaptureFailed(format!("cast to IDXGIDevice failed: {}", e))
        })?;

    // Wrap the DXGI device as a WinRT IDirect3DDevice
    let winrt_device = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
        .map_err(|e| {
            SnipError::CaptureFailed(format!(
                "CreateDirect3D11DeviceFromDXGIDevice failed: {}",
                e
            ))
        })?;

    winrt_device.cast().map_err(|e| {
        SnipError::CaptureFailed(format!("cast to IDirect3DDevice failed: {}", e))
    })
}

// ======================== CAPTURE ITEM ========================

/// Creates a `GraphicsCaptureItem` from an `HMONITOR` via COM interop.
fn create_capture_item(hmonitor: HMONITOR) -> Result<GraphicsCaptureItem, SnipError> {
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(
            |e| {
                SnipError::CaptureFailed(format!(
                    "IGraphicsCaptureItemInterop factory failed: {}",
                    e
                ))
            },
        )?;

    let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(hmonitor) }.map_err(|e| {
        SnipError::CaptureFailed(format!("CreateForMonitor failed: {}", e))
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
            cbSize: mem::size_of::<MONITORINFO>() as u32,
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

    debug!("enumerate_monitors: found {} monitors", monitors.len());
    monitors
}

// ======================== PIXEL DATA ========================

/// Reads raw pixel bytes from a `SoftwareBitmap` via `IMemoryBufferByteAccess`.
fn read_bitmap_pixels(
    bitmap: &SoftwareBitmap,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, SnipError> {
    let buffer = bitmap
        .LockBuffer(BitmapBufferAccessMode::Read)
        .map_err(|e| SnipError::CaptureFailed(format!("LockBuffer failed: {}", e)))?;

    let reference = buffer
        .CreateReference()
        .map_err(|e| SnipError::CaptureFailed(format!("CreateReference failed: {}", e)))?;

    // Actual stride may differ from width * bpp due to padding
    let desc = buffer
        .GetPlaneDescription(0)
        .map_err(|e| SnipError::CaptureFailed(format!("GetPlaneDescription failed: {}", e)))?;
    let actual_stride = desc.Stride as usize;

    // Raw byte pointer via COM IMemoryBufferByteAccess
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

    // Determine bytes-per-pixel from bitmap format
    let pixel_format = bitmap.BitmapPixelFormat().ok();
    let bpp: usize = match pixel_format {
        Some(BitmapPixelFormat::Rgba16) | Some(BitmapPixelFormat::Gray16) => 8,
        _ => 4, // BGRA8 default
    };

    let expected_stride = width * bpp;
    let mut pixels = vec![0u8; height * expected_stride];

    // Copy row by row, handling stride mismatch
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
        "read_bitmap_pixels: {}x{} ({} bytes, stride={}, bpp={})",
        width,
        height,
        pixels.len(),
        actual_stride,
        bpp
    );

    Ok(pixels)
}
