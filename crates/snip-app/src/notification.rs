//! System notifications — shows a balloon tip via the Win32 Shell_NotifyIcon API.
//!
//! Creates a hidden message-only window as the notification anchor. This is
//! required because Shell_NotifyIconW needs a valid HWND to deliver balloon
//! callback messages. The hidden window is created once and reused.

use std::mem;
use std::path::Path;

use image::ImageReader;
use snip_types::SnipError;
use tracing::{debug, error, info};
use windows::core::w;
use windows::Win32::Foundation::{HWND, HINSTANCE, LRESULT, WPARAM, LPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NOTIFYICONDATAW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIM_ADD,
    NIM_DELETE, NIM_MODIFY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, LoadIconW, RegisterClassW,
    IDI_INFORMATION, WINDOW_EX_STYLE, WNDCLASSW, WS_POPUP,
};

/// User-message ID for tray icon callbacks (arbitrary, must be > WM_USER).
const WM_TRAY_CALLBACK: u32 = 0x0401;

/// Unique ID for our notification icon in the tray area.
const NOTIFY_ICON_ID: u32 = 1;

/// HWND_MESSAGE parent — creates a message-only window (not visible, no taskbar).
const HWND_MESSAGE: HWND = HWND(-3isize as *mut _);

/// Hidden message-only window used as the notification anchor.
/// Created on first notification, reused for subsequent ones.
static mut NOTIFY_HWND: HWND = HWND(std::ptr::null_mut());

/// Shows a balloon notification indicating a screenshot was captured.
///
/// Reads the file to determine dimensions and size, then displays a
/// balloon tip with summary info.
///
/// # Arguments
/// * `jpeg_path` — path to the saved JPEG file.
/// * `copied_to_clipboard` — whether the image was copied to clipboard.
///
/// # Errors
/// Returns [`SnipError::Notification`] if the Win32 call fails.
pub fn show_capture_notification(
    jpeg_path: &Path,
    copied_to_clipboard: bool,
) -> Result<(), SnipError> {
    info!(
        "show_capture_notification: path={}, clipboard={}",
        jpeg_path.display(),
        copied_to_clipboard
    );

    // Read file size
    let file_size = jpeg_path
        .metadata()
        .map(|m| m.len())
        .unwrap_or(0);
    let size_str = format_file_size(file_size);

    // Read image dimensions (header only, no full decode)
    let dims = ImageReader::open(jpeg_path)
        .ok()
        .and_then(|r| r.into_dimensions().ok());

    // Build notification body
    let mut body = String::with_capacity(128);

    if let Some((w, h)) = dims {
        body.push_str(&format!("{} x {} | {}", w, h, size_str));
    } else {
        body.push_str(&size_str);
    }

    if copied_to_clipboard {
        body.push_str("\nCopied to clipboard");
    }

    // Show the full save path so the user knows where the file went
    body.push_str(&format!("\n{}", jpeg_path.display()));

    debug!(
        "show_capture_notification: title='Screenshot saved', body='{}'",
        body
    );

    // Ensure the hidden notification window exists
    let notify_hwnd = ensure_notify_window()?;
    debug!("show_capture_notification: using HWND {:?}", notify_hwnd.0);

    // Load the system information icon as a fallback
    // SAFETY: LoadIconW with a system icon ID is always safe.
    let icon = unsafe { LoadIconW(None, IDI_INFORMATION) }
        .map_err(|e| SnipError::Notification(format!("LoadIconW failed: {}", e)))?;

    // SAFETY: zeroed NOTIFYICONDATAW is valid — all fields are either zero or
    // null, and we fill in the required ones below.
    let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
    nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = notify_hwnd;
    nid.uID = NOTIFY_ICON_ID;
    nid.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_INFO;
    nid.uCallbackMessage = WM_TRAY_CALLBACK;
    nid.hIcon = icon;

    // Set tooltip text
    set_wide_string(&mut nid.szTip, "XDR Snip");

    // Set balloon title and body
    set_wide_string(&mut nid.szInfoTitle, "Screenshot saved");
    set_wide_string(&mut nid.szInfo, &body);

    // Add the notification icon and show the balloon.
    // SAFETY: Shell_NotifyIconW is safe with a properly initialized NOTIFYICONDATAW.
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    if !added.as_bool() {
        debug!("show_capture_notification: NIM_ADD failed, trying NIM_MODIFY");
        // Icon already exists from a previous capture — modify to show new balloon
        let modified = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
        if !modified.as_bool() {
            error!("show_capture_notification: NIM_MODIFY also failed");
            return Err(SnipError::Notification(
                "Shell_NotifyIconW NIM_ADD and NIM_MODIFY both failed".to_string(),
            ));
        }
    }

    // Immediately remove the tray icon so the ugly blue "i" doesn't linger.
    // On Windows 10/11 the balloon is converted to a toast notification that
    // persists in the action center independently of the tray icon.
    let mut nid_del: NOTIFYICONDATAW = unsafe { mem::zeroed() };
    nid_del.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid_del.hWnd = notify_hwnd;
    nid_del.uID = NOTIFY_ICON_ID;
    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &nid_del) };
    debug!("show_capture_notification: tray icon removed (toast persists)");

    info!("show_capture_notification: notification shown successfully");
    Ok(())
}

/// Removes the notification icon from the tray area and destroys the hidden
/// message window (call on app exit).
///
/// Safe to call even if no icon was added — failure is logged but not fatal.
pub fn remove_notification_icon() {
    debug!("remove_notification_icon: removing tray notification icon");

    unsafe {
        let hwnd = std::ptr::addr_of!(NOTIFY_HWND).read();

        // Remove the Shell_NotifyIcon entry
        let mut nid: NOTIFYICONDATAW = mem::zeroed();
        nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = NOTIFY_ICON_ID;

        let ok = Shell_NotifyIconW(NIM_DELETE, &nid);
        if !ok.as_bool() {
            debug!("remove_notification_icon: NIM_DELETE returned false (icon may not exist)");
        }

        // Destroy the hidden message window
        if !hwnd.is_invalid() && hwnd.0 != std::ptr::null_mut() {
            let _ = DestroyWindow(hwnd);
            NOTIFY_HWND = HWND(std::ptr::null_mut());
            debug!("remove_notification_icon: hidden window destroyed");
        }
    }
}

// ======================== HIDDEN WINDOW ========================

/// Creates (or returns the existing) hidden message-only window used as the
/// notification anchor for Shell_NotifyIconW.
///
/// A message-only window (parented to HWND_MESSAGE) is invisible, has no
/// taskbar entry, and exists solely to receive Win32 messages.
fn ensure_notify_window() -> Result<HWND, SnipError> {
    // SAFETY: single-threaded — only the main thread calls this.
    unsafe {
        let existing = std::ptr::addr_of!(NOTIFY_HWND).read();
        if !existing.is_invalid() && existing.0 != std::ptr::null_mut() {
            return Ok(existing);
        }
    }

    debug!("ensure_notify_window: creating hidden message-only window");

    let hinstance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(|e| SnipError::Notification(format!("GetModuleHandleW: {}", e)))?
        .into();

    // Register a minimal window class for the notification window
    let wc = WNDCLASSW {
        lpfnWndProc: Some(notify_wndproc),
        hInstance: hinstance,
        lpszClassName: w!("XdrSnipNotify"),
        ..Default::default()
    };

    // RegisterClassW returns 0 if the class already exists — that's fine
    let _ = unsafe { RegisterClassW(&wc) };

    // Create a message-only window (not visible, no taskbar)
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("XdrSnipNotify"),
            w!("XDR Snip Notification"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinstance),
            None,
        )
    }
    .map_err(|e| SnipError::Notification(format!("CreateWindowExW (notify): {}", e)))?;

    debug!("ensure_notify_window: created HWND {:?}", hwnd.0);

    // Store for reuse
    unsafe {
        NOTIFY_HWND = hwnd;
    }

    Ok(hwnd)
}

/// Minimal WNDPROC for the hidden notification window — delegates everything
/// to DefWindowProcW.
///
/// # Safety
/// Called by Windows — must follow the WNDPROC contract.
unsafe extern "system" fn notify_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

// ======================== HELPERS ========================

/// Copies a Rust `&str` into a fixed-size `u16` wide-string buffer, truncating
/// if necessary and always null-terminating.
fn set_wide_string(buf: &mut [u16], text: &str) {
    let max_chars = buf.len().saturating_sub(1); // reserve space for null terminator
    let wide: Vec<u16> = text.encode_utf16().take(max_chars).collect();
    buf[..wide.len()].copy_from_slice(&wide);
    buf[wide.len()] = 0; // null terminator
}

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
