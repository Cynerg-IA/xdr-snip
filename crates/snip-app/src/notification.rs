//! System notifications — shows a balloon tip via the Win32 Shell_NotifyIcon API.
//!
//! For the MVP we use the classic Shell_NotifyIcon balloon rather than the
//! modern ToastNotification API.  This avoids COM/WinRT initialization
//! complexity while still displaying a visible notification with the file path.

use std::mem;
use std::path::Path;

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
/// Creates a temporary notification-area icon, displays a balloon tip with
/// "Screenshot saved" and the file path, then removes the icon.
///
/// # Errors
/// Returns [`SnipError::Notification`] if the Win32 call fails.
pub fn show_capture_notification(jpeg_path: &Path) -> Result<(), SnipError> {
    info!(
        "show_capture_notification: path={}",
        jpeg_path.display()
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

    // Set tooltip text: "HDR Snip"
    set_wide_string(&mut nid.szTip, "HDR Snip");

    // Set balloon title
    set_wide_string(&mut nid.szInfoTitle, "Screenshot saved");

    // Set balloon body: the file path (truncated if too long for the buffer)
    let body = jpeg_path.to_string_lossy();
    set_wide_string(&mut nid.szInfo, &body);

    // Add the temporary icon
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

    // We intentionally do NOT delete the icon immediately — Windows needs time
    // to show the balloon.  The icon will be cleaned up on application exit
    // or on the next notification (NIM_MODIFY).

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

/// Copies a Rust `&str` into a fixed-size `u16` wide-string buffer, truncating
/// if necessary and always null-terminating.
fn set_wide_string(buf: &mut [u16], text: &str) {
    let max_chars = buf.len().saturating_sub(1); // reserve space for null terminator
    let wide: Vec<u16> = text.encode_utf16().take(max_chars).collect();
    buf[..wide.len()].copy_from_slice(&wide);
    buf[wide.len()] = 0; // null terminator
}
