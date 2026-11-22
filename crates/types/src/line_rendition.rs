//! `LineRendition` — the DECDHL/DECSWL/DECDWL line renditions.
//!
//! Ported from `buffer/out/LineRendition.hpp`: the rendition a row carries,
//! and the screen/buffer coordinate conversions that scale double-width lines.

use utils::{InclusiveRect, Point};

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

impl LineRendition {
    /// Returns whether the line is drawn at double width.
    pub const fn is_double_width(&self) -> bool {
        !matches!(self, Self::SingleWidth)
    }
}

/// Converts an inclusive screen rectangle to buffer columns.
///
/// Ported from `ScreenToBufferLine`; double-width lines halve their columns.
pub fn screen_to_buffer_line(line: InclusiveRect, rendition: LineRendition) -> InclusiveRect {
    let scale = if rendition.is_double_width() { 1 } else { 0 };
    InclusiveRect::new(line.left() >> scale, line.top(), line.right() >> scale, line.bottom())
}

/// Converts an inclusive screen point to a buffer column.
///
/// Ported from `ScreenToBufferLineInclusive`.
pub fn screen_to_buffer_line_inclusive(line: Point, rendition: LineRendition) -> Point {
    let scale = if rendition.is_double_width() { 1 } else { 0 };
    Point::new(line.x >> scale, line.y)
}

/// Converts an inclusive buffer rectangle to screen columns.
///
/// Ported from the inclusive `BufferToScreenLine`: the right edge gains one
/// column for the doubled glyph.
pub fn buffer_to_screen_line(line: InclusiveRect, rendition: LineRendition) -> InclusiveRect {
    let scale = if rendition.is_double_width() { 1 } else { 0 };
    InclusiveRect::new(
        line.left() << scale,
        line.top(),
        (line.right() << scale) + scale,
        line.bottom(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_width_conversions_are_identity() {
        let rect = InclusiveRect::new(1, 2, 9, 3);
        assert_eq!(screen_to_buffer_line(rect, LineRendition::SingleWidth), rect);
        assert_eq!(buffer_to_screen_line(rect, LineRendition::SingleWidth), rect);
        assert_eq!(
            screen_to_buffer_line_inclusive(Point::new(7, 2), LineRendition::SingleWidth),
            Point::new(7, 2)
        );
    }

    #[test]
    fn double_width_halves_screen_columns_and_doubles_buffer_columns() {
        let rect = InclusiveRect::new(4, 2, 19, 3);
        assert_eq!(
            screen_to_buffer_line(rect, LineRendition::DoubleWidth),
            InclusiveRect::new(2, 2, 9, 3)
        );
        assert_eq!(
            buffer_to_screen_line(InclusiveRect::new(2, 2, 9, 3), LineRendition::DoubleWidth),
            InclusiveRect::new(4, 2, 19, 3)
        );
        assert_eq!(
            screen_to_buffer_line_inclusive(Point::new(19, 2), LineRendition::DoubleWidth),
            Point::new(9, 2)
        );
    }

    #[test]
    fn the_double_height_renditions_scale_like_double_width() {
        for rendition in [LineRendition::DoubleHeightTop, LineRendition::DoubleHeightBottom] {
            assert!(rendition.is_double_width());
            assert_eq!(
                screen_to_buffer_line_inclusive(Point::new(8, 1), rendition),
                Point::new(4, 1)
            );
        }
        assert!(!LineRendition::SingleWidth.is_double_width());
    }
}
