//! `Cursor` — cursor position, visibility, and style for the terminal.
//!
//! Ported from `Cursor` in the Windows Terminal C++ buffer module.

use types::CursorType;

/// Terminal cursor state.
///
/// Ported from `Cursor` in the Windows Terminal C++ buffer module.
/// Tracks position, visibility, blinking state, and cursor shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Current row position (0-based).
    row: i32,
    /// Current column position (0-based).
    column: i32,
    /// Whether the cursor is visible.
    visible: bool,
    /// Whether the cursor is blinking.
    blinking: bool,
    /// The shape of the cursor.
    shape: CursorType,
    /// Width of the cursor in pixels (for non-rectangular cursors).
    width: i32,
    /// Whether the cursor is drawn double-size to show insert/overtype mode.
    double: bool,
    /// Where the cursor sat when the end of the line was reached.
    ///
    /// The wrap itself happens on the next character written, not on the one
    /// that filled the last column, so the terminal can still overwrite that
    /// column before the line breaks. Ported from `Cursor::DelayEOLWrap`.
    delayed_eol_wrap: Option<(i32, i32)>,
}

impl Cursor {
    /// Creates a default cursor (hidden, at origin).
    pub const fn new() -> Self {
        Self {
            row: 0,
            column: 0,
            // Upstream's `Cursor` starts visible; a fresh terminal shows one.
            visible: true,
            blinking: false,
            shape: CursorType::Legacy,
            width: 0,
            double: false,
            delayed_eol_wrap: None,
        }
    }

    /// Creates a visible cursor at the given position.
    pub const fn visible_at(row: i32, column: i32) -> Self {
        Self {
            row,
            column,
            visible: true,
            ..Self::new()
        }
    }

    /// Returns the current row position.
    pub const fn row(&self) -> i32 {
        self.row
    }

    /// Returns the current column position.
    pub const fn column(&self) -> i32 {
        self.column
    }

    /// Returns the cursor position as a point.
    pub const fn position(&self) -> (i32, i32) {
        (self.row, self.column)
    }

    /// Sets the cursor position.
    ///
    /// A moved cursor no longer wraps where it used to, so the delayed-wrap
    /// marker is cleared here rather than left to go stale.
    pub fn set_position(&mut self, row: i32, column: i32) {
        self.row = row;
        self.column = column;
        self.delayed_eol_wrap = None;
    }

    /// Marks the current position as the place the next character should wrap.
    pub fn delay_eol_wrap(&mut self) {
        self.delayed_eol_wrap = Some((self.row, self.column));
    }

    /// Returns the position recorded by [`Cursor::delay_eol_wrap`], if any.
    pub const fn delayed_eol_wrap(&self) -> Option<(i32, i32)> {
        self.delayed_eol_wrap
    }

    /// Clears the delayed-wrap marker.
    pub fn reset_delayed_eol_wrap(&mut self) {
        self.delayed_eol_wrap = None;
    }

    /// Moves the cursor to the given position.
    pub fn move_to(&mut self, row: i32, column: i32) {
        self.set_position(row, column);
    }

    /// Moves the cursor down by the given number of rows.
    pub fn move_down(&mut self, count: i32) {
        self.row += count;
    }

    /// Moves the cursor up by the given number of rows.
    pub fn move_up(&mut self, count: i32) {
        self.row -= count;
    }

    /// Moves the cursor right by the given number of columns.
    pub fn move_right(&mut self, count: i32) {
        self.column += count;
    }

    /// Moves the cursor left by the given number of columns.
    pub fn move_left(&mut self, count: i32) {
        self.column -= count;
    }

    /// Moves the cursor to the beginning of the current line.
    pub fn move_to_line_start(&mut self) {
        self.column = 0;
    }

    /// Moves the cursor to the end of the current line.
    pub fn move_to_line_end(&mut self, line_width: i32) {
        self.column = line_width - 1;
    }

    /// Returns whether the cursor is visible.
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    /// Sets the cursor visibility.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Toggles the cursor visibility.
    pub fn toggle_visibility(&mut self) {
        self.visible = !self.visible;
    }

    /// Returns whether the cursor is blinking.
    pub const fn is_blinking(&self) -> bool {
        self.blinking
    }

    /// Sets the cursor blinking state.
    pub fn set_blinking(&mut self, blinking: bool) {
        self.blinking = blinking;
    }

    /// Returns the cursor shape.
    pub const fn shape(&self) -> CursorType {
        self.shape
    }

    /// Sets the cursor shape.
    pub fn set_shape(&mut self, shape: CursorType) {
        self.shape = shape;
    }

    /// Returns the cursor width in pixels.
    pub const fn width(&self) -> i32 {
        self.width
    }

    /// Sets the cursor width in pixels.
    pub fn set_width(&mut self, width: i32) {
        self.width = width;
    }

    /// Returns whether the cursor is drawn double-size for insert/overtype.
    pub const fn is_double(&self) -> bool {
        self.double
    }

    /// Draws the cursor double-size for insert/overtype, as upstream's
    /// `SetCursorDBMode` does.
    pub fn set_double(&mut self, double: bool) {
        self.double = double;
    }

    /// Clamps the cursor position to be within buffer bounds.
    pub fn clamp_to_bounds(&mut self, max_row: i32, max_col: i32) {
        self.row = self.row.clamp(0, max_row - 1);
        self.column = self.column.clamp(0, max_col - 1);
    }

    /// Clamps the cursor position to be within the given bounds.
    pub fn clamp(&mut self, max_row: i32, max_col: i32) {
        self.clamp_to_bounds(max_row, max_col);
    }

    /// Returns whether the cursor is at the origin.
    pub const fn is_at_origin(&self) -> bool {
        self.row == 0 && self.column == 0
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_cursor() {
        let cursor = Cursor::new();
        assert_eq!(cursor.row(), 0);
        assert_eq!(cursor.column(), 0);
        assert!(cursor.is_visible(), "upstream starts with a visible cursor");
        assert!(!cursor.is_blinking());
    }

    #[test]
    fn test_visible_cursor() {
        let cursor = Cursor::visible_at(5, 10);
        assert_eq!(cursor.row(), 5);
        assert_eq!(cursor.column(), 10);
        assert!(cursor.is_visible());
    }

    #[test]
    fn test_move_cursor() {
        let mut cursor = Cursor::new();
        cursor.move_to(10, 20);
        assert_eq!(cursor.position(), (10, 20));

        cursor.move_down(3);
        assert_eq!(cursor.row(), 13);

        cursor.move_up(5);
        assert_eq!(cursor.row(), 8);

        cursor.move_right(4);
        assert_eq!(cursor.column(), 24);

        cursor.move_left(2);
        assert_eq!(cursor.column(), 22);
    }

    #[test]
    fn test_clamp() {
        let mut cursor = Cursor::new();
        cursor.set_position(-5, -5);
        cursor.clamp_to_bounds(10, 80);
        assert_eq!(cursor.row(), 0);
        assert_eq!(cursor.column(), 0);

        cursor.set_position(100, 200);
        cursor.clamp_to_bounds(10, 80);
        assert_eq!(cursor.row(), 9);
        assert_eq!(cursor.column(), 79);
    }

    #[test]
    fn test_blinking() {
        let mut cursor = Cursor::new();
        assert!(!cursor.is_blinking());
        cursor.set_blinking(true);
        assert!(cursor.is_blinking());
    }

    #[test]
    fn test_visibility_toggle() {
        let mut cursor = Cursor::new();
        cursor.toggle_visibility();
        assert!(!cursor.is_visible());
        cursor.toggle_visibility();
        assert!(cursor.is_visible());
    }

    #[test]
    fn test_line_navigation() {
        let mut cursor = Cursor::new();
        cursor.set_position(5, 40);
        cursor.move_to_line_start();
        assert_eq!(cursor.column(), 0);
        assert_eq!(cursor.row(), 5);

        cursor.move_to_line_end(80);
        assert_eq!(cursor.column(), 79);
    }

    #[test]
    fn test_cursor_at_origin() {
        let cursor = Cursor::new();
        assert!(cursor.is_at_origin());

        let mut cursor2 = Cursor::new();
        cursor2.move_right(1);
        assert!(!cursor2.is_at_origin());
    }

    #[test]
    fn test_cursor_shape() {
        let mut cursor = Cursor::new();
        assert_eq!(cursor.shape(), CursorType::Legacy);

        cursor.set_shape(CursorType::VerticalBar);
        assert_eq!(cursor.shape(), CursorType::VerticalBar);

        cursor.set_shape(CursorType::FullBox);
        assert_eq!(cursor.shape(), CursorType::FullBox);
    }

    #[test]
    fn a_cursor_can_be_drawn_double_size() {
        let mut cursor = Cursor::new();
        assert!(!cursor.is_double());
        cursor.set_double(true);
        assert!(cursor.is_double());
        cursor.set_double(false);
        assert!(!cursor.is_double());
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn test_clamp_keeps_within_bounds(row in any::<i32>(), col in any::<i32>(), max_row in 1i32..1000i32, max_col in 1i32..1000i32) {
            let mut cursor = Cursor::new();
            cursor.set_position(row, col);
            cursor.clamp_to_bounds(max_row, max_col);
            prop_assert!((0..max_row).contains(&cursor.row()));
            prop_assert!((0..max_col).contains(&cursor.column()));
        }

        #[test]
        fn test_move_down_up_cancel(delta in 0i32..100i32) {
            let mut cursor = Cursor::new();
            cursor.move_down(delta);
            cursor.move_up(delta);
            prop_assert_eq!(cursor.row(), 0);
        }

        #[test]
        fn test_move_left_right_cancel(col in 0i32..100i32, delta in 0i32..100i32) {
            let mut cursor = Cursor::new();
            cursor.move_to(0, col);
            cursor.move_right(delta);
            cursor.move_left(delta);
            prop_assert_eq!(cursor.column(), col);
        }
    }
}
