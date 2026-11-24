//! The WinUI 3 shell.
//!
//! Four modules carry `unsafe`: [`surface`] creates the D3D11 device, the
//! composition swap chain and the Direct2D blit (ADR 041), `wheel` reads the
//! mouse wheel from a low-level hook because the binding reports no wheel event
//! (ADR 047), `icon` loads the embedded icon and hands it to the window
//! (ADR 058), and `profile_icon` asks GDI for a program's icon (ADR 066).
//! Everything else here is safe Rust.

#[allow(unsafe_code)]
pub mod surface;

#[allow(unsafe_code)]
mod gpu;

#[allow(unsafe_code)]
mod wheel;

#[allow(unsafe_code)]
mod icon;

#[allow(unsafe_code)]
mod profile_icon;

mod scrollbar;

mod app;
mod menu;
mod png;
mod settings;
mod wsl;

pub use app::run;
