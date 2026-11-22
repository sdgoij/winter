//! The WinUI 3 shell.
//!
//! Two modules carry `unsafe`: [`surface`] creates the D3D11 device, the
//! composition swap chain and the Direct2D blit (ADR 041), and `wheel` reads the
//! mouse wheel from a low-level hook because the binding reports no wheel event
//! (ADR 047). Everything else here is safe Rust.

#[allow(unsafe_code)]
pub mod surface;

#[allow(unsafe_code)]
mod wheel;

mod app;
mod settings;

pub use app::run;
