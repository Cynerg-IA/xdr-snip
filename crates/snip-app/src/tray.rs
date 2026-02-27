//! System tray icon and context menu using the `tray-icon` crate.
//!
//! Creates a tray icon with a context menu containing "Take Screenshot",
//! "Open Folder", and "Quit" items.  Returns the menu item IDs so the main
//! loop can match events.

use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use snip_types::SnipError;
use tracing::{debug, info};

/// Width and height of the tray icon in pixels.
const ICON_SIZE: u32 = 32;

/// Creates the system tray icon and its context menu.
///
/// Returns a tuple of:
/// - [`TrayIcon`] — must be kept alive for the icon to remain visible.
/// - Three `String` values (menu item IDs) for: Take Screenshot, Open Folder, Quit.
///
/// # Errors
/// Returns [`SnipError::Overlay`] if icon or menu creation fails.
pub fn create_tray() -> Result<(TrayIcon, String, String, String), SnipError> {
    info!("create_tray: building tray icon and menu");

    // Build the context menu
    let menu = Menu::new();

    let item_screenshot = MenuItem::new("Take Screenshot", true, None);
    let item_open_folder = MenuItem::new("Open Folder", true, None);
    let item_quit = MenuItem::new("Quit", true, None);

    // Capture IDs before appending — the ID is a simple value type
    let id_screenshot = item_screenshot.id().0.clone();
    let id_open_folder = item_open_folder.id().0.clone();
    let id_quit = item_quit.id().0.clone();

    debug!(
        "create_tray: menu item IDs — screenshot={}, open_folder={}, quit={}",
        id_screenshot, id_open_folder, id_quit
    );

    menu.append(&item_screenshot).map_err(|e| {
        SnipError::Overlay(format!("failed to append Screenshot menu item: {}", e))
    })?;
    menu.append(&PredefinedMenuItem::separator()).map_err(|e| {
        SnipError::Overlay(format!("failed to append separator: {}", e))
    })?;
    menu.append(&item_open_folder).map_err(|e| {
        SnipError::Overlay(format!("failed to append Open Folder menu item: {}", e))
    })?;
    menu.append(&PredefinedMenuItem::separator()).map_err(|e| {
        SnipError::Overlay(format!("failed to append separator: {}", e))
    })?;
    menu.append(&item_quit).map_err(|e| {
        SnipError::Overlay(format!("failed to append Quit menu item: {}", e))
    })?;

    // Generate a simple default icon (solid color square) since we don't have
    // an icon file yet.  RGBA: 4 bytes per pixel.
    let icon = create_default_icon()?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("HDR Snip")
        .with_icon(icon)
        .build()
        .map_err(|e| {
            SnipError::Overlay(format!("failed to build tray icon: {}", e))
        })?;

    info!("create_tray: tray icon created successfully");

    Ok((tray, id_screenshot, id_open_folder, id_quit))
}

/// Creates a simple solid-color RGBA icon as a placeholder until a real icon
/// is provided.
///
/// The icon is a 32x32 cyan (#00BFFF) square with full opacity.
fn create_default_icon() -> Result<Icon, SnipError> {
    let pixel_count = (ICON_SIZE * ICON_SIZE) as usize;
    let mut rgba = Vec::with_capacity(pixel_count * 4);

    for _ in 0..pixel_count {
        // Cyan: R=0, G=191, B=255, A=255
        rgba.push(0x00); // R
        rgba.push(0xBF); // G
        rgba.push(0xFF); // B
        rgba.push(0xFF); // A
    }

    let icon = Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE).map_err(|e| {
        SnipError::Overlay(format!("failed to create default icon: {}", e))
    })?;

    debug!(
        "create_default_icon: generated {}x{} placeholder icon",
        ICON_SIZE, ICON_SIZE
    );

    Ok(icon)
}
