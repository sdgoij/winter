//! The viewport and the terminal's selection.
//!
//! Upstream splits selection across two owners: the console host owns
//! `SCREEN_INFORMATION`'s selection (ADR 014), and the terminal application
//! owns its own in `ControlCore`/`ControlInteractivity` under `src/cascadia`.
//! This is the application's: cells in document coordinates, moved by the
//! mouse, and copied through the buffer's [`CopyRequest`] path.
//!
//! Document coordinates run through the history first and the visible screen
//! last, so a selection keeps its text when the view scrolls and can cross
//! into the scrollback.
//!
//! The module is toolkit-free (ADR 018). It knows about rows, columns and
//! time; it does not know about pixels, a window, or egui.

use std::time::{Duration, Instant};

use buffer::{CopyRequest, TextBuffer};
use host::cmdline::is_word_delim;
use utils::Point;

/// The interval within which two clicks count as one multi-click.
const MULTI_CLICK: Duration = Duration::from_millis(500);
/// The highest click count a selection distinguishes.
const MAX_CLICK_COUNT: usize = 3;

/// A selection in document cells, `(column, document row)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Where the drag started.
    pub anchor: (i32, i32),
    /// Where the drag currently ends.
    pub end: (i32, i32),
    /// Whether the selection keeps its columns on every row.
    pub block: bool,
}

impl Selection {
    /// Returns the start and end ordered by row, then column.
    fn bounds(&self) -> ((i32, i32), (i32, i32)) {
        let anchor = (self.anchor.1, self.anchor.0);
        let end = (self.end.1, self.end.0);
        if anchor <= end {
            (self.anchor, self.end)
        } else {
            (self.end, self.anchor)
        }
    }

    /// Returns the inclusive column span this selection covers on `row`.
    ///
    /// A line-wise selection that spans rows runs to the end of the first row,
    /// covers whole intermediate rows and stops at the last row's end column.
    /// `i32::MAX` means "to the end" and is clamped by the painter.
    pub fn row_span(&self, row: i32) -> Option<(i32, i32)> {
        let ((start_column, start_row), (end_column, end_row)) = self.bounds();
        if row < start_row || row > end_row {
            return None;
        }

        if self.block {
            return Some((start_column.min(end_column), start_column.max(end_column)));
        }

        if start_row == end_row {
            return Some((start_column, end_column));
        }

        if row == start_row {
            Some((start_column, i32::MAX))
        } else if row == end_row {
            Some((0, end_column))
        } else {
            Some((0, i32::MAX))
        }
    }

    /// Returns the selected text, as the buffer's copy path formats it.
    ///
    /// The selection's end is the cell under the pointer, but `CopyRequest`
    /// takes an exclusive end column, so the end is moved one cell right. The
    /// request is built literally because `CopyRequest::new` clamps the end to
    /// the last column, which would drop a selection that reaches it.
    pub fn text(&self, buffer: &TextBuffer) -> String {
        let ((start_column, start_row), (end_column, end_row)) = self.bounds();
        let request = CopyRequest {
            begin: Point::new(start_column, start_row),
            end: Point::new(end_column + 1, end_row),
            min_x: start_column.min(end_column),
            max_x: start_column.max(end_column) + 1,
            block_selection: self.block,
            trim_trailing_whitespace: false,
            include_line_break: true,
            format_wrapped_rows: true,
            buffer_coordinates: true,
        };
        buffer.get_plain_text(&request)
    }
}

/// The cells a word occupies, for a double-click.
///
/// A run of delimiters is its own word, which is what makes double-clicking
/// whitespace select the whitespace rather than the text beside it.
pub fn word_span(text: &str, column: i32) -> (i32, i32) {
    let characters: Vec<char> = text.chars().collect();
    if characters.is_empty() {
        return (0, 0);
    }

    let index = (column.max(0) as usize).min(characters.len() - 1);
    let delimiter = is_word_delim(characters[index]);

    let mut start = index;
    while start > 0 && is_word_delim(characters[start - 1]) == delimiter {
        start -= 1;
    }

    let mut end = index;
    while end + 1 < characters.len() && is_word_delim(characters[end + 1]) == delimiter {
        end += 1;
    }

    (start as i32, end as i32)
}

/// The screen's scroll and selection state.
#[derive(Debug, Default)]
pub struct View {
    scroll: i32,
    selection: Option<Selection>,
    clicks: ClickTracker,
}

impl View {
    /// Creates a view at the bottom of the buffer with no selection.
    pub const fn new() -> Self {
        Self {
            scroll: 0,
            selection: None,
            clicks: ClickTracker::new(),
        }
    }

    /// Returns how many rows the view is scrolled up from the bottom.
    pub const fn scroll(&self) -> i32 {
        self.scroll
    }

    /// Returns the selection, if there is one.
    pub const fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    /// Scrolls by `lines`, positive towards the history.
    ///
    /// The selection is in document coordinates, so it survives a scroll and
    /// can be extended across the boundary between history and screen.
    pub fn scroll_by(&mut self, lines: i32, max: i32) {
        self.scroll = (self.scroll + lines).clamp(0, max.max(0));
    }

    /// Returns to the live screen.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll = 0;
    }

    /// Starts a selection at `anchor`.
    pub fn select(&mut self, anchor: (i32, i32), block: bool) {
        self.selection = Some(Selection {
            anchor,
            end: anchor,
            block,
        });
    }

    /// Moves the selection's end to `end`.
    pub fn extend(&mut self, end: (i32, i32)) {
        if let Some(selection) = self.selection.as_mut() {
            selection.end = end;
        }
    }

    /// Drops the selection.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Selects the word under `position` in `row_text`.
    pub fn select_word(&mut self, row_text: &str, position: (i32, i32)) {
        let (start, end) = word_span(row_text, position.0);
        self.selection = Some(Selection {
            anchor: (start, position.1),
            end: (end, position.1),
            block: false,
        });
    }

    /// Selects the whole row.
    pub fn select_line(&mut self, row: i32, width: i32) {
        self.selection = Some(Selection {
            anchor: (0, row),
            end: ((width - 1).max(0), row),
            block: false,
        });
    }

    /// Registers a click at `position` and returns its count.
    ///
    /// Counts cycle one, two, three, one: a fourth click is a single-click
    /// again, which is upstream's `MAX_CLICK_COUNT` behaviour.
    pub fn click(&mut self, position: (i32, i32), now: Instant) -> usize {
        self.clicks.click(position, now)
    }
}

#[derive(Debug, Default)]
struct ClickTracker {
    position: (i32, i32),
    last: Option<Instant>,
    count: usize,
}

impl ClickTracker {
    const fn new() -> Self {
        Self {
            position: (0, 0),
            last: None,
            count: 0,
        }
    }

    fn click(&mut self, position: (i32, i32), now: Instant) -> usize {
        let repeated =
            position == self.position && self.last.is_some_and(|last| now.duration_since(last) <= MULTI_CLICK);

        if repeated {
            self.count = self.count % MAX_CLICK_COUNT + 1;
        } else {
            self.count = 1;
        }

        self.position = position;
        self.last = Some(now);
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_clamps_and_keeps_the_selection() {
        let mut view = View::new();
        view.select((1, 1), false);

        view.scroll_by(2, 3);
        assert_eq!(view.scroll(), 2);
        assert!(view.selection().is_some());

        view.scroll_by(5, 3);
        assert_eq!(view.scroll(), 3);

        view.scroll_by(-10, 3);
        assert_eq!(view.scroll(), 0);
    }

    #[test]
    fn a_selection_in_the_history_copies_the_history() {
        let mut buffer = TextBuffer::with_dimensions(6, 2, 4);
        buffer.write_string("one");
        buffer.scroll_region_up(0, 1);
        buffer.cursor_mut().set_position(1, 0);
        buffer.write_string("two");

        let mut view = View::new();
        view.select((0, 0), false);
        view.extend((2, 2));

        let selection = view.selection().expect("selection");
        assert_eq!(selection.text(&buffer), "one   \r\n      \r\ntwo");
    }

    #[test]
    fn clicks_cycle_through_the_selection_granularities() {
        let base = Instant::now();
        let mut view = View::new();

        assert_eq!(view.click((4, 2), base), 1);
        assert_eq!(view.click((4, 2), base + Duration::from_millis(10)), 2);
        assert_eq!(view.click((4, 2), base + Duration::from_millis(20)), 3);
        assert_eq!(view.click((4, 2), base + Duration::from_millis(30)), 1);

        // A different cell, or a pause, starts over.
        assert_eq!(view.click((5, 2), base + Duration::from_millis(40)), 1);
        assert_eq!(view.click((5, 2), base + Duration::from_millis(541)), 1);
        assert_eq!(view.click((5, 2), base + Duration::from_millis(542)), 2);
    }

    #[test]
    fn a_drag_selection_covers_the_rows_it_touches() {
        let mut view = View::new();
        view.select((6, 1), false);
        view.extend((2, 3));
        let selection = *view.selection().expect("selection");

        assert_eq!(selection.row_span(0), None);
        assert_eq!(selection.row_span(1), Some((6, i32::MAX)));
        assert_eq!(selection.row_span(2), Some((0, i32::MAX)));
        assert_eq!(selection.row_span(3), Some((0, 2)));
        assert_eq!(selection.row_span(4), None);
    }

    #[test]
    fn a_backwards_drag_covers_the_same_rows() {
        let mut view = View::new();
        view.select((8, 3), false);
        view.extend((1, 1));
        let selection = *view.selection().expect("selection");

        assert_eq!(selection.row_span(1), Some((1, i32::MAX)));
        assert_eq!(selection.row_span(2), Some((0, i32::MAX)));
        assert_eq!(selection.row_span(3), Some((0, 8)));
    }

    #[test]
    fn a_block_selection_keeps_its_columns_on_every_row() {
        let mut view = View::new();
        view.select((5, 1), true);
        view.extend((2, 3));
        let selection = *view.selection().expect("selection");

        assert_eq!(selection.row_span(1), Some((2, 5)));
        assert_eq!(selection.row_span(2), Some((2, 5)));
        assert_eq!(selection.row_span(3), Some((2, 5)));
        assert_eq!(selection.row_span(0), None);
    }

    #[test]
    fn the_selected_text_comes_from_the_buffer() {
        let mut buffer = TextBuffer::with_dimensions(12, 2, 0);
        buffer.write_string("hello world");

        let mut view = View::new();
        view.select((6, 0), false);
        view.extend((10, 0));

        let selection = view.selection().expect("selection");
        assert_eq!(selection.text(&buffer), "world");
    }

    #[test]
    fn a_block_selection_copies_its_columns() {
        let mut buffer = TextBuffer::with_dimensions(6, 2, 0);
        buffer.write_string("abc");
        buffer.cursor_mut().set_position(1, 0);
        buffer.write_string("def");

        let mut view = View::new();
        view.select((0, 0), true);
        view.extend((1, 1));

        let selection = view.selection().expect("selection");
        assert_eq!(selection.text(&buffer), "ab\r\nde");
    }

    #[test]
    fn a_double_click_selects_the_word_under_the_cursor() {
        let mut view = View::new();
        view.select_word("hello world", (7, 0));
        assert_eq!(view.selection().expect("selection").row_span(0), Some((6, 10)));

        // Delimiters are their own words, and they split the words around them.
        assert_eq!(word_span("foo/bar", 1), (0, 2));
        assert_eq!(word_span("foo/bar", 3), (3, 3));
        assert_eq!(word_span("foo/bar", 5), (4, 6));
        assert_eq!(word_span("", 4), (0, 0));
    }

    #[test]
    fn a_triple_click_selects_the_row() {
        let mut view = View::new();
        view.select_line(2, 80);
        assert_eq!(view.selection().expect("selection").row_span(2), Some((0, 79)));
        assert_eq!(view.selection().expect("selection").row_span(3), None);
    }
}
