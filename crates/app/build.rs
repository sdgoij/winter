//! Embeds the window icon in the Windows executable.
//!
//! The pixels come from the `winter-icon` crate, the same drawing the shells
//! hand their window manager and the macOS bundle draws its iconset from, so
//! the file Explorer shows and the shell's icon cannot drift (ADR 058). The
//! crate has no dependencies, which is why a build script can compile it.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../icon/src/lib.rs");

    #[cfg(windows)]
    embed();
}

/// Generates the ICO and links it into the binary.
#[cfg(windows)]
fn embed() {
    use std::path::PathBuf;

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let icon = out.join("winter.ico");
    std::fs::write(&icon, winter_icon::ico(&[16, 32, 48, 64, 256])).expect("write the icon");

    let resource = out.join("winter.rc");
    let icon_path = icon.display().to_string().replace('\\', "\\\\");
    std::fs::write(&resource, format!("1 ICON \"{icon_path}\"\n")).expect("write the resource script");

    embed_resource::compile(&resource, embed_resource::NONE)
        .manifest_optional()
        .expect("embed the icon");
}
