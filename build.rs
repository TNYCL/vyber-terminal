//! Embeds the app icon on Windows. GPUI gives the window and the taskbar
//! button icon resource 1 of the executable, and Explorer shows it for
//! `Vyber.exe` too; without it every build ran with the default icon.
fn main() {
    println!("cargo:rerun-if-changed=assets/vyber.rc");
    println!("cargo:rerun-if-changed=assets/vyber.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/vyber.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("the app icon could not be embedded");
    }
}
