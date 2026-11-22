//! `TextBufferCellIterator` — a read-only walk over the screen cells.
//!
//! Ported from `textBufferCellIterator.hpp/.cpp`. Upstream dereferences to an
//! `OutputCellView`, a borrowed view over the row's glyph, DBCS attribute and
//! text attribute. This port's rows already store an [`OutputCell`] by value,
//! so the iterator yields `&OutputCell` directly; the paint path reads the same
//! fields from it.
//!
//! Upstream throws on an out-of-bounds position or limits; this port returns
//! `None` from the constructor instead, because a renderer must not panic on
//! inconsistent host state.

use utils::{InclusiveRect, Point};

use crate::output_cell::OutputCell;
use crate::text_buffer::TextBuffer;

/// Walks the cells of a [`TextBuffer`] inside an inclusive viewport.
#[derive(Debug, Clone)]
pub struct TextBufferCellIterator<'a> {
    buffer: &'a TextBuffer,
    bounds: InclusiveRect,
    position: Point,
    exceeded: bool,
}

impl<'a> TextBufferCellIterator<'a> {
    /// Creates an iterator over the whole screen at `position`, or `None` when
    /// the position is outside the buffer.
    pub fn new(buffer: &'a TextBuffer, position: Point) -> Option<Self> {
        let bounds = InclusiveRect::new(0, 0, buffer.screen_width() - 1, buffer.screen_height() - 1);
        Self::with_bounds(buffer, position, bounds)
    }

    /// Creates an iterator restricted to `bounds`, or `None` when `bounds` is
    /// empty, outside the buffer, or does not contain `position`.
    pub fn with_bounds(buffer: &'a TextBuffer, position: Point, bounds: InclusiveRect) -> Option<Self> {
        let fits = bounds.left() >= 0
            && bounds.top() >= 0
            && bounds.right() >= bounds.left()
            && bounds.bottom() >= bounds.top()
            && bounds.right() < buffer.screen_width()
            && bounds.bottom() < buffer.screen_height();

        if !fits || !bounds.contains(position) {
            return None;
        }

        Some(Self {
            buffer,
            bounds,
            position,
            exceeded: false,
        })
    }

    /// Returns whether the iterator still points at a cell.
    pub fn is_valid(&self) -> bool {
        !self.exceeded && self.bounds.contains(self.position)
    }

    /// Returns the current position.
    pub const fn position(&self) -> Point {
        self.position
    }

    /// Returns the cell at the current position, or `None` past the end.
    pub fn cell(&self) -> Option<&'a OutputCell> {
        if !self.is_valid() {
            return None;
        }

        let row = self.buffer.get_row_at_screen_line(self.position.y())?;
        Some(row.get_cell(self.position.x()))
    }

    /// Moves forward by `movement` cells, wrapping at the bounds.
    pub fn advance(&mut self, movement: isize) {
        if movement < 0 {
            self.retreat(movement.unsigned_abs());
            return;
        }

        let mut remaining = movement as usize;
        while remaining > 0 {
            if self.exceeded {
                break;
            }

            if self.position.x() == self.bounds.right() {
                if self.position.y() == self.bounds.bottom() {
                    // Past the last cell: hide the position at the top-left, as
                    // upstream's `IncrementInBounds` does.
                    self.position = Point::new(self.bounds.left(), self.bounds.top());
                    self.exceeded = true;
                    break;
                }
                self.position = Point::new(self.bounds.left(), self.position.y() + 1);
            } else {
                self.position = Point::new(self.position.x() + 1, self.position.y());
            }

            self.exceeded = false;
            remaining -= 1;
        }
    }

    /// Moves backward by `movement` cells, wrapping at the bounds.
    fn retreat(&mut self, movement: usize) {
        let mut remaining = movement;
        while remaining > 0 {
            if self.exceeded {
                break;
            }

            if self.position.x() == self.bounds.left() {
                if self.position.y() == self.bounds.top() {
                    self.exceeded = true;
                    break;
                }
                self.position = Point::new(self.bounds.right(), self.position.y() - 1);
            } else {
                self.position = Point::new(self.position.x() - 1, self.position.y());
            }

            self.exceeded = false;
            remaining -= 1;
        }
    }
}

impl<'a> Iterator for TextBufferCellIterator<'a> {
    type Item = &'a OutputCell;

    fn next(&mut self) -> Option<Self::Item> {
        let cell = self.cell()?;
        self.advance(1);
        Some(cell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output_cell::OutputCell;

    fn make_buffer() -> TextBuffer {
        let mut buffer = TextBuffer::with_dimensions(3, 2, 0);
        for (row, text) in ["abc", "def"].iter().enumerate() {
            let line = buffer.get_row_at_screen_line_mut(row as i32).expect("row exists");
            for (column, ch) in text.chars().enumerate() {
                line.write_cell(column as i32, OutputCell::from_char(ch));
            }
        }
        buffer
    }

    fn collected(iterator: TextBufferCellIterator<'_>) -> Vec<String> {
        iterator.map(|cell| cell.chars().to_string()).collect()
    }

    #[test]
    fn it_walks_every_cell_in_reading_order() {
        let buffer = make_buffer();
        let iterator = TextBufferCellIterator::new(&buffer, Point::new(0, 0)).expect("inside");
        assert_eq!(collected(iterator), ["a", "b", "c", "d", "e", "f"]);
    }

    #[test]
    fn it_respects_a_restricted_viewport() {
        let buffer = make_buffer();
        let bounds = InclusiveRect::new(1, 0, 2, 0);
        let iterator = TextBufferCellIterator::with_bounds(&buffer, Point::new(1, 0), bounds).expect("inside");
        assert_eq!(collected(iterator), ["b", "c"]);
    }

    #[test]
    fn it_wraps_to_the_next_row() {
        let buffer = make_buffer();
        let mut iterator = TextBufferCellIterator::new(&buffer, Point::new(2, 0)).expect("inside");

        iterator.advance(1);
        assert_eq!(iterator.position(), Point::new(0, 1));
        assert_eq!(iterator.cell().map(|cell| cell.chars()), Some("d"));
    }

    #[test]
    fn it_retreats_to_the_previous_row() {
        let buffer = make_buffer();
        let mut iterator = TextBufferCellIterator::new(&buffer, Point::new(0, 1)).expect("inside");

        iterator.advance(-1);
        assert_eq!(iterator.position(), Point::new(2, 0));
        assert_eq!(iterator.cell().map(|cell| cell.chars()), Some("c"));
    }

    #[test]
    fn it_ends_after_the_last_cell() {
        let buffer = make_buffer();
        let mut iterator = TextBufferCellIterator::new(&buffer, Point::new(2, 1)).expect("inside");

        assert!(iterator.is_valid());
        iterator.advance(1);
        assert!(!iterator.is_valid());
        assert!(iterator.cell().is_none());
    }

    #[test]
    fn an_out_of_bounds_position_is_rejected() {
        let buffer = make_buffer();
        assert!(TextBufferCellIterator::new(&buffer, Point::new(3, 0)).is_none());
        assert!(TextBufferCellIterator::new(&buffer, Point::new(0, 2)).is_none());
        assert!(TextBufferCellIterator::new(&buffer, Point::new(-1, 0)).is_none());
    }

    #[test]
    fn restricted_bounds_must_contain_the_position() {
        let buffer = make_buffer();
        let bounds = InclusiveRect::new(1, 0, 2, 0);
        assert!(TextBufferCellIterator::with_bounds(&buffer, Point::new(0, 0), bounds).is_none());
    }
}
