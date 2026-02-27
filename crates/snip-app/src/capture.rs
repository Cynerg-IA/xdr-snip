//! Screenshot capture via embedded `capture-hdr.exe` subprocess.
//!
//! The HDR capture logic lives in a C# executable that handles HDR → SDR
//! tone-mapping via Windows.Graphics.Capture API. This module embeds the
//! C# binary at compile time, extracts it to %APPDATA%/hdr-snip/ on first
//! run, and invokes it as a subprocess.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use snip_types::{Region, SnipError};
use tracing::{debug, error, info};

/// Name of the HDR capture helper executable.
const CAPTURE_EXE_NAME: &str = "capture-hdr.exe";

/// Embedded capture-hdr.exe binary (built by C# project, placed in dist/).
/// The build.ps1 script must build the C# component first.
const EMBEDDED_CAPTURE_EXE: &[u8] = include_bytes!("../../../dist/capture-hdr.exe");

/// Captures a screen region by invoking the embedded `capture-hdr.exe` tool.
///
/// On first call, extracts the embedded binary to `%APPDATA%/hdr-snip/`.
/// Subsequent calls reuse the extracted binary (re-extracted if size differs).
///
/// # Arguments
/// * `region` — monitor-relative rectangle to capture.
/// * `monitor` — monitor index passed to the helper.
/// * `quality` — JPEG quality (1-100).
/// * `output` — destination file path for the JPEG.
///
/// # Errors
/// Returns [`SnipError::CaptureProcess`] if the helper cannot be extracted or
/// launched, and [`SnipError::CaptureFailed`] if it exits with a non-zero code.
pub fn capture_region(
    region: &Region,
    monitor: u32,
    quality: u32,
    output: &Path,
) -> Result<(), SnipError> {
    info!(
        "capture_region: region={}, monitor={}, quality={}, output={}",
        region,
        monitor,
        quality,
        output.display()
    );

    let capture_exe = ensure_capture_exe_extracted()?;

    debug!(
        "capture_region: using helper at {}",
        capture_exe.display()
    );

    // Build the region string: "X,Y,WxH"
    let region_arg = format!("{},{},{}x{}", region.x, region.y, region.w, region.h);

    let args = [
        "--monitor",
        &monitor.to_string(),
        "--region",
        &region_arg,
        "--quality",
        &quality.to_string(),
        "--output",
        &output.to_string_lossy(),
    ];

    debug!(
        "capture_region: spawning {} with args {:?}",
        capture_exe.display(),
        args
    );

    let start = Instant::now();

    let result = Command::new(&capture_exe)
        .args(&args)
        .output()
        .map_err(|e| {
            SnipError::CaptureProcess(format!(
                "failed to spawn {}: {}",
                capture_exe.display(),
                e
            ))
        })?;

    let elapsed = start.elapsed();

    debug!(
        "capture_region: process exited in {:.1}ms, status={}",
        elapsed.as_secs_f64() * 1000.0,
        result.status
    );

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);

        error!(
            "capture_region: capture-hdr.exe failed — exit={}, stderr={}, stdout={}",
            result.status, stderr, stdout
        );

        return Err(SnipError::CaptureFailed(format!(
            "capture-hdr.exe exited with {}: {}",
            result.status,
            stderr.trim()
        )));
    }

    // Verify the output file was actually created
    if !output.exists() {
        error!(
            "capture_region: helper exited 0 but output file not found: {}",
            output.display()
        );
        return Err(SnipError::CaptureFailed(format!(
            "output file not created: {}",
            output.display()
        )));
    }

    let file_size = output.metadata().map(|m| m.len()).unwrap_or(0);

    info!(
        "capture_region: capture complete — output={}, size={} bytes, elapsed={:.1}ms",
        output.display(),
        file_size,
        elapsed.as_secs_f64() * 1000.0
    );

    Ok(())
}

/// Ensures the embedded `capture-hdr.exe` is extracted to the app data directory.
///
/// Extracts on first run or when the embedded binary size differs from the
/// existing file (i.e., after an update). Returns the path to the extracted exe.
fn ensure_capture_exe_extracted() -> Result<PathBuf, SnipError> {
    let app_dir = dirs::config_dir()
        .ok_or_else(|| SnipError::CaptureProcess("cannot determine config dir".to_string()))?
        .join("hdr-snip");

    // Ensure the directory exists
    std::fs::create_dir_all(&app_dir).map_err(|e| {
        SnipError::CaptureProcess(format!("cannot create app dir {}: {}", app_dir.display(), e))
    })?;

    let capture_path = app_dir.join(CAPTURE_EXE_NAME);

    // Check if extraction is needed (missing or size mismatch = new version)
    let needs_extract = if capture_path.exists() {
        let existing_size = capture_path
            .metadata()
            .map(|m| m.len())
            .unwrap_or(0);
        let embedded_size = EMBEDDED_CAPTURE_EXE.len() as u64;

        if existing_size != embedded_size {
            debug!(
                "ensure_capture_exe_extracted: size mismatch (existing={}, embedded={}), re-extracting",
                existing_size, embedded_size
            );
            true
        } else {
            false
        }
    } else {
        debug!("ensure_capture_exe_extracted: not found, extracting");
        true
    };

    if needs_extract {
        std::fs::write(&capture_path, EMBEDDED_CAPTURE_EXE).map_err(|e| {
            SnipError::CaptureProcess(format!(
                "cannot write {}: {}",
                capture_path.display(),
                e
            ))
        })?;

        info!(
            "ensure_capture_exe_extracted: extracted {} bytes to {}",
            EMBEDDED_CAPTURE_EXE.len(),
            capture_path.display()
        );
    } else {
        debug!(
            "ensure_capture_exe_extracted: up to date at {}",
            capture_path.display()
        );
    }

    Ok(capture_path)
}
