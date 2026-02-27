//! Screenshot capture via embedded `capture-hdr.exe` subprocess.
//!
//! The HDR capture logic lives in a C# executable that handles HDR → SDR
//! tone-mapping via Windows.Graphics.Capture API. This module embeds the
//! C# binary at compile time, extracts it on first run, and invokes it
//! as a subprocess with no visible console window.

use std::os::windows::process::CommandExt;
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

/// Win32 CREATE_NO_WINDOW flag — prevents the subprocess from opening a
/// visible console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Captures a screen region by invoking the embedded `capture-hdr.exe` tool.
///
/// On first call, extracts the embedded binary next to the running executable.
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

    // Spawn the capture helper with no visible console window
    let result = Command::new(&capture_exe)
        .args(&args)
        .creation_flags(CREATE_NO_WINDOW)
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

// ======================== EXTRACTION ========================

/// Ensures the embedded `capture-hdr.exe` is extracted and ready to run.
///
/// Tries multiple candidate directories in order:
/// 1. Same directory as the running `hdr-snip.exe` (portable / Program Files)
/// 2. `%LOCALAPPDATA%\Programs\hdr-snip\` (per-user install location)
/// 3. `%APPDATA%\hdr-snip\` (legacy fallback)
///
/// Extracts on first run or when the embedded binary size differs from the
/// existing file (i.e., after an update).
fn ensure_capture_exe_extracted() -> Result<PathBuf, SnipError> {
    let candidates = extraction_candidates();

    if candidates.is_empty() {
        return Err(SnipError::CaptureProcess(
            "cannot determine any extraction directory".to_string(),
        ));
    }

    for dir in &candidates {
        match try_extract_in(dir) {
            Ok(path) => return Ok(path),
            Err(e) => {
                debug!(
                    "ensure_capture_exe_extracted: {} failed: {}",
                    dir.display(),
                    e
                );
            }
        }
    }

    Err(SnipError::CaptureProcess(format!(
        "cannot extract {} to any location (tried {} dirs)",
        CAPTURE_EXE_NAME,
        candidates.len()
    )))
}

/// Builds the list of candidate directories for extracting the capture helper.
fn extraction_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::with_capacity(3);

    // Primary: same directory as the running executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.to_path_buf());
        }
    }

    // Fallback: %LOCALAPPDATA%\Programs\hdr-snip
    if let Some(local_data) = dirs::data_local_dir() {
        candidates.push(local_data.join("Programs").join("hdr-snip"));
    }

    // Last resort: %APPDATA%\hdr-snip
    if let Some(config) = dirs::config_dir() {
        candidates.push(config.join("hdr-snip"));
    }

    debug!(
        "extraction_candidates: {} candidates",
        candidates.len()
    );

    candidates
}

/// Attempts to extract the embedded capture-hdr.exe into `dir`.
///
/// Creates the directory if needed. Skips extraction if the file already exists
/// and the size matches (same version).
fn try_extract_in(dir: &Path) -> Result<PathBuf, SnipError> {
    std::fs::create_dir_all(dir).map_err(|e| {
        SnipError::CaptureProcess(format!("cannot create {}: {}", dir.display(), e))
    })?;

    let capture_path = dir.join(CAPTURE_EXE_NAME);

    // Check if extraction is needed (missing or size mismatch = new version)
    let needs_extract = if capture_path.exists() {
        let existing_size = capture_path
            .metadata()
            .map(|m| m.len())
            .unwrap_or(0);
        let embedded_size = EMBEDDED_CAPTURE_EXE.len() as u64;

        if existing_size != embedded_size {
            debug!(
                "try_extract_in: size mismatch at {} (existing={}, embedded={})",
                capture_path.display(),
                existing_size,
                embedded_size
            );
            true
        } else {
            false
        }
    } else {
        debug!(
            "try_extract_in: not found at {}, extracting",
            capture_path.display()
        );
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
            "try_extract_in: extracted {} bytes to {}",
            EMBEDDED_CAPTURE_EXE.len(),
            capture_path.display()
        );
    } else {
        debug!(
            "try_extract_in: up to date at {}",
            capture_path.display()
        );
    }

    Ok(capture_path)
}
