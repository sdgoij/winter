//! A snapshot of the visible screen, ready for a painter.
//!
//! This module is toolkit-free (ADR 018): it reads the buffer and groups each
//! row into attribute runs. A window backend decides how to draw them.

use buffer::TextBuffer;
use types::TextAttribute;

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

/// The visible screen at one instant.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Screen {
    /// The screen width in cells.
    pub width: i32,
    /// The screen height in cells.
    pub height: i32,
    /// One entry per screen row.
    pub rows: Vec<RowSnapshot>,
    /// The cursor cell, zero-based, `(column, row)`.
    pub cursor: (i32, i32),
    /// Whether the cursor is currently visible.
    pub cursor_visible: bool,
}

/// Takes a snapshot of `buffer`'s visible screen.
pub fn snapshot(buffer: &TextBuffer) -> Screen {
    let width = buffer.screen_width();
    let height = buffer.screen_height();
    let rows = (0..height).map(|row| row_snapshot(buffer, row, width)).collect();
    let cursor = buffer.cursor();

    Screen {
        width,
        height,
        rows,
        cursor: cursor.position(),
        cursor_visible: cursor.is_visible(),
    }
}

fn row_snapshot(buffer: &TextBuffer, row: i32, width: i32) -> RowSnapshot {
    let Some(line) = buffer.get_row_at_screen_line(row) else {
        return RowSnapshot { runs: Vec::new() };
    };

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

        assert_eq!(screen.cursor, (0, 2));
        assert!(screen.cursor_visible);
    }

    #[test]
    fn a_hidden_cursor_is_reported_as_hidden() {
        let mut buffer = TextBuffer::with_dimensions(4, 2, 0);
        buffer.cursor_mut().set_visible(false);

        let screen = snapshot(&buffer);
        assert!(!screen.cursor_visible);
    }
}
