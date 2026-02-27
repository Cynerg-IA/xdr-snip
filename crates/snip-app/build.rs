/// Build script for snip-app.
///
/// Placeholder — will embed `assets/icon.ico` as a Windows resource
/// once the icon file is available.  For now this is a no-op so the
/// build stays green.
fn main() {
    // TODO: embed icon.ico as RT_ICON resource via winres or embed-resource crate
    // Example (once icon exists):
    //
    // let mut res = winres::WindowsResource::new();
    // res.set_icon("../../assets/icon.ico");
    // res.compile().expect("failed to compile Windows resource");
}
