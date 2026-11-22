//! `Row` — a row of terminal buffer cells.
//!
//! Ported from `ROW` in the Windows Terminal C++ buffer module.
//! A row is a fixed-width array of `OutputCell` with methods for writing,
//! reading, and manipulating cell data.

use crate::TextAttribute;
use crate::image_slice::ImageSlice;
use crate::output_cell::OutputCell;
use types::LineRendition;

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
    /// The line rendition (single/double width, double-height half).
    line_rendition: LineRendition,
    /// The image band covering this row, if any (ADR 005).
    image: Option<ImageSlice>,
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
            line_rendition: LineRendition::SingleWidth,
            image: None,
        }
    }

    /// Builds a `width`-wide row from cell data, padding with empty cells.
    ///
    /// Reflow has already sliced a logical line into row-sized pieces, so this
    /// avoids `write_cell`'s per-cell `contains_text` scan on the hot path.
    pub fn from_cells(width: i32, cells: Vec<OutputCell>, wrap: bool) -> Self {
        let mut row_cells = vec![OutputCell::new(); width.max(0) as usize];
        let count = cells.len().min(row_cells.len());
        row_cells[..count].clone_from_slice(&cells[..count]);
        let contains_text = row_cells.iter().any(|cell| !cell.is_whitespace());

        Self {
            cells: row_cells,
            width,
            contains_text,
            wrap,
            trailing_width: 0,
            scrollbar_data: ScrollbarData::default(),
            line_rendition: LineRendition::SingleWidth,
            image: None,
        }
    }

    /// Returns the image band covering this row, if any.
    pub const fn image_slice(&self) -> Option<&ImageSlice> {
        self.image.as_ref()
    }

    /// Returns the image band mutably, if any.
    pub fn image_slice_mut(&mut self) -> Option<&mut ImageSlice> {
        self.image.as_mut()
    }

    /// Replaces the row's image band; `None` clears it.
    pub fn set_image_slice(&mut self, image: Option<ImageSlice>) {
        self.image = image;
    }

    /// Copies the whole band from `source`, as `ImageSlice::CopyRow` does.
    pub fn copy_image_from(&mut self, source: &Row) {
        self.image = source.image.clone();
    }

    /// Erases the pixels under `column_begin..column_end` and drops the band
    /// once it is empty.
    pub fn erase_image_cells(&mut self, column_begin: i32, column_end: i32) {
        if let Some(image) = &mut self.image
            && image.erase_cells(column_begin, column_end)
        {
            self.image = None;
        }
    }

    /// Ported from `ImageSlice::CopyCells`: copies the pixels for a column range
    /// from `source`, erasing the range first, and drops an empty result.
    pub fn copy_image_cells_from(&mut self, source: &Row, source_column: i32, dest_begin: i32, dest_end: i32) {
        self.copy_image_cells_from_slice(source.image.as_ref(), source_column, dest_begin, dest_end);
    }

    /// The same copy, for callers holding a band rather than a whole row
    /// (DECCRA snapshots the source before writing).
    pub fn copy_image_cells_from_slice(
        &mut self,
        source_image: Option<&ImageSlice>,
        source_column: i32,
        dest_begin: i32,
        dest_end: i32,
    ) {
        self.erase_image_cells(dest_begin, dest_end);
        let Some(source_image) = source_image else {
            return;
        };

        let length = dest_end - dest_begin;
        let used_begin = source_column.max(source_image.column_begin());
        let used_end = (source_column + length).min(source_image.column_end());
        if used_begin >= used_end {
            return;
        }

        let image = self
            .image
            .get_or_insert_with(|| ImageSlice::new(source_image.cell_size()));
        let offset = dest_begin - source_column;
        if image.copy_cells_from(source_image, used_begin, used_begin + offset, used_end + offset) {
            self.image = None;
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
        self.erase_image_cells(column, column + 1);
        self.update_contains_text();
    }

    /// Writes a cell without touching the image band.
    ///
    /// Horizontal scrolling moves text only; upstream's `ScrollRectHorizontally`
    /// leaves image content where it is.
    pub(crate) fn write_cell_raw(&mut self, column: i32, cell: OutputCell) {
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
            self.erase_image_cells(column, column + 1);
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
            self.write_cell_raw(dest_col + i as i32, src_cell.clone());
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
        self.erase_image_cells(column, column + 1);
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
        self.erase_image_cells(start_col, end_col);
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
        let mut written = 0;
        for (offset, c) in replacement.chars().take(end.saturating_sub(start)).enumerate() {
            self.cells[start + offset] = OutputCell::from_char(c);
            written += 1;
        }
        self.erase_image_cells(start_column, start_column + written);
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

    /// Returns whether this row carries a non-single-width rendition.
    pub const fn has_line_rendition(&self) -> bool {
        !matches!(self.line_rendition, LineRendition::SingleWidth)
    }

    /// Returns this row's line rendition.
    pub const fn line_rendition(&self) -> LineRendition {
        self.line_rendition
    }

    /// Sets this row's line rendition.
    pub fn set_line_rendition(&mut self, rendition: LineRendition) {
        self.line_rendition = rendition;
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

    /// Gets the column one past the row's readable content.
    ///
    /// Ported from `ROW::MeasureRight`: a wrapped row is read to its full width
    /// and an explicit line ends after its last non-space column.
    pub fn measure_right(&self) -> i32 {
        if self.wrap {
            self.width
        } else {
            self.get_last_non_space_column()
        }
    }

    /// Gets the column one past the last non-space character.
    ///
    /// Ported from `ROW::GetLastNonSpaceColumn`: the result is a column
    /// measurement, so a wide glyph contributes both of its columns, and an
    /// all-space row reports 0.
    pub fn get_last_non_space_column(&self) -> i32 {
        for (index, cell) in self.cells.iter().enumerate().rev() {
            if !cell.is_whitespace() {
                return index as i32 + i32::from(cell.column_width().max(1));
            }
        }
        0
    }

    /// Returns how many columns of this row hold content.
    ///
    /// Ported from `ROW::GetReadableColumnCount`. The double-byte padding
    /// upstream subtracts never occurs here, because the adapter wraps a wide
    /// glyph that would not fit instead of padding it.
    pub fn get_readable_column_count(&self) -> i32 {
        self.width
    }

    /// Returns whether the cell at `column` is the trailing half of a wide
    /// character.
    pub fn is_trailing_half(&self, column: i32) -> bool {
        column > 0 && self.get_cell(column - 1).is_wide_lead()
    }

    /// Returns the starting column of the glyph at `column`.
    ///
    /// Ported from `ROW::AdjustToGlyphStart`: a column that falls on the
    /// trailing half of a wide glyph moves back to its lead.
    pub fn adjust_to_glyph_start(&self, column: i32) -> i32 {
        let mut column = column.clamp(0, self.width - 1);
        while column > 0 && self.is_trailing_half(column) {
            column -= 1;
        }
        column
    }

    /// Returns the exclusive column after the glyph at `column`.
    ///
    /// Ported from `ROW::AdjustToGlyphEnd`.
    pub fn adjust_to_glyph_end(&self, column: i32) -> i32 {
        let mut column = column.clamp(0, self.width);
        while column < self.width && self.is_trailing_half(column) {
            column += 1;
        }
        column
    }

    /// Returns the text of the columns `begin..end`.
    ///
    /// Ported from `ROW::GetText(begin, end)`. A blank cell reads as a space,
    /// which is what upstream stores there, while the trailing half of a wide
    /// character contributes nothing of its own.
    pub fn get_text_range(&self, begin: i32, end: i32) -> String {
        let readable = self.get_readable_column_count();
        let begin = begin.clamp(0, readable);
        let end = end.clamp(begin, readable);

        let mut text = String::new();
        for column in begin..end {
            if self.is_trailing_half(column) {
                continue;
            }

            let cell = self.get_cell(column);
            if cell.is_empty() {
                text.push(' ');
            } else {
                text.push_str(cell.chars());
            }
        }
        text
    }

    /// Returns runs of equal attributes across the columns `begin..end`.
    ///
    /// Upstream reads these from `RowAttributes`; this row stores an
    /// attribute per cell, so they are grouped here. A wide glyph's trailing
    /// half shares its lead's attribute, so a glyph never splits a run.
    pub fn attribute_runs(&self, begin: i32, end: i32) -> Vec<(TextAttribute, i32)> {
        let begin = begin.clamp(0, self.width);
        let end = end.clamp(begin, self.width);
        let mut runs: Vec<(TextAttribute, i32)> = Vec::new();

        for column in begin..end {
            let lead = self.adjust_to_glyph_start(column);
            let attribute = *self.get_cell(lead).get_attributes();

            match runs.last_mut() {
                Some((last, length)) if *last == attribute => *length += 1,
                _ => runs.push((attribute, 1)),
            }
        }

        runs
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
        let mut written = 0;
        for (i, cell) in cells.iter().enumerate() {
            let idx = start + i;
            if idx < self.cells.len() {
                self.cells[idx] = cell.clone();
                written += 1;
            } else {
                break;
            }
        }
        self.erase_image_cells(start_column, start_column + written);
    }

    fn write_cells_insert(&mut self, start_column: i32, cells: &[OutputCell]) {
        let start = self.validate_column(start_column);
        if let Some(image) = &mut self.image {
            image.shift_right_cells(start_column, cells.len() as i32);
        }
        self.shift_right(start, cells.len());
        for (offset, cell) in cells.iter().enumerate() {
            let idx = start + offset;
            if idx < self.cells.len() {
                self.cells[idx] = cell.clone();
            }
        }
        self.erase_image_cells(start_column, start_column + cells.len() as i32);
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
        if let Some(image) = &mut self.image {
            image.shift_right_cells(column, 1);
        }
        self.shift_right(start, 1);
        self.cells[start] = cell;
        self.erase_image_cells(column, column + 1);
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

    fn image_band(columns: i32) -> ImageSlice {
        let mut image = ImageSlice::new((1, 1));
        let pixels = image.mutable_pixels(0, columns);
        for (index, pixel) in pixels.iter_mut().enumerate() {
            *pixel = index as u32 + 1;
        }
        image
    }

    #[test]
    fn writing_a_cell_erases_the_image_under_it() {
        let mut row = Row::new(4);
        row.set_image_slice(Some(image_band(4)));

        row.write_char(1, 'x', false);

        assert_eq!(row.image_slice().unwrap().pixels(), &[1, 0, 3, 4]);
    }

    #[test]
    fn clearing_a_range_erases_its_image_pixels() {
        let mut row = Row::new(4);
        row.set_image_slice(Some(image_band(4)));

        row.clear_range(0, 3);

        assert_eq!(row.image_slice().unwrap().pixels(), &[0, 0, 0, 4]);
    }

    #[test]
    fn copying_a_row_copies_the_image_band() {
        let mut source = Row::new(4);
        source.set_image_slice(Some(image_band(4)));
        let mut destination = Row::new(4);

        destination.copy_image_from(&source);

        assert_eq!(destination.image_slice().unwrap().pixels(), &[1, 2, 3, 4]);
    }

    #[test]
    fn insert_mode_shifts_the_image_band() {
        let mut row = Row::new(4);
        row.set_image_slice(Some(image_band(4)));

        row.write_char(1, 'x', true);

        // The band grows by one column, and the pixels to the right move over.
        assert_eq!(row.image_slice().unwrap().pixels(), &[1, 0, 2, 3, 4]);
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

    #[test]
    fn the_readable_end_is_one_past_the_last_non_space_cell() {
        let mut row = Row::new(10);
        for (column, character) in "abc".chars().enumerate() {
            row.write_char(column as i32, character, false);
        }
        assert_eq!(row.get_last_non_space_column(), 3);

        // A wide glyph contributes both of its columns.
        let mut wide = Row::new(10);
        wide.write_char(0, '中', false);
        assert_eq!(wide.get_last_non_space_column(), 2);

        assert_eq!(Row::new(10).get_last_non_space_column(), 0);
    }

    #[test]
    fn a_wrapped_row_reads_to_its_full_width() {
        let mut row = Row::new(10);
        row.write_char(0, 'a', false);
        row.write_char(1, 'b', false);
        assert_eq!(row.measure_right(), 2);

        row.set_wrap(true);
        assert_eq!(row.measure_right(), 10);
    }

    #[test]
    fn glyph_adjustment_moves_off_the_trailing_half() {
        let mut row = Row::new(10);
        row.write_char(0, '中', false);
        row.write_char(2, 'a', false);

        // Column 1 is the trailing half of the glyph at column 0.
        assert_eq!(row.adjust_to_glyph_start(1), 0);
        assert_eq!(row.adjust_to_glyph_start(2), 2);
        // An end that already sits on a leading column stays there; one inside
        // a glyph moves past it.
        assert_eq!(row.adjust_to_glyph_end(0), 0);
        assert_eq!(row.adjust_to_glyph_end(1), 2);
        assert_eq!(row.adjust_to_glyph_end(2), 2);
    }

    #[test]
    fn text_ranges_read_blanks_as_spaces_and_skip_trailers() {
        let mut row = Row::new(5);
        row.write_char(0, '中', false);
        row.write_char(2, 'a', false);

        assert_eq!(row.get_text_range(0, 5), "中a  ");
        // The range starting on the trailing half picks up only the glyph's
        // leading column's text, then the character after it.
        assert_eq!(row.get_text_range(1, 3), "a");
        assert_eq!(row.get_text_range(3, 3), "");
    }

    #[test]
    fn attribute_runs_group_equal_cells_and_cover_wide_glyphs() {
        let mut row = Row::new(6);
        let mut first = TextAttribute::new();
        first.set_intense(true);

        let mut cell = OutputCell::from_char('a');
        cell.set_attributes(first);
        row.write_cell(0, cell);
        row.write_char(1, 'b', false);
        row.write_char(2, 'c', false);
        row.write_char(3, '中', false);

        // The glyph at column 3 and its trailing half share the default
        // attribute, so cells 1 through 5 form one run.
        assert_eq!(row.attribute_runs(0, 6), vec![(first, 1), (TextAttribute::new(), 5)]);
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
