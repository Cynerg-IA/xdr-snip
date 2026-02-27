//! Settings dialog — GUI window for editing capture settings.
//!
//! Shows a dialog with:
//! - Save path text field + Browse button
//! - JPEG quality slider (50–100) with live size estimate label
//! - Save / Cancel buttons
//!
//! On Save, writes the updated config to disk and returns the new values
//! so the main loop can hot-reload them.

use std::ptr;

use snip_types::{Config, SnipError};
use tracing::{debug, info, warn};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, GetStockObject, DEFAULT_GUI_FONT, HBRUSH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::Shell::{
    SHBrowseForFolderW, SHGetPathFromIDListW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS,
    BROWSEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetDlgItem,
    GetMessageW, GetSystemMetrics, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
    PostQuitMessage, RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowLongW,
    SetWindowTextW, ShowWindow, TranslateMessage, GWL_STYLE, HMENU, MSG, SM_CXSCREEN,
    SM_CYSCREEN, SW_SHOW, WINDOW_EX_STYLE, WM_CLOSE, WM_COMMAND, WM_CREATE,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_HSCROLL, WM_SETFONT, WNDCLASSW, WS_BORDER,
    WS_CAPTION, WS_CHILD, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

use crate::config;

// ======================== TRACKBAR MESSAGES ========================

// Defined manually — the windows crate doesn't export these under a simple feature.
// Values from the Windows SDK: WM_USER = 0x0400.

/// Trackbar message: get current position.
const TBM_GETPOS: u32 = 0x0400;

/// Trackbar message: set current position. wParam=redraw, lParam=position.
const TBM_SETPOS: u32 = 0x0405;

/// Trackbar message: set range. wParam=redraw, lParam=MAKELPARAM(min, max).
const TBM_SETRANGE: u32 = 0x0406;

// ======================== CONTROL IDS ========================

/// Control ID for the save path edit box.
const ID_PATH_EDIT: i32 = 101;

/// Control ID for the Browse button.
const ID_BROWSE: i32 = 102;

/// Control ID for the quality slider (trackbar).
const ID_QUALITY_SLIDER: i32 = 103;

/// Control ID for the quality + size estimate label (updated live by slider).
const ID_SIZE_LABEL: i32 = 104;

/// Control ID for the Save button.
const ID_SAVE: i32 = 105;

/// Control ID for the Cancel button.
const ID_CANCEL: i32 = 106;

// ======================== LAYOUT CONSTANTS ========================

/// Dialog width in pixels.
const DLG_W: i32 = 480;

/// Dialog height in pixels (includes title bar + borders).
const DLG_H: i32 = 290;

/// Left/right margin for controls.
const MARGIN: i32 = 20;

/// Standard control height (text inputs, labels).
const CTRL_H: i32 = 26;

/// Vertical spacing between sections.
const SECTION_SPACE: i32 = 18;

/// Vertical spacing between label and its control.
const LABEL_GAP: i32 = 4;

/// Minimum JPEG quality (below 50 = visible artifacts on text).
const QUALITY_MIN: i32 = 50;

/// Maximum JPEG quality.
const QUALITY_MAX: i32 = 100;

/// Recommended JPEG quality — best compromise between size and sharpness.
const QUALITY_RECOMMENDED: i32 = 85;

/// Background color for the dialog — matches Windows system dialog (BGR).
const DIALOG_BG: u32 = 0x00F0F0F0;

// ======================== STATIC STATE ========================

// WNDPROC cannot capture closures — store config in mutable statics.
// SAFETY: settings dialog runs on the single main thread.

/// Copy of the config being edited.
static mut EDIT_CONFIG: Option<Config> = None;

/// Whether the user clicked Save (true) or Cancel/closed (false).
static mut SAVE_CLICKED: bool = false;

/// Handle to the settings dialog window.
static mut SETTINGS_HWND: HWND = HWND(ptr::null_mut());

// ======================== PUBLIC API ========================

/// Opens the settings dialog modally, blocking until the user closes it.
///
/// If the user clicks Save, returns `Ok(Some(new_config))` with the updated
/// config already written to disk. Returns `Ok(None)` on Cancel.
pub fn open_settings(current: &Config) -> Result<Option<Config>, SnipError> {
    info!("open_settings: opening settings dialog");

    // Prevent opening multiple settings windows
    unsafe {
        let existing = ptr::addr_of!(SETTINGS_HWND).read();
        if !existing.is_invalid() && existing.0 != ptr::null_mut() {
            debug!("open_settings: dialog already open, focusing");
            let _ = SetForegroundWindow(existing);
            return Ok(None);
        }
    }

    // Store a copy of the config for the dialog to edit
    unsafe {
        EDIT_CONFIG = Some(current.clone());
        SAVE_CLICKED = false;
    }

    let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(|e| SnipError::Config(format!("GetModuleHandleW: {}", e)))?
        .into();

    // Register window class (idempotent)
    let wc = WNDCLASSW {
        lpfnWndProc: Some(settings_wndproc),
        hInstance: hinstance,
        lpszClassName: w!("XdrSnipSettings"),
        hbrBackground: HBRUSH(unsafe { CreateSolidBrush(COLORREF(DIALOG_BG)) }.0),
        ..Default::default()
    };
    let _ = unsafe { RegisterClassW(&wc) };

    // Center on screen
    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let x = (screen_w - DLG_W) / 2;
    let y = (screen_h - DLG_H) / 2;

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("XdrSnipSettings"),
            w!("XDR Snip \u{2014} Settings"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            x,
            y,
            DLG_W,
            DLG_H,
            None,
            None,
            Some(hinstance),
            None,
        )
    }
    .map_err(|e| SnipError::Config(format!("CreateWindowExW settings: {}", e)))?;

    unsafe { SETTINGS_HWND = hwnd };

    let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
    let _ = unsafe { SetForegroundWindow(hwnd) };

    // Run a local message loop (pseudo-modal) until the dialog is closed
    debug!("open_settings: entering local message loop");

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // Read result — use raw pointer to avoid creating a mutable reference
    let (saved, edited) = unsafe {
        let s = ptr::addr_of!(SAVE_CLICKED).read();
        let cfg = ptr::addr_of_mut!(EDIT_CONFIG).replace(None);
        (s, cfg)
    };

    if saved {
        if let Some(new_cfg) = edited {
            // Persist to disk
            if let Err(e) = config::save_config(&new_cfg) {
                warn!("open_settings: failed to save config: {}", e);
                return Err(e);
            }
            info!("open_settings: config saved, returning new config");
            return Ok(Some(new_cfg));
        }
    }

    info!("open_settings: cancelled");
    Ok(None)
}

// ======================== WINDOW PROCEDURE ========================

/// WNDPROC for the settings dialog.
///
/// Creates child controls on WM_CREATE, handles button clicks and slider
/// changes, reads values on Save.
///
/// # Safety
/// Called by Windows — must follow the WNDPROC contract.
unsafe extern "system" fn settings_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            debug!("settings_wndproc: WM_CREATE — building controls");
            create_controls(hwnd);
            LRESULT(0)
        }

        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;

            match id {
                ID_BROWSE => {
                    debug!("settings_wndproc: Browse clicked");
                    if let Some(path) = browse_for_folder(hwnd) {
                        if let Ok(edit) = GetDlgItem(Some(hwnd), ID_PATH_EDIT) {
                            set_text(edit, &path);
                        }
                    }
                }
                ID_SAVE => {
                    debug!("settings_wndproc: Save clicked");

                    // Read values from controls
                    let path_text = GetDlgItem(Some(hwnd), ID_PATH_EDIT)
                        .map(|h| get_text(h))
                        .unwrap_or_default();

                    let quality = GetDlgItem(Some(hwnd), ID_QUALITY_SLIDER)
                        .map(|h| {
                            SendMessageW(h, TBM_GETPOS, None, None).0 as u32
                        })
                        .unwrap_or(85);

                    debug!(
                        "settings_wndproc: save_dir='{}', quality={}",
                        path_text, quality
                    );

                    // Update the stored config
                    if let Some(ref mut cfg) = *ptr::addr_of_mut!(EDIT_CONFIG) {
                        cfg.capture.save_dir = path_text;
                        cfg.capture.quality =
                            quality.clamp(QUALITY_MIN as u32, QUALITY_MAX as u32);
                    }

                    SAVE_CLICKED = true;
                    let _ = DestroyWindow(hwnd);
                }
                ID_CANCEL => {
                    debug!("settings_wndproc: Cancel clicked");
                    SAVE_CLICKED = false;
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }

        WM_HSCROLL => {
            // Slider moved — update the quality + size label
            if let (Ok(slider), Ok(label)) = (
                GetDlgItem(Some(hwnd), ID_QUALITY_SLIDER),
                GetDlgItem(Some(hwnd), ID_SIZE_LABEL),
            ) {
                let pos = SendMessageW(slider, TBM_GETPOS, None, None).0 as i32;
                set_text(label, &quality_label(pos));
            }
            LRESULT(0)
        }

        WM_CTLCOLORSTATIC => {
            // Make static labels have the dialog background color
            let bg = CreateSolidBrush(COLORREF(DIALOG_BG));
            windows::Win32::Graphics::Gdi::SetBkColor(
                windows::Win32::Graphics::Gdi::HDC(wparam.0 as _),
                COLORREF(DIALOG_BG),
            );
            LRESULT(bg.0 as isize)
        }

        WM_CLOSE => {
            SAVE_CLICKED = false;
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }

        WM_DESTROY => {
            SETTINGS_HWND = HWND(ptr::null_mut());
            PostQuitMessage(0);
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ======================== CONTROL CREATION ========================

/// Creates all child controls inside the settings dialog.
unsafe fn create_controls(hwnd: HWND) {
    let hinstance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
    let font = GetStockObject(DEFAULT_GUI_FONT);

    // Read current values from the stored config
    let (save_dir, quality) = {
        let cfg = ptr::addr_of!(EDIT_CONFIG).read();
        match cfg {
            Some(ref c) => (c.capture.save_dir.clone(), c.capture.quality),
            None => ("~/Pictures/XDR-Snips".to_string(), 85),
        }
    };

    // Usable width inside margins
    let inner_w = DLG_W - MARGIN * 2;
    let mut y = MARGIN;

    // ─── Section 1: Save folder ───
    let lbl = create_child(
        hwnd, hinstance, w!("STATIC"), "Save folder:",
        MARGIN, y, inner_w, 20, 0,
    );
    send_font(lbl, font);

    y += 20 + LABEL_GAP;

    // Path edit box + Browse button
    let browse_w = 90;
    let edit_w = inner_w - browse_w - 8;
    let path_edit = create_child(
        hwnd, hinstance, w!("EDIT"), &save_dir,
        MARGIN, y, edit_w, CTRL_H, ID_PATH_EDIT,
    );
    send_font(path_edit, font);
    // Add border style to the edit box
    let style = GetWindowLongW(path_edit, GWL_STYLE);
    SetWindowLongW(path_edit, GWL_STYLE, style | WS_BORDER.0 as i32);

    let browse = create_child(
        hwnd, hinstance, w!("BUTTON"), "Browse...",
        MARGIN + edit_w + 8, y, browse_w, CTRL_H, ID_BROWSE,
    );
    send_font(browse, font);

    y += CTRL_H + SECTION_SPACE;

    // ─── Section 2: JPEG quality slider ───
    let ql = create_child(
        hwnd, hinstance, w!("STATIC"),
        &format!("JPEG quality ({}\u{2013}{}):", QUALITY_MIN, QUALITY_MAX),
        MARGIN, y, inner_w, 20, 0,
    );
    send_font(ql, font);

    y += 20 + LABEL_GAP;

    // Slider (trackbar)
    let slider = create_child(
        hwnd, hinstance, w!("msctls_trackbar32"), "",
        MARGIN, y, inner_w, 34, ID_QUALITY_SLIDER,
    );

    // Set slider range: LPARAM = MAKELPARAM(min, max)
    let range_lparam = ((QUALITY_MAX as u32) << 16 | QUALITY_MIN as u32) as isize;
    SendMessageW(slider, TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM(range_lparam)));
    SendMessageW(slider, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(quality as isize)));

    y += 34 + 2;

    // Size estimate label (updated live by slider movement)
    let size_lbl = create_child(
        hwnd, hinstance, w!("STATIC"),
        &quality_label(quality as i32),
        MARGIN, y, inner_w, 20, ID_SIZE_LABEL,
    );
    send_font(size_lbl, font);

    y += 20 + SECTION_SPACE + 4;

    // ─── Bottom: Save + Cancel buttons (right-aligned) ───
    let btn_w = 100;
    let btn_h = 30;

    let cancel_btn = create_child(
        hwnd, hinstance, w!("BUTTON"), "Cancel",
        DLG_W - MARGIN - btn_w, y, btn_w, btn_h, ID_CANCEL,
    );
    send_font(cancel_btn, font);

    let save_btn = create_child(
        hwnd, hinstance, w!("BUTTON"), "Save",
        DLG_W - MARGIN - btn_w * 2 - 10, y, btn_w, btn_h, ID_SAVE,
    );
    send_font(save_btn, font);

    // Focus the path edit by default
    let _ = SetFocus(Some(path_edit));

    debug!(
        "create_controls: save_dir='{}', quality={}", save_dir, quality
    );
}

// ======================== HELPERS ========================

/// Creates a child window (control) with the given class, text, and position.
unsafe fn create_child(
    parent: HWND,
    hinstance: HINSTANCE,
    class: windows::core::PCWSTR,
    text: &str,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: i32,
) -> HWND {
    let text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        class,
        windows::core::PCWSTR(text_wide.as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        x, y, w, h,
        Some(parent),
        Some(HMENU(id as *mut _)),
        Some(hinstance),
        None,
    )
    .unwrap_or(HWND(ptr::null_mut()))
}

/// Sends WM_SETFONT to a control to apply the system default GUI font.
unsafe fn send_font(ctrl: HWND, font: windows::Win32::Graphics::Gdi::HGDIOBJ) {
    SendMessageW(
        ctrl,
        WM_SETFONT,
        Some(WPARAM(font.0 as usize)),
        Some(LPARAM(1)),
    );
}

/// Gets text from a window (control).
unsafe fn get_text(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len + 1) as usize];
    let copied = GetWindowTextW(hwnd, &mut buf);
    String::from_utf16_lossy(&buf[..copied as usize])
}

/// Sets text on a window (control).
unsafe fn set_text(hwnd: HWND, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let _ = SetWindowTextW(hwnd, windows::core::PCWSTR(wide.as_ptr()));
}

/// Returns a descriptive label for the given quality value.
///
/// Shows the quality percentage, estimated 1080p file size, and marks the
/// recommended value. Examples:
/// - `"85% · ~400 KB · Recommended"`
/// - `"70% · ~250 KB"`
fn quality_label(q: i32) -> String {
    // Estimated JPEG size for a typical 1920x1080 screenshot (KB).
    // Based on empirical measurements of mixed-content screen captures.
    let size_kb = match q {
        50 => 120,
        51..=55 => 140,
        56..=60 => 170,
        61..=65 => 200,
        66..=70 => 250,
        71..=75 => 300,
        76..=80 => 380,
        81..=85 => 450,
        86..=90 => 600,
        91..=95 => 900,
        96..=100 => 2000,
        _ => 450,
    };

    let size_str = if size_kb >= 1000 {
        format!("~{:.1} MB", size_kb as f64 / 1000.0)
    } else {
        format!("~{} KB", size_kb)
    };

    if q == QUALITY_RECOMMENDED {
        format!("{}% \u{00B7} {} \u{00B7} Recommended", q, size_str)
    } else {
        format!("{}% \u{00B7} {}", q, size_str)
    }
}

/// Opens a Windows folder picker dialog and returns the selected path.
unsafe fn browse_for_folder(parent: HWND) -> Option<String> {
    debug!("browse_for_folder: opening folder picker");

    let title = w!("Select screenshot save folder");

    let mut bi: BROWSEINFOW = std::mem::zeroed();
    bi.hwndOwner = parent;
    bi.lpszTitle = title;
    bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;

    let pidl = SHBrowseForFolderW(&bi);
    if pidl.is_null() {
        debug!("browse_for_folder: user cancelled");
        return None;
    }

    let mut path_buf = [0u16; 260]; // MAX_PATH
    let ok = SHGetPathFromIDListW(pidl, &mut path_buf);

    // Free the PIDL
    windows::Win32::System::Com::CoTaskMemFree(Some(pidl as *const _));

    if ok.as_bool() {
        let len = path_buf.iter().position(|&c| c == 0).unwrap_or(path_buf.len());
        let path = String::from_utf16_lossy(&path_buf[..len]);
        debug!("browse_for_folder: selected '{}'", path);
        Some(path)
    } else {
        warn!("browse_for_folder: SHGetPathFromIDListW failed");
        None
    }
}
