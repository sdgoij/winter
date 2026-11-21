//! `Row` — a row of terminal buffer cells.
//!
//! Ported from `ROW` in the Windows Terminal C++ buffer module.
//! A row is a fixed-width array of `OutputCell` with methods for writing,
//! reading, and manipulating cell data.

use crate::TextAttribute;
use crate::output_cell::OutputCell;

/// A row of terminal cells with width and modification tracking.
///
/// Ported from `ROW` in the Windows Terminal C++ buffer module.
/// Stores a fixed number of OutputCells, each representing a character position
/// in the terminal display.
#[derive(Debug, Clone)]
pub struct Row {
    /// The cells in this row.
    cells: Vec<OutputCell>,
    /// The width of this row (number of columns).
    width: i32,
    /// Track if this row contains any non-space text.
    contains_text: bool,
    /// Track if the row was wrapped (last character caused a line wrap).
    wrap: bool,
    /// Number of display columns occupied by the last character (for wrapping).
    trailing_width: u8,
    /// Data used for scrollback scrollbar position.
    scrollbar_data: ScrollbarData,
    /// Whether the row has line rendition (scrollback buffer flag).
    has_line_rendition: bool,
}

/// Data for scrollbar positioning within a row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScrollbarData {
    /// The starting column of the scrollbar representation.
    pub start: i32,
    /// The ending column of the scrollbar representation.
    pub end: i32,
}

impl ScrollbarData {
    /// Creates new scrollbar data.
    pub const fn new(start: i32, end: i32) -> Self {
        Self { start, end }
    }
}

// TODO: Implement CharToColumnMapper (cell_width module)
// This maps character offsets to display column positions.

impl Row {
    /// Creates a new row with the given width, filled with empty cells.
    pub fn new(width: i32) -> Self {
        let cells = vec![OutputCell::new(); width as usize];
        Self {
            cells,
            width,
            contains_text: false,
            wrap: false,
            trailing_width: 0,
            scrollbar_data: ScrollbarData::default(),
            has_line_rendition: false,
        }
    }

    /// Returns the width of this row (number of columns).
    pub const fn width(&self) -> i32 {
        self.width
    }

    /// Returns the number of cells in this row.
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Returns the cell at the given column.
    ///
    /// # Panics
    /// Panics if `column` is out of bounds.
    pub fn get_cell(&self, column: i32) -> &OutputCell {
        let idx = self.validate_column(column);
        &self.cells[idx]
    }

    /// Returns a mutable reference to the cell at the given column.
    ///
    /// # Panics
    /// Panics if `column` is out of bounds.
    pub fn get_cell_mut(&mut self, column: i32) -> &mut OutputCell {
        let idx = self.validate_column(column);
        &mut self.cells[idx]
    }

    /// Returns the character data at the given column.
    pub fn get_chars_at(&self, column: i32) -> &str {
        self.get_cell(column).chars()
    }

    /// Writes a cell to a specific column, shifting existing content if needed.
    pub fn write_cell(&mut self, column: i32, cell: OutputCell) {
        let idx = self.validate_column(column);
        self.cells[idx] = cell;
        self.update_contains_text();
    }

    /// Writes cells starting at the given column.
    ///
    /// If the write extends beyond the row width, excess characters are
    /// shifted to earlier columns (for insert mode) or truncated (for replace mode).
    pub fn write_cells(&mut self, start_column: i32, cells: &[OutputCell], insert_mode: bool) {
        if cells.is_empty() {
            return;
        }

        if insert_mode {
            self.write_cells_insert(start_column, cells);
        } else {
            self.write_cells_replace(start_column, cells);
        }

        self.update_contains_text();
    }

    /// Writes a single character at the given column.
    pub fn write_char(&mut self, column: i32, c: char, insert_mode: bool) {
        let cell = OutputCell::from_char(c);
        if insert_mode {
            self.write_char_insert(column, cell);
        } else {
            let idx = self.validate_column(column);
            self.cells[idx] = cell;
            self.update_contains_text();
        }
    }

    /// Copies text from another row starting at the given source and destination columns.
    pub fn copy_text_from(&mut self, dest_col: i32, source: &Row, src_col: i32, count: i32) {
        let count = count.max(0) as usize;
        let dest_end = (dest_col + count as i32).min(self.width);
        let _src_end = (src_col + count as i32).min(source.width);
        let actual_count = (dest_end - dest_col) as usize;

        for i in 0..actual_count {
            let src_cell = source.get_cell(src_col + i as i32);
            self.write_cell(dest_col + i as i32, src_cell.clone());
        }

        self.update_contains_text();
    }

    /// Copies the entire content from another row.
    pub fn copy_from(&mut self, source: &Row) {
        for (i, cell) in source.cells.iter().enumerate() {
            if i < self.cells.len() {
                self.cells[i] = cell.clone();
            }
        }
        self.update_contains_text();
    }

    /// Clears a cell at the given column, filling it with empty cells.
    pub fn clear_cell(&mut self, column: i32) {
        let idx = self.validate_column(column);
        self.cells[idx] = OutputCell::new();
        self.update_contains_text();
    }

    /// Clears cells in a range.
    pub fn clear_range(&mut self, start_col: i32, end_col: i32) {
        let start = self.validate_column(start_col);
        let end_col = end_col.min(self.width);
        let end = self.validate_column(end_col);
        for i in start..end {
            self.cells[i] = OutputCell::new();
        }
        self.update_contains_text();
    }

    /// Sets the attribute of all cells from the given column to the end of the row.
    pub fn set_attr_to_end(&mut self, start_column: i32, attr: TextAttribute) {
        let start = self.validate_column(start_column);
        for cell in &mut self.cells[start..] {
            cell.set_attributes(attr);
        }
    }

    /// Replaces characters starting at the given column.
    pub fn replace_characters(&mut self, start_column: i32, replacement: &str) {
        let start = self.validate_column(start_column);
        let end = (self.width.max(0) as usize).min(self.cells.len());
        for (offset, c) in replacement.chars().take(end.saturating_sub(start)).enumerate() {
            self.cells[start + offset] = OutputCell::from_char(c);
        }
        self.update_contains_text();
    }

    /// Replaces text with the given text, padding with spaces if needed.
    pub fn replace_text(&mut self, start_column: i32, replacement: &str, pad_with_spaces: bool) {
        self.replace_characters(start_column, replacement);
        if pad_with_spaces {
            let start_col = start_column;
            let replacement_width = replacement.chars().map(|c| codepoint_width(c as u32)).sum::<u8>() as i32;
            let remaining = self.width - start_col - replacement_width;
            for i in 0..remaining {
                self.clear_cell(start_col + replacement_width + i);
            }
        }
        self.update_contains_text();
    }

    /// Gets the text content of this row as a string.
    pub fn get_text(&self) -> String {
        let mut result = String::new();
        for cell in &self.cells {
            result.push_str(cell.chars());
        }
        result
    }

    /// Gets the attribute at a specific column.
    pub fn get_attr_by_column(&self, column: i32) -> TextAttribute {
        *self.get_cell(column).get_attributes()
    }

    /// Sets the wrap flag for this row.
    pub fn set_wrap(&mut self, wrapped: bool) {
        self.wrap = wrapped;
    }

    /// Returns whether this row was wrapped (last character caused a line wrap).
    pub const fn is_wrap(&self) -> bool {
        self.wrap
    }

    /// Sets the trailing width (for wrapping calculations).
    pub fn set_trailing_width(&mut self, width: u8) {
        self.trailing_width = width;
    }

    /// Returns the trailing width.
    pub const fn get_trailing_width(&self) -> u8 {
        self.trailing_width
    }

    /// Returns whether the row contains any non-space text.
    pub const fn contains_text(&self) -> bool {
        self.contains_text
    }

    /// Returns the scrollbar data.
    pub const fn get_scrollbar_data(&self) -> ScrollbarData {
        self.scrollbar_data
    }

    /// Sets the scrollbar data.
    pub fn set_scrollbar_data(&mut self, data: ScrollbarData) {
        self.scrollbar_data = data;
    }

    /// Returns whether this row has line rendition.
    pub const fn has_line_rendition(&self) -> bool {
        self.has_line_rendition
    }

    /// Sets whether this row has line rendition.
    pub fn set_line_rendition(&mut self, has: bool) {
        self.has_line_rendition = has;
    }

    /// Gets the column index of the first non-space character from the left.
    pub fn measure_left(&self) -> i32 {
        for (i, cell) in self.cells.iter().enumerate() {
            if !cell.is_whitespace() {
                return i as i32;
            }
        }
        0
    }

    /// Gets the column index of the first non-space character from the right.
    pub fn measure_right(&self) -> i32 {
        for (i, cell) in self.cells.iter().enumerate().rev() {
            if !cell.is_whitespace() {
                return i as i32;
            }
        }
        0
    }

    /// Gets the column position of the last non-space character.
    pub fn get_last_non_space_column(&self) -> i32 {
        self.measure_right()
    }

    /// Gets the number of readable columns (non-whitespace).
    pub fn get_readable_column_count(&self) -> i32 {
        self.cells.iter().filter(|c| !c.is_whitespace()).count() as i32
    }

    /// Returns whether the row is entirely whitespace.
    pub fn is_all_spaces(&self) -> bool {
        self.cells.iter().all(|c| c.is_whitespace())
    }

    /// Forces the last character to wrap (sets wrap flag and trailing width).
    pub fn force_wrap(&mut self, trailing_width: u8) {
        self.set_wrap(true);
        self.set_trailing_width(trailing_width);
    }

    /// Checks if a double-byte character needs padding at a given column.
    pub fn needs_double_byte_padding(&self, column: i32) -> bool {
        if column >= self.width {
            return false;
        }
        if column < 0 {
            return false;
        }
        let idx = self.validate_column(column);
        if self.cells[idx].is_wide_lead() && idx + 1 < self.cells.len() {
            // Wide character at edge might need padding
            return idx + 1 == self.width as usize;
        }
        false
    }

    fn validate_column(&self, column: i32) -> usize {
        if column < 0 || column >= self.width {
            panic!("Column {} out of bounds for row of width {}", column, self.width);
        }
        column as usize
    }

    fn update_contains_text(&mut self) {
        self.contains_text = self.cells.iter().any(|c| !c.is_whitespace());
    }

    // ─── Internal methods ─────────────────────────────────────────────

    fn write_cells_replace(&mut self, start_column: i32, cells: &[OutputCell]) {
        let start = self.validate_column(start_column);
        for (i, cell) in cells.iter().enumerate() {
            let idx = start + i;
            if idx < self.cells.len() {
                self.cells[idx] = cell.clone();
            } else {
                break;
            }
        }
    }

    fn write_cells_insert(&mut self, start_column: i32, cells: &[OutputCell]) {
        let start = self.validate_column(start_column);
        self.shift_right(start, cells.len());
        for (offset, cell) in cells.iter().enumerate() {
            let idx = start + offset;
            if idx < self.cells.len() {
                self.cells[idx] = cell.clone();
            }
        }
    }

    /// Moves `cells[start..]` right by `count`, dropping what falls off the end.
    ///
    /// Walking backwards means a cell is never read after being overwritten.
    fn shift_right(&mut self, start: usize, count: usize) {
        let width = self.cells.len();
        let Some(end) = width.checked_sub(count) else {
            return;
        };
        if start >= end {
            return;
        }
        for column in (start..end).rev() {
            self.cells[column + count] = self.cells[column].clone();
        }
    }

    fn write_char_insert(&mut self, column: i32, cell: OutputCell) {
        let start = self.validate_column(column);
        self.shift_right(start, 1);
        self.cells[start] = cell;
    }
}

impl Default for Row {
    fn default() -> Self {
        Self::new(0)
    }
}

impl std::fmt::Display for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_shifts_the_tail_right_without_growing_the_row() {
        let mut row = Row::new(5);
        row.write_char(0, 'A', false);
        row.write_char(1, 'B', false);
        row.write_char(2, 'C', false);

        row.write_char(0, 'X', true);

        assert_eq!(row.get_chars_at(0), "X");
        assert_eq!(row.get_chars_at(1), "A");
        assert_eq!(row.get_chars_at(2), "B");
        assert_eq!(row.get_chars_at(3), "C");
        assert_eq!(row.cell_count(), 5);
    }

    #[test]
    fn insert_past_the_end_drops_the_overflow() {
        let mut row = Row::new(4);
        row.write_char(0, 'A', false);
        row.write_char(1, 'B', false);
        row.write_char(2, 'C', false);
        row.write_char(3, 'D', false);

        row.write_char(2, 'X', true);

        assert_eq!(row.get_chars_at(2), "X");
        assert_eq!(row.get_chars_at(3), "C");
        assert_eq!(row.cell_count(), 4);
    }

    #[test]
    fn test_row_creation() {
        let row = Row::new(80);
        assert_eq!(row.width(), 80);
        assert_eq!(row.cell_count(), 80);
        assert!(!row.contains_text());
        assert!(!row.is_wrap());
    }

    #[test]
    fn test_write_cell() {
        let mut row = Row::new(80);
        let cell = OutputCell::from_char('A');
        row.write_cell(0, cell);
        assert!(row.contains_text());
        assert_eq!(row.get_chars_at(0), "A");
    }

    #[test]
    fn test_write_cells_replace() {
        let mut row = Row::new(80);
        let cells = vec![OutputCell::from_char('H'), OutputCell::from_char('i')];
        row.write_cells(0, &cells, false);
        assert_eq!(row.get_chars_at(0), "H");
        assert_eq!(row.get_chars_at(1), "i");
    }

    #[test]
    fn test_copy_text_from() {
        let mut source = Row::new(80);
        source.write_char(0, 'H', false);
        source.write_char(1, 'i', false);

        let mut dest = Row::new(80);
        dest.copy_text_from(10, &source, 0, 2);
        assert_eq!(dest.get_chars_at(10), "H");
        assert_eq!(dest.get_chars_at(11), "i");
    }

    #[test]
    fn test_clear_cell() {
        let mut row = Row::new(80);
        row.write_char(0, 'A', false);
        row.clear_cell(0);
        assert!(!row.contains_text());
    }

    #[test]
    fn test_get_text() {
        let mut row = Row::new(80);
        for i in 0..5 {
            row.write_char(i, (b'a' + i as u8) as char, false);
        }
        assert_eq!(row.get_text(), "abcde");
    }

    #[test]
    fn test_wrapping() {
        let mut row = Row::new(80);
        row.force_wrap(1);
        assert!(row.is_wrap());
        assert_eq!(row.get_trailing_width(), 1);
    }

    #[test]
    fn test_cjk_in_row() {
        let mut row = Row::new(80);
        row.write_char(0, '中', false);
        assert!(row.get_cell(0).is_wide_lead());
    }

    #[test]
    fn test_measure_left() {
        let mut row = Row::new(80);
        // Fill first 10 cells with spaces
        for i in 0..10 {
            row.write_char(i, ' ', false);
        }
        // Put a character at position 10
        row.write_char(10, 'A', false);
        assert_eq!(row.measure_left(), 10);
    }

    #[test]
    fn test_is_all_spaces() {
        let mut row = Row::new(80);
        assert!(row.is_all_spaces());

        row.write_char(0, 'A', false);
        assert!(!row.is_all_spaces());
    }

    #[test]
    fn test_replace_characters() {
        let mut row = Row::new(80);
        for i in 0..5 {
            row.write_char(i, 'x', false);
        }
        row.replace_characters(2, "AB");
        assert_eq!(row.get_chars_at(2), "A");
        assert_eq!(row.get_chars_at(3), "B");
    }

    #[test]
    fn test_set_attr_to_end() {
        let mut row = Row::new(80);
        row.write_char(0, 'A', false);
        row.write_char(5, 'B', false);

        let attr = TextAttribute::new();
        row.set_attr_to_end(3, attr);

        // All cells from column 3 onward should have the same attributes
        for col in 3..80 {
            assert_eq!(row.get_attr_by_column(col), attr);
        }
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn test_out_of_bounds_column() {
        let row = Row::new(80);
        row.get_cell(80); // out of bounds
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn test_negative_column() {
        let row = Row::new(80);
        row.get_cell(-1); // negative
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    #[allow(dead_code)]
    fn any_row_strategy() -> impl Strategy<Value = Row> {
        (1i32..1000i32).prop_flat_map(|width| {
            proptest::collection::vec(32u8..127u8, 0..width as usize).prop_map(move |chars| {
                let mut row = Row::new(width);
                for (i, c) in chars.iter().enumerate() {
                    row.write_char(i as i32, *c as char, false);
                }
                row
            })
        })
    }

    proptest! {
        #[test]
        fn test_write_read_roundtrip(width in 1i32..100i32, col in 0i32..100i32) {
            let mut row = Row::new(width);
            let cell = OutputCell::from_char('X');
            let col = col.min(width - 1);
            row.write_cell(col, cell.clone());
            prop_assert_eq!(row.get_cell(col), &cell);
        }

        #[test]
        fn test_clear_then_empty(width in 1i32..100i32) {
            let mut row = Row::new(width);
            row.write_char(0, 'A', false);
            row.clear_cell(0);
            prop_assert!(!row.contains_text());
        }

        #[test]
        fn test_copy_preserves_content(width in 2i32..100i32) {
            let mut source = Row::new(width);
            source.write_char(0, 'A', false);
            source.write_char(1, 'B', false);

            let mut dest = Row::new(width);
            dest.copy_text_from(0, &source, 0, 2);
            prop_assert_eq!(dest.get_chars_at(0), "A");
            prop_assert_eq!(dest.get_chars_at(1), "B");
        }

        #[test]
        fn test_replace_characters(width in 5i32..100i32) {
            let mut row = Row::new(width);
            let original = "Hello";
            for (i, c) in original.chars().enumerate() {
                row.write_char(i as i32, c, false);
            }
            row.replace_characters(0, "World");
            prop_assert_eq!(row.get_chars_at(0), "W");
            prop_assert_eq!(row.get_chars_at(4), "d");
        }
    }
}

// Re-export codepoint_width for use in row methods
use types::codepoint_width;
