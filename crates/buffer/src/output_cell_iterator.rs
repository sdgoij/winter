//! `OutputCellIterator` — iterator over terminal buffer cells.
//!
//! Ported from `OutputCellIterator` in the Windows Terminal C++ buffer module.

use crate::output_cell::OutputCell;

/// An iterator over terminal buffer cells.
///
/// Iterates over a range of cells in a row, providing both the index and the cell.
#[derive(Debug, Clone)]
pub struct OutputCellIterator<'a> {
    /// The underlying slice of cells.
    cells: &'a [OutputCell],
    /// The starting index (inclusive).
    start: usize,
    /// The ending index (exclusive).
    end: usize,
    /// The current position.
    current: usize,
}

impl<'a> OutputCellIterator<'a> {
    /// Creates a new iterator over a range of cells.
    pub fn new(cells: &'a [OutputCell], start: usize, end: usize) -> Self {
        let end = end.min(cells.len());
        let start = start.min(end);
        Self {
            cells,
            start,
            end,
            current: start,
        }
    }

    /// Creates an iterator over all cells.
    pub fn all(cells: &'a [OutputCell]) -> Self {
        Self {
            cells,
            start: 0,
            end: cells.len(),
            current: 0,
        }
    }

    /// Returns the remaining distance to the end.
    pub const fn remaining(&self) -> usize {
        self.end - self.current
    }

    /// Returns the distance from start to end.
    pub const fn distance(&self) -> usize {
        self.end - self.start
    }

    /// Advances the iterator by the given count.
    pub fn advance(&mut self, count: usize) {
        self.current = (self.current + count).min(self.end);
    }

    /// Advances the iterator past a certain number of cells.
    pub fn advance_past(&mut self, count: usize) {
        self.current = (self.current + count).min(self.end);
    }
}

impl<'a> Iterator for OutputCellIterator<'a> {
    type Item = &'a OutputCell;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.end {
            return None;
        }
        let cell = &self.cells[self.current];
        self.current += 1;
        Some(cell)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end - self.current;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for OutputCellIterator<'a> {}

/// A mutable iterator over terminal cells.
#[allow(dead_code)]
pub struct OutputCellMutIterator<'a> {
    cells: &'a mut [OutputCell],
    start: usize,
    end: usize,
    current: usize,
}

impl<'a> OutputCellMutIterator<'a> {
    /// Creates a new mutable iterator over a range of cells.
    pub fn new(cells: &'a mut [OutputCell], start: usize, end: usize) -> Self {
        let end = end.min(cells.len());
        Self {
            cells,
            start: start.min(end),
            end,
            current: start,
        }
    }

    /// Returns the remaining distance to the end.
    pub const fn remaining(&self) -> usize {
        self.end - self.current
    }
}

impl<'a> Iterator for OutputCellMutIterator<'a> {
    type Item = &'a mut OutputCell;

    // Yielding `&'a mut` from `&mut self` is the one place this crate needs
    // `unsafe`; the iterator owns the only `&'a mut` to the slice, and each call
    // hands out a distinct index, so no two returned references alias.
    #[allow(unsafe_code)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.end {
            return None;
        }
        let idx = self.current;
        self.current += 1;
        // SAFETY: `idx < end <= cells.len()`, and `self.cells` is the unique
        // borrow held for the iterator's whole life, so the reference is valid
        // and unaliased for `'a`.
        unsafe {
            let ptr = self.cells.as_mut_ptr().add(idx);
            Some(&mut *ptr)
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end - self.current;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for OutputCellMutIterator<'a> {}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cells(count: usize) -> Vec<OutputCell> {
        (0..count)
            .map(|i| OutputCell::from_char((b'a' + i as u8) as char))
            .collect()
    }

    #[test]
    fn test_empty_range() {
        let cells = make_cells(10);
        let iter = OutputCellIterator::new(&cells, 5, 5);
        assert_eq!(iter.count(), 0);
    }

    #[test]
    fn test_full_range() {
        let cells = make_cells(5);
        let iter = OutputCellIterator::all(&cells);
        assert_eq!(iter.count(), 5);
    }

    #[test]
    fn test_partial_range() {
        let cells = make_cells(10);
        let iter = OutputCellIterator::new(&cells, 3, 7);
        assert_eq!(iter.count(), 4);
    }

    #[test]
    fn test_advance() {
        let cells = make_cells(10);
        let mut iter = OutputCellIterator::all(&cells);
        iter.advance(3);
        assert_eq!(iter.remaining(), 7);
        assert_eq!(iter.count(), 7);
    }

    #[test]
    fn test_bounds_checking() {
        let cells = make_cells(5);
        let iter = OutputCellIterator::new(&cells, 0, 100); // end beyond length
        assert_eq!(iter.count(), 5);
    }

    #[test]
    fn test_distance() {
        let cells = make_cells(10);
        let iter = OutputCellIterator::new(&cells, 3, 7);
        assert_eq!(iter.distance(), 4);
    }

    #[test]
    fn test_mutable_iterator() {
        let mut cells = make_cells(5);
        for cell in OutputCellMutIterator::new(&mut cells, 0, 3) {
            cell.set_attributes(crate::TextAttribute::new());
        }
        assert_eq!(cells.len(), 5);
    }

    #[test]
    fn test_size_hint() {
        let cells = make_cells(10);
        let mut iter = OutputCellIterator::new(&cells, 3, 7);
        let (lower, upper) = iter.size_hint();
        assert_eq!(lower, 4);
        assert_eq!(upper, Some(4));

        iter.advance(2);
        let (lower, upper) = iter.size_hint();
        assert_eq!(lower, 2);
        assert_eq!(upper, Some(2));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_iterate_forward_returns_all_cells(count in 1usize..50usize) {
            let cells = make_cells(count);
            let iter = OutputCellIterator::all(&cells);
            prop_assert_eq!(iter.count(), count);
        }

        #[test]
        fn test_advance_past_returns_remaining(count in 5usize..50usize, advance_amount in 0usize..50usize) {
            let cells = make_cells(count);
            let mut iter = OutputCellIterator::all(&cells);
            iter.advance(advance_amount);
            let remaining = count.saturating_sub(advance_amount);
            prop_assert_eq!(iter.remaining(), remaining);
        }

        #[test]
        fn test_size_hint_matches_remaining(count in 1usize..50usize, start in 0usize..50usize, end in 0usize..50usize) {
            let cells = make_cells(count);
            let iter = OutputCellIterator::new(&cells, start, end);
            let remaining = iter.remaining();
            let (lower, _) = iter.size_hint();
            prop_assert_eq!(lower, remaining);
            // Note: remaining is always >= 0 for usize since current <= end (clamped in new())
        }
    }
}

#[cfg(test)]
fn make_cells(count: usize) -> Vec<OutputCell> {
    (0..count)
        .map(|i| OutputCell::from_char((b'a' + (i % 95) as u8) as char))
        .collect()
}
