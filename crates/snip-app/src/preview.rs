//! Combined capture preview and notification popup.
//!
//! Shows a brief floating popup with the captured screenshot thumbnail and
//! capture info (dimensions, file size, clipboard status, filename).
//! Replaces both the old Shell_NotifyIcon notification and the separate
//! thumbnail preview — everything in one clean widget.
//! Auto-closes after a few seconds or on click.

use std::mem;
use std::path::Path;
use std::ptr;

use snip_types::SnipError;
use tracing::{debug, info};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FrameRect,
    GetMonitorInfoW, GetStockObject, MonitorFromPoint, SelectObject, SetBkMode,
    SetTextColor, StretchDIBits, UpdateWindow, BITMAPINFO, BITMAPINFOHEADER, DEFAULT_GUI_FONT,
    DIB_RGB_COLORS, DT_LEFT, DT_NOPREFIX, DT_WORDBREAK, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, KillTimer,
    RegisterClassW, SetTimer, SetWindowPos, ShowWindow, HWND_TOPMOST, SW_SHOWNORMAL,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, WNDCLASSW, WM_DESTROY,
    WM_CONTEXTMENU, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_PAINT, WM_RBUTTONDOWN, WM_TIMER,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

/// Minimum popup width — ensures text is readable.
const PREVIEW_MIN_W: i32 = 280;

/// Duration the preview stays visible (milliseconds).
const PREVIEW_DURATION_MS: u32 = 5000;

/// Margin from the screen edge in pixels.
const PREVIEW_MARGIN: i32 = 16;

/// Timer ID for auto-close.
const TIMER_ID_CLOSE: usize = 1;

/// Border width around the popup in pixels.
const BORDER_PX: i32 = 1;

/// Padding between thumbnail and text / text and edges.
const TEXT_PADDING: i32 = 8;

/// Height reserved for the text area below the thumbnail.
const TEXT_AREA_H: i32 = 72;

/// Background color — dark charcoal (BGR).
const BG_COLOR: u32 = 0x00222222;

/// Border color — medium gray (BGR).
const BORDER_COLOR: u32 = 0x00555555;

/// Text color — white (BGR: 0x00BBGGRR).
const TEXT_COLOR: u32 = 0x00FFFFFF;

// ======================== STATIC STATE ========================

// The Win32 WNDPROC cannot capture closures, so we use mutable statics.
// SAFETY: the preview runs on the single main thread.

/// Handle to the preview popup window (null when no preview is visible).
static mut PREVIEW_HWND: HWND = HWND(ptr::null_mut());

/// Raw BGRA pixel buffer for the thumbnail (heap-allocated via Box::into_raw).
static mut THUMB_DATA: *mut u8 = ptr::null_mut();

/// Length of THUMB_DATA in bytes.
static mut THUMB_DATA_LEN: usize = 0;

/// Thumbnail width in pixels.
static mut THUMB_W: i32 = 0;

/// Thumbnail height in pixels.
static mut THUMB_H: i32 = 0;

/// Wide-string buffer for the info text shown below the thumbnail.
static mut INFO_TEXT: [u16; 512] = [0u16; 512];

/// Length of INFO_TEXT in u16 code units (not including null terminator).
static mut INFO_TEXT_LEN: i32 = 0;

/// Wide-string buffer for the captured file path (used to open on click).
static mut FILE_PATH: [u16; 512] = [0u16; 512];

// ======================== PUBLIC API ========================

/// Shows a combined thumbnail preview + notification popup after capture.
///
/// The popup appears in the bottom-right corner of the active monitor and
/// auto-closes after [`PREVIEW_DURATION_MS`] milliseconds. Clicking it
/// dismisses it immediately. Does not steal focus.
///
/// Displays: thumbnail image, dimensions, file size, clipboard status, filename.
///
/// Only one popup is shown at a time — calling this while one is visible
/// replaces the old one.
///
/// `thumb_rgb` is a pre-generated thumbnail in RGB8 format (3 bytes/pixel).
/// The thumbnail is generated from raw capture pixels in `handle_capture`,
/// so preview works for ALL output formats without format-specific decoders.
pub fn show_preview(
    file_path: &Path,
    thumb_rgb: &[u8],
    thumb_w: u32,
    thumb_h: u32,
    orig_w: u32,
    orig_h: u32,
    copied_to_clipboard: bool,
) -> Result<(), SnipError> {
    info!(
        "show_preview: path={}, thumb={}x{}, orig={}x{}, clipboard={}",
        file_path.display(), thumb_w, thumb_h, orig_w, orig_h, copied_to_clipboard
    );

    // Close any existing preview first
    close_preview();

    let (tw, th) = (thumb_w, thumb_h);

    info!(
        "show_preview: thumbnail {}x{} from {}x{}",
        tw, th, orig_w, orig_h
    );

    // Convert RGB → BGRA and store in static buffer
    prepare_thumb_pixels_raw(thumb_rgb, tw, th);

    // Build info text: dimensions | size | clipboard status \n filename
    let file_size = file_path.metadata().map(|m| m.len()).unwrap_or(0);
    let size_str = format_file_size(file_size);
    let mut text = format!("{} x {} | {}", orig_w, orig_h, size_str);
    if copied_to_clipboard {
        text.push_str(" | Copied");
    }
    text.push_str(&format!("\n{}", file_path.display()));
    store_info_text(&text);
    store_file_path(&file_path.to_string_lossy());

    // Window dimensions: thumbnail + text area + borders
    let win_w = (tw as i32 + BORDER_PX * 2).max(PREVIEW_MIN_W);
    let win_h = th as i32 + TEXT_AREA_H + BORDER_PX * 2;

    // Position: bottom-right of the monitor where the cursor is
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
    .map_err(|e| SnipError::Notification(format!("CreateWindowExW: {}", e)))?;

    // Show without stealing focus, force topmost z-order, paint immediately
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
    // UpdateWindow sends WM_PAINT synchronously — ensures the popup is
    // painted before we return, rather than waiting for the next message loop
    let _ = unsafe { UpdateWindow(hwnd) };

    // Set auto-close timer
    unsafe {
        let _ = SetTimer(Some(hwnd), TIMER_ID_CLOSE, PREVIEW_DURATION_MS, None);
        PREVIEW_HWND = hwnd;
    }

    info!(
        "show_preview: popup at ({},{}) size {}x{}, auto-close in {}ms",
        win_x, win_y, win_w, win_h, PREVIEW_DURATION_MS
    );

    Ok(())
}

/// Closes the preview popup if one is currently visible.
///
/// Safe to call even when no popup exists.
pub fn close_preview() {
    unsafe {
        let hwnd = ptr::addr_of!(PREVIEW_HWND).read();
        if !hwnd.is_invalid() && hwnd.0 != ptr::null_mut() {
            debug!("close_preview: destroying popup");
            let _ = KillTimer(Some(hwnd), TIMER_ID_CLOSE);
            let _ = DestroyWindow(hwnd);
            // WM_DESTROY handler cleans up statics
        }
    }
}

// ======================== PIXEL BUFFER ========================

/// Converts raw RGB8 pixels to BGRA and stores them in the static buffer.
///
/// Accepts a raw byte slice (3 bytes/pixel RGB) instead of an `image::RgbImage`,
/// so preview works without format-specific image decoders.
fn prepare_thumb_pixels_raw(rgb: &[u8], tw: u32, th: u32) {
    let pixel_count = (tw * th) as usize;
    let mut bgra = vec![0u8; pixel_count * 4];

    for i in 0..pixel_count {
        let si = i * 3;
        let di = i * 4;
        if si + 2 < rgb.len() {
            bgra[di] = rgb[si + 2];     // B
            bgra[di + 1] = rgb[si + 1]; // G
            bgra[di + 2] = rgb[si];     // R
            bgra[di + 3] = 255;         // A (opaque)
        }
    }

    unsafe {
        free_thumb_data();
        let boxed = bgra.into_boxed_slice();
        THUMB_DATA_LEN = boxed.len();
        THUMB_DATA = Box::into_raw(boxed) as *mut u8;
        THUMB_W = tw as i32;
        THUMB_H = th as i32;
    }

    debug!("prepare_thumb_pixels_raw: {}x{} → {} bytes BGRA", tw, th, pixel_count * 4);
}

/// Generates a thumbnail from raw RGB8 pixels using nearest-neighbor downsampling.
///
/// Returns `(rgb_bytes, thumb_w, thumb_h)`. The thumbnail preserves aspect ratio
/// and fits within `max_w × max_h`. Works with any capture format since it
/// operates on raw pixels, not decoded files.
pub fn generate_thumbnail(
    rgb: &[u8],
    w: u32,
    h: u32,
    max_w: u32,
    max_h: u32,
) -> (Vec<u8>, u32, u32) {
    // Scale factor to fit within max dimensions while preserving aspect ratio
    let scale_w = max_w as f64 / w as f64;
    let scale_h = max_h as f64 / h as f64;
    let scale = scale_w.min(scale_h).min(1.0); // Never upscale

    let tw = ((w as f64 * scale) as u32).max(1);
    let th = ((h as f64 * scale) as u32).max(1);

    let mut out = vec![0u8; (tw * th * 3) as usize];
    let src_stride = w as usize * 3;

    for ty in 0..th {
        for tx in 0..tw {
            // Map thumbnail pixel back to source pixel (nearest-neighbor)
            let sx = ((tx as f64 / scale) as u32).min(w - 1);
            let sy = ((ty as f64 / scale) as u32).min(h - 1);

            let si = sy as usize * src_stride + sx as usize * 3;
            let di = (ty * tw + tx) as usize * 3;

            if si + 2 < rgb.len() && di + 2 < out.len() {
                out[di] = rgb[si];
                out[di + 1] = rgb[si + 1];
                out[di + 2] = rgb[si + 2];
            }
        }
    }

    debug!("generate_thumbnail: {}x{} → {}x{} (scale={:.3})", w, h, tw, th, scale);
    (out, tw, th)
}

/// Stores a UTF-8 string as a wide string in the static INFO_TEXT buffer.
///
/// Uses raw pointer writes to avoid creating references to mutable statics.
fn store_info_text(text: &str) {
    unsafe {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let buf_ptr = ptr::addr_of_mut!(INFO_TEXT) as *mut u16;
        let copy_len = wide.len().min(511); // 512 - 1, reserve null terminator
        ptr::copy_nonoverlapping(wide.as_ptr(), buf_ptr, copy_len);
        *buf_ptr.add(copy_len) = 0;
        INFO_TEXT_LEN = copy_len as i32;
    }
    debug!("store_info_text: {} chars: '{}'", text.len(), text);
}

/// Stores a file path as a null-terminated wide string in the static FILE_PATH buffer.
fn store_file_path(path: &str) {
    unsafe {
        let wide: Vec<u16> = path.encode_utf16().collect();
        let buf_ptr = ptr::addr_of_mut!(FILE_PATH) as *mut u16;
        let copy_len = wide.len().min(511);
        ptr::copy_nonoverlapping(wide.as_ptr(), buf_ptr, copy_len);
        *buf_ptr.add(copy_len) = 0; // null terminator
    }
    debug!("store_file_path: '{}'", path);
}

/// Frees the heap-allocated thumbnail pixel buffer.
///
/// # Safety
/// Must only be called from the main thread.
unsafe fn free_thumb_data() {
    if !THUMB_DATA.is_null() {
        let _ = Box::from_raw(ptr::slice_from_raw_parts_mut(THUMB_DATA, THUMB_DATA_LEN));
        THUMB_DATA = ptr::null_mut();
        THUMB_DATA_LEN = 0;
    }
}

// ======================== POSITIONING ========================

/// Returns (x, y) for the popup: bottom-right of the monitor where the
/// cursor currently is, offset by [`PREVIEW_MARGIN`].
fn get_preview_position(win_w: i32, win_h: i32) -> (i32, i32) {
    let mut cursor = POINT::default();
    let _ = unsafe { GetCursorPos(&mut cursor) };

    let mut mi = MONITORINFO {
        cbSize: mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };

    unsafe {
        let hmon = MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY);
        let _ = GetMonitorInfoW(hmon, &mut mi);
    }

    let work = mi.rcWork;
    let x = work.right - win_w - PREVIEW_MARGIN;
    let y = work.bottom - win_h - PREVIEW_MARGIN;

    info!(
        "get_preview_position: cursor=({},{}), work=({},{})→({},{}), pos=({},{})",
        cursor.x, cursor.y, work.left, work.top, work.right, work.bottom, x, y
    );

    (x, y)
}

// ======================== HELPERS ========================

/// Formats a byte count into a human-readable string (KB/MB).
fn format_file_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} B", bytes)
    }
}

// ======================== WINDOW PROCEDURE ========================

/// WNDPROC for the capture preview popup.
///
/// Paints the thumbnail image with info text below, handles auto-close
/// timer, and click-to-dismiss.
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

            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);

            // 1. Fill entire background with dark charcoal
            let bg = CreateSolidBrush(COLORREF(BG_COLOR));
            windows::Win32::Graphics::Gdi::FillRect(hdc, &client, bg);
            let _ = DeleteObject(bg.into());

            // 2. Draw border around the popup
            let border = CreateSolidBrush(COLORREF(BORDER_COLOR));
            FrameRect(hdc, &client, border);
            let _ = DeleteObject(border.into());

            // 3. Draw thumbnail using StretchDIBits (raw pixels → DC, no memory DC needed)
            let data = ptr::addr_of!(THUMB_DATA).read();
            let tw = ptr::addr_of!(THUMB_W).read();
            let th = ptr::addr_of!(THUMB_H).read();

            if !data.is_null() && tw > 0 && th > 0 {
                let bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: tw,
                        biHeight: -th, // negative = top-down scanline order
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: 0, // BI_RGB
                        ..mem::zeroed()
                    },
                    ..mem::zeroed()
                };

                // Center the thumbnail horizontally within the popup
                let thumb_x = (client.right - tw) / 2;

                StretchDIBits(
                    hdc,
                    thumb_x.max(BORDER_PX),
                    BORDER_PX,
                    tw,
                    th,
                    0,
                    0,
                    tw,
                    th,
                    Some(data as *const std::ffi::c_void),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
            }

            // 4. Draw info text below the thumbnail
            let text_len = ptr::addr_of!(INFO_TEXT_LEN).read();
            if text_len > 0 {
                let font = GetStockObject(DEFAULT_GUI_FONT);
                let old_font = SelectObject(hdc, font);
                SetTextColor(hdc, COLORREF(TEXT_COLOR));
                SetBkMode(hdc, TRANSPARENT);

                let mut text_rect = RECT {
                    left: BORDER_PX + TEXT_PADDING,
                    top: BORDER_PX + th + TEXT_PADDING,
                    right: client.right - BORDER_PX - TEXT_PADDING,
                    bottom: client.bottom - BORDER_PX,
                };

                let mut text_buf: [u16; 512] = ptr::addr_of!(INFO_TEXT).read();
                DrawTextW(
                    hdc,
                    &mut text_buf[..text_len as usize],
                    &mut text_rect,
                    DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
                );

                SelectObject(hdc, old_font);
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
            debug!("preview_wndproc: clicked, opening file and dismissing");

            // Open the captured file in the default image viewer
            let path_ptr = ptr::addr_of!(FILE_PATH) as *const u16;
            if *path_ptr != 0 {
                ShellExecuteW(
                    None,
                    w!("open"),
                    windows::core::PCWSTR(path_ptr),
                    None,
                    None,
                    SW_SHOWNORMAL,
                );
            }

            let _ = KillTimer(Some(hwnd), TIMER_ID_CLOSE);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }

        WM_RBUTTONDOWN => {
            // Right-click dismisses without opening the file
            debug!("preview_wndproc: right-clicked, dismissing");
            let _ = KillTimer(Some(hwnd), TIMER_ID_CLOSE);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }

        WM_CONTEXTMENU => {
            // Suppress the default context menu — right-click is handled above
            LRESULT(0)
        }

        WM_DESTROY => {
            // Free the thumbnail pixel buffer and clear stored path
            free_thumb_data();
            *(ptr::addr_of_mut!(FILE_PATH) as *mut u16) = 0;
            PREVIEW_HWND = HWND(ptr::null_mut());
            // Do NOT call PostQuitMessage — this is not the main app window
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
