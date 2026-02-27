# build.ps1 — Build both HDR Snip components
# Usage: .\build.ps1 [-Release]

param(
    [switch]$Release
)

$ErrorActionPreference = "Stop"
$config = if ($Release) { "Release" } else { "Debug" }
$distDir = Join-Path $PSScriptRoot "dist"

Write-Host "=== HDR Snip Build ===" -ForegroundColor Cyan
Write-Host "Configuration: $config"

# Ensure dist directory exists
if (!(Test-Path $distDir)) {
    New-Item -ItemType Directory -Path $distDir | Out-Null
}

# Step 1: Build C# capture component
Write-Host "`n--- Building capture-hdr (C#) ---" -ForegroundColor Yellow
$captureDir = Join-Path $PSScriptRoot "capture-hdr"

if ($Release) {
    dotnet publish $captureDir -c Release -r win-x64 --self-contained -o $distDir
} else {
    dotnet build $captureDir -c $config
    $builtExe = Join-Path $captureDir "bin\$config\net8.0-windows10.0.22621.0\win-x64\capture-hdr.exe"
    if (Test-Path $builtExe) {
        Copy-Item $builtExe $distDir -Force
    }
}

if ($LASTEXITCODE -ne 0) {
    Write-Host "C# build FAILED" -ForegroundColor Red
    exit 1
}
Write-Host "capture-hdr: OK" -ForegroundColor Green

# Step 2: Build Rust main application
Write-Host "`n--- Building snip-app (Rust) ---" -ForegroundColor Yellow

if ($Release) {
    cargo build --release --manifest-path (Join-Path $PSScriptRoot "Cargo.toml")
    $rustExe = Join-Path $PSScriptRoot "target\release\snip-app.exe"
} else {
    cargo build --manifest-path (Join-Path $PSScriptRoot "Cargo.toml")
    $rustExe = Join-Path $PSScriptRoot "target\debug\snip-app.exe"
}

if ($LASTEXITCODE -ne 0) {
    Write-Host "Rust build FAILED" -ForegroundColor Red
    exit 1
}

# Copy Rust exe to dist as hdr-snip.exe
if (Test-Path $rustExe) {
    Copy-Item $rustExe (Join-Path $distDir "hdr-snip.exe") -Force
}
Write-Host "hdr-snip: OK" -ForegroundColor Green

# Step 3: Copy default config if not present
$distConfig = Join-Path $distDir "config.toml"
if (!(Test-Path $distConfig)) {
    Copy-Item (Join-Path $PSScriptRoot "config.toml") $distConfig
}

# Summary
Write-Host "`n=== Build Complete ===" -ForegroundColor Cyan
Write-Host "Output: $distDir"
Get-ChildItem $distDir -Filter "*.exe" | ForEach-Object {
    $sizeMB = [math]::Round($_.Length / 1MB, 2)
    Write-Host "  $($_.Name) — ${sizeMB} MB"
}
