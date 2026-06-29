//! Settings dialog — non-blocking GUI window for editing capture settings.
//!
//! Shows a dialog with:
//! - Save path text field + Browse button
//! - Output format dropdown (JPEG, PNG, WebP, AVIF, TIFF, BMP, QOI, OpenEXR)
//! - Per-format options in Standard mode (quality/compression only)
//! - Advanced checkbox to reveal extra options (chroma subsampling, filters, etc.)
//! - Save / Cancel buttons
//!
//! The dialog is **non-blocking** — the main event loop continues running while
//! settings is open, so the hotkey still works for screenshots. Call `is_open()`
//! and `take_result()` from the main loop to check for completion.

use std::ptr;

use snip_types::{
    AvifOptions, ChromaSubsampling, Config, ExrCompression, ExrOptions, FormatOptions,
    JpegOptions, OutputFormat, PngFilter, PngOptions, SnipError, TiffCompression, TiffOptions,
    WebPOptions,
};
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
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetDlgItem, GetSystemMetrics,
    GetWindowLongW, GetWindowTextLengthW, GetWindowTextW, MoveWindow, RegisterClassW,
    SendMessageW,
    SetForegroundWindow, SetWindowLongW, SetWindowTextW, ShowWindow, GWL_STYLE, HMENU,
    SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_CTLCOLORSTATIC, WM_DESTROY, WM_HSCROLL, WM_SETFONT, WNDCLASSW, WS_BORDER,
    WS_CAPTION, WS_CHILD, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

use crate::config;

// ======================== TRACKBAR MESSAGES ========================

/// Trackbar message: get current position.
const TBM_GETPOS: u32 = 0x0400;

/// Trackbar message: set current position. wParam=redraw, lParam=position.
const TBM_SETPOS: u32 = 0x0405;

/// Trackbar message: set range. wParam=redraw, lParam=MAKELPARAM(min, max).
const TBM_SETRANGE: u32 = 0x0406;

// ======================== COMBO BOX / BUTTON MESSAGES ========================

/// Combo box message: add a string item.
const CB_ADDSTRING: u32 = 0x0143;

/// Combo box message: set current selection (0-based index).
const CB_SETCURSEL: u32 = 0x014E;

/// Combo box message: get current selection index.
const CB_GETCURSEL: u32 = 0x0147;

/// Combo box notification: selection changed.
const CBN_SELCHANGE: u32 = 1;

/// Button message: get check state.
const BM_GETCHECK: u32 = 0x00F0;

/// Button check state: checked.
const BST_CHECKED: usize = 1;

// ======================== CONTROL IDS ========================

/// Control ID for the save path edit box.
const ID_PATH_EDIT: i32 = 101;

/// Control ID for the Browse button.
const ID_BROWSE: i32 = 102;

/// Control ID for the format combo box.
const ID_FORMAT_COMBO: i32 = 107;

/// Control ID for the Save button.
const ID_SAVE: i32 = 105;

/// Control ID for the Cancel button.
const ID_CANCEL: i32 = 106;

/// Control ID for the Advanced options checkbox.
const ID_ADVANCED_CHECK: i32 = 170;

// --- JPEG option controls (standard) ---
const ID_JPEG_QUALITY_LABEL: i32 = 110;
const ID_JPEG_QUALITY_SLIDER: i32 = 111;
const ID_JPEG_QUALITY_VALUE: i32 = 112;
// --- JPEG option controls (advanced) ---
const ID_JPEG_CHROMA_LABEL: i32 = 113;
const ID_JPEG_CHROMA_COMBO: i32 = 114;

// --- PNG option controls (standard) ---
const ID_PNG_COMPRESS_LABEL: i32 = 120;
const ID_PNG_COMPRESS_SLIDER: i32 = 121;
const ID_PNG_COMPRESS_VALUE: i32 = 122;
// --- PNG option controls (advanced) ---
const ID_PNG_FILTER_LABEL: i32 = 123;
const ID_PNG_FILTER_COMBO: i32 = 124;

// --- WebP option controls (standard) ---
const ID_WEBP_MODE_LABEL: i32 = 130;
const ID_WEBP_MODE_COMBO: i32 = 131;
const ID_WEBP_QUALITY_LABEL: i32 = 132;
const ID_WEBP_QUALITY_SLIDER: i32 = 133;
const ID_WEBP_QUALITY_VALUE: i32 = 134;

// --- AVIF option controls (standard) ---
const ID_AVIF_QUALITY_LABEL: i32 = 140;
const ID_AVIF_QUALITY_SLIDER: i32 = 141;
const ID_AVIF_QUALITY_VALUE: i32 = 142;
// --- AVIF option controls (advanced) ---
const ID_AVIF_SPEED_LABEL: i32 = 143;
const ID_AVIF_SPEED_SLIDER: i32 = 144;
const ID_AVIF_SPEED_VALUE: i32 = 145;

// --- TIFF option controls (advanced only) ---
const ID_TIFF_COMPRESS_LABEL: i32 = 150;
const ID_TIFF_COMPRESS_COMBO: i32 = 151;

// --- EXR option controls (advanced only) ---
const ID_EXR_COMPRESS_LABEL: i32 = 160;
const ID_EXR_COMPRESS_COMBO: i32 = 161;

// --- Size estimate label ---
const ID_SIZE_ESTIMATE: i32 = 180;

// --- Preset buttons ---
/// Control ID for the "Reset Recommended" button (resets current format to defaults).
const ID_RESET_RECOMMENDED: i32 = 181;

/// Control ID for the "Best Preset" button (switches to WebP lossy 85).
const ID_BEST_COMPROMISE: i32 = 182;

// --- Auto-resize controls ---
/// Control ID for the Auto-resize checkbox.
const ID_RESIZE_CHECK: i32 = 200;

/// Control ID for the max-width spin (up-down) control + accompanying edit.
const ID_RESIZE_WIDTH: i32 = 201;

/// Control ID for the max-height spin (up-down) control + accompanying edit.
const ID_RESIZE_HEIGHT: i32 = 202;

// ======================== LAYOUT CONSTANTS ========================

/// Dialog width in pixels.
const DLG_W: i32 = 500;

/// Dialog height in pixels (includes title bar + borders).
const DLG_H: i32 = 490;

/// Left/right margin for controls.
const MARGIN: i32 = 20;

/// Standard control height (text inputs, labels).
const CTRL_H: i32 = 26;

/// Vertical spacing between sections.
const SECTION_SPACE: i32 = 14;

/// Vertical spacing between label and its control.
const LABEL_GAP: i32 = 4;

/// Minimum JPEG quality. JPEG is fast enough that 25 is practical.
const JPEG_QUALITY_MIN: i32 = 25;

/// Maximum JPEG quality.
const JPEG_QUALITY_MAX: i32 = 100;

/// Background color for the dialog — matches Windows system dialog (BGR).
const DIALOG_BG: u32 = 0x00F0F0F0;

/// Y position where format-specific options start.
const OPTIONS_Y_START: i32 = 148;

/// Y position where advanced controls start (right below standard controls).
/// Standard option sections are ~80px tall, so 148 + 90 = 238.
const ADVANCED_Y_START: i32 = 238;

// ======================== STATIC STATE ========================

// WNDPROC cannot capture closures — store config in mutable statics.
// SAFETY: settings dialog runs on the single main thread.

/// Copy of the config being edited.
static mut EDIT_CONFIG: Option<Config> = None;

/// Whether the user clicked Save (true) or Cancel/closed (false).
static mut SAVE_CLICKED: bool = false;

/// Whether a result is ready to be consumed by the main loop.
static mut RESULT_READY: bool = false;

/// Handle to the settings dialog window.
static mut SETTINGS_HWND: HWND = HWND(ptr::null_mut());

/// Cached background brush for WM_CTLCOLORSTATIC (created once, reused).
static mut BG_BRUSH: HBRUSH = HBRUSH(ptr::null_mut());

// ======================== PUBLIC API ========================

/// Opens the settings dialog non-blocking. Returns immediately.
///
/// The dialog runs as a normal window — the main event loop's
/// `drain_win32_messages()` dispatches messages to its WNDPROC.
/// Call `is_open()` to check if the dialog is still visible, and
/// `take_result()` to consume the result after it closes.
pub fn open_settings(current: &Config) -> Result<(), SnipError> {
    info!("open_settings: opening settings dialog (non-blocking)");

    // Prevent opening multiple settings windows
    unsafe {
        let existing = ptr::addr_of!(SETTINGS_HWND).read();
        if !existing.is_invalid() && existing.0 != ptr::null_mut() {
            debug!("open_settings: dialog already open, focusing");
            let _ = SetForegroundWindow(existing);
            return Ok(());
        }
    }

    // Store a copy of the config for the dialog to edit
    unsafe {
        EDIT_CONFIG = Some(current.clone());
        SAVE_CLICKED = false;
        RESULT_READY = false;
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

    // No local message loop — return immediately.
    // The main loop's drain_win32_messages() handles dispatching.
    debug!("open_settings: dialog created (non-blocking)");

    Ok(())
}

/// Returns true if the settings dialog is currently open.
pub fn is_open() -> bool {
    unsafe {
        let hwnd = ptr::addr_of!(SETTINGS_HWND).read();
        !hwnd.is_invalid() && hwnd.0 != ptr::null_mut()
    }
}

/// Consumes the settings result if the dialog was closed with Save.
///
/// Returns `Some(new_config)` if the user saved, `None` if cancelled or
/// still open. The result can only be consumed once.
pub fn take_result() -> Option<Config> {
    unsafe {
        if !ptr::addr_of!(RESULT_READY).read() {
            return None;
        }

        RESULT_READY = false;

        if ptr::addr_of!(SAVE_CLICKED).read() {
            let cfg = ptr::addr_of_mut!(EDIT_CONFIG).replace(None);
            if let Some(new_cfg) = cfg {
                // Persist to disk
                if let Err(e) = config::save_config(&new_cfg) {
                    warn!("take_result: failed to save config: {}", e);
                    return None;
                }
                info!("take_result: config saved");
                return Some(new_cfg);
            }
        }

        info!("take_result: cancelled");
        None
    }
}

// ======================== WINDOW PROCEDURE ========================

/// WNDPROC for the settings dialog.
///
/// Creates child controls on WM_CREATE, handles button clicks, combo box
/// changes, and slider events. Reads all values on Save.
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
            let notification = ((wparam.0 >> 16) & 0xFFFF) as u32;

            match id {
                ID_BROWSE => {
                    debug!("settings_wndproc: Browse clicked");
                    if let Some(path) = browse_for_folder(hwnd) {
                        if let Ok(edit) = GetDlgItem(Some(hwnd), ID_PATH_EDIT) {
                            set_text(edit, &path);
                        }
                    }
                }

                ID_FORMAT_COMBO if notification == CBN_SELCHANGE => {
                    debug!("settings_wndproc: format selection changed");
                    if let Ok(combo) = GetDlgItem(Some(hwnd), ID_FORMAT_COMBO) {
                        let idx = SendMessageW(combo, CB_GETCURSEL, None, None).0 as usize;
                        if idx < OutputFormat::ALL.len() {
                            let format = OutputFormat::ALL[idx];
                            debug!("settings_wndproc: selected format: {}", format);
                            show_format_options(hwnd, format);
                        }
                    }
                }

                ID_WEBP_MODE_COMBO if notification == CBN_SELCHANGE => {
                    // Show/hide quality slider based on lossy vs lossless
                    if let Ok(combo) = GetDlgItem(Some(hwnd), ID_WEBP_MODE_COMBO) {
                        let idx = SendMessageW(combo, CB_GETCURSEL, None, None).0;
                        let is_lossy = idx == 0;
                        let show = if is_lossy { SW_SHOW } else { SW_HIDE };
                        show_control(hwnd, ID_WEBP_QUALITY_LABEL, show);
                        show_control(hwnd, ID_WEBP_QUALITY_SLIDER, show);
                        show_control(hwnd, ID_WEBP_QUALITY_VALUE, show);
                    }
                    update_size_estimate(hwnd, OutputFormat::WebP);
                }

                ID_TIFF_COMPRESS_COMBO if notification == CBN_SELCHANGE => {
                    update_size_estimate(hwnd, OutputFormat::Tiff);
                }

                ID_EXR_COMPRESS_COMBO if notification == CBN_SELCHANGE => {
                    update_size_estimate(hwnd, OutputFormat::OpenExr);
                }

                ID_ADVANCED_CHECK => {
                    debug!("settings_wndproc: Advanced checkbox toggled");
                    // Re-apply visibility with new advanced state
                    if let Ok(combo) = GetDlgItem(Some(hwnd), ID_FORMAT_COMBO) {
                        let idx = SendMessageW(combo, CB_GETCURSEL, None, None).0 as usize;
                        if idx < OutputFormat::ALL.len() {
                            show_format_options(hwnd, OutputFormat::ALL[idx]);
                        }
                    }
                }

                ID_RESET_RECOMMENDED => {
                    debug!("settings_wndproc: Reset Recommended clicked");
                    if let Ok(combo) = GetDlgItem(Some(hwnd), ID_FORMAT_COMBO) {
                        let idx = SendMessageW(combo, CB_GETCURSEL, None, None).0 as usize;
                        if idx < OutputFormat::ALL.len() {
                            apply_recommended(hwnd, OutputFormat::ALL[idx]);
                        }
                    }
                }

                ID_BEST_COMPROMISE => {
                    debug!("settings_wndproc: Best Preset clicked");
                    apply_best_compromise(hwnd);
                }

                ID_SAVE => {
                    info!("settings_wndproc: Save clicked");
                    handle_save(hwnd);
                    let _ = DestroyWindow(hwnd);
                }
                ID_CANCEL => {
                    debug!("settings_wndproc: Cancel clicked");
                    SAVE_CLICKED = false;
                    RESULT_READY = true;
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }

        WM_HSCROLL => {
            // Slider moved — update the corresponding value label
            update_slider_labels(hwnd);
            LRESULT(0)
        }

        WM_CTLCOLORSTATIC => {
            // Make static labels have the dialog background color.
            let brush = ptr::addr_of!(BG_BRUSH).read();
            let bg = if brush.is_invalid() {
                let b = CreateSolidBrush(COLORREF(DIALOG_BG));
                BG_BRUSH = b;
                b
            } else {
                brush
            };
            windows::Win32::Graphics::Gdi::SetBkColor(
                windows::Win32::Graphics::Gdi::HDC(wparam.0 as _),
                COLORREF(DIALOG_BG),
            );
            LRESULT(bg.0 as isize)
        }

        WM_CLOSE => {
            SAVE_CLICKED = false;
            RESULT_READY = true;
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }

        WM_DESTROY => {
            SETTINGS_HWND = HWND(ptr::null_mut());
            // Do NOT call PostQuitMessage — we're non-blocking, the main loop must keep running
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ======================== SAVE HANDLER ========================

/// Reads all control values and writes them back to the static config.
unsafe fn handle_save(hwnd: HWND) {
    // Read save directory
    let path_text = GetDlgItem(Some(hwnd), ID_PATH_EDIT)
        .map(|h| get_text(h))
        .unwrap_or_default();

    // Read selected format
    let format_idx = GetDlgItem(Some(hwnd), ID_FORMAT_COMBO)
        .map(|h| SendMessageW(h, CB_GETCURSEL, None, None).0 as usize)
        .unwrap_or(0);
    let format = if format_idx < OutputFormat::ALL.len() {
        OutputFormat::ALL[format_idx]
    } else {
        OutputFormat::Jpeg
    };

    // Read JPEG options
    let jpeg_quality = read_slider(hwnd, ID_JPEG_QUALITY_SLIDER, 85) as u32;
    let jpeg_chroma_idx = read_combo(hwnd, ID_JPEG_CHROMA_COMBO, 1);
    let chroma = if jpeg_chroma_idx < ChromaSubsampling::ALL.len() {
        ChromaSubsampling::ALL[jpeg_chroma_idx]
    } else {
        ChromaSubsampling::Half
    };

    // Read PNG options
    let png_compression = read_slider(hwnd, ID_PNG_COMPRESS_SLIDER, 6) as u8;
    let png_filter_idx = read_combo(hwnd, ID_PNG_FILTER_COMBO, 0);
    let png_filter = if png_filter_idx < PngFilter::ALL.len() {
        PngFilter::ALL[png_filter_idx]
    } else {
        PngFilter::Adaptive
    };

    // Read WebP options
    let webp_mode_idx = read_combo(hwnd, ID_WEBP_MODE_COMBO, 0);
    let webp_lossless = webp_mode_idx == 1;
    let webp_quality = read_slider(hwnd, ID_WEBP_QUALITY_SLIDER, 80) as f32;

    // Read AVIF options
    let avif_quality = read_slider(hwnd, ID_AVIF_QUALITY_SLIDER, 80) as u8;
    let avif_speed = read_slider(hwnd, ID_AVIF_SPEED_SLIDER, 4) as u8;

    // Read TIFF options
    let tiff_idx = read_combo(hwnd, ID_TIFF_COMPRESS_COMBO, 1);
    let tiff_compression = if tiff_idx < TiffCompression::ALL.len() {
        TiffCompression::ALL[tiff_idx]
    } else {
        TiffCompression::Lzw
    };

    // Read EXR options
    let exr_idx = read_combo(hwnd, ID_EXR_COMPRESS_COMBO, 3);
    let exr_compression = if exr_idx < ExrCompression::ALL.len() {
        ExrCompression::ALL[exr_idx]
    } else {
        ExrCompression::Zip16
    };

    // Read resize options
    let resize_enabled = GetDlgItem(Some(hwnd), ID_RESIZE_CHECK)
        .map(|h| SendMessageW(h, BM_GETCHECK, None, None).0 as usize == BST_CHECKED)
        .unwrap_or(false);
    let resize_width: u32 = GetDlgItem(Some(hwnd), ID_RESIZE_WIDTH)
        .and_then(|h| {
            let txt = get_text(h);
            txt.parse().ok()
        })
        .unwrap_or(2560);
    let resize_height: u32 = GetDlgItem(Some(hwnd), ID_RESIZE_HEIGHT)
        .and_then(|h| {
            let txt = get_text(h);
            txt.parse().ok()
        })
        .unwrap_or(2560);

    info!(
        "handle_save: format={}, save_dir='{}'",
        format, path_text
    );

    // Take the config out, modify locally, put back.
    let mut config = ptr::addr_of_mut!(EDIT_CONFIG).replace(None);
    if let Some(ref mut cfg) = config {
        cfg.capture.save_dir = path_text;
        cfg.capture.format = format;
        cfg.capture.format_options = FormatOptions {
            jpeg: JpegOptions {
                quality: jpeg_quality.clamp(JPEG_QUALITY_MIN as u32, JPEG_QUALITY_MAX as u32),
                chroma_subsampling: chroma,
            },
            png: PngOptions {
                compression: png_compression.clamp(0, 9),
                filter: png_filter,
            },
            webp: WebPOptions {
                lossless: webp_lossless,
                quality: webp_quality.clamp(25.0, 100.0),
            },
            avif: AvifOptions {
                quality: avif_quality.clamp(50, 100),
                speed: avif_speed.clamp(4, 10),
            },
            tiff: TiffOptions {
                compression: tiff_compression,
            },
            exr: ExrOptions {
                compression: exr_compression,
            },
        };
        cfg.capture.resize = snip_types::ResizeOptions {
            enabled: resize_enabled,
            max_width: resize_width,
            max_height: resize_height,
        };
    }
    ptr::addr_of_mut!(EDIT_CONFIG).write(config);

    SAVE_CLICKED = true;
    RESULT_READY = true;
    info!("handle_save: config updated, result ready");
}

/// Reads the current position of a slider control, or returns the default.
unsafe fn read_slider(hwnd: HWND, id: i32, default: i32) -> i32 {
    GetDlgItem(Some(hwnd), id)
        .map(|h| SendMessageW(h, TBM_GETPOS, None, None).0 as i32)
        .unwrap_or(default)
}

/// Reads the current selection index of a combo box, or returns the default.
unsafe fn read_combo(hwnd: HWND, id: i32, default: usize) -> usize {
    GetDlgItem(Some(hwnd), id)
        .map(|h| {
            let idx = SendMessageW(h, CB_GETCURSEL, None, None).0;
            if idx < 0 { default } else { idx as usize }
        })
        .unwrap_or(default)
}

/// Returns whether the Advanced checkbox is currently checked.
unsafe fn is_advanced(hwnd: HWND) -> bool {
    GetDlgItem(Some(hwnd), ID_ADVANCED_CHECK)
        .map(|h| SendMessageW(h, BM_GETCHECK, None, None).0 as usize == BST_CHECKED)
        .unwrap_or(false)
}

// ======================== CONTROL CREATION ========================

/// Creates all child controls inside the settings dialog.
unsafe fn create_controls(hwnd: HWND) {
    let hinstance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
    let font = GetStockObject(DEFAULT_GUI_FONT);

    // Read current values from the stored config.
    let (save_dir, format, opts, resize_cfg) = {
        let cfg = &*ptr::addr_of!(EDIT_CONFIG);
        match cfg {
            Some(c) => (
                c.capture.save_dir.clone(),
                c.capture.format,
                c.capture.format_options.clone(),
                c.capture.resize.clone(),
            ),
            None => (
                "~/Pictures/XDR-Snips".to_string(),
                OutputFormat::Jpeg,
                FormatOptions::default(),
                snip_types::ResizeOptions::default(),
            ),
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
    let style = GetWindowLongW(path_edit, GWL_STYLE);
    SetWindowLongW(path_edit, GWL_STYLE, style | WS_BORDER.0 as i32);

    let browse = create_child(
        hwnd, hinstance, w!("BUTTON"), "Browse...",
        MARGIN + edit_w + 8, y, browse_w, CTRL_H, ID_BROWSE,
    );
    send_font(browse, font);

    y += CTRL_H + SECTION_SPACE;

    // ─── Section 2: Output format combo box ───
    let fmt_lbl = create_child(
        hwnd, hinstance, w!("STATIC"), "Output format:",
        MARGIN, y, inner_w, 20, 0,
    );
    send_font(fmt_lbl, font);

    y += 20 + LABEL_GAP;

    let format_combo = create_combo_box(hwnd, hinstance, MARGIN, y, inner_w, 200, ID_FORMAT_COMBO);
    send_font(format_combo, font);

    let mut selected_idx: usize = 0;
    for (i, fmt) in OutputFormat::ALL.iter().enumerate() {
        add_combo_string(format_combo, fmt.display_name());
        if *fmt == format {
            selected_idx = i;
        }
    }
    SendMessageW(format_combo, CB_SETCURSEL, Some(WPARAM(selected_idx)), None);

    // ─── Section 3: Per-format options (standard) ───
    let oy = OPTIONS_Y_START;

    create_jpeg_controls(hwnd, hinstance, font, oy, inner_w, &opts.jpeg);
    create_png_controls(hwnd, hinstance, font, oy, inner_w, &opts.png);
    create_webp_controls(hwnd, hinstance, font, oy, inner_w, &opts.webp);
    create_avif_controls(hwnd, hinstance, font, oy, inner_w, &opts.avif);

    // ─── Section 4: Advanced option controls ───
    let ay = ADVANCED_Y_START;

    create_jpeg_advanced(hwnd, hinstance, font, ay, inner_w, &opts.jpeg);
    create_png_advanced(hwnd, hinstance, font, ay, inner_w, &opts.png);
    create_avif_advanced(hwnd, hinstance, font, ay, inner_w, &opts.avif);
    create_tiff_controls(hwnd, hinstance, font, ay, inner_w, &opts.tiff);
    create_exr_controls(hwnd, hinstance, font, ay, inner_w, &opts.exr);

    // Size estimate label — positioned dynamically by show_format_options()
    let est = create_child(
        hwnd, hinstance, w!("STATIC"), "",
        MARGIN, OPTIONS_Y_START, inner_w, 20, ID_SIZE_ESTIMATE,
    );
    send_font(est, font);

    // Preset buttons — positioned dynamically by show_format_options()
    let reset_btn = create_child(
        hwnd, hinstance, w!("BUTTON"), "Reset Recommended",
        MARGIN, OPTIONS_Y_START + 25, 160, CTRL_H, ID_RESET_RECOMMENDED,
    );
    send_font(reset_btn, font);

    let best_btn = create_child(
        hwnd, hinstance, w!("BUTTON"), "Best Preset (WebP 85)",
        MARGIN + 170, OPTIONS_Y_START + 25, 180, CTRL_H, ID_BEST_COMPROMISE,
    );
    send_font(best_btn, font);

    // Show standard options for the current format, hide all others
    show_format_options(hwnd, format);

    // ─── Section 5: Auto-resize ───
    create_resize_controls(
        hwnd, hinstance, font, &opts, resize_cfg,
    );

    // ─── Bottom row: Advanced checkbox (left) + Save/Cancel (right) ───
    let btn_y = DLG_H - 75;
    let btn_w = 100;
    let btn_h = 30;

    // Advanced options checkbox — BS_AUTOCHECKBOX = 0x0003
    let adv_check = create_child(
        hwnd, hinstance, w!("BUTTON"), "Advanced options",
        MARGIN, btn_y + 5, 160, 22, ID_ADVANCED_CHECK,
    );
    let adv_style = GetWindowLongW(adv_check, GWL_STYLE);
    SetWindowLongW(adv_check, GWL_STYLE, (adv_style & !0x000F) | 0x0003);
    send_font(adv_check, font);

    let cancel_btn = create_child(
        hwnd, hinstance, w!("BUTTON"), "Cancel",
        DLG_W - MARGIN - btn_w, btn_y, btn_w, btn_h, ID_CANCEL,
    );
    send_font(cancel_btn, font);

    let save_btn = create_child(
        hwnd, hinstance, w!("BUTTON"), "Save",
        DLG_W - MARGIN - btn_w * 2 - 10, btn_y, btn_w, btn_h, ID_SAVE,
    );
    send_font(save_btn, font);

    // Focus the path edit by default
    let _ = SetFocus(Some(path_edit));

    debug!(
        "create_controls: save_dir='{}', format={}", save_dir, format
    );
}

// ======================== STANDARD PER-FORMAT CONTROLS ========================

/// JPEG standard: quality slider.
unsafe fn create_jpeg_controls(
    hwnd: HWND,
    hi: HINSTANCE,
    font: windows::Win32::Graphics::Gdi::HGDIOBJ,
    base_y: i32,
    inner_w: i32,
    opts: &JpegOptions,
) {
    let mut y = base_y;

    let ql = create_child(
        hwnd, hi, w!("STATIC"),
        &format!("Quality ({}\u{2013}{}):", JPEG_QUALITY_MIN, JPEG_QUALITY_MAX),
        MARGIN, y, inner_w, 20, ID_JPEG_QUALITY_LABEL,
    );
    send_font(ql, font);
    y += 20 + LABEL_GAP;

    let slider = create_child(
        hwnd, hi, w!("msctls_trackbar32"), "",
        MARGIN, y, inner_w, 34, ID_JPEG_QUALITY_SLIDER,
    );
    let range_lparam = ((JPEG_QUALITY_MAX as u32) << 16 | JPEG_QUALITY_MIN as u32) as isize;
    SendMessageW(slider, TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM(range_lparam)));
    SendMessageW(slider, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(opts.quality as isize)));
    y += 34 + 2;

    let vl = create_child(
        hwnd, hi, w!("STATIC"),
        &jpeg_quality_label(opts.quality as i32),
        MARGIN, y, inner_w, 20, ID_JPEG_QUALITY_VALUE,
    );
    send_font(vl, font);
}

/// PNG standard: compression slider.
unsafe fn create_png_controls(
    hwnd: HWND,
    hi: HINSTANCE,
    font: windows::Win32::Graphics::Gdi::HGDIOBJ,
    base_y: i32,
    inner_w: i32,
    opts: &PngOptions,
) {
    let mut y = base_y;

    let cl = create_child(
        hwnd, hi, w!("STATIC"), "Compression level (0\u{2013}9):",
        MARGIN, y, inner_w, 20, ID_PNG_COMPRESS_LABEL,
    );
    send_font(cl, font);
    y += 20 + LABEL_GAP;

    let slider = create_child(
        hwnd, hi, w!("msctls_trackbar32"), "",
        MARGIN, y, inner_w, 34, ID_PNG_COMPRESS_SLIDER,
    );
    let range_lparam = ((9u32) << 16 | 0u32) as isize;
    SendMessageW(slider, TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM(range_lparam)));
    SendMessageW(slider, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(opts.compression as isize)));
    y += 34 + 2;

    let vl = create_child(
        hwnd, hi, w!("STATIC"),
        &png_compression_label(opts.compression as i32),
        MARGIN, y, inner_w, 20, ID_PNG_COMPRESS_VALUE,
    );
    send_font(vl, font);
}

/// WebP standard: mode combo + quality slider.
unsafe fn create_webp_controls(
    hwnd: HWND,
    hi: HINSTANCE,
    font: windows::Win32::Graphics::Gdi::HGDIOBJ,
    base_y: i32,
    inner_w: i32,
    opts: &WebPOptions,
) {
    let mut y = base_y;

    let ml = create_child(
        hwnd, hi, w!("STATIC"), "Mode:",
        MARGIN, y, inner_w, 20, ID_WEBP_MODE_LABEL,
    );
    send_font(ml, font);
    y += 20 + LABEL_GAP;

    let combo = create_combo_box(hwnd, hi, MARGIN, y, inner_w, 100, ID_WEBP_MODE_COMBO);
    send_font(combo, font);

    add_combo_string(combo, "Lossy");
    add_combo_string(combo, "Lossless");
    let mode_sel = if opts.lossless { 1usize } else { 0 };
    SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(mode_sel)), None);
    y += CTRL_H + SECTION_SPACE;

    let ql = create_child(
        hwnd, hi, w!("STATIC"), "Quality (25\u{2013}100):",
        MARGIN, y, inner_w, 20, ID_WEBP_QUALITY_LABEL,
    );
    send_font(ql, font);
    y += 20 + LABEL_GAP;

    let slider = create_child(
        hwnd, hi, w!("msctls_trackbar32"), "",
        MARGIN, y, inner_w, 34, ID_WEBP_QUALITY_SLIDER,
    );
    // Minimum 25%: lower values produce unusable quality for screenshots
    let range_lparam = ((100u32) << 16 | 25u32) as isize;
    SendMessageW(slider, TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM(range_lparam)));
    let clamped_q = (opts.quality as isize).max(25);
    SendMessageW(slider, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(clamped_q)));
    y += 34 + 2;

    let vl = create_child(
        hwnd, hi, w!("STATIC"),
        &format!("{}%", opts.quality as i32),
        MARGIN, y, inner_w, 20, ID_WEBP_QUALITY_VALUE,
    );
    send_font(vl, font);
}

/// AVIF standard: quality slider.
unsafe fn create_avif_controls(
    hwnd: HWND,
    hi: HINSTANCE,
    font: windows::Win32::Graphics::Gdi::HGDIOBJ,
    base_y: i32,
    inner_w: i32,
    opts: &AvifOptions,
) {
    let mut y = base_y;

    let ql = create_child(
        hwnd, hi, w!("STATIC"), "Quality (50\u{2013}100):",
        MARGIN, y, inner_w, 20, ID_AVIF_QUALITY_LABEL,
    );
    send_font(ql, font);
    y += 20 + LABEL_GAP;

    let slider = create_child(
        hwnd, hi, w!("msctls_trackbar32"), "",
        MARGIN, y, inner_w, 34, ID_AVIF_QUALITY_SLIDER,
    );
    // Minimum 50: AVIF below 50 produces poor quality and is slow
    let range_lparam = ((100u32) << 16 | 50u32) as isize;
    SendMessageW(slider, TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM(range_lparam)));
    let clamped_q = (opts.quality as isize).max(50);
    SendMessageW(slider, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(clamped_q)));
    y += 34 + 2;

    let vl = create_child(
        hwnd, hi, w!("STATIC"),
        &format!("{}%", opts.quality),
        MARGIN, y, inner_w, 20, ID_AVIF_QUALITY_VALUE,
    );
    send_font(vl, font);
}

// ======================== ADVANCED PER-FORMAT CONTROLS ========================

/// JPEG advanced: chroma subsampling combo.
unsafe fn create_jpeg_advanced(
    hwnd: HWND,
    hi: HINSTANCE,
    font: windows::Win32::Graphics::Gdi::HGDIOBJ,
    base_y: i32,
    inner_w: i32,
    opts: &JpegOptions,
) {
    let y = base_y;

    let cl = create_child(
        hwnd, hi, w!("STATIC"), "Chroma subsampling:",
        MARGIN, y, inner_w, 20, ID_JPEG_CHROMA_LABEL,
    );
    send_font(cl, font);

    let combo = create_combo_box(hwnd, hi, MARGIN, y + 20 + LABEL_GAP, inner_w, 150, ID_JPEG_CHROMA_COMBO);
    send_font(combo, font);

    let mut sel = 1usize;
    for (i, cs) in ChromaSubsampling::ALL.iter().enumerate() {
        add_combo_string(combo, cs.label());
        if *cs == opts.chroma_subsampling {
            sel = i;
        }
    }
    SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(sel)), None);
}

/// PNG advanced: filter strategy combo.
unsafe fn create_png_advanced(
    hwnd: HWND,
    hi: HINSTANCE,
    font: windows::Win32::Graphics::Gdi::HGDIOBJ,
    base_y: i32,
    inner_w: i32,
    opts: &PngOptions,
) {
    let y = base_y;

    let fl = create_child(
        hwnd, hi, w!("STATIC"), "Filter strategy:",
        MARGIN, y, inner_w, 20, ID_PNG_FILTER_LABEL,
    );
    send_font(fl, font);

    let combo = create_combo_box(hwnd, hi, MARGIN, y + 20 + LABEL_GAP, inner_w, 200, ID_PNG_FILTER_COMBO);
    send_font(combo, font);

    let mut sel = 0usize;
    for (i, f) in PngFilter::ALL.iter().enumerate() {
        add_combo_string(combo, f.label());
        if *f == opts.filter {
            sel = i;
        }
    }
    SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(sel)), None);
}

/// AVIF advanced: speed slider.
unsafe fn create_avif_advanced(
    hwnd: HWND,
    hi: HINSTANCE,
    font: windows::Win32::Graphics::Gdi::HGDIOBJ,
    base_y: i32,
    inner_w: i32,
    opts: &AvifOptions,
) {
    let mut y = base_y;

    let sl = create_child(
        hwnd, hi, w!("STATIC"), "Speed (4=slow/best \u{2013} 10=fastest):",
        MARGIN, y, inner_w, 20, ID_AVIF_SPEED_LABEL,
    );
    send_font(sl, font);
    y += 20 + LABEL_GAP;

    let speed_slider = create_child(
        hwnd, hi, w!("msctls_trackbar32"), "",
        MARGIN, y, inner_w, 34, ID_AVIF_SPEED_SLIDER,
    );
    // Range 4-10: speeds 1-3 are impractical (minutes per encode on large images)
    let speed_range = ((10u32) << 16 | 4u32) as isize;
    SendMessageW(speed_slider, TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM(speed_range)));
    let clamped_speed = opts.speed.max(4) as isize;
    SendMessageW(speed_slider, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(clamped_speed)));
    y += 34 + 2;

    let svl = create_child(
        hwnd, hi, w!("STATIC"),
        &avif_speed_label(opts.speed as i32),
        MARGIN, y, inner_w, 20, ID_AVIF_SPEED_VALUE,
    );
    send_font(svl, font);
}

/// TIFF advanced: compression combo.
unsafe fn create_tiff_controls(
    hwnd: HWND,
    hi: HINSTANCE,
    font: windows::Win32::Graphics::Gdi::HGDIOBJ,
    base_y: i32,
    inner_w: i32,
    opts: &TiffOptions,
) {
    let y = base_y;

    let cl = create_child(
        hwnd, hi, w!("STATIC"), "Compression:",
        MARGIN, y, inner_w, 20, ID_TIFF_COMPRESS_LABEL,
    );
    send_font(cl, font);

    let combo = create_combo_box(hwnd, hi, MARGIN, y + 20 + LABEL_GAP, inner_w, 150, ID_TIFF_COMPRESS_COMBO);
    send_font(combo, font);

    let mut sel = 1usize;
    for (i, tc) in TiffCompression::ALL.iter().enumerate() {
        add_combo_string(combo, tc.label());
        if *tc == opts.compression {
            sel = i;
        }
    }
    SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(sel)), None);
}

/// EXR advanced: compression combo.
unsafe fn create_exr_controls(
    hwnd: HWND,
    hi: HINSTANCE,
    font: windows::Win32::Graphics::Gdi::HGDIOBJ,
    base_y: i32,
    inner_w: i32,
    opts: &ExrOptions,
) {
    let y = base_y;

    let cl = create_child(
        hwnd, hi, w!("STATIC"), "EXR compression:",
        MARGIN, y, inner_w, 20, ID_EXR_COMPRESS_LABEL,
    );
    send_font(cl, font);

    let combo = create_combo_box(hwnd, hi, MARGIN, y + 20 + LABEL_GAP, inner_w, 250, ID_EXR_COMPRESS_COMBO);
    send_font(combo, font);

    let mut sel = 3usize;
    for (i, ec) in ExrCompression::ALL.iter().enumerate() {
        add_combo_string(combo, ec.label());
        if *ec == opts.compression {
            sel = i;
        }
    }
    SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(sel)), None);
}

// ======================== AUTO-RESIZE CONTROLS ========================

/// Creates the auto-resize section: checkbox + max-width input + max-height input.
///
/// Pleases at the bottom of the options area, above the bottom row buttons.
/// The controls are always visible (not affiliated the "Advanced" checkbox).
unsafe fn create_resize_controls(
    hwnd: HWND,
    hi: HINSTANCE,
    font: windows::Win32::Graphics::Gdi::HGDIOBJ,
    _opts: &FormatOptions,
    resize: &snip_types::ResizeOptions,
) {
    let inner_w = DLG_W - MARGIN * 2;

    // Separate from format options with a gap — hover below advanced area
    let y = ADVANCED_Y_START + 60;

    // Section header
    send_font(
        create_child(
            hwnd, hi, w!("STATIC"), "Auto-resize (downscale crops exceeding these limits):",
            MARGIN, y, inner_w, 20, 0,
        ), font,
    );

    let mut y = y + 20 + LABEL_GAP;

    // Resize checkbox — BS_AUTOCHECKBOX = 0x0003
    let resize_check = create_child(
        hwnd, hi, w!("BUTTON"), "Enable auto-resize",
        MARGIN, y, 220, 22, ID_RESIZE_CHECK,
    );
    SetWindowLongW(resize_check, GWL_STYLE, (GetWindowLongW(resize_check, GWL_STYLE) & !0x000F) | 0x0003);
    send_font(resize_check, font);

    // Set initial check state
    if resize.enabled {
        SendMessageW(resize_check, windows::Win32::UI::WindowsAndMessaging::BM_SETCHECK, Some(WPARAM(BST_CHECKED)), None);
    }

    y += 22 + SECTION_SPACE;

    // Max width label
    send_font(
        create_child(
            hwnd, hi, w!("STATIC"), "Max width (px):",
            MARGIN, y, 120, 20, 0,
        ), font,
    );
    y += 20 + LABEL_GAP;

    // Max width edit (numeric)
    let width_edit = create_child(
        hwnd, hi, w!("EDIT"), &format!("{}", resize.max_width),
        MARGIN, y, inner_w, CTRL_H, ID_RESIZE_WIDTH,
    );
    send_font(width_edit, font);
    let wstyle = GetWindowLongW(width_edit, GWL_STYLE);
    SetWindowLongW(width_edit, GWL_STYLE, wstyle | WS_BORDER.0 as i32 | 0x0008); // ES_NUMBER

    y += CTRL_H + SECTION_SPACE;

    // Max height label
    send_font(
        create_child(
            hwnd, hi, w!("STATIC"), "Max height (px):",
            MARGIN, y, 120, 20, 0,
        ), font,
    );
    y += 20 + LABEL_GAP;

    // Max height edit (numeric)
    let height_edit = create_child(
        hwnd, hi, w!("EDIT"), &format!("{}", resize.max_height),
        MARGIN, y, inner_w, CTRL_H, ID_RESIZE_HEIGHT,
    );
    send_font(height_edit, font);
    let hstyle = GetWindowLongW(height_edit, GWL_STYLE);
    SetWindowLongW(height_edit, GWL_STYLE, hstyle | WS_BORDER.0 as i32 | 0x0008); // ES_NUMBER
}

// ======================== FORMAT OPTIONS VISIBILITY ========================

/// Shows the option controls for the selected format and hides all others.
///
/// Standard controls are always shown. Advanced controls only show if the
/// Advanced checkbox is checked.
unsafe fn show_format_options(hwnd: HWND, format: OutputFormat) {
    let advanced = is_advanced(hwnd);
    debug!("show_format_options: format={}, advanced={}", format, advanced);

    // ─── JPEG ───
    let jpeg_std = if format == OutputFormat::Jpeg { SW_SHOW } else { SW_HIDE };
    let jpeg_adv = if format == OutputFormat::Jpeg && advanced { SW_SHOW } else { SW_HIDE };
    show_control(hwnd, ID_JPEG_QUALITY_LABEL, jpeg_std);
    show_control(hwnd, ID_JPEG_QUALITY_SLIDER, jpeg_std);
    show_control(hwnd, ID_JPEG_QUALITY_VALUE, jpeg_std);
    show_control(hwnd, ID_JPEG_CHROMA_LABEL, jpeg_adv);
    show_control(hwnd, ID_JPEG_CHROMA_COMBO, jpeg_adv);

    // ─── PNG ───
    let png_std = if format == OutputFormat::Png { SW_SHOW } else { SW_HIDE };
    let png_adv = if format == OutputFormat::Png && advanced { SW_SHOW } else { SW_HIDE };
    show_control(hwnd, ID_PNG_COMPRESS_LABEL, png_std);
    show_control(hwnd, ID_PNG_COMPRESS_SLIDER, png_std);
    show_control(hwnd, ID_PNG_COMPRESS_VALUE, png_std);
    show_control(hwnd, ID_PNG_FILTER_LABEL, png_adv);
    show_control(hwnd, ID_PNG_FILTER_COMBO, png_adv);

    // ─── WebP ───
    let webp_std = if format == OutputFormat::WebP { SW_SHOW } else { SW_HIDE };
    show_control(hwnd, ID_WEBP_MODE_LABEL, webp_std);
    show_control(hwnd, ID_WEBP_MODE_COMBO, webp_std);
    // Quality only visible in lossy mode
    if format == OutputFormat::WebP {
        let is_lossy = read_combo(hwnd, ID_WEBP_MODE_COMBO, 0) == 0;
        let q_show = if is_lossy { SW_SHOW } else { SW_HIDE };
        show_control(hwnd, ID_WEBP_QUALITY_LABEL, q_show);
        show_control(hwnd, ID_WEBP_QUALITY_SLIDER, q_show);
        show_control(hwnd, ID_WEBP_QUALITY_VALUE, q_show);
    } else {
        show_control(hwnd, ID_WEBP_QUALITY_LABEL, SW_HIDE);
        show_control(hwnd, ID_WEBP_QUALITY_SLIDER, SW_HIDE);
        show_control(hwnd, ID_WEBP_QUALITY_VALUE, SW_HIDE);
    }

    // ─── AVIF ───
    let avif_std = if format == OutputFormat::Avif { SW_SHOW } else { SW_HIDE };
    let avif_adv = if format == OutputFormat::Avif && advanced { SW_SHOW } else { SW_HIDE };
    show_control(hwnd, ID_AVIF_QUALITY_LABEL, avif_std);
    show_control(hwnd, ID_AVIF_QUALITY_SLIDER, avif_std);
    show_control(hwnd, ID_AVIF_QUALITY_VALUE, avif_std);
    show_control(hwnd, ID_AVIF_SPEED_LABEL, avif_adv);
    show_control(hwnd, ID_AVIF_SPEED_SLIDER, avif_adv);
    show_control(hwnd, ID_AVIF_SPEED_VALUE, avif_adv);

    // ─── TIFF (advanced only) ───
    let tiff_adv = if format == OutputFormat::Tiff && advanced { SW_SHOW } else { SW_HIDE };
    show_control(hwnd, ID_TIFF_COMPRESS_LABEL, tiff_adv);
    show_control(hwnd, ID_TIFF_COMPRESS_COMBO, tiff_adv);

    // ─── EXR (advanced only) ───
    let exr_adv = if format == OutputFormat::OpenExr && advanced { SW_SHOW } else { SW_HIDE };
    show_control(hwnd, ID_EXR_COMPRESS_LABEL, exr_adv);
    show_control(hwnd, ID_EXR_COMPRESS_COMBO, exr_adv);

    // BMP and QOI have no configurable options

    // ─── Reposition controls to eliminate gaps ───
    let inner_w = DLG_W - MARGIN * 2;

    // For formats with no standard controls, move advanced to top of options area
    if format == OutputFormat::Tiff && advanced {
        move_control_y(hwnd, ID_TIFF_COMPRESS_LABEL, OPTIONS_Y_START, inner_w, 20);
        move_control_y(hwnd, ID_TIFF_COMPRESS_COMBO, OPTIONS_Y_START + 24, inner_w, 150);
    }
    if format == OutputFormat::OpenExr && advanced {
        move_control_y(hwnd, ID_EXR_COMPRESS_LABEL, OPTIONS_Y_START, inner_w, 20);
        move_control_y(hwnd, ID_EXR_COMPRESS_COMBO, OPTIONS_Y_START + 24, inner_w, 250);
    }

    // ─── Update size estimate ───
    update_size_estimate(hwnd, format);

    // ─── Position preset buttons below the size estimate ───
    let has_options = !matches!(format, OutputFormat::Bmp | OutputFormat::Qoi);
    let btn_show = if has_options { SW_SHOW } else { SW_HIDE };
    show_control(hwnd, ID_RESET_RECOMMENDED, btn_show);
    // "Best Preset" always visible — it switches format
    show_control(hwnd, ID_BEST_COMPROMISE, SW_SHOW);

    // Dynamic Y: read the size estimate position and place buttons 25px below
    let estimate_y = estimate_y_for_format(format, advanced, hwnd);
    let buttons_y = estimate_y + 25;
    move_control_y(hwnd, ID_RESET_RECOMMENDED, buttons_y, 160, CTRL_H);
    // Place "Best Preset" to the right of "Reset Recommended"
    if let Ok(ctrl) = GetDlgItem(Some(hwnd), ID_BEST_COMPROMISE) {
        let x = if has_options { MARGIN + 170 } else { MARGIN };
        let _ = MoveWindow(ctrl, x, buttons_y, 180, CTRL_H, true);
    }
}

/// Shows or hides a control by its ID.
unsafe fn show_control(
    hwnd: HWND,
    id: i32,
    show: windows::Win32::UI::WindowsAndMessaging::SHOW_WINDOW_CMD,
) {
    if let Ok(ctrl) = GetDlgItem(Some(hwnd), id) {
        let _ = ShowWindow(ctrl, show);
    }
}

/// Repositions a control to a new Y coordinate at MARGIN x-offset.
///
/// Win32 `MoveWindow` sets both position and size, so width/height must be provided.
unsafe fn move_control_y(parent: HWND, id: i32, new_y: i32, w: i32, h: i32) {
    if let Ok(ctrl) = GetDlgItem(Some(parent), id) {
        let _ = MoveWindow(ctrl, MARGIN, new_y, w, h, true);
    }
}

// ======================== SLIDER LABEL UPDATES ========================

/// Updates all slider value labels after any slider movement.
unsafe fn update_slider_labels(hwnd: HWND) {
    // JPEG quality
    if let (Ok(slider), Ok(label)) = (
        GetDlgItem(Some(hwnd), ID_JPEG_QUALITY_SLIDER),
        GetDlgItem(Some(hwnd), ID_JPEG_QUALITY_VALUE),
    ) {
        let pos = SendMessageW(slider, TBM_GETPOS, None, None).0 as i32;
        set_text(label, &jpeg_quality_label(pos));
    }

    // PNG compression
    if let (Ok(slider), Ok(label)) = (
        GetDlgItem(Some(hwnd), ID_PNG_COMPRESS_SLIDER),
        GetDlgItem(Some(hwnd), ID_PNG_COMPRESS_VALUE),
    ) {
        let pos = SendMessageW(slider, TBM_GETPOS, None, None).0 as i32;
        set_text(label, &png_compression_label(pos));
    }

    // WebP quality
    if let (Ok(slider), Ok(label)) = (
        GetDlgItem(Some(hwnd), ID_WEBP_QUALITY_SLIDER),
        GetDlgItem(Some(hwnd), ID_WEBP_QUALITY_VALUE),
    ) {
        let pos = SendMessageW(slider, TBM_GETPOS, None, None).0 as i32;
        set_text(label, &format!("{}%", pos));
    }

    // AVIF quality
    if let (Ok(slider), Ok(label)) = (
        GetDlgItem(Some(hwnd), ID_AVIF_QUALITY_SLIDER),
        GetDlgItem(Some(hwnd), ID_AVIF_QUALITY_VALUE),
    ) {
        let pos = SendMessageW(slider, TBM_GETPOS, None, None).0 as i32;
        set_text(label, &format!("{}%", pos));
    }

    // AVIF speed
    if let (Ok(slider), Ok(label)) = (
        GetDlgItem(Some(hwnd), ID_AVIF_SPEED_SLIDER),
        GetDlgItem(Some(hwnd), ID_AVIF_SPEED_VALUE),
    ) {
        let pos = SendMessageW(slider, TBM_GETPOS, None, None).0 as i32;
        set_text(label, &avif_speed_label(pos));
    }

    // Update size estimate (reads current format from combo)
    if let Ok(combo) = GetDlgItem(Some(hwnd), ID_FORMAT_COMBO) {
        let idx = SendMessageW(combo, CB_GETCURSEL, None, None).0 as usize;
        if idx < OutputFormat::ALL.len() {
            update_size_estimate(hwnd, OutputFormat::ALL[idx]);
        }
    }
}

// ======================== SIZE ESTIMATE ========================

/// Updates the 1080p size estimate label position and text.
///
/// Reads current slider/combo values to calculate the estimate, then
/// repositions the label below the last visible control for the format.
unsafe fn update_size_estimate(hwnd: HWND, format: OutputFormat) {
    let inner_w = DLG_W - MARGIN * 2;
    let advanced = is_advanced(hwnd);

    // Read format-specific parameters for the estimate
    let (param1, param2) = match format {
        OutputFormat::Jpeg => (read_slider(hwnd, ID_JPEG_QUALITY_SLIDER, 85), 0),
        OutputFormat::Png => (read_slider(hwnd, ID_PNG_COMPRESS_SLIDER, 6), 0),
        OutputFormat::WebP => {
            let quality = read_slider(hwnd, ID_WEBP_QUALITY_SLIDER, 80);
            let lossless = read_combo(hwnd, ID_WEBP_MODE_COMBO, 0) as i32;
            (quality, lossless)
        }
        OutputFormat::Avif => (read_slider(hwnd, ID_AVIF_QUALITY_SLIDER, 80), 0),
        OutputFormat::Tiff => (read_combo(hwnd, ID_TIFF_COMPRESS_COMBO, 1) as i32, 0),
        OutputFormat::OpenExr => (read_combo(hwnd, ID_EXR_COMPRESS_COMBO, 3) as i32, 0),
        _ => (0, 0),
    };

    let text = size_estimate_text(format, param1, param2);

    // Position after the last visible control for this format
    let estimate_y = estimate_y_for_format(format, advanced, hwnd);

    move_control_y(hwnd, ID_SIZE_ESTIMATE, estimate_y, inner_w, 20);

    if let Ok(ctrl) = GetDlgItem(Some(hwnd), ID_SIZE_ESTIMATE) {
        set_text(ctrl, &text);
        let _ = ShowWindow(ctrl, SW_SHOW);
    }
}

/// Calculates the Y position for the size estimate label based on format and advanced state.
///
/// Extracted as a helper so both `update_size_estimate()` and `show_format_options()`
/// can compute consistent button placement.
unsafe fn estimate_y_for_format(format: OutputFormat, advanced: bool, hwnd: HWND) -> i32 {
    match format {
        OutputFormat::Jpeg | OutputFormat::Png => {
            if advanced { ADVANCED_Y_START + 60 } else { OPTIONS_Y_START + 90 }
        }
        OutputFormat::Avif => {
            if advanced { ADVANCED_Y_START + 90 } else { OPTIONS_Y_START + 90 }
        }
        OutputFormat::WebP => {
            let is_lossy = read_combo(hwnd, ID_WEBP_MODE_COMBO, 0) == 0;
            if is_lossy { OPTIONS_Y_START + 154 } else { OPTIONS_Y_START + 60 }
        }
        OutputFormat::Tiff | OutputFormat::OpenExr => {
            if advanced { OPTIONS_Y_START + 60 } else { OPTIONS_Y_START }
        }
        _ => OPTIONS_Y_START, // BMP, QOI — no controls
    }
}

/// Estimated 1080p file size for the given format and settings.
///
/// `param1` is quality (JPEG/WebP/AVIF), compression level (PNG), or
/// compression index (TIFF/EXR). `param2` is 1 for WebP lossless, 0 otherwise.
/// Estimates assume typical mixed-content 1920x1080 screenshots.
fn size_estimate_text(format: OutputFormat, param1: i32, param2: i32) -> String {
    let kb: u32 = match format {
        OutputFormat::Jpeg => match param1 {
            50..=55 => 120,
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
        },
        OutputFormat::Png => match param1 {
            0 => 6000,
            1..=3 => 4000,
            4..=6 => 3000,
            7..=9 => 2500,
            _ => 3000,
        },
        OutputFormat::WebP => {
            if param2 == 1 {
                2500 // lossless
            } else {
                match param1 {
                    0..=30 => 50,
                    31..=50 => 100,
                    51..=70 => 150,
                    71..=80 => 200,
                    81..=90 => 350,
                    91..=95 => 600,
                    96..=100 => 1500,
                    _ => 200,
                }
            }
        }
        OutputFormat::Avif => match param1 {
            1..=30 => 40,
            31..=50 => 80,
            51..=70 => 130,
            71..=80 => 200,
            81..=90 => 400,
            91..=95 => 700,
            96..=100 => 1000,
            _ => 200,
        },
        OutputFormat::Tiff => match param1 {
            0 => 6000,  // None
            1 => 3500,  // LZW
            2 => 3000,  // Deflate
            _ => 3500,
        },
        OutputFormat::Bmp => 6000,
        OutputFormat::Qoi => 3500,
        OutputFormat::OpenExr => match param1 {
            0 => 12000, // None
            1 => 8000,  // RLE
            2 => 5000,  // ZIP
            3 => 5000,  // ZIP16
            4 => 4000,  // PIZ
            5 => 4000,  // PXR24
            6 => 3000,  // B44
            _ => 5000,
        },
    };

    if kb >= 1000 {
        format!("1080p estimate: ~{:.1} MB", kb as f64 / 1000.0)
    } else {
        format!("1080p estimate: ~{} KB", kb)
    }
}

// ======================== LABEL FORMATTERS ========================

/// Descriptive label for JPEG quality value.
fn jpeg_quality_label(q: i32) -> String {
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

    if q == 85 {
        format!("{}% \u{00B7} {} \u{00B7} Recommended", q, size_str)
    } else {
        format!("{}% \u{00B7} {}", q, size_str)
    }
}

/// Descriptive label for PNG compression level.
fn png_compression_label(level: i32) -> String {
    let desc = match level {
        0 => "fastest, no compression",
        1..=3 => "fast",
        4..=6 => "balanced",
        7..=8 => "slow, smaller files",
        9 => "maximum compression, slowest",
        _ => "balanced",
    };

    if level == 6 {
        format!("{} \u{00B7} {} \u{00B7} Recommended", level, desc)
    } else {
        format!("{} \u{00B7} {}", level, desc)
    }
}

/// Descriptive label for AVIF speed level.
fn avif_speed_label(speed: i32) -> String {
    let desc = match speed {
        4 => "slow, best quality",
        5..=6 => "balanced",
        7..=8 => "fast, lower quality",
        9..=10 => "fastest, lowest quality",
        _ => "balanced",
    };

    if speed == 6 {
        format!("{} \u{00B7} {} \u{00B7} Recommended", speed, desc)
    } else {
        format!("{} \u{00B7} {}", speed, desc)
    }
}

// ======================== PRESET FUNCTIONS ========================

/// Sets a trackbar (slider) control to the given position.
unsafe fn set_slider(hwnd: HWND, id: i32, value: i32) {
    if let Ok(ctrl) = GetDlgItem(Some(hwnd), id) {
        SendMessageW(ctrl, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(value as isize)));
    }
}

/// Sets a combo box to the given selection index.
unsafe fn set_combo_selection(hwnd: HWND, id: i32, index: usize) {
    if let Ok(ctrl) = GetDlgItem(Some(hwnd), id) {
        SendMessageW(ctrl, CB_SETCURSEL, Some(WPARAM(index)), None);
    }
}

/// Resets the current format's options to recommended defaults.
///
/// Recommended defaults per format:
/// - JPEG: quality=85, chroma=4:2:2
/// - PNG: compression=6, filter=Adaptive
/// - WebP: lossy, quality=85
/// - AVIF: quality=80, speed=6
/// - TIFF: LZW
/// - OpenEXR: ZIP16
unsafe fn apply_recommended(hwnd: HWND, format: OutputFormat) {
    info!("apply_recommended: resetting {} to recommended defaults", format);

    match format {
        OutputFormat::Jpeg => {
            set_slider(hwnd, ID_JPEG_QUALITY_SLIDER, 85);
            set_combo_selection(hwnd, ID_JPEG_CHROMA_COMBO, 1); // 4:2:2
        }
        OutputFormat::Png => {
            set_slider(hwnd, ID_PNG_COMPRESS_SLIDER, 6);
            set_combo_selection(hwnd, ID_PNG_FILTER_COMBO, 0); // Adaptive
        }
        OutputFormat::WebP => {
            set_combo_selection(hwnd, ID_WEBP_MODE_COMBO, 0); // Lossy
            set_slider(hwnd, ID_WEBP_QUALITY_SLIDER, 85);
            // Ensure quality controls are visible (switching from lossless → lossy)
            show_control(hwnd, ID_WEBP_QUALITY_LABEL, SW_SHOW);
            show_control(hwnd, ID_WEBP_QUALITY_SLIDER, SW_SHOW);
            show_control(hwnd, ID_WEBP_QUALITY_VALUE, SW_SHOW);
        }
        OutputFormat::Avif => {
            set_slider(hwnd, ID_AVIF_QUALITY_SLIDER, 80);
            set_slider(hwnd, ID_AVIF_SPEED_SLIDER, 6);
        }
        OutputFormat::Tiff => {
            set_combo_selection(hwnd, ID_TIFF_COMPRESS_COMBO, 1); // LZW
        }
        OutputFormat::OpenExr => {
            set_combo_selection(hwnd, ID_EXR_COMPRESS_COMBO, 3); // ZIP16
        }
        _ => {} // BMP, QOI — no configurable options
    }

    // Refresh labels and size estimate
    update_slider_labels(hwnd);
}

/// Applies the "best compromise" preset: WebP lossy quality 85.
///
/// WebP lossy at quality 85 offers the best balance of:
/// - **Size**: ~3x smaller than JPEG at equivalent visual quality
/// - **Compatibility**: supported by all modern browsers and AI tools
/// - **Speed**: sub-100ms encoding (no UI freeze)
/// - **Quality**: sharp text, minimal artifacts on screenshots
unsafe fn apply_best_compromise(hwnd: HWND) {
    info!("apply_best_compromise: switching to WebP lossy quality 85");

    // Switch format combo to WebP (index 2 in OutputFormat::ALL)
    set_combo_selection(hwnd, ID_FORMAT_COMBO, 2);

    // Apply WebP recommended defaults
    set_combo_selection(hwnd, ID_WEBP_MODE_COMBO, 0); // Lossy
    set_slider(hwnd, ID_WEBP_QUALITY_SLIDER, 85);

    // Refresh visibility for WebP format
    show_format_options(hwnd, OutputFormat::WebP);

    // Refresh labels
    update_slider_labels(hwnd);
}

// ======================== HELPERS ========================

/// Creates a combo box with CBS_DROPDOWNLIST style.
///
/// CBS_DROPDOWNLIST (0x0003) **must** be present at creation time — setting it
/// via `SetWindowLongW` after the fact does not work (the control renders as
/// CBS_SIMPLE with an inline list instead of a proper dropdown popup).
unsafe fn create_combo_box(
    parent: HWND,
    hinstance: HINSTANCE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: i32,
) -> HWND {
    // CBS_DROPDOWNLIST = 0x0003 — compose via WINDOW_STYLE arithmetic
    let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(0x0003);

    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("COMBOBOX"),
        windows::core::PCWSTR::null(),
        style,
        x, y, w, h,
        Some(parent),
        Some(HMENU(id as *mut _)),
        Some(hinstance),
        None,
    )
    .unwrap_or(HWND(ptr::null_mut()))
}

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

/// Adds a string item to a combo box.
unsafe fn add_combo_string(combo: HWND, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    SendMessageW(
        combo,
        CB_ADDSTRING,
        None,
        Some(LPARAM(wide.as_ptr() as isize)),
    );
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
