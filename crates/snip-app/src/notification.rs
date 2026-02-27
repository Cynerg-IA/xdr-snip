//! System notifications — shows a balloon tip via the Win32 Shell_NotifyIcon API.
//!
//! For the MVP we use the classic Shell_NotifyIcon balloon rather than the
//! modern ToastNotification API.  This avoids COM/WinRT initialization
//! complexity while still displaying a visible notification with capture details.

use std::mem;
use std::path::Path;

use image::ImageReader;
use snip_types::SnipError;
use tracing::{debug, error, info, warn};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NOTIFYICONDATAW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIM_ADD,
    NIM_DELETE, NIM_MODIFY,
};
use windows::Win32::UI::WindowsAndMessaging::{LoadIconW, IDI_INFORMATION};

/// User-message ID for tray icon callbacks (arbitrary, must be > WM_USER).
const WM_TRAY_CALLBACK: u32 = 0x0401;

/// Unique ID for our notification icon in the tray area.
const NOTIFY_ICON_ID: u32 = 1;

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

    // Show the filename (just the name, not full path — no PII)
    if let Some(name) = jpeg_path.file_name() {
        body.push_str(&format!("\n{}", name.to_string_lossy()));
    }

    debug!(
        "show_capture_notification: title='Screenshot saved', body='{}'",
        body
    );

    // Load the system information icon as a fallback
    // SAFETY: LoadIconW with a system icon ID is always safe.
    let icon = unsafe { LoadIconW(None, IDI_INFORMATION) }
        .map_err(|e| SnipError::Notification(format!("LoadIconW failed: {}", e)))?;

    // SAFETY: zeroed NOTIFYICONDATAW is valid — all fields are either zero or
    // null, and we fill in the required ones below.
    let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
    nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = HWND::default(); // no associated window
    nid.uID = NOTIFY_ICON_ID;
    nid.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_INFO;
    nid.uCallbackMessage = WM_TRAY_CALLBACK;
    nid.hIcon = icon;

    // Set tooltip text
    set_wide_string(&mut nid.szTip, "HDR Snip");

    // Set balloon title and body
    set_wide_string(&mut nid.szInfoTitle, "Screenshot saved");
    set_wide_string(&mut nid.szInfo, &body);

    // Add the notification icon
    // SAFETY: Shell_NotifyIconW is safe with a properly initialized NOTIFYICONDATAW.
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    if !added.as_bool() {
        warn!("show_capture_notification: NIM_ADD failed, trying NIM_MODIFY");
        // Icon might already exist from a previous capture — try modifying instead
        let modified = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
        if !modified.as_bool() {
            error!("show_capture_notification: NIM_MODIFY also failed");
            return Err(SnipError::Notification(
                "Shell_NotifyIconW NIM_ADD and NIM_MODIFY both failed".to_string(),
            ));
        }
    }

    debug!("show_capture_notification: balloon displayed");
    info!("show_capture_notification: notification shown successfully");
    Ok(())
}

/// Removes the notification icon from the tray area (call on app exit).
///
/// Safe to call even if no icon was added — failure is logged but not fatal.
pub fn remove_notification_icon() {
    debug!("remove_notification_icon: removing tray notification icon");

    // SAFETY: zeroed struct with only cbSize and uID set is valid for NIM_DELETE.
    let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
    nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = HWND::default();
    nid.uID = NOTIFY_ICON_ID;

    // SAFETY: Shell_NotifyIconW NIM_DELETE is safe with a minimal struct and valid ID.
    let ok = unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
    if !ok.as_bool() {
        debug!("remove_notification_icon: NIM_DELETE returned false (icon may not exist)");
    }
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
