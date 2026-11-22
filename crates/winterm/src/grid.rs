//! A snapshot of the visible screen, ready for a painter.
//!
//! This module is toolkit-free (ADR 018): it reads the buffer, including the
//! rows scrolled into history, and groups each row into attribute runs. A
//! window backend decides how to draw them.

use buffer::{Row, TextBuffer};
use types::{CursorType, MarkKind, TextAttribute};

/// A run of adjacent cells that share an attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct RunSnapshot {
    /// The first column the run covers.
    pub column: i32,
    /// How many columns it covers.
    pub cells: i32,
    /// The attribute every cell in the run has.
    pub attribute: TextAttribute,
    /// The run's text. A wide glyph contributes one `char` but two cells.
    pub text: String,
}

/// One screen row.
#[derive(Debug, Clone, PartialEq)]
pub struct RowSnapshot {
    /// The runs, left to right.
    pub runs: Vec<RunSnapshot>,
}

/// A shell-integration mark the scrollbar draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkTick {
    /// The document row the mark starts on.
    pub row: i32,
    /// Whether the command that followed it failed.
    pub error: bool,
}

/// The visible screen at one instant.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Screen {
    /// The screen width in cells.
    pub width: i32,
    /// The screen height in cells.
    pub height: i32,
    /// The document row shown at the top of the screen.
    pub top: i32,
    /// How many rows the document holds, history included.
    pub document_height: i32,
    /// One entry per screen row.
    pub rows: Vec<RowSnapshot>,
    /// The prompt marks the scrollbar draws.
    pub marks: Vec<MarkTick>,
    /// The cursor cell, zero-based, `(column, row)`.
    pub cursor: (i32, i32),
    /// Whether the cursor is currently visible.
    pub cursor_visible: bool,
    /// The shape the cursor is drawn with.
    pub cursor_shape: CursorType,
    /// Whether the cursor is blinking.
    pub cursor_blinking: bool,
}

/// Takes a snapshot of `buffer`'s live screen.
pub fn snapshot(buffer: &TextBuffer) -> Screen {
    snapshot_at(buffer, 0)
}

/// Takes a snapshot of the screen scrolled `scroll` rows into the history.
///
/// `scroll` is clamped to the rows the buffer holds. The cursor is hidden while
/// the view is scrolled, because it belongs to the live screen.
pub fn snapshot_at(buffer: &TextBuffer, scroll: i32) -> Screen {
    let width = buffer.screen_width();
    let height = buffer.screen_height();
    let total = buffer.document_height();
    let scroll = scroll.clamp(0, (total - height).max(0));

    // The document is history first, then the screen, so the first visible
    // row is the document height minus the screen and the scroll.
    let first = (total - height - scroll).max(0);
    let rows = (0..height)
        .map(|offset| {
            buffer
                .get_row_by_document_line(first + offset)
                .map_or_else(|| RowSnapshot { runs: Vec::new() }, |line| row_snapshot(line, width))
        })
        .collect();

    let cursor = buffer.cursor();
    // `Cursor::position` is `(row, column)`; the snapshot is `(column, row)`,
    // the order a painter indexes a cell by.
    let (row, column) = cursor.position();
    Screen {
        width,
        height,
        top: first,
        document_height: total,
        rows,
        marks: mark_ticks(buffer),
        cursor: (column, row),
        cursor_visible: cursor.is_visible() && scroll == 0,
        cursor_shape: cursor.shape(),
        cursor_blinking: cursor.is_blinking(),
    }
}

/// One tick per prompt, flagged when the command that followed it failed.
pub(crate) fn mark_ticks(buffer: &TextBuffer) -> Vec<MarkTick> {
    let marks = buffer.marks();
    let mut ticks = Vec::new();

    for prompt in marks.iter().filter(|mark| mark.kind() == MarkKind::Prompt) {
        // The command that follows a prompt carries its exit code. The nearest
        // command at or after the prompt is that one; insertion order alone
        // cannot say, because marks on the same row may be stored in either
        // order.
        let error = marks
            .iter()
            .filter(|mark| mark.kind() == MarkKind::Command && mark.start_row() >= prompt.start_row())
            .min_by_key(|mark| mark.start_row())
            .and_then(|mark| mark.error())
            .is_some_and(|code| code != 0);

        ticks.push(MarkTick {
            row: prompt.start_row(),
            error,
        });
    }

    ticks
}

fn row_snapshot(line: &Row, width: i32) -> RowSnapshot {
    let mut column = 0;
    let runs = line
        .attribute_runs(0, width)
        .into_iter()
        .map(|(attribute, cells)| {
            let text = line.get_text_range(column, column + cells);
            let run = RunSnapshot {
                column,
                cells,
                attribute,
                text,
            };
            column += cells;
            run
        })
        .collect();

    RowSnapshot { runs }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_carries_the_rows_runs_and_cursor() {
        let mut buffer = TextBuffer::with_dimensions(6, 3, 0);
        buffer.write_string("hi");

        let screen = snapshot(&buffer);
        assert_eq!((screen.width, screen.height), (6, 3));
        assert_eq!(screen.rows.len(), 3);

        // Empty cells share the default attribute, so the row is one run.
        assert_eq!(screen.rows[0].runs.len(), 1);
        let run = &screen.rows[0].runs[0];
        assert_eq!((run.column, run.cells), (0, 6));
        // The row is padded to the readable width, not trimmed.
        assert_eq!(run.text, "hi    ");

        // Empty rows still produce a row of runs, so painting indexes by row.
        assert_eq!(screen.rows[2].runs.iter().map(|run| run.cells).sum::<i32>(), 6);

        assert_eq!(screen.cursor, (2, 0));
        assert!(screen.cursor_visible);
    }

    #[test]
    fn the_cursor_pair_is_column_then_row() {
        let mut buffer = TextBuffer::with_dimensions(10, 5, 0);
        buffer.cursor_mut().set_position(3, 7);

        // The buffer stores `(row, column)`; a painter needs `(column, row)`.
        assert_eq!(snapshot(&buffer).cursor, (7, 3));
    }

    #[test]
    fn a_snapshot_carries_the_document_height_and_its_prompt_marks() {
        let mut buffer = TextBuffer::with_dimensions(5, 2, 4);
        buffer.write_string("one");
        // A full-page scroll keeps the evicted row in the scrollback.
        buffer.scroll_region_up(0, 1);

        buffer.marks_mut().start_prompt(0);
        buffer.marks_mut().start_command(0);
        buffer.marks_mut().start_output(1);
        buffer.marks_mut().end_current_command(2, Some(1));

        let screen = snapshot(&buffer);
        assert_eq!(screen.document_height, 3);
        assert_eq!(screen.marks, vec![MarkTick { row: 0, error: true }]);
    }

    #[test]
    fn a_successful_command_marks_no_error() {
        let mut buffer = TextBuffer::with_dimensions(5, 2, 0);
        buffer.marks_mut().start_prompt(0);
        buffer.marks_mut().start_command(0);
        buffer.marks_mut().end_current_command(0, Some(0));

        assert_eq!(snapshot(&buffer).marks, vec![MarkTick { row: 0, error: false }]);
    }

    #[test]
    fn a_hidden_cursor_is_reported_as_hidden() {
        let mut buffer = TextBuffer::with_dimensions(4, 2, 0);
        buffer.cursor_mut().set_visible(false);

        let screen = snapshot(&buffer);
        assert!(!screen.cursor_visible);
    }

    #[test]
    fn a_snapshot_carries_the_cursor_shape_and_blink() {
        let mut buffer = TextBuffer::with_dimensions(4, 2, 0);
        buffer.cursor_mut().set_shape(CursorType::VerticalBar);
        buffer.cursor_mut().set_blinking(true);

        let screen = snapshot(&buffer);
        assert_eq!(screen.cursor_shape, CursorType::VerticalBar);
        assert!(screen.cursor_blinking);
    }

    #[test]
    fn scrolling_reveals_the_history_and_hides_the_cursor() {
        let mut buffer = TextBuffer::with_dimensions(5, 2, 4);
        buffer.write_string("one");
        // A full-page scroll keeps the evicted row in the scrollback.
        buffer.scroll_region_up(0, 1);
        buffer.cursor_mut().set_position(1, 0);
        buffer.write_string("two");
        assert_eq!(buffer.scrollback_len(), 1);

        let live = snapshot_at(&buffer, 0);
        assert_eq!(live.top, 1);
        assert_eq!(live.rows[1].runs[0].text, "two  ");
        assert!(live.cursor_visible);

        let history = snapshot_at(&buffer, 1);
        assert_eq!(history.top, 0);
        assert_eq!(history.rows[0].runs[0].text, "one  ");
        assert_eq!(history.rows[1].runs[0].text, "     ");
        assert!(!history.cursor_visible);

        // A scroll past the end clamps to the oldest row.
        let oldest = snapshot_at(&buffer, 99);
        assert_eq!(oldest.rows[0].runs[0].text, "one  ");
    }
}
