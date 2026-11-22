//! The WinUI 3 shell.
//!
//! [`surface`] is the only module allowed `unsafe`: it creates the D3D11 device
//! and composition swap chain the `SwapChainPanel` shows, and blits the software
//! renderer's pixels with Direct2D (ADRs 040, 041).

#[allow(unsafe_code)]
pub mod surface;

#[allow(unsafe_code)]
mod wheel;

mod app;

pub use app::run;
