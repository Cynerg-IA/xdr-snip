//! System tray icon and context menu using the `tray-icon` crate.
//!
//! Creates a tray icon with a context menu containing "Take Screenshot",
//! "Open Folder", and "Quit" items.  Returns the menu item IDs so the main
//! loop can match events.

use snip_types::SnipError;
use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use tracing::{debug, info};

/// Width and height of the tray icon in pixels.
const ICON_SIZE: u32 = 32;

/// IDs for each tray menu item, returned by [`create_tray`].
pub struct TrayMenuIds {
    pub screenshot: String,
    pub open_folder: String,
    pub settings: String,
    pub quit: String,
}

/// Creates the system tray icon and its context menu.
///
/// Menu layout:
/// ```text
///   Take Screenshot
///   ─────────────
///   Open Folder
///   Settings
///   Open Log
///   ─────────────
///   Quit
/// ```
///
/// Returns the [`TrayIcon`] (must be kept alive) and the [`TrayMenuIds`].
///
/// # Errors
/// Returns [`SnipError::Overlay`] if icon or menu creation fails.
pub fn create_tray() -> Result<(TrayIcon, TrayMenuIds), SnipError> {
    info!("create_tray: building tray icon and menu");

    // Build the context menu
    let menu = Menu::new();

    let item_screenshot = MenuItem::new("Take Screenshot", true, None);
    let item_open_folder = MenuItem::new("Open Folder", true, None);
    let item_settings = MenuItem::new("Settings", true, None);
    let item_quit = MenuItem::new("Quit", true, None);

    // Capture IDs before appending — the ID is a simple value type
    let ids = TrayMenuIds {
        screenshot: item_screenshot.id().0.clone(),
        open_folder: item_open_folder.id().0.clone(),
        settings: item_settings.id().0.clone(),
        quit: item_quit.id().0.clone(),
    };

    debug!(
        "create_tray: menu IDs — screenshot={}, open_folder={}, settings={}, quit={}",
        ids.screenshot, ids.open_folder, ids.settings, ids.quit
    );

    let append = |item: &dyn tray_icon::menu::IsMenuItem, label: &str| -> Result<(), SnipError> {
        menu.append(item).map_err(|e| {
            SnipError::Overlay(format!("failed to append {} menu item: {}", label, e))
        })
    };

    append(&item_screenshot, "Screenshot")?;
    append(&PredefinedMenuItem::separator(), "separator")?;
    append(&item_open_folder, "Open Folder")?;
    append(&item_settings, "Settings")?;
    append(&PredefinedMenuItem::separator(), "separator")?;
    append(&item_quit, "Quit")?;

    // Generate the snip icon — four crop-mark corner brackets
    let icon = create_snip_icon()?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("XDR Snip — Press PrintScreen to capture")
        .with_icon(icon)
        .build()
        .map_err(|e| {
            SnipError::Overlay(format!("failed to build tray icon: {}", e))
        })?;

    info!("create_tray: tray icon created successfully");

    Ok((tray, ids))
}

// ======================== ICON GENERATION ========================

/// Creates a 32x32 tray icon with four corner bracket marks (crop handles)
/// on a transparent background — immediately recognizable as a region
/// selection / screenshot tool.
///
/// Layout (conceptual):
/// ```text
///   ████                ████
///   █                      █
///   █                      █
///
///   █                      █
///   █                      █
///   ████                ████
/// ```
fn create_snip_icon() -> Result<Icon, SnipError> {
    let size = ICON_SIZE;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    // White (#FFFFFF) corner brackets on transparent background.
    // White works on both dark (Win11 default) and light taskbars.
    let (r, g, b, a) = (255u8, 255u8, 255u8, 255u8);

    let arm = 9u32;     // length of each bracket arm in pixels
    let thick = 2u32;   // line thickness
    let margin = 4u32;  // inset from edge
    let far = size - margin; // far edge coordinate

    // Top-left bracket: horizontal + vertical
    fill_rect(&mut rgba, size, margin, margin, arm, thick, r, g, b, a);
    fill_rect(&mut rgba, size, margin, margin, thick, arm, r, g, b, a);

    // Top-right bracket: horizontal + vertical
    fill_rect(&mut rgba, size, far - arm, margin, arm, thick, r, g, b, a);
    fill_rect(&mut rgba, size, far - thick, margin, thick, arm, r, g, b, a);

    // Bottom-left bracket: horizontal + vertical
    fill_rect(&mut rgba, size, margin, far - thick, arm, thick, r, g, b, a);
    fill_rect(&mut rgba, size, margin, far - arm, thick, arm, r, g, b, a);

    // Bottom-right bracket: horizontal + vertical
    fill_rect(&mut rgba, size, far - arm, far - thick, arm, thick, r, g, b, a);
    fill_rect(&mut rgba, size, far - thick, far - arm, thick, arm, r, g, b, a);

    let icon = Icon::from_rgba(rgba, size, size).map_err(|e| {
        SnipError::Overlay(format!("failed to create snip icon: {}", e))
    })?;

    debug!(
        "create_snip_icon: generated {}x{} crop-marks icon",
        size, size
    );

    Ok(icon)
}

/// Fills a rectangle in an RGBA pixel buffer.
fn fill_rect(
    rgba: &mut [u8],
    stride: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) {
    for dy in 0..h {
        for dx in 0..w {
            let px = x + dx;
            let py = y + dy;
            if px < stride && py < stride {
                let idx = ((py * stride + px) * 4) as usize;
                rgba[idx] = r;
                rgba[idx + 1] = g;
                rgba[idx + 2] = b;
                rgba[idx + 3] = a;
            }
        }
    }
}
