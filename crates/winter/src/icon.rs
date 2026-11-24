//! The window's icon, drawn rather than shipped.
//!
//! Upstream embeds `.ico` resources through the app packaging; this port has no
//! resource pipeline, and a binary asset in the tree would be the only one
//! nobody can read. The pixels are computed instead (`icon_draw`), the Windows
//! build embeds the same drawing in the executable, and the macOS bundle draws
//! its iconset from the same geometry (ADR 058).

#[path = "icon_draw.rs"]
mod draw;

/// The size of the window icon, in pixels.
pub use draw::SIZE;

/// The icon's pixels at `size`, as straight RGBA.
///
/// The `.app` bundle's iconset is drawn at each size macOS asks for, from the
/// same geometry as the 64-pixel window icon (ADR 058).
pub fn rgba(size: usize) -> Vec<u8> {
    draw::rgba(size)
}
