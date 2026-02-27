/// Build script for snip-app.
///
/// Sets up rerun-if-changed for the embedded capture-hdr.exe binary.
/// The C# exe must be built first (via build.ps1 or manually) and placed
/// in the `dist/` directory at the workspace root.
fn main() {
    // Re-run if the embedded capture helper changes
    println!("cargo:rerun-if-changed=../../dist/capture-hdr.exe");

    // TODO: embed icon.ico as RT_ICON resource via winres or embed-resource crate
}
