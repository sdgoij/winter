//! Embeds the window icon in the Windows executable.
//!
//! The pixels come from `src/icon_draw.rs`, the same drawing the egui shell
//! hands its window manager, so the file Explorer shows and the window icon
//! cannot drift (ADR 058). The ICO is generated into `OUT_DIR` and compiled
//! with the rest of the resources.

#[path = "src/icon_draw.rs"]
#[allow(dead_code)]
mod icon_draw;

fn main() {
    println!("cargo:rerun-if-changed=src/icon_draw.rs");
    println!("cargo:rerun-if-changed=build.rs");

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
    let icon = out.join("winterm.ico");
    std::fs::write(&icon, icon_draw::ico(&[16, 32, 48, 64, 256])).expect("write the icon");

    let resource = out.join("winterm.rc");
    let icon_path = icon.display().to_string().replace('\\', "\\\\");
    std::fs::write(&resource, format!("1 ICON \"{icon_path}\"\n")).expect("write the resource script");

    embed_resource::compile(&resource, embed_resource::NONE)
        .manifest_optional()
        .expect("embed the icon");
}
