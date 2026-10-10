//! Embeds version information and the icon in notify-send.exe (Properties →
//! Details, Task Manager, Explorer).

fn main() {
    // The resources come from Cargo.toml and the icon; nothing else affects the
    // output. Without these lines, cargo would rerun this script whenever any
    // file changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=assets/notify-send.ico");

    // Needs the resource compiler from the Windows SDK, which MSVC builds already
    // require. Other toolchains (e.g. windows-gnu) build without the resources.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/notify-send.ico")
        .set("FileDescription", "notify-send - desktop notifications")
        .set("ProductName", "notify-send for Windows")
        .set("InternalName", "notify-send")
        .set("OriginalFilename", "notify-send.exe")
        .set(
            "LegalCopyright",
            "Copyright (c) 2026 Metin Ata YETER. MIT License.",
        );
    if let Err(e) = resource.compile() {
        panic!("failed to embed Windows resources: {e}");
    }
}
