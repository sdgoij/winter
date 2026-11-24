//! The find bar's state and the rules every shell shares.
//!
//! `buffer`'s `find_text` reports matches in document coordinates; this module
//! keeps the query, the current match, the scroll that brings it into view and
//! the conversion to the screen spans a renderer paints. Each shell draws its
//! own bar — a `TextBox` in WinUI, a `GtkSearchEntry` in the GTK shell, a field
//! in AppKit — but all ask the same questions.

use buffer::TextMatch;
#[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
use utils::{Point, PointSpan};

use crate::term_control::Control;
use crate::view::View;

/// The find bar's state.
#[derive(Debug, Default)]
pub(crate) struct FindBar {
    /// Whether the bar is showing.
    pub(crate) open: bool,
    /// What the user is looking for.
    pub(crate) query: String,
    /// The query `matches` were computed for.
    pub(crate) searched: String,
    /// Every match in the document, in reading order.
    pub(crate) matches: Vec<TextMatch>,
    /// The match Enter moved to, if any.
    pub(crate) current: Option<usize>,
    /// The document height when the matches were computed.
    pub(crate) document_height: i32,
    /// Whether the field still needs the keyboard focus.
    pub(crate) focus: bool,
}

impl FindBar {
    pub(crate) fn show(&mut self) {
        self.open = true;
        self.focus = true;
    }

    pub(crate) fn hide(&mut self) {
        self.open = false;
        self.focus = false;
        self.current = None;
    }

    pub(crate) fn toggle(&mut self) {
        if self.open { self.hide() } else { self.show() }
    }

    /// Recomputes the matches after the query changed and selects the first.
    pub(crate) fn refresh(&mut self, control: &mut Control) {
        let (matches, document_height) = control.find_matches(&self.query);
        self.matches = matches;
        self.document_height = document_height;
        self.searched = self.query.clone();
        self.current = (!self.matches.is_empty()).then_some(0);
    }

    /// Moves to the next (or previous) match, wrapping at both ends.
    pub(crate) fn step(&mut self, forward: bool) {
        self.current = next_match(self.current, self.matches.len(), forward);
    }

    /// Brings the current match into the middle of the view.
    pub(crate) fn scroll_to(&self, control: &Control, view: &mut View) {
        let Some(found) = self.current.and_then(|index| self.matches.get(index)) else {
            return;
        };
        view.scroll_to_row(found.start_row, control.size().1, self.document_height);
    }
}

/// The match Enter moves to, wrapping at both ends.
pub(crate) fn next_match(current: Option<usize>, len: usize, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }

    Some(match current {
        Some(index) if forward => (index + 1) % len,
        Some(index) => (index + len - 1) % len,
        None if forward => 0,
        None => len - 1,
    })
}

/// Converts document matches to the screen spans a renderer paints.
///
/// `history` and `scroll` place the screen in the document, and `current`
/// names the match whose span is reported as the focused one.
///
/// Only the shells that hand these spans to a renderer call this: the WinUI
/// pane, the macOS pane and the GTK pane. On the other platforms the function
/// exists for its tests.
#[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
pub(crate) fn highlight_spans(
    matches: &[TextMatch],
    current: Option<usize>,
    history: i32,
    scroll: i32,
    columns: i32,
    rows: i32,
) -> (Vec<PointSpan>, Option<PointSpan>) {
    let mut spans = Vec::new();
    let mut focused = None;
    let last_column = (columns - 1).max(0);

    for (index, found) in matches.iter().enumerate() {
        for row in 0..rows {
            let document = crate::input::screen_to_document(history, scroll, row);
            let Some((column, cells)) = found.row_span(document, columns) else {
                continue;
            };
            let start = column.clamp(0, last_column);
            let end = (column + cells - 1).clamp(start, last_column);
            let span = PointSpan::new(Point::new(start, row), Point::new(end, row));
            if current == Some(index) {
                focused.get_or_insert(span);
            }
            spans.push(span);
        }
    }

    (spans, focused)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(start_row: i32, start_column: i32, end_row: i32, end_column: i32) -> TextMatch {
        TextMatch {
            start_row,
            start_column,
            end_row,
            end_column,
        }
    }

    #[test]
    fn the_next_match_wraps_at_both_ends() {
        assert_eq!(next_match(None, 0, true), None);
        assert_eq!(next_match(None, 3, true), Some(0));
        assert_eq!(next_match(None, 3, false), Some(2));
        assert_eq!(next_match(Some(1), 3, true), Some(2));
        assert_eq!(next_match(Some(2), 3, true), Some(0));
        assert_eq!(next_match(Some(0), 3, false), Some(2));
    }

    #[test]
    fn a_match_is_reported_in_screen_rows() {
        // Five rows of history and a scroll of four puts document row two on
        // screen row one.
        let (spans, focused) = highlight_spans(&[found(2, 3, 2, 6)], Some(0), 5, 4, 80, 3);

        let expected = PointSpan::new(Point::new(3, 1), Point::new(5, 1));
        assert_eq!(spans, [expected]);
        assert_eq!(focused, Some(expected));
    }

    #[test]
    fn a_match_through_an_autowrap_covers_both_rows() {
        let (spans, focused) = highlight_spans(&[found(2, 3, 3, 2)], None, 2, 0, 10, 2);

        assert_eq!(
            spans,
            [
                PointSpan::new(Point::new(3, 0), Point::new(9, 0)),
                PointSpan::new(Point::new(0, 1), Point::new(1, 1)),
            ]
        );
        assert_eq!(focused, None);
    }

    #[test]
    fn a_match_outside_the_screen_is_not_reported() {
        let (spans, focused) = highlight_spans(&[found(1, 0, 1, 3)], Some(0), 5, 0, 80, 2);

        assert!(spans.is_empty());
        assert_eq!(focused, None);
    }
}
