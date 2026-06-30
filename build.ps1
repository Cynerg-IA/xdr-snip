# build.ps1 — Build XDR Snip
# Usage: .\build.ps1 [-Release]

param(
    [switch]$Release
)

$ErrorActionPreference = "Stop"
$config = if ($Release) { "Release" } else { "Debug" }
$distDir = Join-Path $PSScriptRoot "dist"

# Read version from crates\snip-app\Cargo.toml
$verLine = Select-String -Path (Join-Path $PSScriptRoot 'crates\snip-app\Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (!$verLine) {
    Write-Host "ERROR: Could not find version in crates\snip-app\Cargo.toml" -ForegroundColor Red
    exit 1
}
$version = $verLine.Matches[0].Groups[1].Value

Write-Host "=== XDR Snip Build ===" -ForegroundColor Cyan
Write-Host "Configuration: $config"
Write-Host "Version: $version"

# Ensure dist directory exists
if (!(Test-Path $distDir)) {
    New-Item -ItemType Directory -Path $distDir | Out-Null
}

# Build Rust application (single exe - no C# dependency)
Write-Host "`n--- Building xdr-snip (Rust) ---" -ForegroundColor Yellow

if ($Release) {
    cargo build --release --manifest-path (Join-Path $PSScriptRoot "Cargo.toml")
    $rustExe = Join-Path $PSScriptRoot "target\release\xdr-snip.exe"
} else {
    cargo build --manifest-path (Join-Path $PSScriptRoot "Cargo.toml")
    $rustExe = Join-Path $PSScriptRoot "target\debug\xdr-snip.exe"
}

if ($LASTEXITCODE -ne 0) {
    Write-Host "Rust build FAILED" -ForegroundColor Red
    exit 1
}

# Copy Rust exe to dist
if (Test-Path $rustExe) {
    Copy-Item $rustExe (Join-Path $distDir "xdr-snip-v$version.exe") -Force
}
Write-Host "xdr-snip: OK" -ForegroundColor Green

# Copy default config if not present
$distConfig = Join-Path $distDir "config.toml"
if (!(Test-Path $distConfig)) {
    Copy-Item (Join-Path $PSScriptRoot "config.toml") $distConfig
}

# Summary
Write-Host "`n=== Build Complete ===" -ForegroundColor Cyan
Write-Host "Output: $distDir"
Get-ChildItem $distDir -Filter "*.exe" | ForEach-Object {
    $sizeMB = [math]::Round($_.Length / 1MB, 2)
    Write-Host "  $($_.Name) - ${sizeMB} MB"
}
