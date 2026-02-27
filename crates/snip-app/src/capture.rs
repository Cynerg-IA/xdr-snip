//! Screenshot capture via external `capture-hdr.exe` subprocess.
//!
//! The HDR capture logic lives in a separate C++/DirectX executable that handles
//! the complexity of HDR → SDR tone-mapping and multi-monitor DPI.  This module
//! locates the executable, spawns it with the correct arguments, and checks the
//! result.

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use snip_types::{Region, SnipError};
use tracing::{debug, error, info};

/// Name of the HDR capture helper executable expected next to our own binary.
const CAPTURE_EXE_NAME: &str = "capture-hdr.exe";

/// Captures a screen region by invoking the external `capture-hdr.exe` tool.
///
/// # Arguments
/// * `region` — monitor-relative rectangle to capture.
/// * `monitor` — opaque monitor identifier passed to the helper.
/// * `quality` — JPEG quality (1-100).
/// * `output` — destination file path for the JPEG.
///
/// # Errors
/// Returns [`SnipError::CaptureProcess`] if the helper cannot be found or
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

    let capture_exe = locate_capture_exe()?;

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

/// Locates `capture-hdr.exe` adjacent to the running executable.
///
/// The helper is expected to live in the same directory as `snip-app.exe`.
fn locate_capture_exe() -> Result<std::path::PathBuf, SnipError> {
    let exe_dir = std::env::current_exe()
        .map_err(|e| {
            SnipError::CaptureProcess(format!("cannot determine own exe path: {}", e))
        })?
        .parent()
        .ok_or_else(|| {
            SnipError::CaptureProcess("exe path has no parent directory".to_string())
        })?
        .to_path_buf();

    let capture_path = exe_dir.join(CAPTURE_EXE_NAME);

    if !capture_path.exists() {
        return Err(SnipError::CaptureProcess(format!(
            "{} not found at {}",
            CAPTURE_EXE_NAME,
            capture_path.display()
        )));
    }

    debug!(
        "locate_capture_exe: found at {}",
        capture_path.display()
    );

    Ok(capture_path)
}
