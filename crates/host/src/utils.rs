//! Console utilities — window arithmetic, decimal parsing and coordinate
//! helpers shared by the host.
//!
//! Ported from `src/host/utils.cpp`. `_LoadString` and `s_LoadStringEx` load
//! Win32 string resources through the window layer and are not portable, so
//! they stay out of this module.
//!
//! `s_CompareCoords` and `s_GetOppositeCorner` are the helpers selection has
//! been using inline; they live here now so the rule has one home.

use types::Viewport;
use utils::{InclusiveRect, Point, Size};

/// Returns the width of an inclusive rectangle.
///
/// Ported from `CalcWindowSizeX`.
pub fn calc_window_size_x(rect: InclusiveRect) -> i32 {
    rect.width()
}

/// Returns the height of an inclusive rectangle.
///
/// Ported from `CalcWindowSizeY`.
pub fn calc_window_size_y(rect: InclusiveRect) -> i32 {
    rect.height()
}

/// Returns how far the cursor's top sits below the top of a font cell.
///
/// Ported from `CalcCursorYOffsetInPixels`.
pub fn calc_cursor_y_offset_in_pixels(font_size_y: i32, size: u32) -> i32 {
    font_size_y - size as i32
}

/// Parses a leading run of decimal digits.
///
/// Ported from `ConvertStringToDec`: the value wraps like the C++ `WORD`, and
/// the second element is the input from the first non-digit on, upstream's
/// `ppwchEnd`.
pub fn convert_string_to_dec(text: &str) -> (u16, &str) {
    let mut value: u16 = 0;

    for (offset, character) in text.char_indices() {
        let Some(digit) = character.to_digit(10) else {
            return (value, &text[offset..]);
        };
        value = value.wrapping_mul(10).wrapping_add(digit as u16);
    }

    (value, "")
}

/// Compares two positions in reading order within a buffer.
///
/// Ported from `Utils::s_CompareCoords`: negative when `first` comes before
/// `second`, zero when they are the same cell, positive when it comes after.
/// The magnitude is the distance in cells.
pub fn compare_coords(buffer_size: Size, first: Point, second: Point) -> i32 {
    Viewport::new(buffer_size).compare_in_bounds(first, second)
}

/// Returns the corner of `rect` opposite `corner`.
///
/// Ported from `Utils::s_GetOppositeCorner`. `corner` must be one of the
/// rectangle's corners; upstream asserts that.
pub fn get_opposite_corner(rect: InclusiveRect, corner: Point) -> Point {
    debug_assert!(
        corner.x == rect.left() || corner.x == rect.right(),
        "corner x must be a vertical edge of the rectangle"
    );
    debug_assert!(
        corner.y == rect.top() || corner.y == rect.bottom(),
        "corner y must be a horizontal edge of the rectangle"
    );

    Point::new(
        if rect.left() == corner.x {
            rect.right()
        } else {
            rect.left()
        },
        if rect.top() == corner.y {
            rect.bottom()
        } else {
            rect.top()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_sizes_count_both_inclusive_edges() {
        let rect = InclusiveRect::new(10, 20, 19, 29);
        assert_eq!(calc_window_size_x(rect), 10);
        assert_eq!(calc_window_size_y(rect), 10);

        let single = InclusiveRect::new(5, 5, 5, 5);
        assert_eq!(calc_window_size_x(single), 1);
        assert_eq!(calc_window_size_y(single), 1);
    }

    #[test]
    fn the_cursor_offset_is_the_font_height_less_the_cursor_size() {
        assert_eq!(calc_cursor_y_offset_in_pixels(16, 4), 12);
        assert_eq!(calc_cursor_y_offset_in_pixels(12, 0), 12);
    }

    #[test]
    fn decimal_parsing_stops_at_the_first_non_digit() {
        assert_eq!(convert_string_to_dec("437"), (437, ""));
        assert_eq!(convert_string_to_dec("437abc"), (437, "abc"));
        assert_eq!(convert_string_to_dec(""), (0, ""));
        assert_eq!(convert_string_to_dec("abc"), (0, "abc"));
        assert_eq!(convert_string_to_dec("007"), (7, ""));
    }

    #[test]
    fn decimal_parsing_wraps_like_a_word() {
        // 65536 * 10 + 9 wraps to 9.
        assert_eq!(convert_string_to_dec("655369").0, 9);
    }

    #[test]
    fn comparison_follows_reading_order() {
        let size = Size::new(80, 25);

        // Equal positions.
        assert_eq!(compare_coords(size, Point::new(5, 3), Point::new(5, 3)), 0);

        // Same row, left and right.
        assert_eq!(compare_coords(size, Point::new(4, 3), Point::new(5, 3)), -1);
        assert_eq!(compare_coords(size, Point::new(5, 3), Point::new(4, 3)), 1);

        // Same column, above and below.
        assert_eq!(compare_coords(size, Point::new(5, 2), Point::new(5, 3)), -80);
        assert_eq!(compare_coords(size, Point::new(5, 3), Point::new(5, 2)), 80);

        // Up and to the left, and down and to the right.
        assert!(compare_coords(size, Point::new(4, 2), Point::new(5, 3)) < 0);
        assert!(compare_coords(size, Point::new(5, 3), Point::new(4, 2)) > 0);

        // Up and to the right; the far edge of the row is still before the
        // next row's first cell.
        assert!(compare_coords(size, Point::new(79, 2), Point::new(0, 3)) < 0);
        assert!(compare_coords(size, Point::new(0, 3), Point::new(79, 2)) > 0);
    }

    #[test]
    fn the_opposite_corner_swaps_each_axis() {
        let rect = InclusiveRect::new(1, 2, 10, 20);

        assert_eq!(get_opposite_corner(rect, Point::new(1, 2)), Point::new(10, 20));
        assert_eq!(get_opposite_corner(rect, Point::new(10, 2)), Point::new(1, 20));
        assert_eq!(get_opposite_corner(rect, Point::new(1, 20)), Point::new(10, 2));
        assert_eq!(get_opposite_corner(rect, Point::new(10, 20)), Point::new(1, 2));
    }
}
