//! `OutputCellRect` — rectangular region of terminal buffer cells.
//!
//! Ported from `OutputCellRect` in the Windows Terminal C++ buffer module.

use crate::output_cell::OutputCell;
use crate::row::Row;

/// A rectangular region of cells, defined by top-left and bottom-right corners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputCellRect {
    /// Top-left row (inclusive).
    top: i32,
    /// Top-left column (inclusive).
    left: i32,
    /// Bottom-right row (inclusive).
    bottom: i32,
    /// Bottom-right column (inclusive).
    right: i32,
}

impl OutputCellRect {
    /// Creates a new rectangle.
    pub fn new(top: i32, left: i32, bottom: i32, right: i32) -> Self {
        Self {
            top: top.min(bottom),
            left: left.min(right),
            bottom: bottom.max(top),
            right: right.max(left),
        }
    }

    /// Creates a rectangle from the given origin point.
    pub const fn from_origin(top: i32, left: i32, width: i32, height: i32) -> Self {
        Self {
            top,
            left,
            bottom: top + height - 1,
            right: left + width - 1,
        }
    }

    /// Returns the top row.
    pub const fn top(&self) -> i32 {
        self.top
    }

    /// Returns the left column.
    pub const fn left(&self) -> i32 {
        self.left
    }

    /// Returns the bottom row.
    pub const fn bottom(&self) -> i32 {
        self.bottom
    }

    /// Returns the right column.
    pub const fn right(&self) -> i32 {
        self.right
    }

    /// Returns the width (number of columns).
    pub const fn width(&self) -> i32 {
        self.right - self.left + 1
    }

    /// Returns the height (number of rows).
    pub const fn height(&self) -> i32 {
        self.bottom - self.top + 1
    }

    /// Returns the area (number of cells).
    pub const fn area(&self) -> i32 {
        self.width() * self.height()
    }

    /// Returns whether the rectangle is empty (zero area).
    pub const fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    /// Returns whether this rectangle contains the given point.
    pub fn contains(&self, row: i32, col: i32) -> bool {
        row >= self.top && row <= self.bottom && col >= self.left && col <= self.right
    }

    /// Returns whether this rectangle contains another rectangle.
    pub fn contains_rect(&self, other: &OutputCellRect) -> bool {
        other.top >= self.top && other.bottom <= self.bottom && other.left >= self.left && other.right <= self.right
    }

    /// Returns the intersection of this rectangle with another.
    pub fn intersection(&self, other: &OutputCellRect) -> Option<OutputCellRect> {
        let new_top = self.top.max(other.top);
        let new_left = self.left.max(other.left);
        let new_bottom = self.bottom.min(other.bottom);
        let new_right = self.right.min(other.right);

        if new_top <= new_bottom && new_left <= new_right {
            Some(OutputCellRect::new(new_top, new_left, new_bottom, new_right))
        } else {
            None
        }
    }

    /// Returns the union of this rectangle with another.
    pub fn union(&self, other: &OutputCellRect) -> OutputCellRect {
        OutputCellRect::new(
            self.top.min(other.top),
            self.left.min(other.left),
            self.bottom.max(other.bottom),
            self.right.max(other.right),
        )
    }

    /// Returns an iterator over all cells in the rectangle, row by row.
    pub fn iter_rows<'a>(
        &'a self,
        row: &'a Row,
        start_col: i32,
        end_col: i32,
    ) -> impl Iterator<Item = &'a OutputCell> + 'a {
        (start_col..=end_col).filter_map(|col| {
            if col >= 0 && col < row.width() {
                // get_cell panics on out of bounds, but we've checked
                Some(row.get_cell(col))
            } else {
                None
            }
        })
    }

    /// Returns a new rectangle shifted by the given offset.
    pub fn shifted(&self, row_offset: i32, col_offset: i32) -> OutputCellRect {
        OutputCellRect::new(
            self.top + row_offset,
            self.left + col_offset,
            self.bottom + row_offset,
            self.right + col_offset,
        )
    }
}

impl Default for OutputCellRect {
    fn default() -> Self {
        Self::new(0, 0, 0, 0)
    }
}

impl std::fmt::Display for OutputCellRect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({},{})-({},{})", self.top, self.left, self.bottom, self.right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creation() {
        let rect = OutputCellRect::new(0, 0, 10, 20);
        assert_eq!(rect.top(), 0);
        assert_eq!(rect.left(), 0);
        assert_eq!(rect.bottom(), 10);
        assert_eq!(rect.right(), 20);
        assert_eq!(rect.width(), 21);
        assert_eq!(rect.height(), 11);
        assert_eq!(rect.area(), 231);
    }

    #[test]
    fn test_from_origin() {
        let rect = OutputCellRect::from_origin(5, 10, 20, 15);
        assert_eq!(rect.top(), 5);
        assert_eq!(rect.left(), 10);
        assert_eq!(rect.bottom(), 19);
        assert_eq!(rect.right(), 29);
        assert_eq!(rect.width(), 20);
        assert_eq!(rect.height(), 15);
    }

    #[test]
    fn test_contains() {
        let rect = OutputCellRect::new(0, 0, 10, 20);
        assert!(rect.contains(5, 10));
        assert!(rect.contains(0, 0));
        assert!(rect.contains(10, 20));
        assert!(!rect.contains(11, 10));
        assert!(!rect.contains(5, 21));
    }

    #[test]
    fn test_intersection() {
        let rect1 = OutputCellRect::new(0, 0, 10, 10);
        let rect2 = OutputCellRect::new(5, 5, 15, 15);
        let inter = rect1.intersection(&rect2).unwrap();
        assert_eq!(inter.top(), 5);
        assert_eq!(inter.left(), 5);
        assert_eq!(inter.bottom(), 10);
        assert_eq!(inter.right(), 10);

        // Disjoint rectangles
        let rect3 = OutputCellRect::new(20, 20, 30, 30);
        assert!(rect1.intersection(&rect3).is_none());
    }

    #[test]
    fn test_union() {
        let rect1 = OutputCellRect::new(0, 0, 5, 5);
        let rect2 = OutputCellRect::new(3, 3, 10, 10);
        let union = rect1.union(&rect2);
        assert_eq!(union.top(), 0);
        assert_eq!(union.left(), 0);
        assert_eq!(union.bottom(), 10);
        assert_eq!(union.right(), 10);
    }

    #[test]
    fn test_empty_rect() {
        // Create a zero-width rect using from_origin
        let rect = OutputCellRect::from_origin(0, 0, 0, 0);
        assert!(rect.is_empty());
        assert_eq!(rect.width(), 0);
        assert_eq!(rect.height(), 0);

        // Create a rect with reversed corners
        let rect = OutputCellRect::new(5, 5, 5, 5);
        assert!(!rect.is_empty()); // This is actually a single point, not empty

        // Empty via from_origin with zero dimensions
        let empty = OutputCellRect::from_origin(0, 0, 0, 0);
        assert!(empty.is_empty());
    }

    #[test]
    fn test_shifted() {
        let rect = OutputCellRect::new(0, 0, 10, 20);
        let shifted = rect.shifted(5, 5);
        assert_eq!(shifted.top(), 5);
        assert_eq!(shifted.left(), 5);
        assert_eq!(shifted.bottom(), 15);
        assert_eq!(shifted.right(), 25);
    }

    #[test]
    fn test_contains_rect() {
        let outer = OutputCellRect::new(0, 0, 10, 20);
        let inner = OutputCellRect::new(2, 5, 8, 15);
        assert!(outer.contains_rect(&inner));

        let outer2 = OutputCellRect::new(0, 0, 10, 20);
        let not_inner = OutputCellRect::new(5, 15, 15, 25);
        assert!(!outer2.contains_rect(&not_inner));
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    prop_compose! {
        fn any_rect()(
            top in 0i32..100i32,
            left in 0i32..100i32,
            width in 1i32..50i32,
            height in 1i32..50i32,
        ) -> OutputCellRect {
            OutputCellRect::from_origin(top, left, width, height)
        }
    }

    proptest! {
        #[test]
        fn test_intersection_is_subset(rect1 in any_rect(), rect2 in any_rect()) {
            if let Some(inter) = rect1.intersection(&rect2) {
                prop_assert!(rect1.contains_rect(&inter));
                prop_assert!(rect2.contains_rect(&inter));
            }
        }

        #[test]
        fn test_union_contains_both(rect1 in any_rect(), rect2 in any_rect()) {
            let union = rect1.union(&rect2);
            prop_assert!(union.contains_rect(&rect1));
            prop_assert!(union.contains_rect(&rect2));
        }

        #[test]
        fn test_shifted_contains_same_shape(rect in any_rect(), row_off in -100i32..100i32, col_off in -100i32..100i32) {
            let shifted = rect.shifted(row_off, col_off);
            prop_assert_eq!(shifted.width(), rect.width());
            prop_assert_eq!(shifted.height(), rect.height());
        }

        #[test]
        fn test_rect_width_height_positive(rect in any_rect()) {
            prop_assert!(rect.width() > 0);
            prop_assert!(rect.height() > 0);
        }
    }
}
