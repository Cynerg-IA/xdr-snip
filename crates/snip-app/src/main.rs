//! # HDR Snip — main entry point
//!
//! Sets DPI awareness, initializes logging, loads config, creates the system
//! tray, registers the global hotkey, and runs a Win32 message loop that
//! dispatches hotkey and tray-menu events.

// Suppress the console window — this is a GUI application.
#![windows_subsystem = "windows"]

mod capture;
mod clipboard;
mod config;
mod hotkey;
mod notification;
mod overlay;
mod tray;

use std::fs;
use std::path::PathBuf;

use global_hotkey::GlobalHotKeyEvent;
use snip_types::SnipError;
use tray_icon::menu::MenuEvent;
use tracing::{debug, error, info, warn};
use windows::core::w;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, SW_SHOWNORMAL,
};

// ======================== DPI AWARENESS ========================

/// Sets per-monitor DPI awareness v2.  Must be called before any window
/// creation to ensure correct coordinates on mixed-DPI setups.
fn set_dpi_awareness() {
    // SAFETY: SetProcessDpiAwarenessContext is safe to call at process start.
    // If it fails (already set, or OS too old), we log and continue.
    unsafe {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        let result = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if result.is_err() {
            // Non-fatal — may already be set via manifest or older OS
            debug!("set_dpi_awareness: SetProcessDpiAwarenessContext returned error (may already be set)");
        } else {
            debug!("set_dpi_awareness: per-monitor DPI v2 enabled");
        }
    }
}

// ======================== MAIN ========================

fn main() {
    // 1. DPI awareness — must come first
    set_dpi_awareness();

    // 2. Logging
    init_tracing();

    info!("hdr-snip starting");

    // 3. Run the application; report top-level errors
    if let Err(e) = run() {
        error!("fatal error: {}", e);
        std::process::exit(1);
    }

    info!("hdr-snip exiting cleanly");
}

/// Core application logic, separated from `main` for clean error propagation.
fn run() -> Result<(), SnipError> {
    // Load configuration
    let cfg = config::load_config()?;
    info!("config loaded: {:?}", cfg);

    // Resolve the save directory early so we fail fast on bad paths
    let save_dir = config::expand_tilde(&cfg.capture.save_dir);
    if !save_dir.exists() {
        info!(
            "save directory does not exist, creating: {}",
            save_dir.display()
        );
        fs::create_dir_all(&save_dir)?;
    }

    // Create the system tray icon + menu
    let (_tray, id_screenshot, id_open_folder, id_quit) = tray::create_tray()?;

    // Register the global hotkey
    let (_hk_manager, hotkey_handle) = hotkey::register_hotkey(&cfg.hotkey)?;
    let target_hotkey_id = hotkey_handle.id();

    info!("entering main event loop");

    // Channel receivers for hotkey and menu events
    let hotkey_rx = GlobalHotKeyEvent::receiver();
    let menu_rx = MenuEvent::receiver();

    loop {
        // Drain Win32 messages (paint, input, etc.)
        drain_win32_messages();

        // Check for global hotkey events
        if let Ok(event) = hotkey_rx.try_recv() {
            if event.id() == target_hotkey_id {
                debug!("main: hotkey triggered");
                handle_capture(&cfg, &save_dir);
            }
        }

        // Check for tray menu events
        if let Ok(event) = menu_rx.try_recv() {
            let clicked_id: String = event.id().0.clone();
            debug!("main: menu event id={}", clicked_id);

            if clicked_id == id_screenshot {
                debug!("main: menu -> Take Screenshot");
                handle_capture(&cfg, &save_dir);
            } else if clicked_id == id_open_folder {
                debug!("main: menu -> Open Folder");
                open_folder(&save_dir);
            } else if clicked_id == id_quit {
                info!("main: menu -> Quit");
                break;
            }
        }

        // Yield CPU to avoid busy-spinning (~60 Hz poll rate)
        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    // Cleanup
    notification::remove_notification_icon();
    info!("main: cleanup complete");

    Ok(())
}

// ======================== CAPTURE WORKFLOW ========================

/// Runs the full capture pipeline: overlay -> capture -> clipboard -> notification.
///
/// Errors are logged and displayed via notification — they do not crash the app.
fn handle_capture(cfg: &snip_types::Config, save_dir: &PathBuf) {
    info!("handle_capture: starting capture workflow");

    // Step 1: Region selection overlay
    let selection = match overlay::select_region() {
        Ok(Some(sel)) => sel,
        Ok(None) => {
            info!("handle_capture: user cancelled region selection");
            return;
        }
        Err(e) => {
            error!("handle_capture: overlay failed: {}", e);
            return;
        }
    };

    let (region, monitor) = selection;
    info!(
        "handle_capture: region={}, monitor={}",
        region, monitor
    );

    // Step 2: Generate output path
    let filename = config::generate_filename(&cfg.capture.filename_pattern);
    let output_path = save_dir.join(format!("{}.jpg", filename));

    debug!(
        "handle_capture: output_path={}",
        output_path.display()
    );

    // Step 3: Capture via external helper
    if cfg.behavior.save_to_file {
        if let Err(e) =
            capture::capture_region(&region, monitor, cfg.capture.quality, &output_path)
        {
            error!("handle_capture: capture failed: {}", e);
            return;
        }
    }

    // Step 4: Copy to clipboard
    let clipboard_ok = if cfg.behavior.copy_to_clipboard {
        match clipboard::copy_to_clipboard(&output_path) {
            Ok(()) => true,
            Err(e) => {
                warn!("handle_capture: clipboard copy failed: {}", e);
                false
            }
        }
    } else {
        false
    };

    // Step 5: Show notification with capture details
    if cfg.behavior.show_notification {
        if let Err(e) = notification::show_capture_notification(&output_path, clipboard_ok) {
            warn!("handle_capture: notification failed: {}", e);
            // Non-fatal
        }
    }

    info!("handle_capture: capture workflow complete");
}

// ======================== HELPERS ========================

/// Initializes the `tracing` subscriber with an env-filter.
///
/// Default level: INFO.  Override with `RUST_LOG=debug` or `RUST_LOG=snip_app=trace`.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}

/// Drains all pending Win32 messages without blocking.
fn drain_win32_messages() {
    // SAFETY: PeekMessageW with PM_REMOVE is safe and the standard non-blocking
    // message drain pattern.
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Opens a folder in Windows Explorer via ShellExecuteW.
fn open_folder(path: &PathBuf) {
    debug!("open_folder: opening {}", path.display());

    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: ShellExecuteW with "open" verb and a directory path is safe.
    // The path_wide Vec lives until after the call returns.
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            windows::core::PCWSTR(path_wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }

    info!(
        "open_folder: ShellExecuteW dispatched for {}",
        path.display()
    );
}
