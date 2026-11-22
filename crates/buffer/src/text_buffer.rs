//! `TextBuffer` — the core circular scrollback buffer for the terminal.
//!
//! Ported from `TextBuffer` in the Windows Terminal C++ buffer module.
//! Manages a circular buffer of rows with support for scrollback, alternate
//! screen buffer, and row indexing for screen-to-buffer position conversion.

use std::collections::HashMap;

use crate::TextAttribute;
use crate::cursor::Cursor;
use crate::hyperlinks::HyperlinkMap;
use crate::image_slice::ImageSlice;
use crate::marks::Marks;
use crate::output_cell::OutputCell;
use crate::row::Row;
use types::LineRendition;
use types::UnderlineStyle;
use types::Viewport;
use types::line_rendition::{screen_to_buffer_line, screen_to_buffer_line_inclusive};
use utils::{Color, InclusiveRect, Point, PointSpan, Size};

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

/// One logical line of the buffer: rows joined where the wrap flag is set.
///
/// Upstream has no public equivalent; `TextBuffer::Reflow` joins rows as it
/// copies them. Keeping the join in one place lets it be tested directly and
/// stops `resize` from re-deriving the trailing-whitespace rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalLine {
    /// The cells of the line, left to right.
    cells: Vec<OutputCell>,
    /// The index into `cells` the cursor sits on, if this is the cursor's line.
    cursor_offset: Option<usize>,
    /// The image band of the line's first row, if any.
    image: Option<ImageSlice>,
}

impl LogicalLine {
    /// Returns the cells of the line, left to right.
    pub fn cells(&self) -> &[OutputCell] {
        &self.cells
    }

    /// Returns the cell offset of the cursor, when the cursor is on this line.
    pub fn cursor_offset(&self) -> Option<usize> {
        self.cursor_offset
    }

    /// Returns the image band of the line's first row, if any.
    pub const fn image(&self) -> Option<&ImageSlice> {
        self.image.as_ref()
    }

    /// Returns whether the line holds nothing but blanks with default attributes.
    pub fn is_blank(&self) -> bool {
        self.cells
            .iter()
            .all(|cell| cell.is_whitespace() && *cell.get_attributes() == TextAttribute::new())
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

/// A request to copy a region of the buffer.
///
/// Ported from `TextBuffer::CopyRequest`. The row range `begin.y..=end.y` is
/// inclusive; on the last row `end.x` is exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyRequest {
    /// The first cell of the region.
    pub begin: Point,
    /// The last row's exclusive column.
    pub end: Point,
    /// The leftmost column of a block selection.
    pub min_x: i32,
    /// The rightmost column of a block selection.
    pub max_x: i32,
    /// The region is a rectangle rather than a line-wise selection.
    pub block_selection: bool,
    /// Trim trailing whitespace on rows that are formatted.
    pub trim_trailing_whitespace: bool,
    /// Insert a line break after formatted rows.
    pub include_line_break: bool,
    /// Format wrapped rows instead of joining them to the next row.
    pub format_wrapped_rows: bool,
    /// The points are buffer coordinates rather than screen coordinates.
    pub buffer_coordinates: bool,
}

impl CopyRequest {
    /// Builds a request, clamping it into `buffer`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        buffer: &TextBuffer,
        begin: Point,
        end: Point,
        block_selection: bool,
        include_line_break: bool,
        trim_trailing_whitespace: bool,
        format_wrapped_rows: bool,
        buffer_coordinates: bool,
    ) -> Self {
        let begin = begin.max(Point::new(0, 0));
        // Upstream clamps with a point comparison (row first), not column by
        // column, so a point above the last row keeps its column.
        let end = end.min(Point::new(buffer.screen_width - 1, buffer.buffer_height - 1));

        Self {
            begin,
            end,
            min_x: begin.x.min(end.x),
            max_x: begin.x.max(end.x),
            block_selection,
            trim_trailing_whitespace,
            include_line_break,
            format_wrapped_rows,
            buffer_coordinates,
        }
    }

    /// Builds a request from the clipboard's configuration.
    ///
    /// Ported from `CopyRequest::FromConfig`: a single line collapses the rows
    /// unless it is a block selection, and rows are trimmed only when they are
    /// formatted.
    pub fn from_config(
        buffer: &TextBuffer,
        begin: Point,
        end: Point,
        single_line: bool,
        block_selection: bool,
        trim_block_selection: bool,
        buffer_coordinates: bool,
    ) -> Self {
        Self::new(
            buffer,
            begin,
            end,
            block_selection,
            !single_line || block_selection,
            !single_line && (!block_selection || trim_block_selection),
            block_selection,
            buffer_coordinates,
        )
    }
}

/// Resolves an attribute to the colours it is drawn with: foreground,
/// background and underline.
///
/// Upstream passes this in as `GetAttributeColors`; the renderer's colour
/// table supplies the values.
pub type AttributeColorsFn<'a> = dyn Fn(&TextAttribute) -> (Color, Color, Color) + 'a;

impl TextBuffer {
    /// Creates a new text buffer with default dimensions.
    pub fn new() -> Self {
        Self::with_dimensions(80, 24, DEFAULT_SCROLLBACK_HEIGHT)
    }

    /// Creates a new text buffer with the given dimensions.
    pub fn with_dimensions(width: i32, height: i32, scrollback_height: i32) -> Self {
        let mut buffer = Self {
            screen_width: width,
            screen_height: height,
            scrollback_height,
            buffer_height: height + scrollback_height,
            rows: VecDequeCompat::new(0),
            screen_line_index: HashMap::new(),
            is_alternate: false,
            cursor: Cursor::new(),
            marks: Marks::new(),
            hyperlinks: HyperlinkMap::new(),
            mutation_id: 0,
            scrollback: std::collections::VecDeque::new(),
        };
        buffer.allocate_rows();
        buffer
    }

    /// Allocates blank rows for the current dimensions and rebuilds the screen
    /// index.
    ///
    /// The scrollback slots are allocated as well as the visible rows: the
    /// screen index is offset by the scrollback height, so a buffer that only
    /// allocated its visible rows could not look any of them up.
    fn allocate_rows(&mut self) {
        self.rows = VecDequeCompat::new(self.buffer_height as usize);
        for index in 0..self.buffer_height {
            self.rows.push_back(BufferedRow::new(
                Row::new(self.screen_width),
                index - self.scrollback_height,
            ));
        }

        self.screen_line_index.clear();
        for line in 0..self.screen_height {
            let buffer_index = line + self.scrollback_height;
            self.screen_line_index
                .insert(line, (buffer_index as usize) % self.buffer_height as usize);
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

    /// Registers an OSC 8 hyperlink and returns the id cells carry.
    ///
    /// Ported from `TextBuffer::GetHyperlinkId`.
    pub fn get_hyperlink_id(&mut self, uri: &str, custom_id: &str) -> u16 {
        self.hyperlinks.register(uri, custom_id)
    }

    /// Resolves a cell's hyperlink id back to its URI.
    ///
    /// Ported from `TextBuffer::GetHyperlinkUriFromId`.
    pub fn get_hyperlink_uri(&self, id: u16) -> Option<&str> {
        self.hyperlinks.uri(id)
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

    /// Joins the buffer's rows into logical lines using the wrap flags.
    ///
    /// A row whose `is_wrap` is set continues the line above: `do_line_feed`
    /// records the flag when autowrap breaks a line. The last row of each line
    /// contributes up to its last non-space column, matching upstream
    /// `ROW::MeasureRight`; a wrapped row contributes its full width, because
    /// the whitespace before a wrap is content, not padding. Scrollback rows
    /// come first, so a logical line may span the scrollback/screen boundary.
    pub fn logical_lines(&self) -> Vec<LogicalLine> {
        let (cursor_row, cursor_column) = self.cursor.position();
        let cursor_row = cursor_row + self.scrollback.len() as i32;

        let mut lines = Vec::new();
        let mut cells: Vec<OutputCell> = Vec::new();
        let mut cursor_offset: Option<usize> = None;
        let mut line_image: Option<ImageSlice> = None;

        for (index, row) in self.document_rows().enumerate() {
            let is_cursor_row = index as i32 == cursor_row;
            let mut limit = row_content_limit(row);
            if is_cursor_row {
                // REFLOW_JANK_CURSOR_WRAP: treat the cell under the cursor as
                // content so the cursor keeps its distance from the text before
                // it, even when it sits in trailing whitespace.
                limit = limit.max(cursor_column + 1).min(row.width());
            }

            if cells.is_empty() {
                // The first row of a line owns the image band (ADR 005);
                // upstream's reflow copies exactly that band.
                line_image = row.image_slice().cloned();
            }

            let start = cells.len();
            for column in 0..limit {
                cells.push(row.get_cell(column).clone());
            }
            if is_cursor_row {
                cursor_offset = Some(start + cursor_column.clamp(0, (limit - 1).max(0)) as usize);
            }

            if !row.is_wrap() {
                lines.push(LogicalLine {
                    cells: std::mem::take(&mut cells),
                    cursor_offset: cursor_offset.take(),
                    image: line_image.take(),
                });
            }
        }

        if !cells.is_empty() || cursor_offset.is_some() {
            lines.push(LogicalLine {
                cells,
                cursor_offset,
                image: line_image,
            });
        }

        lines
    }

    /// Iterates the rows in document order: scrollback first, then the screen.
    fn document_rows(&self) -> impl Iterator<Item = &Row> + '_ {
        self.scrollback
            .iter()
            .chain((0..self.screen_height).filter_map(|line| self.get_row_at_screen_line(line)))
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
            if let Some(buffer_row) = self.get_buffer_row_mut_at_screen(r, start_col) {
                for c in start_col..=end_col {
                    buffer_row.row.write_cell_raw(c, cell.clone());
                }
                buffer_row.row.erase_image_cells(start_col, end_col + 1);
            }
        }
        self.mutation_id += 1;
    }

    /// Returns the row at a screen line, mutably.
    pub fn get_row_at_screen_line_mut(&mut self, screen_line: i32) -> Option<&mut Row> {
        self.get_buffer_row_mut_at_screen(screen_line, 0)
            .map(|buffered| &mut buffered.row)
    }

    /// Returns a row's line rendition, or `SingleWidth` outside the page.
    pub fn get_line_rendition(&self, screen_line: i32) -> LineRendition {
        self.get_row_at_screen_line(screen_line)
            .map_or(LineRendition::SingleWidth, Row::line_rendition)
    }

    /// Returns the usable width of a row, halved for double-width renditions.
    pub fn get_line_width(&self, screen_line: i32) -> i32 {
        let scale = u32::from(!matches!(
            self.get_line_rendition(screen_line),
            LineRendition::SingleWidth
        ));
        self.screen_width >> scale
    }

    /// Sets the current row's line rendition.
    ///
    /// Ported from `TextBuffer::SetCurrentLineRendition`: a changed rendition
    /// clears the wrap flag and the image band, a non-single-width line erases
    /// its right half, and the cursor is clamped inside the new width.
    pub fn set_current_line_rendition(&mut self, rendition: LineRendition, fill_attributes: TextAttribute) {
        let row = self.cursor.row();
        if self.get_line_rendition(row) == rendition {
            return;
        }

        if let Some(row_data) = self.get_row_at_screen_line_mut(row) {
            row_data.set_line_rendition(rendition);
            row_data.set_wrap(false);
            row_data.set_image_slice(None);
        }

        if !matches!(rendition, LineRendition::SingleWidth) {
            let mut blank = OutputCell::from_char(' ');
            blank.set_attributes(fill_attributes);
            let line_width = self.get_line_width(row);
            self.fill_rect(row, line_width, row, self.screen_width - 1, &blank);

            let column = self.cursor.column().min((line_width - 1).max(0));
            self.cursor.set_position(row, column);
        }

        self.mutation_id += 1;
    }

    /// Resets the rendition of every row in `start..end` to single width.
    pub fn reset_line_rendition_range(&mut self, start: i32, end: i32) {
        for row in start..end {
            if let Some(row_data) = self.get_row_at_screen_line_mut(row) {
                row_data.set_line_rendition(LineRendition::SingleWidth);
            }
        }
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

    /// Returns how many rows the document holds: history plus the screen.
    pub fn document_height(&self) -> i32 {
        self.scrollback.len() as i32 + self.screen_height
    }

    /// Returns the row at `line` in document order: history first, then the
    /// visible screen.
    ///
    /// This is the coordinate system a copy request with
    /// `buffer_coordinates` uses, and the one a scrolled viewport paints from.
    /// It is not the fixed-slot mapping of
    /// [`get_row_by_buffer_line`](Self::get_row_by_buffer_line): document row 0
    /// is the oldest row still held, wherever the ring happens to sit.
    pub fn get_row_by_document_line(&self, line: i32) -> Option<&Row> {
        if line < 0 {
            return None;
        }

        let history = self.scrollback.len() as i32;
        if line < history {
            self.scrollback.get(line as usize)
        } else {
            self.get_row_at_screen_line(line - history)
        }
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

    /// Resizes the buffer, rewrapping the text into the new width.
    ///
    /// Ported from `TextBuffer::Reflow`: logical lines are rewrapped at the new
    /// width, the wrap flags are re-derived, and the cursor is remapped by its
    /// position within its line rather than by its column. The bottom of the
    /// document stays at the bottom of the page, so rows that no longer fit move
    /// into the scrollback and become visible again when the buffer grows. A
    /// cursor whose line is pushed entirely into the scrollback is clamped to
    /// the top visible row.
    pub fn resize(&mut self, new_width: i32, new_height: i32) {
        if new_width <= 0 || new_height <= 0 {
            return;
        }
        if new_width == self.screen_width && new_height == self.screen_height {
            return;
        }

        let lines = self.logical_lines();
        // Rows after the last content row are re-derived blank rows, so there is
        // nothing to rewrap there; the cursor's line is kept even when blank.
        let used_lines = lines
            .iter()
            .rposition(|line| !line.is_blank() || line.cursor_offset().is_some())
            .map_or(0, |index| index + 1);
        let (mut new_rows, cursor) = rewrap_lines(&lines[..used_lines], new_width);

        while new_rows.len() < new_height as usize {
            new_rows.push(Row::new(new_width));
        }

        let visible_start = new_rows.len() - new_height as usize;
        let mut scrollback: std::collections::VecDeque<Row> = new_rows.drain(..visible_start).collect();
        while scrollback.len() > self.scrollback_height.max(0) as usize {
            scrollback.pop_front();
        }

        self.screen_width = new_width;
        self.screen_height = new_height;
        self.buffer_height = new_height + self.scrollback_height;
        self.allocate_rows();
        for (offset, row) in new_rows.into_iter().enumerate() {
            if let Some(buffered) = self.rows.get_mut(self.scrollback_height as usize + offset) {
                buffered.row = row;
            }
        }
        self.scrollback = scrollback;

        let (row, column) = cursor.unwrap_or((0, 0));
        let row = (row - visible_start as i32).clamp(0, new_height - 1);
        let column = column.clamp(0, new_width - 1);
        self.cursor.set_position(row, column);
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
        let blank = OutputCell::from_char(' ');
        self.scroll_rect_horizontally(top, bottom, left, right, -1, &blank);
    }

    /// Shifts the cells in `left..=right` one column right on every row of the
    /// region, blanking the leftmost column (DECBI).
    pub fn scroll_region_right(&mut self, top: i32, bottom: i32, left: i32, right: i32) {
        let blank = OutputCell::from_char(' ');
        self.scroll_rect_horizontally(top, bottom, left, right, 1, &blank);
    }

    /// Scrolls a rectangle vertically by `delta` rows.
    ///
    /// Positive `delta` moves content down (insert lines); negative moves it up
    /// (delete lines). The rows revealed at the edge are filled with `blank`,
    /// and image pixels inside the rectangle travel with the cells.
    pub fn scroll_rect_vertically(
        &mut self,
        top: i32,
        bottom: i32,
        left: i32,
        right: i32,
        delta: i32,
        blank: &OutputCell,
    ) {
        if bottom < top || right < left {
            return;
        }

        let height = bottom - top + 1;
        let amount = delta.unsigned_abs().min(height as u32) as i32;
        if amount == 0 {
            return;
        }

        if amount < height {
            if delta > 0 {
                // Content moves down: start at the bottom so a destination row
                // is never a later source.
                for dest in (top + amount..=bottom).rev() {
                    self.copy_rect_row(dest, dest - amount, left, right);
                }
            } else {
                // Content moves up: sources are below their destinations.
                for dest in top..=bottom - amount {
                    self.copy_rect_row(dest, dest + amount, left, right);
                }
            }
        }

        let erase_top = if delta > 0 { top } else { bottom - amount + 1 };
        self.fill_rect(erase_top, left, erase_top + amount - 1, right, blank);
        self.mutation_id += 1;
    }

    /// Scrolls a rectangle horizontally by `delta` columns.
    ///
    /// Positive `delta` moves content right (insert columns); negative moves it
    /// left (delete columns). The columns revealed at the edge are filled with
    /// `blank`.
    pub fn scroll_rect_horizontally(
        &mut self,
        top: i32,
        bottom: i32,
        left: i32,
        right: i32,
        delta: i32,
        blank: &OutputCell,
    ) {
        if bottom < top || right < left {
            return;
        }

        let width = right - left + 1;
        let amount = delta.unsigned_abs().min(width as u32) as i32;
        if amount == 0 {
            return;
        }

        if amount < width {
            let copy_width = width - amount;
            let (source_left, dest_left) = if delta > 0 {
                (left, left + amount)
            } else {
                (left + amount, left)
            };

            for row in top..=bottom {
                let Some(source) = self.get_row_at_screen_line(row).cloned() else {
                    continue;
                };
                let Some(destination) = self.get_row_at_screen_line_mut(row) else {
                    continue;
                };
                destination.copy_text_from(dest_left, &source, source_left, copy_width);
                destination.copy_image_cells_from(&source, source_left, dest_left, dest_left + copy_width);
            }
        }

        let erase_left = if delta > 0 { left } else { right - amount + 1 };
        self.fill_rect(top, erase_left, bottom, erase_left + amount - 1, blank);
        self.mutation_id += 1;
    }

    /// Copies one rectangle-wide slice of a row into another row.
    fn copy_rect_row(&mut self, dest_row: i32, source_row: i32, left: i32, right: i32) {
        let Some(source) = self.get_row_at_screen_line(source_row).cloned() else {
            return;
        };
        let Some(destination) = self.get_row_at_screen_line_mut(dest_row) else {
            return;
        };
        let width = right - left + 1;
        destination.copy_text_from(left, &source, left, width);
        destination.copy_image_cells_from(&source, left, left, right + 1);
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
            dest_row.row.copy_image_from(&source_row);
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
    ///
    /// A diagnostic helper, not upstream's `GetPlainText`; the copy path is
    /// [`get_plain_text`](Self::get_plain_text).
    pub fn get_all_text(&self) -> String {
        let mut result = String::new();
        for i in 0..self.rows.len() {
            let row = &self.rows[i];
            result.push_str(&row.row.get_text());
            result.push('\n');
        }
        result
    }

    /// Returns the rows a selection covers as inclusive rectangles.
    ///
    /// Ported from `TextBuffer::GetTextRects`. A block selection uses the same
    /// columns on every row; a line-wise one runs from the first point's column
    /// to the end of its row, then full rows, then the last point's column.
    /// Each row is widened to whole wide characters.
    pub fn get_text_rects(
        &self,
        start: Point,
        end: Point,
        block_selection: bool,
        buffer_coordinates: bool,
    ) -> Vec<InclusiveRect> {
        let (higher, lower) = if start <= end { (start, end) } else { (end, start) };
        let buffer = self.buffer_viewport();
        let mut rects = Vec::with_capacity((1 + lower.y - higher.y).max(0) as usize);

        for row in higher.y..=lower.y {
            let mut text_row = if block_selection || higher.y == lower.y {
                InclusiveRect::new(higher.x.min(lower.x), row, higher.x.max(lower.x), row)
            } else {
                let left = if row == higher.y { higher.x } else { buffer.left() };
                let right = if row == lower.y {
                    lower.x
                } else {
                    buffer.right_inclusive()
                };
                InclusiveRect::new(left, row, right, row)
            };

            if !buffer_coordinates {
                text_row = screen_to_buffer_line(text_row, self.get_line_rendition(row));
            }

            self.expand_text_row(&mut text_row);
            rects.push(text_row);
        }

        rects
    }

    /// Returns the spans a selection covers.
    ///
    /// Ported from `TextBuffer::GetTextSpans`: one span per row for a block
    /// selection, and a single span for a line-wise one.
    pub fn get_text_spans(
        &self,
        start: Point,
        end: Point,
        block_selection: bool,
        buffer_coordinates: bool,
    ) -> Vec<PointSpan> {
        if block_selection {
            return self
                .get_text_rects(start, end, true, buffer_coordinates)
                .into_iter()
                .map(|rect| {
                    PointSpan::new(
                        Point::new(rect.left(), rect.top()),
                        Point::new(rect.right(), rect.bottom()),
                    )
                })
                .collect();
        }

        let (mut higher, mut lower) = if start <= end { (start, end) } else { (end, start) };
        if !buffer_coordinates {
            higher = screen_to_buffer_line_inclusive(higher, self.get_line_rendition(higher.y));
            lower = screen_to_buffer_line_inclusive(lower, self.get_line_rendition(lower.y));
        }

        let mut span = InclusiveRect::new(higher.x, higher.y, lower.x, lower.y);
        self.expand_text_row(&mut span);
        vec![PointSpan::new(
            Point::new(span.left(), span.top()),
            Point::new(span.right(), span.bottom()),
        )]
    }

    /// Widens a row so it does not split a wide character.
    ///
    /// Ported from `TextBuffer::_ExpandTextRow`.
    fn expand_text_row(&self, row: &mut InclusiveRect) {
        let buffer = self.buffer_viewport();

        let mut left = Point::new(row.left(), row.top());
        if buffer.contains(left) && self.cell_is_trailing_half(left) {
            buffer.decrement_in_exclusive_bounds(&mut left);
            *row = InclusiveRect::new(left.x, row.top(), row.right(), row.bottom());
        }

        let mut right = Point::new(row.right(), row.bottom());
        if buffer.contains(right) && self.cell_is_trailing_half(right) {
            buffer.increment_in_exclusive_bounds(&mut right);
            *row = InclusiveRect::new(row.left(), row.top(), right.x, row.bottom());
        }
    }

    /// Returns whether the cell at `position` is the trailing half of a wide
    /// character.
    fn cell_is_trailing_half(&self, position: Point) -> bool {
        self.get_row_at_screen_line(position.y)
            .is_some_and(|row| row.is_trailing_half(position.x))
    }

    /// Returns the columns and line-break flag for one row of a copy request.
    ///
    /// Ported from `TextBuffer::_RowCopyHelper`.
    /// Returns the row a copy request addresses at `row`.
    ///
    /// Screen coordinates address the visible screen; buffer coordinates
    /// address the document, history first. Upstream's `GetRowByOffset` is the
    /// second mapping, which is what lets a selection cross into scrollback.
    fn request_row(&self, request: &CopyRequest, row: i32) -> Option<&Row> {
        if request.buffer_coordinates {
            self.get_row_by_document_line(row)
        } else {
            self.get_row_at_screen_line(row)
        }
    }

    fn row_copy_helper(&self, request: &CopyRequest, row_index: i32, row: &Row) -> (i32, i32, bool) {
        let rendition = row.line_rendition();

        let (row_begin, mut row_end) = if request.block_selection {
            let min_x = if request.buffer_coordinates {
                request.min_x
            } else {
                screen_to_buffer_line_inclusive(Point::new(request.min_x, row_index), rendition).x
            };
            let max_x = if request.buffer_coordinates {
                request.max_x
            } else {
                screen_to_buffer_line_inclusive(Point::new(request.max_x, row_index), rendition).x
            };
            (min_x, max_x)
        } else {
            let begin = if request.buffer_coordinates {
                request.begin
            } else {
                screen_to_buffer_line_inclusive(request.begin, rendition)
            };
            let end = if request.buffer_coordinates {
                request.end
            } else {
                screen_to_buffer_line_inclusive(request.end, rendition)
            };

            let row_begin = if row_index != begin.y { 0 } else { begin.x };
            let row_end = if row_index != end.y {
                row.get_readable_column_count()
            } else {
                end.x
            };
            (row_begin, row_end)
        };

        // The selection does not stop on glyph boundaries, so widen the range
        // rather than copy half a wide character.
        let row_begin = row.adjust_to_glyph_start(row_begin);
        row_end = row.adjust_to_glyph_end(row_end);

        // Wrapped rows are joined unless the caller asks for them to be
        // formatted like explicit lines.
        let should_format_row = request.format_wrapped_rows || !row.is_wrap();

        if should_format_row && request.trim_trailing_whitespace {
            row_end = row_end.min(row.get_last_non_space_column());
        }

        (row_begin, row_end, should_format_row && request.include_line_break)
    }

    /// Returns the cells of `request` as plain text.
    ///
    /// Ported from `TextBuffer::GetPlainText(const CopyRequest&)`. Rows are
    /// joined by `\r\n` when the request asks for line breaks; a row that was
    /// wrapped by the buffer is joined to the next one instead.
    pub fn get_plain_text(&self, request: &CopyRequest) -> String {
        if request.begin > request.end {
            return String::new();
        }

        let mut text = String::new();
        for row_index in request.begin.y..=request.end.y {
            let Some(row) = self.request_row(request, row_index) else {
                continue;
            };

            let (row_begin, row_end, add_line_break) = self.row_copy_helper(request, row_index, row);
            text.push_str(&row.get_text_range(row_begin, row_end));

            if add_line_break && row_index != request.end.y {
                text.push_str("\r\n");
            }
        }
        text
    }

    /// Returns the cells of `request` as a CF_HTML clipboard fragment.
    ///
    /// Ported from `TextBuffer::GenHTML`. `attribute_colors` resolves each
    /// attribute to the colours it is drawn with, which is the renderer's
    /// colour table.
    pub fn gen_html(
        &self,
        request: &CopyRequest,
        font_height_points: f32,
        font_face: &str,
        background: Color,
        is_intense_bold: bool,
        attribute_colors: &AttributeColorsFn<'_>,
    ) -> String {
        if request.begin > request.end {
            return String::new();
        }

        // The CF_HTML header is a fixed size so its own offsets are stable.
        const HTML_HEADER: &str = "<!DOCTYPE><HTML><HEAD></HEAD><BODY>";
        const HTML_FOOTER: &str = "</BODY></HTML>";
        const CLIPBOARD_HEADER_SIZE: usize = 157;

        let mut html = String::new();
        html.push_str(HTML_HEADER);
        html.push_str("<!--StartFragment -->");

        html.push_str("<DIV STYLE=\"");
        html.push_str("display:inline-block;");
        html.push_str("white-space:pre;");
        html.push_str(&format!("background-color:{};", background.to_hex_short()));
        // Even with a different face, keep a monospace fallback.
        html.push_str(&format!("font-family:'{font_face}',monospace;"));
        html.push_str(&format!("font-size:{}pt;", font_height_points.round() as i32));
        html.push_str("padding:4px;");
        html.push_str("\">");

        for row_index in request.begin.y..=request.end.y {
            let Some(row) = self.request_row(request, row_index) else {
                continue;
            };

            let (row_begin, row_end, add_line_break) = self.row_copy_helper(request, row_index, row);
            let mut x = row_begin;

            for (attribute, length) in row.attribute_runs(row_begin, row_end) {
                let next_x = x + length;
                let (foreground, row_background, underline) = attribute_colors(&attribute);
                let foreground_hex = foreground.to_hex_short();
                let background_hex = row_background.to_hex_short();
                let underline_hex = underline.to_hex_short();
                let underline_style = attribute.get_underline_style();
                let is_underlined = underline_style != UnderlineStyle::NoUnderline;
                let is_crossed_out = attribute.is_crossed_out();
                let is_overlined = attribute.is_top_horizontal_displayed();

                html.push_str("<SPAN STYLE=\"");
                html.push_str(&format!("color:{foreground_hex};"));
                html.push_str(&format!("background-color:{background_hex};"));

                if attribute.is_bold(is_intense_bold) {
                    html.push_str("font-weight:bold;");
                }
                if attribute.is_italic() {
                    html.push_str("font-style:italic;");
                }

                if is_crossed_out || is_overlined {
                    html.push_str(&format!(
                        "text-decoration:{} {} {};",
                        if is_crossed_out { "line-through" } else { "" },
                        if is_overlined { "overline" } else { "" },
                        foreground_hex
                    ));
                }

                if is_underlined {
                    // Underline, overline and strike share one CSS property,
                    // so the underline gets its own nested span with its own
                    // colour.
                    html.push_str("\"><SPAN STYLE=\"");

                    match underline_style {
                        UnderlineStyle::NoUnderline => {}
                        UnderlineStyle::DoublyUnderlined => {
                            html.push_str(&format!("text-decoration:underline double {underline_hex};"));
                        }
                        UnderlineStyle::CurlyUnderlined => {
                            html.push_str(&format!("text-decoration:underline wavy {underline_hex};"));
                        }
                        UnderlineStyle::DottedUnderlined => {
                            html.push_str(&format!("text-decoration:underline dotted {underline_hex};"));
                        }
                        UnderlineStyle::DashedUnderlined => {
                            html.push_str(&format!("text-decoration:underline dashed {underline_hex};"));
                        }
                        UnderlineStyle::SinglyUnderlined => {
                            html.push_str(&format!("text-decoration:underline {underline_hex};"));
                        }
                    }
                }

                html.push_str("\">");

                for character in row.get_text_range(x, next_x).chars() {
                    match character {
                        '<' => html.push_str("&lt;"),
                        '>' => html.push_str("&gt;"),
                        '&' => html.push_str("&amp;"),
                        _ => html.push(character),
                    }
                }

                if is_underlined {
                    html.push_str("</SPAN>");
                }
                html.push_str("</SPAN>");

                x = next_x;
            }

            if add_line_break && row_index < request.end.y {
                html.push_str("<BR>");
            }
        }

        html.push_str("</DIV>");
        html.push_str("<!--EndFragment -->");
        html.push_str(HTML_FOOTER);

        let html_start = CLIPBOARD_HEADER_SIZE;
        let html_end = CLIPBOARD_HEADER_SIZE + html.len();
        let fragment_start = CLIPBOARD_HEADER_SIZE + HTML_HEADER.len();
        let fragment_end = html_end - HTML_FOOTER.len();

        let mut clipboard = String::new();
        clipboard.push_str("Version:0.9\r\n");
        clipboard.push_str(&format!("StartHTML:{html_start:0>10}\r\n"));
        clipboard.push_str(&format!("EndHTML:{html_end:0>10}\r\n"));
        clipboard.push_str(&format!("StartFragment:{fragment_start:0>10}\r\n"));
        clipboard.push_str(&format!("EndFragment:{fragment_end:0>10}\r\n"));
        clipboard.push_str(&format!("StartSelection:{fragment_start:0>10}\r\n"));
        clipboard.push_str(&format!("EndSelection:{fragment_end:0>10}\r\n"));

        clipboard + &html
    }

    /// Returns the cells of `request` as an RTF document.
    ///
    /// Ported from `TextBuffer::GenRTF`.
    pub fn gen_rtf(
        &self,
        request: &CopyRequest,
        font_height_points: f32,
        font_face: &str,
        background: Color,
        is_intense_bold: bool,
        attribute_colors: &AttributeColorsFn<'_>,
    ) -> String {
        if request.begin > request.end {
            return String::new();
        }

        let mut content = String::new();
        let mut color_table = String::from("{\\colortbl ;");

        {
            let mut color_map: HashMap<Color, usize> = HashMap::new();
            let mut color_index = |color: Color| -> usize {
                let next = color_map.len() + 1;
                match color_map.entry(color) {
                    std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(next);
                        color_table.push_str(&format!("\\red{}\\green{}\\blue{};", color.r, color.g, color.b));
                        next
                    }
                }
            };

            // Normal view, one unicode fallback per code point.
            content.push_str("\\viewkind4\\uc1");
            content.push_str("\\pard\\slmult1\\f0");
            // RTF sizes are half-points.
            content.push_str(&format!("\\fs{}", (2.0 * font_height_points).round() as i32));
            content.push_str(&format!("\\chshdng0\\chcbpat{}", color_index(background)));

            for row_index in request.begin.y..=request.end.y {
                let Some(row) = self.request_row(request, row_index) else {
                    continue;
                };

                let (row_begin, row_end, add_line_break) = self.row_copy_helper(request, row_index, row);
                let mut x = row_begin;

                for (attribute, length) in row.attribute_runs(row_begin, row_end) {
                    let next_x = x + length;
                    let (foreground, row_background, underline) = attribute_colors(&attribute);
                    let foreground_index = color_index(foreground);
                    let background_index = color_index(row_background);

                    // Each run is a group so the default attribute comes back
                    // at its close.
                    content.push('{');
                    content.push_str(&format!("\\cf{foreground_index}"));
                    content.push_str(&format!("\\chshdng0\\chcbpat{background_index}"));

                    if attribute.is_bold(is_intense_bold) {
                        content.push_str("\\b");
                    }
                    if attribute.is_italic() {
                        content.push_str("\\i");
                    }
                    if attribute.is_crossed_out() {
                        content.push_str("\\strike");
                    }

                    let underline_index = color_index(underline);
                    match attribute.get_underline_style() {
                        UnderlineStyle::NoUnderline => {}
                        UnderlineStyle::DoublyUnderlined => {
                            content.push_str(&format!("\\uldb\\ulc{underline_index}"));
                        }
                        UnderlineStyle::CurlyUnderlined => {
                            content.push_str(&format!("\\ulwave\\ulc{underline_index}"));
                        }
                        UnderlineStyle::DottedUnderlined => {
                            content.push_str(&format!("\\uld\\ulc{underline_index}"));
                        }
                        UnderlineStyle::DashedUnderlined => {
                            content.push_str(&format!("\\uldash\\ulc{underline_index}"));
                        }
                        UnderlineStyle::SinglyUnderlined => {
                            content.push_str(&format!("\\ul\\ulc{underline_index}"));
                        }
                    }

                    // A command and text must be separated by a space, or a
                    // leading space in the text is taken as part of the command.
                    content.push(' ');
                    Self::append_rtf_text(&mut content, &row.get_text_range(x, next_x));
                    content.push('}');

                    x = next_x;
                }

                if add_line_break && row_index < request.end.y {
                    content.push_str("\\line");
                }
            }
        }

        let mut rtf = String::new();
        rtf.push('{');
        rtf.push_str("\\rtf1\\ansi\\ansicpg1252\\deff0\\nouicompat");
        rtf.push_str(&format!("{{\\fonttbl{{\\f0\\fmodern\\fcharset0 {font_face};}}}}"));
        rtf.push_str(&color_table);
        rtf.push('}');
        rtf.push_str(&content);
        rtf.push('}');
        rtf
    }

    /// Appends text escaped for an RTF stream.
    ///
    /// Ported from `TextBuffer::_AppendRTFText`: braces and backslashes are
    /// escaped, and anything outside ASCII becomes a signed `\uN?` escape per
    /// UTF-16 code unit.
    fn append_rtf_text(builder: &mut String, text: &str) {
        for unit in text.encode_utf16() {
            if unit <= 127 {
                let character = char::from(unit as u8);
                match character {
                    '\\' | '{' | '}' => {
                        builder.push('\\');
                        builder.push(character);
                    }
                    _ => builder.push(character),
                }
            } else {
                builder.push_str(&format!("\\u{}?", unit as i16));
            }
        }
    }

    /// Returns the buffer's extent as a viewport.
    fn buffer_viewport(&self) -> Viewport {
        Viewport::new(Size::new(self.screen_width, self.buffer_height))
    }

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

/// The number of columns a row contributes to its logical line.
///
/// `Row::measure_right` already gives one past the last non-space column for
/// an explicit line and the full width for a wrapped one.
fn row_content_limit(row: &Row) -> i32 {
    row.measure_right()
}

/// Rewraps logical lines at `width`.
///
/// Returns the new rows and the cursor's `(row, column)` within them. A row is
/// marked wrapped only when the next cell starts a new row; the last row of a
/// logical line is an explicit line ending and is left unwrapped.
fn rewrap_lines(lines: &[LogicalLine], width: i32) -> (Vec<Row>, Option<(i32, i32)>) {
    let width_usize = width as usize;
    let mut rows = Vec::new();
    let mut cursor = None;

    for line in lines {
        let mut first_row = true;
        let mut cells: Vec<OutputCell> = Vec::with_capacity(width_usize);
        for (offset, cell) in line.cells.iter().enumerate() {
            if cells.len() == width_usize {
                let mut row = Row::from_cells(width, std::mem::take(&mut cells), true);
                if first_row {
                    row.set_image_slice(line.image.clone());
                    first_row = false;
                }
                rows.push(row);
            }
            if line.cursor_offset == Some(offset) {
                cursor = Some((rows.len() as i32, cells.len() as i32));
            }
            cells.push(cell.clone());
        }
        if line.cursor_offset == Some(line.cells.len()) {
            cursor = Some((rows.len() as i32, cells.len() as i32));
        }
        let mut row = Row::from_cells(width, cells, false);
        if first_row {
            row.set_image_slice(line.image.clone());
        }
        rows.push(row);
    }

    (rows, cursor)
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
    fn scroll_rect_vertically_respects_columns() {
        let mut buffer = TextBuffer::with_dimensions(4, 3, 0);
        write_text_at(&mut buffer, 0, 0, "abXX");
        write_text_at(&mut buffer, 1, 0, "cdXX");
        write_text_at(&mut buffer, 2, 0, "efXX");

        let blank = OutputCell::from_char(' ');
        buffer.scroll_rect_vertically(0, 2, 0, 1, -1, &blank);

        assert_eq!(text_at(&buffer, 0, 0), "c");
        assert_eq!(text_at(&buffer, 1, 0), "e");
        assert_eq!(text_at(&buffer, 2, 0), " ");
        // Columns outside the rectangle keep their content.
        assert_eq!(text_at(&buffer, 0, 2), "X");
        assert_eq!(text_at(&buffer, 2, 2), "X");
    }

    #[test]
    fn scroll_rect_vertically_moves_by_a_whole_count() {
        let mut buffer = TextBuffer::with_dimensions(1, 4, 0);
        for (row, text) in ["a", "b", "c", "d"].iter().enumerate() {
            write_text_at(&mut buffer, row as i32, 0, text);
        }

        let blank = OutputCell::from_char(' ');
        buffer.scroll_rect_vertically(0, 3, 0, 0, 2, &blank);

        assert_eq!(text_at(&buffer, 0, 0), " ");
        assert_eq!(text_at(&buffer, 1, 0), " ");
        assert_eq!(text_at(&buffer, 2, 0), "a");
        assert_eq!(text_at(&buffer, 3, 0), "b");
    }

    #[test]
    fn scroll_rect_horizontally_respects_rows() {
        let mut buffer = TextBuffer::with_dimensions(4, 2, 0);
        write_text_at(&mut buffer, 0, 0, "abcd");
        write_text_at(&mut buffer, 1, 0, "efgh");

        let blank = OutputCell::from_char(' ');
        buffer.scroll_rect_horizontally(0, 0, 0, 3, -1, &blank);

        assert_eq!(text_at(&buffer, 0, 0), "b");
        assert_eq!(text_at(&buffer, 0, 2), "d");
        assert_eq!(text_at(&buffer, 0, 3), " ");
        // The other row is outside the rectangle.
        assert_eq!(text_at(&buffer, 1, 0), "e");
        assert_eq!(text_at(&buffer, 1, 3), "h");
    }

    #[test]
    fn a_double_width_line_reports_half_its_width() {
        let mut buffer = TextBuffer::with_dimensions(10, 2, 0);
        assert_eq!(buffer.get_line_width(0), 10);

        buffer.set_current_line_rendition(LineRendition::DoubleWidth, TextAttribute::new());
        assert_eq!(buffer.get_line_width(0), 5);
        assert_eq!(buffer.get_line_rendition(0), LineRendition::DoubleWidth);
    }

    #[test]
    fn a_double_width_line_erases_its_right_half_and_clamps_the_cursor() {
        let mut buffer = TextBuffer::with_dimensions(10, 2, 0);
        for column in 0..10 {
            buffer
                .get_row_at_screen_line_mut(0)
                .unwrap()
                .write_char(column, 'x', false);
        }
        buffer.cursor_mut().set_position(0, 9);

        buffer.set_current_line_rendition(LineRendition::DoubleWidth, TextAttribute::new());

        assert_eq!(buffer.cursor().position(), (0, 4));
        assert_eq!(text_at(&buffer, 0, 4), "x");
        assert_eq!(text_at(&buffer, 0, 5), " ");
    }

    #[test]
    fn resetting_the_rendition_range_restores_full_width() {
        let mut buffer = TextBuffer::with_dimensions(10, 3, 0);
        buffer.cursor_mut().set_position(1, 0);
        buffer.set_current_line_rendition(LineRendition::DoubleHeightTop, TextAttribute::new());
        assert_eq!(buffer.get_line_width(1), 5);

        buffer.reset_line_rendition_range(0, 3);
        assert_eq!(buffer.get_line_width(1), 10);
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
        let plain = buf.get_all_text();
        // Should be empty (all rows are empty)
        assert!(!plain.contains("Hello"));
    }

    /// Writes a line into a screen row, advancing by each character's width.
    fn write_row(buffer: &mut TextBuffer, row_index: i32, text: &str, wrap: bool) {
        let row = buffer
            .get_row_at_screen_line_mut(row_index)
            .expect("row is within the page");
        row.set_wrap(wrap);

        let mut column = 0;
        for character in text.chars() {
            row.write_char(column, character, false);
            column += i32::from(types::codepoint_width(character as u32).max(1));
        }
    }

    #[test]
    fn copy_rects_follow_the_selection_shape() {
        // Ported from `TextBufferTests::GetTextRects`.
        let mut buffer = TextBuffer::with_dimensions(20, 50, 0);
        for (index, text) in ["0123456789", " 中3456中", "  中45中", "中234567中", "0123456789"]
            .iter()
            .enumerate()
        {
            write_row(&mut buffer, index as i32, text, false);
        }

        let start = Point::new(1, 0);
        let end = Point::new(8, 4);

        assert_eq!(
            buffer.get_text_rects(start, end, false, false),
            vec![
                InclusiveRect::new(1, 0, 19, 0),
                InclusiveRect::new(0, 1, 19, 1),
                InclusiveRect::new(0, 2, 19, 2),
                InclusiveRect::new(0, 3, 19, 3),
                InclusiveRect::new(0, 4, 8, 4),
            ]
        );

        // A block selection stops on the same columns; widening only kicks in
        // when an edge lands on the trailing half of a wide glyph.
        assert_eq!(
            buffer.get_text_rects(start, end, true, false),
            vec![
                InclusiveRect::new(1, 0, 8, 0),
                InclusiveRect::new(1, 1, 9, 1),
                InclusiveRect::new(1, 2, 8, 2),
                InclusiveRect::new(0, 3, 8, 3),
                InclusiveRect::new(1, 4, 8, 4),
            ]
        );
    }

    #[test]
    fn copy_spans_are_one_per_block_row_and_one_for_a_line() {
        let mut buffer = TextBuffer::with_dimensions(10, 10, 0);
        write_row(&mut buffer, 0, "ab", false);
        write_row(&mut buffer, 1, "cd", false);

        let start = Point::new(0, 0);
        let end = Point::new(2, 1);

        assert_eq!(
            buffer.get_text_spans(start, end, true, false),
            vec![
                PointSpan::new(Point::new(0, 0), Point::new(2, 0)),
                PointSpan::new(Point::new(0, 1), Point::new(2, 1)),
            ]
        );
        assert_eq!(
            buffer.get_text_spans(start, end, false, false),
            vec![PointSpan::new(Point::new(0, 0), Point::new(2, 1))]
        );
    }

    #[test]
    fn plain_text_follows_the_copy_configuration() {
        // Ported from the non-wrapped half of `TextBufferTests::GetPlainText`.
        let mut buffer = TextBuffer::with_dimensions(10, 20, 0);
        for (index, text) in ["12345", "  345", "123  ", "  3  "].iter().enumerate() {
            write_row(&mut buffer, index as i32, text, false);
        }

        let request = |block: bool, include_line_break: bool, trim: bool| {
            CopyRequest::new(
                &buffer,
                Point::new(0, 0),
                Point::new(5, 4),
                block,
                include_line_break,
                trim,
                false,
                false,
            )
        };

        assert_eq!(
            buffer.get_plain_text(&request(false, true, true)),
            "12345\r\n  345\r\n123\r\n  3\r\n"
        );
        assert_eq!(
            buffer.get_plain_text(&request(false, true, false)),
            "12345     \r\n  345     \r\n123       \r\n  3       \r\n     "
        );
        assert_eq!(
            buffer.get_plain_text(&request(true, true, false)),
            "12345\r\n  345\r\n123  \r\n  3  \r\n     "
        );
        assert_eq!(
            buffer.get_plain_text(&request(false, false, false)),
            concat!("12345     ", "  345     ", "123       ", "  3       ", "     ")
        );
        assert_eq!(
            buffer.get_plain_text(&request(true, false, false)),
            concat!("12345", "  345", "123  ", "  3  ", "     ")
        );
        assert_eq!(buffer.get_plain_text(&request(false, false, true)), "12345  345123  3");
    }

    #[test]
    fn document_rows_reach_into_the_scrollback() {
        let mut buffer = TextBuffer::with_dimensions(5, 2, 4);
        buffer.write_string("one");
        buffer.scroll_region_up(0, 1);
        buffer.cursor_mut().set_position(1, 0);
        buffer.write_string("two");

        assert_eq!(buffer.document_height(), 3);
        assert_eq!(
            buffer.get_row_by_document_line(0).map(Row::get_text),
            Some("one".to_string())
        );
        assert_eq!(
            buffer.get_row_by_document_line(1).map(Row::get_text),
            Some(String::new())
        );
        assert_eq!(
            buffer.get_row_by_document_line(2).map(Row::get_text),
            Some("two  ".to_string())
        );
        assert!(buffer.get_row_by_document_line(3).is_none());
        assert!(buffer.get_row_by_document_line(-1).is_none());
    }

    #[test]
    fn a_document_copy_reaches_into_the_scrollback() {
        let mut buffer = TextBuffer::with_dimensions(6, 2, 4);
        buffer.write_string("one");
        buffer.scroll_region_up(0, 1);
        buffer.cursor_mut().set_position(1, 0);
        buffer.write_string("two");

        let request = CopyRequest {
            begin: Point::new(0, 0),
            end: Point::new(3, 2),
            min_x: 0,
            max_x: 3,
            block_selection: false,
            trim_trailing_whitespace: false,
            include_line_break: true,
            format_wrapped_rows: true,
            buffer_coordinates: true,
        };

        assert_eq!(buffer.get_plain_text(&request), "one   \r\n      \r\ntwo");
    }

    #[test]
    fn wrapped_rows_join_unless_they_are_formatted() {
        // Ported from the wrapped half of `TextBufferTests::GetPlainText`. Rows
        // 0, 2 and 3 were filled to the width by a wrapping write.
        let mut buffer = TextBuffer::with_dimensions(5, 20, 0);
        for (index, (text, wrap)) in [
            ("12345", true),
            ("67   ", false),
            ("  345", true),
            ("123  ", true),
            ("  ", false),
        ]
        .iter()
        .enumerate()
        {
            write_row(&mut buffer, index as i32, text, *wrap);
        }

        let request = |block: bool, include_line_break: bool, trim: bool| {
            CopyRequest::new(
                &buffer,
                Point::new(0, 0),
                Point::new(5, 5),
                block,
                include_line_break,
                trim,
                block,
                false,
            )
        };

        assert_eq!(
            buffer.get_plain_text(&request(false, true, true)),
            "1234567\r\n  345123  \r\n"
        );
        assert_eq!(
            buffer.get_plain_text(&request(false, true, false)),
            "1234567   \r\n  345123       \r\n     "
        );
        assert_eq!(buffer.get_plain_text(&request(false, false, true)), "1234567  345123  ");
        assert_eq!(
            buffer.get_plain_text(&request(false, false, false)),
            concat!("12345", "67   ", "  345", "123  ", "     ", "     ")
        );

        assert_eq!(
            buffer.get_plain_text(&request(true, true, true)),
            "12345\r\n67\r\n  345\r\n123\r\n\r\n"
        );
        assert_eq!(
            buffer.get_plain_text(&request(true, true, false)),
            "12345\r\n67   \r\n  345\r\n123  \r\n     \r\n     "
        );
        assert_eq!(buffer.get_plain_text(&request(true, false, true)), "1234567  345123");
        assert_eq!(
            buffer.get_plain_text(&request(true, false, false)),
            concat!("12345", "67   ", "  345", "123  ", "     ", "     ")
        );
    }

    #[test]
    fn rtf_text_is_escaped() {
        // Ported from `TextBufferTests::TestAppendRTFText`.
        let mut out = String::new();
        TextBuffer::append_rtf_text(&mut out, "This is some Ascii \\ {}");
        assert_eq!(out, "This is some Ascii \\\\ \\{\\}");

        let mut out = String::new();
        TextBuffer::append_rtf_text(
            &mut out,
            "Low code units: \u{00E1} \u{00E9} \u{00ED} \u{00F3} \u{00FA} \u{2B81} \u{2B82}",
        );
        assert_eq!(
            out,
            "Low code units: \\u225? \\u233? \\u237? \\u243? \\u250? \\u11137? \\u11138?"
        );

        let mut out = String::new();
        TextBuffer::append_rtf_text(&mut out, "High code units: \u{A7B5} \u{A7B7}");
        assert_eq!(out, "High code units: \\u-22603? \\u-22601?");

        let mut out = String::new();
        TextBuffer::append_rtf_text(&mut out, "Surrogates: \u{1F366} \u{1F47E} \u{1F440}");
        assert_eq!(
            out,
            "Surrogates: \\u-10180?\\u-8346? \\u-10179?\\u-9090? \\u-10179?\\u-9152?"
        );
    }

    fn painted_buffer() -> TextBuffer {
        let mut buffer = TextBuffer::with_dimensions(10, 5, 0);
        write_row(&mut buffer, 0, "hi<x", false);
        buffer
    }

    fn fixed_colors(_: &TextAttribute) -> (Color, Color, Color) {
        (
            Color::rgb(0x11, 0x22, 0x33),
            Color::rgb(0x44, 0x55, 0x66),
            Color::rgb(0x77, 0x88, 0x99),
        )
    }

    #[test]
    fn html_wraps_the_text_in_a_cf_html_fragment() {
        let buffer = painted_buffer();
        let request = CopyRequest::from_config(&buffer, Point::new(0, 0), Point::new(4, 0), false, false, false, false);
        let colors: &AttributeColorsFn<'_> = &fixed_colors;

        let html = buffer.gen_html(
            &request,
            12.0,
            "Courier New",
            Color::rgb(0xAA, 0xBB, 0xCC),
            false,
            colors,
        );

        let body = concat!(
            "<!DOCTYPE><HTML><HEAD></HEAD><BODY>",
            "<!--StartFragment -->",
            "<DIV STYLE=\"display:inline-block;white-space:pre;",
            "background-color:#AABBCC;",
            "font-family:'Courier New',monospace;",
            "font-size:12pt;",
            "padding:4px;\">",
            "<SPAN STYLE=\"color:#112233;background-color:#445566;\">hi&lt;x</SPAN>",
            "</DIV>",
            "<!--EndFragment -->",
            "</BODY></HTML>"
        );

        let start = 157;
        let end = start + body.len();
        let fragment_start = start + "<!DOCTYPE><HTML><HEAD></HEAD><BODY>".len();
        let fragment_end = end - "</BODY></HTML>".len();
        let header = format!(
            "Version:0.9\r\nStartHTML:{start:0>10}\r\nEndHTML:{end:0>10}\r\n\
             StartFragment:{fragment_start:0>10}\r\nEndFragment:{fragment_end:0>10}\r\n\
             StartSelection:{fragment_start:0>10}\r\nEndSelection:{fragment_end:0>10}\r\n"
        );

        assert_eq!(html, format!("{header}{body}"));
    }

    #[test]
    fn rtf_opens_a_group_per_run() {
        let buffer = painted_buffer();
        let request = CopyRequest::from_config(&buffer, Point::new(0, 0), Point::new(4, 0), false, false, false, false);
        let colors: &AttributeColorsFn<'_> = &fixed_colors;

        assert_eq!(
            buffer.gen_rtf(
                &request,
                12.0,
                "Courier New",
                Color::rgb(0xAA, 0xBB, 0xCC),
                false,
                colors
            ),
            concat!(
                "{\\rtf1\\ansi\\ansicpg1252\\deff0\\nouicompat",
                "{\\fonttbl{\\f0\\fmodern\\fcharset0 Courier New;}}",
                "{\\colortbl ;\\red170\\green187\\blue204;",
                "\\red17\\green34\\blue51;",
                "\\red68\\green85\\blue102;",
                "\\red119\\green136\\blue153;}",
                "\\viewkind4\\uc1\\pard\\slmult1\\f0\\fs24",
                "\\chshdng0\\chcbpat1",
                "{\\cf2\\chshdng0\\chcbpat3 hi<x}",
                "}"
            )
        );
    }

    #[test]
    fn html_and_rtf_render_decorations() {
        let mut buffer = TextBuffer::with_dimensions(10, 5, 0);
        let mut attribute = TextAttribute::new();
        attribute.set_intense(true);
        attribute.set_italic(true);
        attribute.set_crossed_out(true);
        attribute.set_underline_style(UnderlineStyle::CurlyUnderlined);

        let mut cell = OutputCell::from_char('x');
        cell.set_attributes(attribute);
        buffer.get_row_at_screen_line_mut(0).expect("row").write_cell(0, cell);

        let request = CopyRequest::from_config(&buffer, Point::new(0, 0), Point::new(1, 0), false, false, false, false);
        let colors: &AttributeColorsFn<'_> = &fixed_colors;

        let html = buffer.gen_html(
            &request,
            12.0,
            "Cascadia Mono",
            Color::rgb(0xAA, 0xBB, 0xCC),
            false,
            colors,
        );
        assert!(html.contains(concat!(
            "<SPAN STYLE=\"color:#112233;background-color:#445566;",
            "font-weight:bold;font-style:italic;",
            "text-decoration:line-through  #112233;\">",
            "<SPAN STYLE=\"text-decoration:underline wavy #778899;\">x</SPAN></SPAN>"
        )));

        let rtf = buffer.gen_rtf(
            &request,
            12.0,
            "Cascadia Mono",
            Color::rgb(0xAA, 0xBB, 0xCC),
            false,
            colors,
        );
        assert!(rtf.contains("{\\cf2\\chshdng0\\chcbpat3\\b\\i\\strike\\ulwave\\ulc4 x}"));
    }

    #[test]
    fn the_config_form_builds_the_clipboard_flags() {
        let buffer = TextBuffer::with_dimensions(10, 20, 0);
        let begin = Point::new(0, 0);
        let end = Point::new(5, 4);

        let single_line = CopyRequest::from_config(&buffer, begin, end, true, false, false, false);
        assert!(!single_line.include_line_break);
        assert!(!single_line.trim_trailing_whitespace);
        assert!(!single_line.format_wrapped_rows);

        let lines = CopyRequest::from_config(&buffer, begin, end, false, false, false, false);
        assert!(lines.include_line_break);
        assert!(lines.trim_trailing_whitespace);
        assert!(!lines.format_wrapped_rows);

        let block = CopyRequest::from_config(&buffer, begin, end, false, true, false, false);
        assert!(block.include_line_break);
        assert!(!block.trim_trailing_whitespace);
        assert!(block.format_wrapped_rows);

        let block_single = CopyRequest::from_config(&buffer, begin, end, true, true, false, false);
        assert!(block_single.include_line_break);
        assert!(!block_single.trim_trailing_whitespace);
        assert!(block_single.format_wrapped_rows);
    }

    #[test]
    fn a_request_clamps_by_row_like_upstream() {
        let buffer = TextBuffer::with_dimensions(5, 20, 0);

        // A point above the last row keeps its column, because upstream clamps
        // with a point comparison rather than column by column.
        let request = CopyRequest::new(
            &buffer,
            Point::new(-2, 0),
            Point::new(5, 5),
            false,
            true,
            true,
            false,
            false,
        );
        assert_eq!(request.begin, Point::new(0, 0));
        assert_eq!(request.end, Point::new(5, 5));
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
        let id = buf.get_hyperlink_id("https://example.com", "custom");
        assert!(id > 0);
        assert_eq!(buf.get_hyperlink_uri(id), Some("https://example.com"));
        assert!(!buf.hyperlinks().is_empty());
    }

    #[test]
    fn test_screen_to_buffer_conversion() {
        let buf = TextBuffer::new();
        let buffer_row = buf.screen_to_buffer_row(0);
        assert_eq!(buffer_row, DEFAULT_SCROLLBACK_HEIGHT);

        let screen_row = buf.buffer_to_screen_row(DEFAULT_SCROLLBACK_HEIGHT);
        assert_eq!(screen_row, 0);
    }

    fn buffer_with_rows(width: i32, height: i32, rows: &[(&str, bool)], cursor: (i32, i32)) -> TextBuffer {
        let mut buffer = TextBuffer::with_dimensions(width, height, 0);
        for (index, (text, wrap)) in rows.iter().enumerate() {
            let row = buffer
                .get_row_at_screen_line_mut(index as i32)
                .expect("row is within the page");
            row.set_wrap(*wrap);
            for (column, ch) in text.chars().enumerate() {
                row.write_char(column as i32, ch, false);
            }
        }
        buffer.cursor_mut().set_position(cursor.0, cursor.1);
        buffer
    }

    fn assert_rows(buffer: &TextBuffer, expected: &[(&str, bool)]) {
        for (index, (expected_text, wrap)) in expected.iter().enumerate() {
            let row = buffer
                .get_row_at_screen_line(index as i32)
                .expect("row is within the page");
            // Untouched cells hold no character, so pad the row out to its width.
            let actual = format!("{:<width$}", row.get_text(), width = row.width() as usize);
            assert_eq!(actual, *expected_text, "row {index} text");
            assert_eq!(row.is_wrap(), *wrap, "row {index} wrap");
        }
    }

    #[test]
    fn logical_lines_join_wrapped_rows_and_report_the_cursor() {
        let buffer = buffer_with_rows(
            4,
            5,
            &[("ab  ", true), ("cdef", true), ("g   ", false), ("h   ", false)],
            (2, 0),
        );

        let lines = buffer.logical_lines();
        assert_eq!(lines.len(), 3);

        let first: String = lines[0].cells().iter().map(OutputCell::chars).collect();
        assert_eq!(first, "ab  cdefg");
        assert_eq!(lines[0].cursor_offset(), Some(8));

        let second: String = lines[1].cells().iter().map(OutputCell::chars).collect();
        assert_eq!(second, "h");
        assert_eq!(lines[1].cursor_offset(), None);

        assert!(lines[2].is_blank());
    }

    // Ported from `ut_textbuffer/ReflowTests.cpp`: "No reflow required".
    #[test]
    fn reflow_keeps_explicit_lines_when_the_width_changes() {
        let mut buffer = buffer_with_rows(
            6,
            5,
            &[
                ("AB    ", false),
                ("$     ", false),
                ("CD    ", false),
                ("EFG   ", false),
            ],
            (1, 0),
        );

        buffer.resize(5, 5);
        assert_eq!(buffer.cursor().position(), (1, 0));
        assert_rows(
            &buffer,
            &[
                ("AB   ", false),
                ("$    ", false),
                ("CD   ", false),
                ("EFG  ", false),
                ("     ", false),
            ],
        );

        buffer.resize(4, 5);
        assert_eq!(buffer.cursor().position(), (1, 0));
        assert_rows(
            &buffer,
            &[
                ("AB  ", false),
                ("$   ", false),
                ("CD  ", false),
                ("EFG ", false),
                ("    ", false),
            ],
        );
    }

    // Ported from `ut_textbuffer/ReflowTests.cpp`: "SBCS, cursor remains in
    // buffer, no circling, no original wrap".
    #[test]
    fn reflow_wraps_a_long_row_and_restores_it_when_widened() {
        let mut buffer = buffer_with_rows(6, 5, &[("ABCDEF", false), ("$     ", false)], (1, 0));

        buffer.resize(5, 5);
        assert_eq!(buffer.cursor().position(), (2, 0));
        assert_rows(
            &buffer,
            &[
                ("ABCDE", true),
                ("F    ", false),
                ("$    ", false),
                ("     ", false),
                ("     ", false),
            ],
        );

        buffer.resize(6, 5);
        assert_eq!(buffer.cursor().position(), (1, 0));
        assert_rows(
            &buffer,
            &[
                ("ABCDEF", false),
                ("$     ", false),
                ("      ", false),
                ("      ", false),
                ("      ", false),
            ],
        );

        buffer.resize(7, 5);
        assert_eq!(buffer.cursor().position(), (1, 0));
        assert_rows(
            &buffer,
            &[
                ("ABCDEF ", false),
                ("$      ", false),
                ("       ", false),
                ("       ", false),
                ("       ", false),
            ],
        );
    }

    // Ported from `ut_textbuffer/ReflowTests.cpp`: "SBCS, cursor remains in
    // buffer, no circling, with original wrap".
    #[test]
    fn reflow_maps_the_cursor_by_its_place_in_the_logical_line() {
        let mut buffer = buffer_with_rows(6, 5, &[("ABCDEF", true), ("G$    ", false)], (1, 1));

        buffer.resize(5, 5);
        assert_eq!(buffer.cursor().position(), (1, 2));
        assert_rows(
            &buffer,
            &[
                ("ABCDE", true),
                ("FG$  ", false),
                ("     ", false),
                ("     ", false),
                ("     ", false),
            ],
        );

        buffer.resize(6, 5);
        assert_eq!(buffer.cursor().position(), (1, 1));
        assert_rows(
            &buffer,
            &[
                ("ABCDEF", true),
                ("G$    ", false),
                ("      ", false),
                ("      ", false),
                ("      ", false),
            ],
        );

        buffer.resize(7, 5);
        assert_eq!(buffer.cursor().position(), (1, 0));
        assert_rows(
            &buffer,
            &[
                ("ABCDEFG", true),
                ("$      ", false),
                ("       ", false),
                ("       ", false),
                ("       ", false),
            ],
        );
    }

    // Ported from `ut_textbuffer/ReflowTests.cpp`: "SBCS line padded with
    // spaces (to wrap)": whitespace inside a wrapped line is content.
    #[test]
    fn reflow_keeps_whitespace_inside_a_wrapped_line() {
        let mut buffer = buffer_with_rows(
            6,
            5,
            &[("AB    ", true), ("$     ", true), ("CD    ", false), ("EFG   ", false)],
            (1, 0),
        );

        buffer.resize(7, 5);
        assert_eq!(buffer.cursor().position(), (0, 6));
        assert_rows(
            &buffer,
            &[
                ("AB    $", true),
                ("     CD", false),
                ("EFG    ", false),
                ("       ", false),
                ("       ", false),
            ],
        );

        buffer.resize(8, 5);
        assert_eq!(buffer.cursor().position(), (0, 6));
        assert_rows(
            &buffer,
            &[
                ("AB    $ ", true),
                ("    CD  ", false),
                ("EFG     ", false),
                ("        ", false),
                ("        ", false),
            ],
        );
    }

    // Ported from `ut_textbuffer/ReflowTests.cpp`: "SBCS, cursor off rightmost
    // char in non-wrapped line".
    #[test]
    fn reflow_carries_a_cursor_that_sits_past_the_text() {
        let mut buffer = buffer_with_rows(6, 5, &[("ABCDEF", false), ("$     ", false)], (1, 1));

        buffer.resize(5, 5);
        assert_eq!(buffer.cursor().position(), (2, 1));
        assert_rows(
            &buffer,
            &[
                ("ABCDE", true),
                ("F    ", false),
                ("$    ", false),
                ("     ", false),
                ("     ", false),
            ],
        );
    }

    // Ported from `ut_textbuffer/ReflowTests.cpp`: "SBCS, cursor off in space to
    // far right of text (end of buffer content)".
    #[test]
    fn reflow_pads_the_cursor_row_so_widening_restores_the_cursor() {
        let mut buffer = buffer_with_rows(6, 5, &[("ABCDEF", false), ("$     ", false)], (1, 5));

        buffer.resize(5, 5);
        assert_eq!(buffer.cursor().position(), (3, 0));
        assert_rows(
            &buffer,
            &[
                ("ABCDE", true),
                ("F    ", false),
                ("$    ", true),
                ("     ", false),
                ("     ", false),
            ],
        );

        buffer.resize(6, 5);
        assert_eq!(buffer.cursor().position(), (1, 5));
        assert_rows(
            &buffer,
            &[
                ("ABCDEF", false),
                ("$     ", false),
                ("      ", false),
                ("      ", false),
                ("      ", false),
            ],
        );
    }

    #[test]
    fn copy_row_moves_the_image_band() {
        let mut buffer = TextBuffer::with_dimensions(4, 3, 0);
        {
            let row = buffer.get_row_at_screen_line_mut(0).unwrap();
            row.set_image_slice(Some(test_image_band(4)));
        }

        buffer.copy_row(2, 0);

        let image = buffer.get_row_at_screen_line(2).unwrap().image_slice().unwrap();
        assert_eq!(image.pixels(), &[1, 2, 3, 4]);
    }

    #[test]
    fn filling_a_rectangle_erases_the_band() {
        let mut buffer = TextBuffer::with_dimensions(4, 3, 0);
        {
            let row = buffer.get_row_at_screen_line_mut(0).unwrap();
            row.set_image_slice(Some(test_image_band(4)));
        }

        buffer.fill_rect(0, 1, 0, 2, &OutputCell::from_char(' '));

        let image = buffer.get_row_at_screen_line(0).unwrap().image_slice().unwrap();
        assert_eq!(image.pixels(), &[1, 0, 0, 4]);
    }

    #[test]
    fn scrolling_up_moves_the_image_band() {
        let mut buffer = TextBuffer::with_dimensions(4, 2, 0);
        {
            let row = buffer.get_row_at_screen_line_mut(1).unwrap();
            row.set_image_slice(Some(test_image_band(4)));
        }

        buffer.scroll_region_up(0, 1);

        assert_eq!(
            buffer
                .get_row_at_screen_line(0)
                .unwrap()
                .image_slice()
                .unwrap()
                .pixels(),
            &[1, 2, 3, 4]
        );
        assert!(buffer.get_row_at_screen_line(1).unwrap().image_slice().is_none());
    }

    #[test]
    fn reflow_keeps_the_first_rows_image_band() {
        let mut buffer = TextBuffer::with_dimensions(4, 2, 0);
        {
            let row = buffer.get_row_at_screen_line_mut(0).unwrap();
            row.set_image_slice(Some(test_image_band(4)));
        }

        buffer.resize(2, 2);

        let image = buffer.get_row_at_screen_line(0).unwrap().image_slice().unwrap();
        assert_eq!(image.pixels(), &[1, 2, 3, 4]);
        assert!(buffer.get_row_at_screen_line(1).unwrap().image_slice().is_none());
    }

    fn test_image_band(columns: i32) -> ImageSlice {
        let mut image = ImageSlice::new((1, 1));
        let pixels = image.mutable_pixels(0, columns);
        for (index, pixel) in pixels.iter_mut().enumerate() {
            *pixel = index as u32 + 1;
        }
        image
    }

    #[test]
    fn reflow_rewraps_across_the_scrollback_boundary() {
        let mut buffer = TextBuffer::with_dimensions(4, 2, 2);
        write_text_at(&mut buffer, 0, 0, "abcd");
        write_text_at(&mut buffer, 1, 0, "efgh");
        buffer.scroll_region_up(0, 1);
        assert_eq!(buffer.scrollback_row_text(0).unwrap(), "abcd");

        buffer.resize(2, 2);

        assert_eq!(buffer.scrollback_len(), 2);
        assert_eq!(buffer.scrollback_row_text(0).unwrap(), "ab");
        assert_eq!(buffer.scrollback_row_text(1).unwrap(), "cd");
        assert_rows(&buffer, &[("ef", true), ("gh", false)]);
        assert_eq!(buffer.cursor().position(), (0, 0));
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
            let text = buf.get_all_text();
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

        #[test]
        fn test_resize_keeps_the_cursor_and_rows_in_bounds(
            width in 2i32..120i32,
            height in 1i32..60i32,
            scrollback in 0i32..50i32,
            new_width in 1i32..120i32,
            new_height in 1i32..60i32,
        ) {
            let mut buf = TextBuffer::with_dimensions(width, height, scrollback);
            buf.write_string("hi");
            buf.resize(new_width, new_height);

            prop_assert_eq!(buf.screen_width(), new_width);
            prop_assert_eq!(buf.screen_height(), new_height);
            let (row, column) = buf.cursor().position();
            prop_assert!((0..new_height).contains(&row));
            prop_assert!((0..new_width).contains(&column));
            for line in 0..new_height {
                let row = buf.get_row_at_screen_line(line).expect("screen row");
                prop_assert_eq!(row.width(), new_width);
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
