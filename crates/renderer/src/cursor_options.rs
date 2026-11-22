//! `CursorOptions` — the state a render engine needs to draw the cursor.
//!
//! Ported from `CursorOptions.h`.

use types::{CursorType, LineRendition};
use utils::Point;

/// The cursor state a render engine reads when painting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CursorOptions {
    /// Cell in the grid to draw at, relative to the top of the viewport.
    pub coord_cursor: Point,
    /// Left offset of the viewport, which shifts the horizontal position.
    pub viewport_left: i32,
    /// Line rendition of the current row, which can change the cursor width.
    pub line_rendition: LineRendition,
    /// Height of an underscore cursor as a percentage of the cell height.
    pub cursor_height_percent: u32,
    /// Width of a vertical-bar cursor in pixels.
    pub cursor_pixel_width: u32,
    /// Whether to draw the cursor two cells wide.
    pub is_double_width: bool,
    /// The cursor shape to draw.
    pub cursor_type: CursorType,
    /// Whether to use [`Self::cursor_color`] instead of the default colour.
    pub use_color: bool,
    /// Colour to draw the cursor with when [`Self::use_color`] is set.
    pub cursor_color: u32,
    /// Whether the application has asked for the cursor to be shown.
    pub is_visible: bool,
    /// Whether the blink phase is currently on.
    pub is_on: bool,
    /// Whether the cursor lies inside the renderer's viewport.
    pub in_viewport: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_a_legacy_cursor_at_the_origin() {
        let options = CursorOptions::default();
        assert_eq!(options.coord_cursor, Point::new(0, 0));
        assert_eq!(options.cursor_type, CursorType::Legacy);
        assert_eq!(options.line_rendition, LineRendition::SingleWidth);
        assert!(!options.is_visible);
        assert!(!options.use_color);
    }

    #[test]
    fn a_recolour_is_read_back() {
        let options = CursorOptions {
            coord_cursor: Point::new(3, 4),
            cursor_type: CursorType::FullBox,
            use_color: true,
            cursor_color: 0x00FF_0000,
            is_visible: true,
            is_on: true,
            ..CursorOptions::default()
        };

        assert_eq!(options.coord_cursor, Point::new(3, 4));
        assert_eq!(options.cursor_type, CursorType::FullBox);
        assert!(options.use_color);
        assert_eq!(options.cursor_color, 0x00FF_0000);
    }
}
