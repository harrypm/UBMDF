// Embeds the app icon into Windows executables (Explorer / taskbar / shortcut).
// Other targets get their window icon at runtime from the GUI (raylib).
// Temporary art — regenerate with tools/make-icon.ps1.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon/ubdmf.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/ubdmf.ico");
        if let Err(e) = res.compile() {
            // Never fail the build over an icon (e.g. no Windows SDK rc.exe).
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
}
