//! `TextBuffer` — the core circular scrollback buffer for the terminal.
//!
//! Ported from `TextBuffer` in the Windows Terminal C++ buffer module.
//! Manages a circular buffer of rows with support for scrollback, alternate
//! screen buffer, and row indexing for screen-to-buffer position conversion.

use std::collections::HashMap;

use crate::cursor::Cursor;
use crate::hyperlinks::HyperlinkMap;
use crate::marks::Marks;
use crate::output_cell::OutputCell;
use crate::row::Row;
use crate::selection::{Selection, SelectionType};

/// The default scrollback buffer height (number of rows).
pub const DEFAULT_SCROLLBACK_HEIGHT: i32 = 300;

/// A row in the text buffer with screen-to-buffer index mapping.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct BufferedRow {
    /// The row data.
    row: Row,
    /// The screen line index for this buffer row (relative to the bottom).
    /// For rows in the visible area: 0..screen_height-1
    /// For rows in scrollback: screen_height..screen_height+scrollback_height-1
    screen_line: i32,
}

impl BufferedRow {
    fn new(row: Row, screen_line: i32) -> Self {
        Self { row, screen_line }
    }
}

/// The core text buffer for a terminal.
///
/// Ported from `TextBuffer` in the Windows Terminal C++ buffer module.
/// Implements a circular scrollback buffer with support for:
/// - Fixed-height scrollback with row eviction
/// - Alternate screen buffer
/// - Row indexing for efficient screen-to-buffer conversion
/// - Cell-level and row-level operations
/// - Scrollback marks and hyperlink tracking
#[derive(Debug, Clone)]
pub struct TextBuffer {
    /// The visible screen dimensions.
    screen_width: i32,
    screen_height: i32,
    /// The maximum scrollback height (number of scrollback rows).
    scrollback_height: i32,
    /// The total buffer height (screen_height + scrollback_height).
    buffer_height: i32,
    /// The circular buffer of rows.
    rows: VecDequeCompat<BufferedRow>,
    /// Index mapping screen lines to buffer row indices.
    /// screen_line -> row_index in the circular buffer.
    screen_line_index: HashMap<i32, usize>,
    /// Whether this is the alternate screen buffer.
    is_alternate: bool,
    /// The cursor state.
    cursor: Cursor,
    /// Scrollback marks (prompts, commands, output boundaries).
    marks: Marks,
    /// Hyperlink map for OSC 8 hyperlinks.
    hyperlinks: HyperlinkMap,
    /// Current selection (if any).
    selection: Option<Selection>,
    /// The mutation ID, incremented on each write.
    mutation_id: u64,
    /// Rows pushed off the top of the screen, oldest first.
    ///
    /// This is the storage reflow will rewrap into; until the view offset exists
    /// it is written by scrolling and cleared by `ED 3`, not displayed.
    scrollback: std::collections::VecDeque<Row>,
}

/// A simple VecDeque-like wrapper for circular buffer operations.
/// We use a Vec with a start offset for simplicity.
#[derive(Debug, Clone)]
struct VecDequeCompat<T> {
    data: Vec<T>,
    start: usize,
    len: usize,
}

#[allow(dead_code)]
impl<T: Clone> VecDequeCompat<T> {
    fn new(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            start: 0,
            len: 0,
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn push_back(&mut self, value: T) {
        if self.len < self.data.len() {
            // There's space in the existing buffer
            let idx = (self.start + self.len) % self.data.len();
            self.data[idx] = value;
            self.len += 1;
        } else if self.len == self.data.len() {
            // Buffer is full, need to grow
            self.data.push(value);
            self.len += 1;
        } else {
            // Shouldn't happen
            self.data.push(value);
            self.len += 1;
        }
    }

    fn push_front(&mut self, value: T) {
        if self.start > 0 {
            // There's space before the start
            self.start -= 1;
            self.data[self.start] = value;
            self.len += 1;
        } else {
            // No space before, need to grow
            self.data.insert(0, value);
            self.len += 1;
        }
    }

    fn pop_front(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let value = self.data[self.start].clone();
        self.start = (self.start + 1) % self.data.len();
        self.len -= 1;
        Some(value)
    }

    fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        let abs_idx = (self.start + index) % self.data.len();
        self.data.get(abs_idx)
    }

    fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len {
            return None;
        }
        let abs_idx = (self.start + index) % self.data.len();
        self.data.get_mut(abs_idx)
    }

    fn set(&mut self, index: usize, value: T) {
        if index < self.len {
            let abs_idx = (self.start + index) % self.data.len();
            self.data[abs_idx] = value;
        }
    }

    fn clear(&mut self) {
        self.data.clear();
        self.start = 0;
        self.len = 0;
    }
}

impl<T: Clone> Default for VecDequeCompat<T> {
    fn default() -> Self {
        Self::new(16)
    }
}

impl<T: Clone> std::ops::Index<usize> for VecDequeCompat<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("Index out of bounds")
    }
}

impl<T: Clone> std::ops::IndexMut<usize> for VecDequeCompat<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index).expect("Index out of bounds")
    }
}

impl TextBuffer {
    /// Creates a new text buffer with default dimensions.
    pub fn new() -> Self {
        Self::with_dimensions(80, 24, DEFAULT_SCROLLBACK_HEIGHT)
    }

    /// Creates a new text buffer with the given dimensions.
    pub fn with_dimensions(width: i32, height: i32, scrollback_height: i32) -> Self {
        let buffer_height = height + scrollback_height;

        // Allocate scrollback rows as well as the visible ones: the screen
        // index is offset by the scrollback height, so a buffer that only
        // allocated its visible rows could not look any of them up.
        let mut rows = VecDequeCompat::new(buffer_height as usize);
        let mut screen_line_index = HashMap::new();

        for i in 0..buffer_height {
            let row = Row::new(width);
            rows.push_back(BufferedRow::new(row, i - scrollback_height));
        }
        for i in 0..height {
            let buffer_idx = i + scrollback_height;
            screen_line_index.insert(i, (buffer_idx as usize) % buffer_height as usize);
        }

        Self {
            screen_width: width,
            screen_height: height,
            scrollback_height,
            buffer_height,
            rows,
            screen_line_index,
            is_alternate: false,
            cursor: Cursor::new(),
            marks: Marks::new(),
            hyperlinks: HyperlinkMap::new(),
            selection: None,
            mutation_id: 0,
            scrollback: std::collections::VecDeque::new(),
        }
    }

    /// Creates an alternate screen buffer.
    pub fn new_alternate(width: i32, height: i32) -> Self {
        let mut buf = Self::with_dimensions(width, height, 0);
        buf.is_alternate = true;
        buf
    }

    /// Returns the screen width.
    pub const fn screen_width(&self) -> i32 {
        self.screen_width
    }

    /// Returns the screen height.
    pub const fn screen_height(&self) -> i32 {
        self.screen_height
    }

    /// Returns the scrollback height.
    pub const fn scrollback_height(&self) -> i32 {
        self.scrollback_height
    }

    /// Returns the total buffer height.
    pub const fn buffer_height(&self) -> i32 {
        self.buffer_height
    }

    /// Returns whether this is the alternate screen buffer.
    pub const fn is_alternate(&self) -> bool {
        self.is_alternate
    }

    /// Returns the cursor.
    pub const fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Returns the cursor mutably.
    pub fn cursor_mut(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    /// Returns the scrollback marks.
    pub const fn marks(&self) -> &Marks {
        &self.marks
    }

    /// Returns the hyperlink map.
    pub const fn hyperlinks(&self) -> &HyperlinkMap {
        &self.hyperlinks
    }

    /// Returns the selection.
    pub const fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    /// Returns the current mutation ID.
    pub const fn mutation_id(&self) -> u64 {
        self.mutation_id
    }

    /// Gets the row by buffer offset.
    pub fn get_row_by_offset(&self, offset: i32) -> Option<&Row> {
        let abs_idx = (self.rows.start + offset as usize) % self.rows.data.len();
        self.rows.data.get(abs_idx).map(|r| &r.row)
    }

    /// Gets the row at a specific screen line.
    pub fn get_row_at_screen_line(&self, screen_line: i32) -> Option<&Row> {
        if screen_line < 0 || screen_line >= self.screen_height {
            return None;
        }

        if let Some(&row_idx) = self.screen_line_index.get(&screen_line)
            && row_idx < self.rows.len()
        {
            return Some(&self.rows[row_idx].row);
        }

        None
    }

    /// Gets the row at a specific buffer line.
    pub fn get_row_by_buffer_line(&self, buffer_line: i32) -> Option<&Row> {
        let screen_line = buffer_line - self.scrollback_height;
        if screen_line < 0 || screen_line >= self.screen_height {
            return None;
        }
        self.get_row_at_screen_line(screen_line)
    }

    /// Gets the cell data at a screen position.
    pub fn get_cell_data_at(&self, screen_row: i32, screen_col: i32) -> Option<&OutputCell> {
        let row = self.get_row_at_screen_line(screen_row)?;
        if screen_col < 0 || screen_col >= row.width() {
            return None;
        }
        Some(row.get_cell(screen_col))
    }

    /// Gets the text at a screen position.
    pub fn get_text_data_at(&self, screen_row: i32, screen_col: i32) -> Option<&str> {
        let cell = self.get_cell_data_at(screen_row, screen_col)?;
        Some(cell.chars())
    }

    /// Writes a character at the current cursor position.
    pub fn write(&mut self, c: char) {
        let row = self.cursor.row();
        let col = self.cursor.column();

        if let Some(buffer_row) = self.get_buffer_row_mut_at_screen(row, col) {
            buffer_row.row.write_cell(col, OutputCell::from_char(c));
        }

        self.cursor.move_right(1);
        self.mutation_id += 1;
    }

    /// Writes a string at the current cursor position.
    pub fn write_string(&mut self, s: &str) {
        for c in s.chars() {
            self.write(c);
        }
    }

    /// Writes a row and advances to the next line.
    pub fn write_line(&mut self, row: &Row) {
        if let Some(buffer_row) = self.get_buffer_row_mut_at_screen(self.cursor.row(), 0) {
            buffer_row.row.copy_from(row);
        }
        self.new_line();
    }

    /// Fills a rectangular area with a cell.
    pub fn fill_rect(&mut self, start_row: i32, start_col: i32, end_row: i32, end_col: i32, cell: &OutputCell) {
        for r in start_row..=end_row {
            for c in start_col..=end_col {
                if let Some(buffer_row) = self.get_buffer_row_mut_at_screen(r, c) {
                    buffer_row.row.write_cell(c, cell.clone());
                }
            }
        }
        self.mutation_id += 1;
    }

    /// Returns the row at a screen line, mutably.
    pub fn get_row_at_screen_line_mut(&mut self, screen_line: i32) -> Option<&mut Row> {
        self.get_buffer_row_mut_at_screen(screen_line, 0)
            .map(|buffered| &mut buffered.row)
    }

    /// Scrolls the rows in `top..=bottom` up by one, clearing the bottom row.
    ///
    /// Ported from `_ScrollRectVertically` with a count of one: a line feed at
    /// the bottom margin moves every row in the region up and reveals a blank
    /// bottom row, without touching rows outside it. A full-page scroll keeps
    /// the row it pushes off the top in the scrollback.
    pub fn scroll_region_up(&mut self, top: i32, bottom: i32) {
        if bottom <= top || top < 0 || bottom >= self.screen_height {
            return;
        }

        if top == 0
            && bottom == self.screen_height - 1
            && self.scrollback_height > 0
            && let Some(evicted) = self.get_row_at_screen_line(0).cloned()
        {
            self.scrollback.push_back(evicted);
            while self.scrollback.len() > self.scrollback_height as usize {
                self.scrollback.pop_front();
            }
        }

        for row in top..bottom {
            self.copy_row(row, row + 1);
        }
        let blank = OutputCell::from_char(' ');
        self.fill_rect(bottom, 0, bottom, self.screen_width - 1, &blank);
        self.mutation_id += 1;
    }

    /// Returns how many rows are held in the scrollback.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Returns the text of a scrollback row, oldest first.
    pub fn scrollback_row_text(&self, index: usize) -> Option<String> {
        self.scrollback.get(index).map(Row::get_text)
    }

    /// Discards the scrollback, which is what `ED 3` asks for.
    pub fn clear_scrollback(&mut self) {
        self.scrollback.clear();
        self.mutation_id += 1;
    }

    /// Scrolls the rows in `top..=bottom` down by one, clearing the top row.
    ///
    /// The reverse of [`TextBuffer::scroll_region_up`], used by RI when the
    /// cursor is already at the top margin.
    pub fn scroll_region_down(&mut self, top: i32, bottom: i32) {
        if bottom <= top || top < 0 || bottom >= self.screen_height {
            return;
        }
        for row in (top + 1..=bottom).rev() {
            self.copy_row(row, row - 1);
        }
        let blank = OutputCell::from_char(' ');
        self.fill_rect(top, 0, top, self.screen_width - 1, &blank);
        self.mutation_id += 1;
    }

    /// Shifts the cells in `left..=right` one column left on every row of the
    /// region, blanking the rightmost column (DECFI).
    pub fn scroll_region_left(&mut self, top: i32, bottom: i32, left: i32, right: i32) {
        if right <= left {
            return;
        }
        for row_index in top..=bottom {
            let Some(source) = self.get_row_at_screen_line(row_index).cloned() else {
                continue;
            };
            let Some(destination) = self.get_row_at_screen_line_mut(row_index) else {
                continue;
            };
            destination.copy_text_from(left, &source, left + 1, right - left);
            destination.write_cell(right, OutputCell::from_char(' '));
        }
        self.mutation_id += 1;
    }

    /// Shifts the cells in `left..=right` one column right on every row of the
    /// region, blanking the leftmost column (DECBI).
    pub fn scroll_region_right(&mut self, top: i32, bottom: i32, left: i32, right: i32) {
        if right <= left {
            return;
        }
        for row_index in top..=bottom {
            let Some(source) = self.get_row_at_screen_line(row_index).cloned() else {
                continue;
            };
            let Some(destination) = self.get_row_at_screen_line_mut(row_index) else {
                continue;
            };
            destination.copy_text_from(left + 1, &source, left, right - left);
            destination.write_cell(left, OutputCell::from_char(' '));
        }
        self.mutation_id += 1;
    }

    /// Scrolls rows up by the given count.
    pub fn scroll_rows_up(&mut self, count: i32) {
        if count <= 0 {
            return;
        }
        for _ in 0..count {
            self.scroll_one_row_up();
        }
    }

    /// Scrolls rows down by the given count.
    pub fn scroll_rows_down(&mut self, count: i32) {
        if count <= 0 {
            return;
        }
        for _ in 0..count {
            self.scroll_one_row_down();
        }
    }

    /// Copies a row from one position to another.
    pub fn copy_row(&mut self, dest_screen_line: i32, source_screen_line: i32) {
        // Clone the source to avoid borrow conflict
        let source_data = self.get_row_at_screen_line(source_screen_line).cloned();
        if let Some(source_row) = source_data
            && let Some(dest_row) = self.get_buffer_row_mut_at_screen(dest_screen_line, 0)
        {
            dest_row.row.copy_from(&source_row);
        }
        self.mutation_id += 1;
    }

    /// Converts screen row to buffer row.
    pub fn screen_to_buffer_row(&self, screen_row: i32) -> i32 {
        screen_row + self.scrollback_height
    }

    /// Converts buffer row to screen row.
    pub fn buffer_to_screen_row(&self, buffer_row: i32) -> i32 {
        buffer_row - self.scrollback_height
    }

    /// Gets the screen row for a buffer line.
    pub fn get_screen_line_for_buffer_line(&self, buffer_line: i32) -> i32 {
        buffer_line - self.scrollback_height
    }

    /// Gets the text content of all rows as a string.
    pub fn get_plain_text(&self) -> String {
        let mut result = String::new();
        for i in 0..self.rows.len() {
            let row = &self.rows[i];
            result.push_str(&row.row.get_text());
            result.push('\n');
        }
        result
    }

    /// Creates a new selection starting at the given position.
    pub fn start_selection(&mut self, row: i32, col: i32, r#type: SelectionType) {
        self.selection = Some(Selection::new(row, col, r#type));
    }

    /// Extends the selection to the given position.
    pub fn extend_selection(&mut self, row: i32, col: i32) {
        if let Some(ref mut selection) = self.selection {
            selection.set_end(row, col);
        }
    }

    /// Gets the selected text.
    pub fn get_selection_text(&self) -> Option<String> {
        self.selection.as_ref().map(|s| s.get_text(self))
    }

    /// Clears the current selection.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Returns whether there is an active selection.
    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    /// Returns the extent of the current selection.
    pub fn get_selection_extent(&self) -> Option<(i32, i32, i32, i32)> {
        self.selection.as_ref().map(|s| s.bounds())
    }

    // ─── Internal ─────────────────────────────────────────────────────

    fn new_line(&mut self) {
        let col = self.cursor.column();
        if col >= self.screen_width {
            // Wrap to next line
            self.cursor.set_position(self.cursor.row() + 1, 0);
        }

        // If cursor is past the screen, scroll up
        if self.cursor.row() >= self.screen_height {
            self.scroll_one_row_up();
            self.cursor.set_position(self.screen_height - 1, self.cursor.column());
        }

        // Update screen line index
        if let Some(screen_line) = self.screen_line_index.get_mut(&self.cursor.row()) {
            let buffer_idx = self.cursor.row() + self.scrollback_height;
            *screen_line = (buffer_idx as usize) % self.buffer_height as usize;
        }
    }

    fn scroll_one_row_up(&mut self) {
        // Remove the top row
        if let Some(top_row) = self.rows.pop_front() {
            // Remove from screen line index
            let _ = top_row.row.width().checked_sub(1);
        }

        // Add a new row at the bottom
        let new_row = Row::new(self.screen_width);
        let new_screen_line = if self.scrollback_height > 0 {
            self.screen_height + self.scrollback_height - 1
        } else {
            self.screen_height - 1
        };

        self.rows.push_back(BufferedRow::new(new_row, new_screen_line));

        // Update screen line index for the shifted rows
        self.rebuild_screen_line_index();
    }

    fn scroll_one_row_down(&mut self) {
        // Remove the bottom row
        self.rows.pop_front();

        // Add a new row at the top
        let new_row = Row::new(self.screen_width);
        self.rows.push_front(BufferedRow::new(new_row, 0));

        self.rebuild_screen_line_index();
    }

    fn rebuild_screen_line_index(&mut self) {
        self.screen_line_index.clear();
        for i in 0..self.screen_height {
            let buffer_idx = i + self.scrollback_height;
            if let Some(_row) = self.rows.get(i as usize) {
                let _abs_idx = (self.rows.start + i as usize) % self.rows.data.len();
                self.screen_line_index
                    .insert(i, (buffer_idx as usize) % self.buffer_height as usize);
            }
        }
    }

    fn get_buffer_row_mut_at_screen(&mut self, screen_row: i32, _screen_col: i32) -> Option<&mut BufferedRow> {
        let abs_idx = (screen_row + self.scrollback_height) as usize;
        if abs_idx >= self.rows.data.len() {
            return None;
        }

        // For screen rows (scrollback_height..screen_height+scrollback_height)
        if screen_row >= 0 && screen_row < self.screen_height {
            let index = screen_row as usize + self.scrollback_height as usize;
            if index < self.rows.len() {
                return self.rows.get_mut(index);
            }
        }
        None
    }
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TextBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for i in 0..self.screen_height {
            if let Some(row) = self.get_row_at_screen_line(i) {
                writeln!(f, "{}", row.get_text())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_text_at(buffer: &mut TextBuffer, row: i32, column: i32, text: &str) {
        for (offset, ch) in text.chars().enumerate() {
            if let Some(buffer_row) = buffer.get_row_at_screen_line_mut(row) {
                buffer_row.write_char(column + offset as i32, ch, false);
            }
        }
    }

    fn text_at(buffer: &TextBuffer, row: i32, column: i32) -> String {
        buffer.get_text_data_at(row, column).unwrap_or_default().to_string()
    }

    #[test]
    fn a_full_page_scroll_keeps_the_evicted_row() {
        let mut buffer = TextBuffer::with_dimensions(4, 2, 4);
        write_text_at(&mut buffer, 0, 0, "top");

        buffer.scroll_region_up(0, 1);

        assert_eq!(buffer.scrollback_len(), 1);
        assert!(buffer.scrollback_row_text(0).unwrap().starts_with("top"));
    }

    #[test]
    fn the_scrollback_is_capped_at_its_height() {
        let mut buffer = TextBuffer::with_dimensions(4, 2, 2);
        for _ in 0..5 {
            buffer.scroll_region_up(0, 1);
        }
        assert_eq!(buffer.scrollback_len(), 2);
    }

    #[test]
    fn clear_scrollback_discards_the_rows() {
        let mut buffer = TextBuffer::with_dimensions(4, 2, 4);
        buffer.scroll_region_up(0, 1);
        assert_eq!(buffer.scrollback_len(), 1);

        buffer.clear_scrollback();
        assert_eq!(buffer.scrollback_len(), 0);
    }

    #[test]
    fn scroll_region_down_moves_rows_and_blanks_the_top() {
        let mut buffer = TextBuffer::with_dimensions(10, 4, 0);
        write_text_at(&mut buffer, 0, 0, "top");
        write_text_at(&mut buffer, 1, 0, "mid");

        buffer.scroll_region_down(0, 3);

        assert_eq!(text_at(&buffer, 1, 0), "t");
        assert_eq!(text_at(&buffer, 2, 0), "m");
        assert_eq!(text_at(&buffer, 0, 0), " ");
    }

    #[test]
    fn scroll_region_left_shifts_cells_and_blanks_the_right_column() {
        let mut buffer = TextBuffer::with_dimensions(4, 1, 0);
        write_text_at(&mut buffer, 0, 0, "abcd");

        buffer.scroll_region_left(0, 0, 0, 3);

        assert_eq!(text_at(&buffer, 0, 0), "b");
        assert_eq!(text_at(&buffer, 0, 2), "d");
        assert_eq!(text_at(&buffer, 0, 3), " ");
    }

    #[test]
    fn scroll_region_right_shifts_cells_and_blanks_the_left_column() {
        let mut buffer = TextBuffer::with_dimensions(4, 1, 0);
        write_text_at(&mut buffer, 0, 0, "abcd");

        buffer.scroll_region_right(0, 0, 0, 3);

        assert_eq!(text_at(&buffer, 0, 0), " ");
        assert_eq!(text_at(&buffer, 0, 1), "a");
        assert_eq!(text_at(&buffer, 0, 3), "c");
    }

    #[test]
    fn test_new_buffer() {
        let buf = TextBuffer::new();
        assert_eq!(buf.screen_width(), 80);
        assert_eq!(buf.screen_height(), 24);
        assert_eq!(buf.scrollback_height(), DEFAULT_SCROLLBACK_HEIGHT);
        assert!(!buf.is_alternate());
    }

    #[test]
    fn test_custom_dimensions() {
        let buf = TextBuffer::with_dimensions(100, 50, 500);
        assert_eq!(buf.screen_width(), 100);
        assert_eq!(buf.screen_height(), 50);
        assert_eq!(buf.scrollback_height(), 500);
        assert_eq!(buf.buffer_height(), 550);
    }

    #[test]
    fn test_alternate_buffer() {
        let buf = TextBuffer::new_alternate(80, 24);
        assert!(buf.is_alternate());
        assert_eq!(buf.scrollback_height(), 0);
    }

    #[test]
    fn test_write_character() {
        let mut buf = TextBuffer::new();
        buf.write('A');
        assert_eq!(buf.cursor().column(), 1);
    }

    #[test]
    fn test_write_string() {
        let mut buf = TextBuffer::new();
        buf.write_string("Hello");
        assert_eq!(buf.cursor().column(), 5);
    }

    #[test]
    fn test_cursor_position() {
        let mut buf = TextBuffer::new();
        buf.cursor_mut().set_position(10, 20);
        assert_eq!(buf.cursor().position(), (10, 20));
    }

    #[test]
    fn test_get_plain_text() {
        let buf = TextBuffer::new();
        let plain = buf.get_plain_text();
        // Should be empty (all rows are empty)
        assert!(!plain.contains("Hello"));
    }

    #[test]
    fn test_marks() {
        let mut buf = TextBuffer::new();
        buf.marks_mut().start_prompt(100);
        assert!(!buf.marks().is_empty());
    }

    #[test]
    fn test_hyperlinks() {
        let mut buf = TextBuffer::new();
        buf.hyperlinks_mut()
            .add("https://example.com".into(), "id1".into(), None);
        assert!(!buf.hyperlinks().is_empty());
    }

    #[test]
    fn test_selection() {
        let mut buf = TextBuffer::new();
        buf.start_selection(0, 0, SelectionType::Regular);
        assert!(buf.has_selection());

        buf.extend_selection(5, 10);
        assert!(buf.get_selection_extent().is_some());

        buf.clear_selection();
        assert!(!buf.has_selection());
    }

    #[test]
    fn test_screen_to_buffer_conversion() {
        let buf = TextBuffer::new();
        let buffer_row = buf.screen_to_buffer_row(0);
        assert_eq!(buffer_row, DEFAULT_SCROLLBACK_HEIGHT);

        let screen_row = buf.buffer_to_screen_row(DEFAULT_SCROLLBACK_HEIGHT);
        assert_eq!(screen_row, 0);
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    prop_compose! {
        fn any_text_buffer()(
            width in 10i32..200i32,
            height in 10i32..200i32,
            scrollback in 0i32..1000i32,
        ) -> TextBuffer {
            TextBuffer::with_dimensions(width, height, scrollback)
        }
    }

    prop_compose! {
        fn any_printable_char()(
            c in 32u8..127u8,
        ) -> char {
            c as char
        }
    }

    proptest! {
        #[test]
        fn test_write_and_read(width in 10i32..100i32, height in 5i32..50i32) {
            let mut buf = TextBuffer::with_dimensions(width, height, 0);
            buf.cursor_mut().set_position(0, 0);
            buf.write('X');
            let cell = buf.get_cell_data_at(0, 0).unwrap();
            prop_assert_eq!(cell.chars(), "X");
        }

        #[test]
        fn test_cursor_always_within_bounds(buf in any_text_buffer()) {
            let (row, col) = buf.cursor().position();
            prop_assert!((0..buf.screen_height()).contains(&row));
            prop_assert!((0..buf.screen_width()).contains(&col));
        }

        #[test]
        fn test_plain_text_is_readable(buf in any_text_buffer()) {
            let text = buf.get_plain_text();
            // Plain text should contain only printable chars and newlines
            prop_assert!(text.chars().all(|c| c.is_ascii_graphic() || c == '\n'));
        }

        #[test]
        fn test_screen_buffer_conversion_roundtrip(scrollback in 100i32..500i32) {
            let buf = TextBuffer::with_dimensions(80, 24, scrollback);
            for screen_row in 0..buf.screen_height() {
                let buffer_row = buf.screen_to_buffer_row(screen_row);
                let back = buf.buffer_to_screen_row(buffer_row);
                prop_assert_eq!(back, screen_row);
            }
        }
    }
}

// Helper methods for tests
impl TextBuffer {
    pub fn marks_mut(&mut self) -> &mut Marks {
        &mut self.marks
    }

    pub fn hyperlinks_mut(&mut self) -> &mut HyperlinkMap {
        &mut self.hyperlinks
    }
}
