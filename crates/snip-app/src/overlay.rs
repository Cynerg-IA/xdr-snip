//! Fullscreen overlay for region selection with frozen screen.
//!
//! Captures the entire virtual screen via BitBlt before showing the overlay,
//! then displays the frozen image with a dark tint. The selected region is
//! shown at full brightness. Pressing Escape or right-clicking cancels.
//! Returns the selected [`Region`] plus the monitor index.

use std::mem;

use snip_types::{Region, SnipError};
use tracing::{debug, info, warn};
use windows::core::w;
use windows::Win32::Foundation::{
    BOOL, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    AlphaBlend, BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    CreateSolidBrush, DeleteDC, DeleteObject, EndPaint, EnumDisplayMonitors, FrameRect, GetDC,
    GetMonitorInfoW, InvalidateRect, MonitorFromPoint, ReleaseDC, SelectObject, AC_SRC_OVER,
    BLENDFUNCTION, HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT,
    SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{SetCapture, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, LoadCursorW, PostQuitMessage, RegisterClassW, SetCursor,
    SetForegroundWindow, SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    HWND_TOPMOST, IDC_CROSS, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SWP_NOMOVE, SWP_NOSIZE, SW_SHOW, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN, WM_SETCURSOR,
    WNDCLASSW, WS_EX_TOPMOST, WS_POPUP,
};

// ======================== THREAD-LOCAL STATE ========================

// The Win32 WNDPROC callback cannot capture closures, so we use mutable statics.
// SAFETY: the overlay runs on a single thread and only one overlay exists at a time.

/// Drag start point in virtual-screen coordinates.
static mut DRAG_START: POINT = POINT { x: 0, y: 0 };

/// Current mouse position during drag.
static mut DRAG_CURRENT: POINT = POINT { x: 0, y: 0 };

/// Whether the user is currently dragging.
static mut IS_DRAGGING: bool = false;

/// Result set by the wndproc: `Some(region)` on success, `None` on cancel.
static mut OVERLAY_RESULT: Option<Region> = None;

/// Whether the overlay was cancelled (Escape / right-click).
static mut OVERLAY_CANCELLED: bool = false;

/// Virtual screen origin X (can be negative on multi-monitor).
static mut VSCREEN_X: i32 = 0;

/// Virtual screen origin Y.
static mut VSCREEN_Y: i32 = 0;

/// Memory DC holding the original (bright) screen capture.
static mut SCREEN_DC: HDC = HDC(std::ptr::null_mut());

/// Memory DC holding the dimmed screen capture (dark tint overlay).
static mut DIM_DC: HDC = HDC(std::ptr::null_mut());

/// Virtual screen width in pixels.
static mut SCREEN_W: i32 = 0;

/// Virtual screen height in pixels.
static mut SCREEN_H: i32 = 0;

/// Minimum drag size — prevents accidental single clicks.
const MIN_REGION_SIZE: u32 = 4;

/// Alpha for the dark tint outside selection (0-255). 128 = ~50%.
const DIM_ALPHA: u8 = 128;

// ======================== PUBLIC API ========================

/// Shows a fullscreen overlay with a frozen screen snapshot and lets the user
/// select a rectangular region by dragging.
///
/// Returns `Ok(Some((region, monitor_index)))` when a region is selected,
/// `Ok(None)` when the user cancels, or `Err` on Win32 failure.
pub fn select_region() -> Result<Option<(Region, u32)>, SnipError> {
    info!("select_region: starting overlay");

    // Reset state
    unsafe {
        IS_DRAGGING = false;
        OVERLAY_RESULT = None;
        OVERLAY_CANCELLED = false;
    }

    // Read virtual screen geometry
    let vscreen_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let vscreen_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let vscreen_w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let vscreen_h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };

    unsafe {
        VSCREEN_X = vscreen_x;
        VSCREEN_Y = vscreen_y;
        SCREEN_W = vscreen_w;
        SCREEN_H = vscreen_h;
    }

    debug!(
        "select_region: virtual screen = {}x{} at ({}, {})",
        vscreen_w, vscreen_h, vscreen_x, vscreen_y
    );

    // Capture the screen BEFORE showing the overlay window
    capture_screen_snapshot(vscreen_x, vscreen_y, vscreen_w, vscreen_h)?;

    let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(|e| SnipError::Overlay(format!("GetModuleHandleW failed: {}", e)))?
        .into();

    register_overlay_class(hinstance)?;

    let hwnd = create_overlay_window(hinstance, vscreen_x, vscreen_y, vscreen_w, vscreen_h)?;

    let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE,
        )
    };

    // Give the overlay keyboard focus so Escape works.
    // WS_POPUP windows don't automatically receive focus.
    let _ = unsafe { SetForegroundWindow(hwnd) };

    // Run the message loop until the overlay is closed
    run_message_loop();

    // Clean up the captured screen bitmaps
    cleanup_screen_snapshot();

    // Read result
    let (result, cancelled) = unsafe { (OVERLAY_RESULT, OVERLAY_CANCELLED) };

    if cancelled {
        info!("select_region: user cancelled selection");
        return Ok(None);
    }

    match result {
        Some(region) => {
            debug!("select_region: raw virtual-screen region = {}", region);

            let center = POINT {
                x: region.x + (region.w as i32 / 2),
                y: region.y + (region.h as i32 / 2),
            };

            let hmonitor = unsafe { MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST) };

            let mut mi = MONITORINFO {
                cbSize: mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };

            let monitor_ok = unsafe { GetMonitorInfoW(hmonitor, &mut mi) };
            if !monitor_ok.as_bool() {
                warn!("select_region: GetMonitorInfoW failed, using monitor 0");
            }

            // Translate virtual-screen coordinates to monitor-relative
            let monitor_region = Region {
                x: region.x - mi.rcMonitor.left,
                y: region.y - mi.rcMonitor.top,
                w: region.w,
                h: region.h,
            };

            let monitor_index = hmonitor_to_index(hmonitor);

            info!(
                "select_region: selected region={}, monitor_index={}",
                monitor_region, monitor_index
            );

            Ok(Some((monitor_region, monitor_index)))
        }
        None => {
            info!("select_region: no region captured");
            Ok(None)
        }
    }
}

// ======================== SCREEN CAPTURE ========================

/// Captures the entire virtual screen via BitBlt into two memory DCs:
/// one with the original image, one with a dark tint overlay.
fn capture_screen_snapshot(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Result<(), SnipError> {
    debug!("capture_screen_snapshot: capturing {}x{} at ({},{})", w, h, x, y);

    unsafe {
        // Get the screen DC
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err(SnipError::Overlay("GetDC(null) failed".into()));
        }

        // Create memory DC + bitmap for the original screen capture
        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        if mem_dc.is_invalid() {
            ReleaseDC(None, screen_dc);
            return Err(SnipError::Overlay("CreateCompatibleDC failed".into()));
        }

        let bmp = CreateCompatibleBitmap(screen_dc, w, h);
        if bmp.is_invalid() {
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
            return Err(SnipError::Overlay("CreateCompatibleBitmap failed".into()));
        }

        let _old_bmp = SelectObject(mem_dc, bmp.into());

        // BitBlt the entire virtual screen into the memory DC
        let ok = BitBlt(mem_dc, 0, 0, w, h, Some(screen_dc), x, y, SRCCOPY);
        if ok.is_err() {
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
            return Err(SnipError::Overlay("BitBlt screen capture failed".into()));
        }

        // Create the dimmed version: copy original, then alpha-blend black on top
        let dim_dc = CreateCompatibleDC(Some(screen_dc));
        let dim_bmp = CreateCompatibleBitmap(screen_dc, w, h);
        let _old_dim = SelectObject(dim_dc, dim_bmp.into());

        // Copy original into the dim DC
        let _ = BitBlt(dim_dc, 0, 0, w, h, Some(mem_dc), 0, 0, SRCCOPY);

        // Create a small black bitmap for the tint overlay
        let black_dc = CreateCompatibleDC(Some(screen_dc));
        let black_bmp = CreateCompatibleBitmap(screen_dc, 1, 1);
        let _old_black = SelectObject(black_dc, black_bmp.into());

        // Fill the 1x1 bitmap with black (it's already black by default, but be explicit)
        let black_rect = RECT {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        };
        let black_brush = CreateSolidBrush(COLORREF(0));
        windows::Win32::Graphics::Gdi::FillRect(black_dc, &black_rect, black_brush);
        let _ = DeleteObject(black_brush.into());

        // Alpha-blend the black tint over the dim DC
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: DIM_ALPHA,
            AlphaFormat: 0,
        };

        let _ = AlphaBlend(dim_dc, 0, 0, w, h, black_dc, 0, 0, 1, 1, blend);

        // Clean up the black bitmap DC
        let _ = DeleteObject(black_bmp.into());
        let _ = DeleteDC(black_dc);

        // Done with the screen DC
        ReleaseDC(None, screen_dc);

        // Store the memory DCs for the WNDPROC to use
        SCREEN_DC = mem_dc;
        DIM_DC = dim_dc;
    }

    debug!("capture_screen_snapshot: screen captured and dimmed version created");
    Ok(())
}

/// Cleans up the memory DCs and bitmaps from the screen capture.
fn cleanup_screen_snapshot() {
    unsafe {
        let screen = SCREEN_DC;
        if !screen.is_invalid() {
            let _ = DeleteDC(screen);
            SCREEN_DC = HDC(std::ptr::null_mut());
        }
        let dim = DIM_DC;
        if !dim.is_invalid() {
            let _ = DeleteDC(dim);
            DIM_DC = HDC(std::ptr::null_mut());
        }
    }
    debug!("cleanup_screen_snapshot: memory DCs released");
}

// ======================== WIN32 HELPERS ========================

/// Registers the overlay window class (idempotent per process).
fn register_overlay_class(hinstance: HINSTANCE) -> Result<(), SnipError> {
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(overlay_wndproc),
        hInstance: hinstance,
        lpszClassName: w!("XdrSnipOverlay"),
        hCursor: unsafe { LoadCursorW(None, IDC_CROSS) }
            .map_err(|e| SnipError::Overlay(format!("LoadCursorW failed: {}", e)))?,
        ..Default::default()
    };

    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 {
        debug!("register_overlay_class: class already registered or failed");
    }

    Ok(())
}

/// Creates the fullscreen overlay window — opaque, topmost, no layered transparency.
fn create_overlay_window(
    hinstance: HINSTANCE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Result<HWND, SnipError> {
    // No WS_EX_LAYERED — window is opaque, we paint the frozen screen bitmap
    let ex_style = WS_EX_TOPMOST;

    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            w!("XdrSnipOverlay"),
            w!("XDR Snip Overlay"),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(hinstance),
            None,
        )
    }
    .map_err(|e| SnipError::Overlay(format!("CreateWindowExW failed: {}", e)))?;

    debug!("create_overlay_window: created {}x{} at ({}, {})", w, h, x, y);

    Ok(hwnd)
}

/// Standard Win32 blocking message loop.
fn run_message_loop() {
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Window procedure for the frozen-screen overlay.
///
/// Paints the dimmed screen capture as background, shows the selected region
/// at full brightness, and draws a cyan border around the selection.
///
/// # Safety
/// Called by Windows — must follow the WNDPROC contract.
unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            let (lx, ly) = lparam_to_point(lparam);

            DRAG_START = POINT {
                x: lx + VSCREEN_X,
                y: ly + VSCREEN_Y,
            };
            DRAG_CURRENT = DRAG_START;
            IS_DRAGGING = true;

            SetCapture(hwnd);

            let sx = std::ptr::addr_of!(DRAG_START).read().x;
            let sy = std::ptr::addr_of!(DRAG_START).read().y;
            debug!("overlay_wndproc: LButtonDown at ({}, {})", sx, sy);
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            if IS_DRAGGING {
                let (lx, ly) = lparam_to_point(lparam);
                DRAG_CURRENT = POINT {
                    x: lx + VSCREEN_X,
                    y: ly + VSCREEN_Y,
                };
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }

        WM_LBUTTONUP => {
            if IS_DRAGGING {
                IS_DRAGGING = false;

                let (lx, ly) = lparam_to_point(lparam);
                DRAG_CURRENT = POINT {
                    x: lx + VSCREEN_X,
                    y: ly + VSCREEN_Y,
                };

                let x = DRAG_START.x.min(DRAG_CURRENT.x);
                let y = DRAG_START.y.min(DRAG_CURRENT.y);
                let w = (DRAG_START.x - DRAG_CURRENT.x).unsigned_abs();
                let h = (DRAG_START.y - DRAG_CURRENT.y).unsigned_abs();

                if w >= MIN_REGION_SIZE && h >= MIN_REGION_SIZE {
                    OVERLAY_RESULT = Some(Region { x, y, w, h });
                    debug!(
                        "overlay_wndproc: selection = {}x{} at ({}, {})",
                        w, h, x, y
                    );
                } else {
                    debug!(
                        "overlay_wndproc: selection too small ({}x{})",
                        w, h
                    );
                    OVERLAY_CANCELLED = true;
                }

                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }

        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                debug!("overlay_wndproc: Escape pressed");
                IS_DRAGGING = false;
                OVERLAY_CANCELLED = true;
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }

        WM_RBUTTONDOWN => {
            debug!("overlay_wndproc: RButtonDown, cancelling");
            IS_DRAGGING = false;
            OVERLAY_CANCELLED = true;
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }

        WM_SETCURSOR => {
            if let Ok(cursor) = LoadCursorW(None, IDC_CROSS) {
                SetCursor(Some(cursor));
            }
            LRESULT(1)
        }

        WM_ERASEBKGND => {
            // We handle all painting in WM_PAINT
            LRESULT(1)
        }

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let dim = std::ptr::addr_of!(DIM_DC).read();
            let screen = std::ptr::addr_of!(SCREEN_DC).read();
            let sw = std::ptr::addr_of!(SCREEN_W).read();
            let sh = std::ptr::addr_of!(SCREEN_H).read();

            // Double-buffer: paint everything to a back-buffer DC, then
            // single BitBlt to screen. Eliminates flickering from intermediate
            // states (dimmed bg painted before bright selection appears).
            let back_dc = CreateCompatibleDC(Some(hdc));
            let back_bmp = CreateCompatibleBitmap(hdc, sw, sh);
            let old_bmp = SelectObject(back_dc, back_bmp.into());

            // 1. Paint the dimmed (dark) frozen screen as full background
            if !dim.is_invalid() {
                let _ = BitBlt(back_dc, 0, 0, sw, sh, Some(dim), 0, 0, SRCCOPY);
            }

            // 2. If dragging, show the selected area at full brightness
            if IS_DRAGGING && !screen.is_invalid() {
                let sel_left = DRAG_START.x.min(DRAG_CURRENT.x) - VSCREEN_X;
                let sel_top = DRAG_START.y.min(DRAG_CURRENT.y) - VSCREEN_Y;
                let sel_right = DRAG_START.x.max(DRAG_CURRENT.x) - VSCREEN_X;
                let sel_bottom = DRAG_START.y.max(DRAG_CURRENT.y) - VSCREEN_Y;
                let sel_w = sel_right - sel_left;
                let sel_h = sel_bottom - sel_top;

                if sel_w > 0 && sel_h > 0 {
                    // BitBlt the original (bright) image for just the selection area
                    let _ = BitBlt(
                        back_dc,
                        sel_left,
                        sel_top,
                        sel_w,
                        sel_h,
                        Some(screen),
                        sel_left,
                        sel_top,
                        SRCCOPY,
                    );

                    // Draw cyan border around the selection
                    let sel_rect = RECT {
                        left: sel_left,
                        top: sel_top,
                        right: sel_right,
                        bottom: sel_bottom,
                    };

                    let cyan = COLORREF(0x00FFFF00); // BGR: cyan
                    let brush = CreateSolidBrush(cyan);
                    FrameRect(back_dc, &sel_rect, brush);
                    let _ = DeleteObject(brush.into());
                }
            }

            // 3. Single blit from back buffer to screen — flicker-free
            let _ = BitBlt(hdc, 0, 0, sw, sh, Some(back_dc), 0, 0, SRCCOPY);

            // Clean up back buffer
            SelectObject(back_dc, old_bmp);
            let _ = DeleteObject(back_bmp.into());
            let _ = DeleteDC(back_dc);

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }

        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Extracts client-area x/y from LPARAM (low-word = x, high-word = y).
fn lparam_to_point(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam.0 & 0xFFFF) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
    (x, y)
}

/// Converts an HMONITOR handle to a 0-based monitor index.
fn hmonitor_to_index(target: HMONITOR) -> u32 {
    let mut monitors: Vec<HMONITOR> = Vec::new();
    let monitors_ptr = &mut monitors as *mut Vec<HMONITOR>;

    unsafe extern "system" fn enum_proc(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<HMONITOR>);
        list.push(hmon);
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

    let index = monitors
        .iter()
        .position(|&m| m == target)
        .unwrap_or(0) as u32;

    debug!(
        "hmonitor_to_index: target={:?}, {} monitors, index={}",
        target.0, monitors.len(), index
    );

    index
}
