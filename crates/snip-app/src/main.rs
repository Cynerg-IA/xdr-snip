//! # XDR Snip — main entry point
//!
//! Sets DPI awareness, initializes logging, loads config, creates the system
//! tray, registers the global hotkey, and runs a Win32 message loop that
//! dispatches hotkey and tray-menu events.

// Suppress the console window — this is a GUI application.
#![windows_subsystem = "windows"]

mod capture;
mod clipboard;
mod config;
mod hdr_capture;
mod hotkey;
mod overlay;
mod preview;
mod settings;
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

    info!("xdr-snip starting");

    // 3. Run the application; report top-level errors
    if let Err(e) = run() {
        error!("fatal error: {}", e);
        std::process::exit(1);
    }

    info!("xdr-snip exiting cleanly");
}

/// Core application logic, separated from `main` for clean error propagation.
fn run() -> Result<(), SnipError> {
    // Load configuration (mutable — settings dialog can hot-reload)
    let mut cfg = config::load_config()?;
    info!("config loaded: {:?}", cfg);

    // Resolve the save directory early so we fail fast on bad paths
    let mut save_dir = config::expand_tilde(&cfg.capture.save_dir);
    if !save_dir.exists() {
        info!(
            "save directory does not exist, creating: {}",
            save_dir.display()
        );
        fs::create_dir_all(&save_dir)?;
    }

    // Create the system tray icon + menu
    let (_tray, tray_ids) = tray::create_tray()?;

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
                // Drain any hotkey events that queued while the overlay was
                // pumping Win32 messages — prevents the overlay from re-opening
                // immediately after a capture.
                let mut drained = 0u32;
                while hotkey_rx.try_recv().is_ok() {
                    drained += 1;
                }
                if drained > 0 {
                    debug!("main: drained {} stale hotkey events", drained);
                }
            }
        }

        // Check for tray menu events
        if let Ok(event) = menu_rx.try_recv() {
            let clicked_id: String = event.id().0.clone();
            debug!("main: menu event id={}", clicked_id);

            if clicked_id == tray_ids.screenshot {
                debug!("main: menu -> Take Screenshot");
                handle_capture(&cfg, &save_dir);
                // Drain stale hotkey events here too
                while hotkey_rx.try_recv().is_ok() {}
            } else if clicked_id == tray_ids.open_folder {
                debug!("main: menu -> Open Folder");
                open_folder(&save_dir);
            } else if clicked_id == tray_ids.settings {
                debug!("main: menu -> Settings");
                if settings::is_open() {
                    debug!("main: settings already open, ignoring");
                } else if let Err(e) = settings::open_settings(&cfg) {
                    warn!("main: failed to open settings: {}", e);
                }
            } else if clicked_id == tray_ids.quit {
                info!("main: menu -> Quit");
                break;
            }
        }

        // Check for settings dialog result (non-blocking)
        if let Some(new_cfg) = settings::take_result() {
            info!("main: settings saved, hot-reloading config");
            let new_dir = config::expand_tilde(&new_cfg.capture.save_dir);
            if !new_dir.exists() {
                info!("main: creating new save directory: {}", new_dir.display());
                let _ = fs::create_dir_all(&new_dir);
            }
            save_dir = new_dir;
            cfg = new_cfg;
        }

        // Yield CPU to avoid busy-spinning (~60 Hz poll rate)
        std::thread::sleep(std::time::Duration::from_millis(16));
    }

    // Cleanup
    preview::close_preview();
    info!("main: cleanup complete");

    Ok(())
}

// ======================== CAPTURE WORKFLOW ========================

/// Runs the full capture pipeline: HDR capture → overlay → encode → clipboard → preview.
///
/// The dual-capture approach:
/// 1. WinRT captures HDR frames per-monitor (may fail → empty map).
/// 2. GDI BitBlt captures the frozen overlay display.
/// 3. After region selection, HDR frame is preferred for output; GDI is fallback.
///
/// Errors are logged — they do not crash the app.
fn handle_capture(cfg: &snip_types::Config, save_dir: &PathBuf) {
    info!("handle_capture: starting capture workflow");

    // Step 1: WinRT HDR capture — grab all monitors before showing overlay.
    // Returns empty map on failure; caller falls back to GDI pixels.
    let hdr_frames = hdr_capture::capture_all_monitors();
    debug!(
        "handle_capture: WinRT captured {} monitor frame(s)",
        hdr_frames.len()
    );

    // Step 2: Region selection overlay (GDI capture + frozen UI)
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

    let mut region = selection.region;
    info!(
        "handle_capture: region={}, monitor={}, hmonitor=0x{:X}",
        region, selection.monitor, selection.hmonitor
    );

    // Step 3: Choose pixel source — HDR frame if available, else GDI fallback
    let hdr_frame = hdr_frames.get(&selection.hmonitor);
    let mut pixels_rgb = if let Some(frame) = hdr_frame {
        info!(
            "handle_capture: using WinRT HDR pixels ({}x{}, hdr={})",
            frame.width, frame.height, frame.is_hdr
        );
        let hdr_pixels = capture::extract_hdr_region(frame, &selection.vscreen_region);
        if hdr_pixels.is_empty() {
            warn!("handle_capture: HDR extraction returned empty, falling back to GDI");
            selection.pixels_rgb
        } else {
            hdr_pixels
        }
    } else {
        debug!("handle_capture: no WinRT frame for this monitor, using GDI fallback");
        selection.pixels_rgb
    };

    info!(
        "handle_capture: pixel source ready — {} bytes for {}x{}",
        pixels_rgb.len(), region.w, region.h
    );

    // Step 3b: If format preserves HDR and we have HDR data, extract raw pixels
    let raw_hdr = if cfg.capture.format.preserves_hdr() {
        if let Some(frame) = hdr_frame {
            if frame.is_hdr {
                debug!("handle_capture: extracting raw HDR data for {}", cfg.capture.format);
                Some(capture::extract_hdr_region_raw(frame, &selection.vscreen_region))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Step 4: Generate output path with format-specific extension
    let filename = config::generate_filename(&cfg.capture.filename_pattern);
    let ext = cfg.capture.format.extension();
    let output_path = save_dir.join(format!("{}.{}", filename, ext));

    debug!(
        "handle_capture: output_path={} (format={})",
        output_path.display(), cfg.capture.format
    );

    // Step 5: Copy to clipboard first (fast ~40ms, needs Win32 main thread)
    let clipboard_ok = if cfg.behavior.copy_to_clipboard {
        match clipboard::copy_to_clipboard_pixels(&pixels_rgb, region.w, region.h) {
            Ok(()) => true,
            Err(e) => {
                warn!("handle_capture: clipboard copy failed: {}", e);
                false
            }
        }
    } else {
        false
    };

    // Step 5b: Generate preview thumbnail from raw pixels BEFORE encoding moves them.
    // This works for ALL output formats — no format-specific decoders needed.
    let (thumb_rgb, thumb_w, thumb_h) =
        preview::generate_thumbnail(&pixels_rgb, region.w, region.h, 300, 300);
    let orig_w = region.w;
    let orig_h = region.h;

    // Step 5b: Auto-resize (downscale) — applied to BOTH file and clipboard.
    // Downscale ONCE on the shared buffer so file and clipboard stay identical.
    let resize_cfg = &cfg.capture.resize;
    if resize_cfg.enabled && (region.w > resize_cfg.max_width || region.h > resize_cfg.max_height) {
        debug!(
            "handle_capture: auto-resize requested {}x{} -> caps {}x{}",
            region.w, region.h, resize_cfg.max_width, resize_cfg.max_height
        );

        let new_w = region.w.min(resize_cfg.max_width);
        let new_h = region.h.min(resize_cfg.max_height);

        // Compute aspect-ratio-preserving scale factor so both dimensions fit.
        let ratio = (new_w as f64 / region.w as f64).min(new_h as f64 / region.h as f64);
        let resized_w = (region.w as f64 * ratio).round() as u32;
        let resized_h = (region.h as f64 * ratio).round() as u32;

        // Build an ImageBuffer from our RGB8 Vec, resize with Lanczos3, convert back.
        if let Some(img) = image::ImageBuffer::<image::Rgb<u8>, Vec<u8>>::from_raw(
            region.w, region.h, std::mem::take(&mut pixels_rgb),
        ) {
            let resized_img = image::imageops::resize(&img, resized_w, resized_h, image::imageops::FilterType::Lanczos3);
            pixels_rgb = resized_img.into_raw();
            region.w = resized_w;
            region.h = resized_h;
            info!(
                "handle_capture: auto-resized {}x{} -> {}x{} (ratio={:.3})",
                orig_w, orig_h, resized_w, resized_h, ratio
            );
        }
    }

    // Step 6: Encode in configured format (on background thread for UI responsiveness)
    if cfg.behavior.save_to_file {
        if pixels_rgb.is_empty() {
            error!("handle_capture: no pixels from either HDR or GDI source");
            return;
        }

        let format = cfg.capture.format;
        let options = cfg.capture.format_options.clone();
        let out_path = output_path.clone();
        let w = region.w;
        let h = region.h;

        // Spawn encoding on a background thread so the main loop stays responsive.
        // Encoding can take several seconds; without threading the UI freezes.
        let encode_handle = std::thread::spawn(move || {
            capture::encode_image(&pixels_rgb, w, h, format, &options, &out_path, raw_hdr.as_ref())
        });

        // Keep Win32 message pump running while encoding (repaints, settings dialog, etc.)
        while !encode_handle.is_finished() {
            drain_win32_messages();
            std::thread::sleep(std::time::Duration::from_millis(16));
        }

        match encode_handle.join() {
            Ok(Ok(())) => { /* encode_image already logged timing */ }
            Ok(Err(e)) => {
                error!("handle_capture: {} encode failed: {}", format, e);
                return;
            }
            Err(_) => {
                error!("handle_capture: encoding thread panicked");
                return;
            }
        }
    }

    // Step 7: Show capture preview popup (thumbnail + info text)
    if let Err(e) = preview::show_preview(
        &output_path,
        &thumb_rgb,
        thumb_w,
        thumb_h,
        orig_w,
        orig_h,
        clipboard_ok,
    ) {
        warn!("handle_capture: preview failed: {}", e);
        // Non-fatal — capture was still successful
    }

    info!("handle_capture: capture workflow complete");
}

// ======================== HELPERS ========================

/// Initializes the `tracing` subscriber with file output.
///
/// Logs to `%APPDATA%/xdr-snip/xdr-snip.log` since this is a GUI app with no
/// console. The log file is truncated on each launch.
/// Override level with `RUST_LOG=debug` or `RUST_LOG=xdr_snip=trace`.
fn init_tracing() {
    use std::fs::OpenOptions;
    use std::sync::Mutex;
    use tracing_subscriber::EnvFilter;

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Write logs to file — GUI app has no console
    let log_dir = dirs::config_dir()
        .map(|d| d.join("xdr-snip"))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let _ = std::fs::create_dir_all(&log_dir);

    let log_path = log_dir.join("xdr-snip.log");
    match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
    {
        Ok(file) => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(true)
                .with_writer(Mutex::new(file))
                .with_ansi(false)
                .init();
        }
        Err(_) => {
            // Fallback to stderr if log file can't be created
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(true)
                .init();
        }
    }
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
    shell_open(&path.to_string_lossy());
    info!("open_folder: dispatched for {}", path.display());
}

/// Opens a path via `ShellExecuteW` with the "open" verb.
///
/// Works for files (opens in default app) and directories (opens in Explorer).
fn shell_open(path: &str) {
    let path_wide: Vec<u16> = path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: ShellExecuteW with "open" verb is safe.
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
}
