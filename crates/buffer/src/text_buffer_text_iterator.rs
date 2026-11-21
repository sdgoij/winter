//! `TextBufferTextIterator` — iterator over text content in the terminal buffer.
//!
//! Iterates through the visible buffer rows, providing text content.

use crate::row::Row;

/// Iterates over text content from the terminal buffer rows.
#[derive(Debug, Clone)]
pub struct TextBufferTextIterator<'a> {
    /// The underlying rows.
    rows: &'a [Row],
    /// Current row index.
    current_row: usize,
    /// Current column within the row.
    current_col: usize,
}

impl<'a> TextBufferTextIterator<'a> {
    /// Creates a new iterator over all rows.
    pub fn new(rows: &'a [Row]) -> Self {
        Self {
            rows,
            current_row: 0,
            current_col: 0,
        }
    }

    /// Returns the number of remaining rows.
    pub const fn remaining_rows(&self) -> usize {
        self.rows.len() - self.current_row
    }
}

impl<'a> Iterator for TextBufferTextIterator<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        while self.current_row < self.rows.len() {
            let row = &self.rows[self.current_row];
            if self.current_col < row.cell_count() {
                let cell = row.get_cell(self.current_col as i32);
                let text = cell.chars();
                if !text.is_empty() {
                    self.current_col += 1;
                    return Some(text);
                } else {
                    // Empty cell, skip to next
                    self.current_col += 1;
                }
            } else {
                // Move to next row
                self.current_row += 1;
                self.current_col = 0;
            }
        }
        None
    }
}

/// A mutable iterator over rows in the terminal buffer.
#[derive(Debug)]
pub struct TextBufferRowIterator<'a> {
    rows: &'a mut [Row],
    current: usize,
}

impl<'a> TextBufferRowIterator<'a> {
    /// Creates a new row iterator.
    pub fn new(rows: &'a mut [Row]) -> Self {
        Self { rows, current: 0 }
    }
}

impl<'a> Iterator for TextBufferRowIterator<'a> {
    type Item = &'a mut Row;

    // Yielding `&'a mut` from `&mut self` is the one place this crate needs
    // `unsafe`; the iterator owns the only `&'a mut` to the rows, and each call
    // hands out a distinct index, so no two returned references alias.
    #[allow(unsafe_code)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.current < self.rows.len() {
            let idx = self.current;
            self.current += 1;
            // SAFETY: `idx < rows.len()`, and `self.rows` is the unique borrow
            // held for the iterator's whole life, so the reference is valid and
            // unaliased for `'a`.
            unsafe {
                let ptr = self.rows.as_mut_ptr().add(idx);
                Some(&mut *ptr)
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output_cell::OutputCell;

    fn make_row(width: i32, content: &str) -> Row {
        let mut row = Row::new(width);
        for (i, c) in content.chars().enumerate() {
            if i < width as usize {
                row.write_cell(i as i32, OutputCell::from_char(c));
            }
        }
        row
    }

    #[test]
    fn test_empty_rows() {
        let rows: Vec<Row> = (0..5).map(|_| Row::new(10)).collect();
        let iter = TextBufferTextIterator::new(&rows);
        let texts: Vec<&str> = iter.collect();
        assert!(texts.is_empty());
    }

    #[test]
    fn test_single_row() {
        let rows = vec![make_row(10, "Hello")];
        let iter = TextBufferTextIterator::new(&rows);
        let texts: Vec<&str> = iter.collect();
        assert_eq!(texts, vec!["H", "e", "l", "l", "o"]);
    }

    #[test]
    fn test_multiple_rows() {
        let rows = vec![make_row(10, "Hello"), make_row(10, "World")];
        let iter = TextBufferTextIterator::new(&rows);
        let texts: Vec<&str> = iter.collect();
        assert_eq!(texts.len(), 10);
    }

    #[test]
    fn test_row_iterator() {
        let mut rows = vec![Row::new(10), Row::new(10)];
        let count = TextBufferRowIterator::new(&mut rows).count();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_remaining_rows() {
        let rows: Vec<Row> = (0..5)
            .map(|_r| {
                let mut row = Row::new(10);
                row.write_char(0, 'A', false);
                row
            })
            .collect();

        let iter = TextBufferTextIterator::new(&rows);
        assert_eq!(iter.remaining_rows(), 5);
        assert_eq!(iter.count(), 5); // 5 rows with content
    }
}
