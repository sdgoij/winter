//! # winterm-renderer
//!
//! Renderer foundation ported from Microsoft's Windows Terminal C++ renderer.
//!
//! This crate holds the platform-independent vocabulary a render engine needs:
//! how a font is described, how a CSS length resolves to pixels, what a glyph
//! cluster covers, and the cursor state to draw. The Windows-only back ends
//! (D2D, D3D, GDI) are not ported; neither is the atlas engine.
//!
//! ## Modules
//!
//! - [`cluster`] — `Cluster`, a shaped glyph cluster and its column count
//! - [`css_length`] — `CSSLengthPercentage` parsing and resolution
//! - [`cursor_options`] — `CursorOptions` for drawing the cursor
//! - [`font_info_base`] — `FontInfoBase` face name, family, weight, code page
//! - [`font_info_desired`] — `FontInfoDesired` requested cell metrics
//! - [`font_info`] — `FontInfo` measured cell metrics

pub mod cluster;
pub mod css_length;
pub mod cursor_options;
pub mod font_info;
pub mod font_info_base;
pub mod font_info_desired;
pub mod render_settings;

pub use cluster::Cluster;
pub use css_length::{CSSLengthPercentage, ReferenceFrame};
pub use cursor_options::CursorOptions;
pub use font_info::{CellSizeInDIP, FontInfo};
pub use font_info_base::{DEFAULT_RASTER_FONT_FACENAME, DEFAULT_TT_FONT_FACENAME, FontInfoBase, LF_FACESIZE};
pub use font_info_desired::FontInfoDesired;
pub use render_settings::{INVALID_COLOR, Mode, RenderSettings};
