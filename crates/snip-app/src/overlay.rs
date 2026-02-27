//! Fullscreen overlay for region selection.
//!
//! Creates a semi-transparent, topmost popup window spanning the entire virtual
//! screen.  The user draws a rectangle by clicking and dragging; pressing Escape
//! or right-clicking cancels.  Returns the selected [`Region`] plus the monitor
//! index that contains the center of the selection.

use std::mem;

use snip_types::{Region, SnipError};
use tracing::{debug, info, warn};
use windows::core::w;
use windows::Win32::Foundation::{
    BOOL, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DeleteObject, EndPaint, EnumDisplayMonitors,
    FillRect, FrameRect, GetMonitorInfoW, HDC, InvalidateRect, MonitorFromPoint, SelectObject,
    HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, PS_SOLID,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{SetCapture, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, LoadCursorW, PostQuitMessage, RegisterClassW, SetCursor,
    SetLayeredWindowAttributes, SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW,
    CS_VREDRAW, HWND_TOPMOST, IDC_CROSS, LWA_ALPHA, MSG, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOMOVE, SWP_NOSIZE, SW_SHOW,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_PAINT, WM_RBUTTONDOWN, WM_SETCURSOR, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOPMOST, WS_POPUP,
};

// ======================== THREAD-LOCAL STATE ========================

// The Win32 WNDPROC callback cannot capture closures, so we use mutable statics
// to communicate between the message loop and the window procedure.
// SAFETY justification: the overlay runs on a single thread and only one
// overlay exists at a time.

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

/// Overlay background dim opacity (0-255). 128 = ~50%.
const DIM_ALPHA: u8 = 128;

/// Selection rectangle border thickness in pixels.
const SELECTION_BORDER_PX: i32 = 2;

/// Minimum drag size in pixels — prevents accidental single clicks from being
/// treated as a region selection.
const MIN_REGION_SIZE: u32 = 4;

// ======================== PUBLIC API ========================

/// Shows a fullscreen overlay and lets the user select a rectangular region.
///
/// Returns `Ok(Some((region, monitor_index)))` when a region is selected,
/// `Ok(None)` when the user cancels, or `Err` on Win32 failure.
///
/// The returned `Region` coordinates are relative to the monitor's top-left
/// corner.  `monitor_index` identifies which monitor contains the selection
/// center.
pub fn select_region() -> Result<Option<(Region, u32)>, SnipError> {
    info!("select_region: starting overlay");

    // SAFETY: Single-threaded Win32 UI — only one overlay runs at a time.
    unsafe {
        IS_DRAGGING = false;
        OVERLAY_RESULT = None;
        OVERLAY_CANCELLED = false;
    }

    // Read virtual screen geometry
    // SAFETY: GetSystemMetrics is always safe to call.
    let vscreen_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let vscreen_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let vscreen_w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let vscreen_h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };

    // SAFETY: Writing to statics — single-threaded UI context.
    unsafe {
        VSCREEN_X = vscreen_x;
        VSCREEN_Y = vscreen_y;
    }

    debug!(
        "select_region: virtual screen = {}x{} at ({}, {})",
        vscreen_w, vscreen_h, vscreen_x, vscreen_y
    );

    // SAFETY: GetModuleHandleW(None) returns the current exe's HINSTANCE.
    let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(|e| SnipError::Overlay(format!("GetModuleHandleW failed: {}", e)))?
        .into();

    register_overlay_class(hinstance)?;

    let hwnd = create_overlay_window(hinstance, vscreen_x, vscreen_y, vscreen_w, vscreen_h)?;

    // SAFETY: ShowWindow is safe to call with a valid HWND.
    let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };

    // Bring to absolute foreground
    // SAFETY: SetWindowPos with valid HWND and HWND_TOPMOST is safe.
    let _ = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE) };

    // Run the message loop until the overlay is closed
    run_message_loop();

    // SAFETY: Reading statics after message loop exits — single-threaded.
    let (result, cancelled) = unsafe { (OVERLAY_RESULT, OVERLAY_CANCELLED) };

    if cancelled {
        info!("select_region: user cancelled selection");
        return Ok(None);
    }

    match result {
        Some(region) => {
            debug!("select_region: raw virtual-screen region = {}", region);

            // Determine which monitor contains the center of the selection
            let center = POINT {
                x: region.x + (region.w as i32 / 2),
                y: region.y + (region.h as i32 / 2),
            };

            // SAFETY: MonitorFromPoint is safe with any POINT value.
            let hmonitor = unsafe { MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST) };

            let mut mi = MONITORINFO {
                cbSize: mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };

            // SAFETY: GetMonitorInfoW is safe with a valid HMONITOR and
            // properly sized MONITORINFO.
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

            // Convert HMONITOR handle to 0-based index by enumerating all monitors
            let monitor_index = hmonitor_to_index(hmonitor);

            info!(
                "select_region: selected region={}, monitor_index={}",
                monitor_region, monitor_index
            );

            Ok(Some((monitor_region, monitor_index)))
        }
        None => {
            info!("select_region: no region captured (possible edge case)");
            Ok(None)
        }
    }
}

// ======================== WIN32 HELPERS ========================

/// Registers the overlay window class (idempotent per process).
fn register_overlay_class(hinstance: HINSTANCE) -> Result<(), SnipError> {
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(overlay_wndproc),
        hInstance: hinstance,
        lpszClassName: w!("HdrSnipOverlay"),
        hCursor: unsafe { LoadCursorW(None, IDC_CROSS) }
            .map_err(|e| SnipError::Overlay(format!("LoadCursorW failed: {}", e)))?,
        ..Default::default()
    };

    // SAFETY: RegisterClassW is safe with a properly initialized WNDCLASSW.
    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 {
        // May already be registered from a previous invocation — not fatal
        debug!("register_overlay_class: class already registered or registration failed");
    }

    Ok(())
}

/// Creates the fullscreen overlay window with layered + topmost styles.
fn create_overlay_window(
    hinstance: HINSTANCE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Result<HWND, SnipError> {
    let ex_style = WS_EX_LAYERED | WS_EX_TOPMOST;

    // SAFETY: CreateWindowExW is safe with valid class name and HINSTANCE.
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            w!("HdrSnipOverlay"),
            w!("HDR Snip Overlay"),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,               // no parent
            None,  // no menu
            Some(hinstance),
            None,
        )
    }
    .map_err(|e| SnipError::Overlay(format!("CreateWindowExW failed: {}", e)))?;

    // Set overlay transparency
    // SAFETY: SetLayeredWindowAttributes is safe with a valid HWND.
    unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), DIM_ALPHA, LWA_ALPHA) }.map_err(
        |e| SnipError::Overlay(format!("SetLayeredWindowAttributes failed: {}", e)),
    )?;

    debug!(
        "create_overlay_window: created {}x{} at ({}, {})",
        w, h, x, y
    );

    Ok(hwnd)
}

/// Standard Win32 blocking message loop.
fn run_message_loop() {
    // SAFETY: GetMessageW/TranslateMessage/DispatchMessageW form the standard
    // Win32 message pump.  Safe when called from the thread that created the
    // window.
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Window procedure for the overlay.
///
/// Handles mouse input for rectangle drawing, keyboard for cancel, and paint
/// for rendering the selection rectangle.
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
            // Start drag — record origin in virtual-screen coordinates
            let (lx, ly) = lparam_to_point(lparam);

            // SAFETY: Single-threaded UI, only one overlay active.
            DRAG_START = POINT {
                x: lx + VSCREEN_X,
                y: ly + VSCREEN_Y,
            };
            DRAG_CURRENT = DRAG_START;
            IS_DRAGGING = true;

            // Capture mouse so we get WM_MOUSEMOVE even outside the window
            // SAFETY: SetCapture is safe with a valid HWND.
            SetCapture(hwnd);

            let start_x = DRAG_START.x;
            let start_y = DRAG_START.y;
            debug!(
                "overlay_wndproc: LButtonDown at ({}, {})",
                start_x, start_y
            );
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            if IS_DRAGGING {
                let (lx, ly) = lparam_to_point(lparam);
                DRAG_CURRENT = POINT {
                    x: lx + VSCREEN_X,
                    y: ly + VSCREEN_Y,
                };
                // Request repaint to show updated selection rectangle
                let _ = InvalidateRect(Some(hwnd), None, true);
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

                // Compute normalized region (handle drag in any direction)
                let x = DRAG_START.x.min(DRAG_CURRENT.x);
                let y = DRAG_START.y.min(DRAG_CURRENT.y);
                let w = (DRAG_START.x - DRAG_CURRENT.x).unsigned_abs();
                let h = (DRAG_START.y - DRAG_CURRENT.y).unsigned_abs();

                if w >= MIN_REGION_SIZE && h >= MIN_REGION_SIZE {
                    OVERLAY_RESULT = Some(Region { x, y, w, h });
                    debug!(
                        "overlay_wndproc: selection complete = {}x{} at ({}, {})",
                        w, h, x, y
                    );
                } else {
                    debug!(
                        "overlay_wndproc: selection too small ({}x{}), ignoring",
                        w, h
                    );
                    OVERLAY_CANCELLED = true;
                }

                // Close the overlay
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }

        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            if vk == VK_ESCAPE.0 {
                debug!("overlay_wndproc: Escape pressed, cancelling");
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
            // Force crosshair cursor at all times
            if let Ok(cursor) = LoadCursorW(None, IDC_CROSS) {
                SetCursor(Some(cursor));
            }
            LRESULT(1) // non-zero = we handled it
        }

        WM_ERASEBKGND => {
            // We handle painting ourselves
            LRESULT(1)
        }

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            // Fill entire area with dark overlay
            let bg_brush = CreateSolidBrush(COLORREF(0x00404040));
            FillRect(hdc, &ps.rcPaint, bg_brush);
            let _ = DeleteObject(bg_brush.into());

            // Draw selection rectangle if dragging
            if IS_DRAGGING {
                let sel_left = (DRAG_START.x.min(DRAG_CURRENT.x)) - VSCREEN_X;
                let sel_top = (DRAG_START.y.min(DRAG_CURRENT.y)) - VSCREEN_Y;
                let sel_right = (DRAG_START.x.max(DRAG_CURRENT.x)) - VSCREEN_X;
                let sel_bottom = (DRAG_START.y.max(DRAG_CURRENT.y)) - VSCREEN_Y;

                let sel_rect = RECT {
                    left: sel_left,
                    top: sel_top,
                    right: sel_right,
                    bottom: sel_bottom,
                };

                // Cyan border for the selection rectangle (BGR: 0x00FFFF00 = cyan)
                let cyan = COLORREF(0x00FFFF00);
                let pen = CreatePen(PS_SOLID, SELECTION_BORDER_PX, cyan);
                let brush = CreateSolidBrush(cyan);
                let _old_pen = SelectObject(hdc, pen.into());

                // Draw frame around the selection
                FrameRect(hdc, &sel_rect, brush);

                let _ = DeleteObject(pen.into());
                let _ = DeleteObject(brush.into());
            }

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }

        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }

        // Default handler for everything else
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Extracts client-area x/y from LPARAM (low-word = x, high-word = y).
/// These are signed 16-bit values packed into the LPARAM.
fn lparam_to_point(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam.0 & 0xFFFF) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
    (x, y)
}

/// Converts an HMONITOR handle to a 0-based monitor index by enumerating
/// all display monitors and finding the matching handle.
///
/// Falls back to index 0 if the handle is not found.
fn hmonitor_to_index(target: HMONITOR) -> u32 {
    let mut monitors: Vec<HMONITOR> = Vec::new();
    let monitors_ptr = &mut monitors as *mut Vec<HMONITOR>;

    /// Callback for `EnumDisplayMonitors` — pushes each monitor handle into the Vec.
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

    // SAFETY: EnumDisplayMonitors with a valid callback and pointer is safe.
    // The monitors Vec lives for the duration of the call.
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
        "hmonitor_to_index: target={:?}, found {} monitors, index={}",
        target.0, monitors.len(), index
    );

    index
}
