//! `LineRendition` — the DECDHL/DECSWL/DECDWL line renditions.
//!
//! Ported from `buffer/out/LineRendition.hpp`. This is the rendition a row
//! carries, not the screen/buffer coordinate conversion that shares its name
//! in the buffer module.

/// The width and height a row is rendered at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineRendition {
    /// Normal width and height.
    #[default]
    SingleWidth,
    /// Double width, single height.
    DoubleWidth,
    /// Double width, top half of a double-height line.
    DoubleHeightTop,
    /// Double width, bottom half of a double-height line.
    DoubleHeightBottom,
}
