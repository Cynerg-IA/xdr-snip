//! Brief floating thumbnail preview shown after a screenshot is captured.
//!
//! Displays a scaled-down version of the captured JPEG in a small always-on-top
//! popup in the bottom-right corner. Auto-closes after a few seconds or on click.
//! Does not steal focus from the active application.

use std::mem;
use std::path::Path;
use std::ptr;

use snip_types::SnipError;
use tracing::{debug, info};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, FillRect, GetDC, GetMonitorInfoW, MonitorFromPoint, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, MONITORINFO,
    MONITOR_DEFAULTTOPRIMARY, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, KillTimer, RegisterClassW,
    SetTimer, ShowWindow, SW_SHOWNOACTIVATE, WNDCLASSW, WM_DESTROY, WM_ERASEBKGND,
    WM_LBUTTONDOWN, WM_PAINT, WM_TIMER, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

/// Maximum thumbnail width in pixels.
const PREVIEW_MAX_W: u32 = 300;

/// Maximum thumbnail height in pixels.
const PREVIEW_MAX_H: u32 = 300;

/// Duration the preview stays visible (milliseconds).
const PREVIEW_DURATION_MS: u32 = 3000;

/// Margin from the screen edge in pixels.
const PREVIEW_MARGIN: i32 = 16;

/// Timer ID for auto-close.
const TIMER_ID_CLOSE: usize = 1;

/// Border width around the thumbnail in pixels.
const BORDER_PX: i32 = 2;

/// Border color — dark gray (BGR format).
const BORDER_COLOR: u32 = 0x00404040;

// ======================== STATIC STATE ========================

// The Win32 WNDPROC cannot capture closures, so we use mutable statics.
// SAFETY: the preview runs on the single main thread.

/// Handle to the preview popup window (null when no preview is visible).
static mut PREVIEW_HWND: HWND = HWND(ptr::null_mut());

/// Memory DC containing the thumbnail bitmap.
static mut THUMB_DC: HDC = HDC(ptr::null_mut());

/// Thumbnail width in pixels.
static mut THUMB_W: i32 = 0;

/// Thumbnail height in pixels.
static mut THUMB_H: i32 = 0;

// ======================== PUBLIC API ========================

/// Shows a brief thumbnail preview of the captured screenshot.
///
/// The preview appears in the bottom-right corner of the primary monitor's
/// work area and auto-closes after [`PREVIEW_DURATION_MS`] milliseconds.
/// Clicking the preview dismisses it immediately.
///
/// Only one preview is shown at a time — calling this while a preview is
/// visible replaces the old one.
pub fn show_preview(jpeg_path: &Path) -> Result<(), SnipError> {
    info!("show_preview: path={}", jpeg_path.display());

    // Close any existing preview first
    close_preview();

    // Load the JPEG and create a thumbnail (preserving aspect ratio)
    let img = image::open(jpeg_path)
        .map_err(|e| SnipError::Notification(format!("image::open for preview: {}", e)))?;

    let thumb = img.thumbnail(PREVIEW_MAX_W, PREVIEW_MAX_H);
    let rgb = thumb.to_rgb8();
    let (tw, th) = (rgb.width(), rgb.height());

    debug!("show_preview: thumbnail {}x{}", tw, th);

    // Create a memory DC with a DIB section containing the thumbnail pixels
    create_thumb_dc(&rgb, tw, th)?;

    // Calculate window size (thumbnail + border on all sides)
    let win_w = tw as i32 + BORDER_PX * 2;
    let win_h = th as i32 + BORDER_PX * 2;

    // Position: bottom-right of the primary monitor's work area
    let (win_x, win_y) = get_preview_position(win_w, win_h);

    // Register the window class (idempotent per process)
    let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(|e| SnipError::Notification(format!("GetModuleHandleW: {}", e)))?
        .into();

    let wc = WNDCLASSW {
        lpfnWndProc: Some(preview_wndproc),
        hInstance: hinstance,
        lpszClassName: w!("XdrSnipPreview"),
        ..Default::default()
    };
    let _ = unsafe { RegisterClassW(&wc) };

    // Create the popup — no taskbar entry (TOOLWINDOW), always on top
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("XdrSnipPreview"),
            w!(""),
            WS_POPUP,
            win_x,
            win_y,
            win_w,
            win_h,
            None,
            None,
            Some(hinstance),
            None,
        )
    }
    .map_err(|e| SnipError::Notification(format!("CreateWindowExW (preview): {}", e)))?;

    // Show without stealing focus from the active application
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };

    // Set auto-close timer
    unsafe {
        let _ = SetTimer(Some(hwnd), TIMER_ID_CLOSE, PREVIEW_DURATION_MS, None);
        PREVIEW_HWND = hwnd;
    }

    info!(
        "show_preview: window at ({},{}) size {}x{}, auto-close in {}ms",
        win_x, win_y, win_w, win_h, PREVIEW_DURATION_MS
    );

    Ok(())
}

/// Closes the preview window if one is currently visible.
///
/// Safe to call even when no preview exists. Called automatically before
/// showing a new preview.
pub fn close_preview() {
    unsafe {
        let hwnd = ptr::addr_of!(PREVIEW_HWND).read();
        if !hwnd.is_invalid() && hwnd.0 != ptr::null_mut() {
            debug!("close_preview: destroying preview window");
            let _ = KillTimer(Some(hwnd), TIMER_ID_CLOSE);
            let _ = DestroyWindow(hwnd);
            // WM_DESTROY handler cleans up THUMB_DC and PREVIEW_HWND
        }
    }
}

// ======================== THUMBNAIL CREATION ========================

/// Creates a memory DC with a DIB section containing the thumbnail pixels.
///
/// Converts RGB pixels from the `image` crate to BGRA for GDI.
fn create_thumb_dc(rgb: &image::RgbImage, tw: u32, th: u32) -> Result<(), SnipError> {
    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err(SnipError::Notification("GetDC(null) failed for preview".into()));
        }

        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        if mem_dc.is_invalid() {
            ReleaseDC(None, screen_dc);
            return Err(SnipError::Notification(
                "CreateCompatibleDC failed for preview".into(),
            ));
        }

        // BITMAPINFOHEADER — top-down DIB (negative biHeight)
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: tw as i32,
                biHeight: -(th as i32), // negative = top-down scanline order
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..mem::zeroed()
            },
            ..mem::zeroed()
        };

        let mut bits_ptr: *mut std::ffi::c_void = ptr::null_mut();
        let dib = CreateDIBSection(
            Some(screen_dc),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            None,
            0,
        )
        .map_err(|e| {
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
            SnipError::Notification(format!("CreateDIBSection: {}", e))
        })?;

        // Convert RGB → BGRA (GDI native pixel format)
        let pixel_count = (tw * th) as usize;
        let dst = std::slice::from_raw_parts_mut(bits_ptr as *mut u8, pixel_count * 4);
        let src = rgb.as_raw();

        for i in 0..pixel_count {
            let si = i * 3;
            let di = i * 4;
            dst[di] = src[si + 2];     // B
            dst[di + 1] = src[si + 1]; // G
            dst[di + 2] = src[si];     // R
            dst[di + 3] = 255;         // A (opaque)
        }

        let _old = SelectObject(mem_dc, dib.into());
        ReleaseDC(None, screen_dc);

        // Store in statics for the WNDPROC to use
        THUMB_DC = mem_dc;
        THUMB_W = tw as i32;
        THUMB_H = th as i32;

        debug!("create_thumb_dc: created {}x{} DIB in memory DC", tw, th);
    }

    Ok(())
}

// ======================== POSITIONING ========================

/// Returns (x, y) for the preview window: bottom-right of the primary
/// monitor's work area, offset by [`PREVIEW_MARGIN`].
fn get_preview_position(win_w: i32, win_h: i32) -> (i32, i32) {
    let mut mi = MONITORINFO {
        cbSize: mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };

    unsafe {
        let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let _ = GetMonitorInfoW(hmon, &mut mi);
    }

    let work = mi.rcWork;
    let x = work.right - win_w - PREVIEW_MARGIN;
    let y = work.bottom - win_h - PREVIEW_MARGIN;

    debug!(
        "get_preview_position: work_area=({},{})→({},{}), pos=({},{})",
        work.left, work.top, work.right, work.bottom, x, y
    );

    (x, y)
}

// ======================== WINDOW PROCEDURE ========================

/// WNDPROC for the preview popup.
///
/// Handles painting the thumbnail with a border, auto-close timer, and
/// click-to-dismiss.
///
/// # Safety
/// Called by Windows — must follow the WNDPROC contract.
unsafe extern "system" fn preview_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            // All painting handled in WM_PAINT — skip erase to avoid flicker
            LRESULT(1)
        }

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let thumb_dc = ptr::addr_of!(THUMB_DC).read();
            let tw = ptr::addr_of!(THUMB_W).read();
            let th = ptr::addr_of!(THUMB_H).read();

            // Fill entire client area with border color (acts as the border)
            let border_brush = CreateSolidBrush(COLORREF(BORDER_COLOR));
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            FillRect(hdc, &client, border_brush);
            let _ = DeleteObject(border_brush.into());

            // Draw the thumbnail inside the border
            if !thumb_dc.is_invalid() {
                let _ = BitBlt(
                    hdc,
                    BORDER_PX,
                    BORDER_PX,
                    tw,
                    th,
                    Some(thumb_dc),
                    0,
                    0,
                    SRCCOPY,
                );
            }

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }

        WM_TIMER => {
            if wparam.0 == TIMER_ID_CLOSE {
                debug!("preview_wndproc: auto-close timer fired");
                let _ = KillTimer(Some(hwnd), TIMER_ID_CLOSE);
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            debug!("preview_wndproc: clicked, dismissing");
            let _ = KillTimer(Some(hwnd), TIMER_ID_CLOSE);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }

        WM_DESTROY => {
            // Clean up the thumbnail memory DC
            let dc = ptr::addr_of!(THUMB_DC).read();
            if !dc.is_invalid() {
                let _ = DeleteDC(dc);
                THUMB_DC = HDC(ptr::null_mut());
            }
            PREVIEW_HWND = HWND(ptr::null_mut());
            // Do NOT call PostQuitMessage — this is not the main app window
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
