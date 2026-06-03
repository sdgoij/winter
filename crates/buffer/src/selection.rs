//! `Selection` — text selection in the terminal buffer.
//!
//! Ported from `Selection` in the Windows Terminal C++ buffer module.
//! Supports regular, rectangular, and word selections.

/// The type of selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionType {
    /// Regular (line-based) selection.
    #[default]
    Regular,
    /// Rectangular (block) selection.
    Rectangular,
    /// Word selection (selects whole words).
    Word,
    /// Line selection (selects entire lines).
    Line,
}

impl SelectionType {
    /// Returns the display name of this selection type.
    pub const fn name(&self) -> &'static str {
        match self {
            SelectionType::Regular => "regular",
            SelectionType::Rectangular => "rectangular",
            SelectionType::Word => "word",
            SelectionType::Line => "line",
        }
    }
}

/// The anchor and extent of a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionPoint {
    /// Row position.
    pub row: i32,
    /// Column position.
    pub col: i32,
}

impl SelectionPoint {
    /// Creates a new selection point.
    pub const fn new(row: i32, col: i32) -> Self {
        Self { row, col }
    }
}

/// A text selection in the terminal buffer.
#[derive(Debug, Clone)]
pub struct Selection {
    /// The type of selection.
    selection_type: SelectionType,
    /// The anchor (start) position.
    anchor: SelectionPoint,
    /// The extent (current/end) position.
    extent: SelectionPoint,
}

impl Selection {
    /// Creates a new selection with the given anchor point.
    pub fn new(row: i32, col: i32, selection_type: SelectionType) -> Self {
        Self {
            selection_type,
            anchor: SelectionPoint::new(row, col),
            extent: SelectionPoint::new(row, col),
        }
    }

    /// Returns the selection type.
    pub const fn selection_type(&self) -> SelectionType {
        self.selection_type
    }

    /// Returns the anchor point.
    pub const fn anchor(&self) -> SelectionPoint {
        self.anchor
    }

    /// Returns the end point of the selection.
    pub const fn end_point(&self) -> SelectionPoint {
        self.extent
    }

    /// Sets the end position.
    pub fn set_end(&mut self, row: i32, col: i32) {
        self.extent = SelectionPoint::new(row, col);
    }

    /// Sets the anchor position.
    pub fn set_anchor(&mut self, row: i32, col: i32) {
        self.anchor = SelectionPoint::new(row, col);
    }

    /// Returns the selection bounds (top, left, bottom, right).
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        let top = self.anchor.row.min(self.extent.row);
        let bottom = self.anchor.row.max(self.extent.row);
        let left = self.anchor.col.min(self.extent.col);
        let right = self.anchor.col.max(self.extent.col);
        (top, left, bottom, right)
    }

    /// Returns the selection extent as ((start_row, start_col), (end_row, end_col)).
    pub fn extent(&self) -> ((i32, i32), (i32, i32)) {
        let (top, left, bottom, right) = self.bounds();
        ((top, left), (bottom, right))
    }

    /// Returns the width of the selection.
    pub fn width(&self) -> i32 {
        self.extent.col.abs_diff(self.anchor.col) as i32 + 1
    }

    /// Returns the height of the selection.
    pub fn height(&self) -> i32 {
        self.extent.row.abs_diff(self.anchor.row) as i32 + 1
    }

    /// Returns whether the selection has a zero area.
    pub fn is_empty(&self) -> bool {
        self.anchor.row == self.extent.row && self.anchor.col == self.extent.col
    }

    /// Returns the selection text from the given buffer.
    pub fn get_text(&self, buffer: &crate::text_buffer::TextBuffer) -> String {
        let (top, left, bottom, right) = self.bounds();
        let mut result = String::new();

        for row in top..=bottom {
            if row < 0 {
                continue;
            }
            if let Some(row_data) = buffer.get_row_at_screen_line(row) {
                for col in left..=right {
                    if col >= 0 && col < row_data.width() {
                        let cell = row_data.get_cell(col);
                        if cell.chars().len() > 0 {
                            result.push_str(cell.chars());
                        }
                    }
                }
            }
            result.push('\n');
        }

        result
    }

    /// Returns the selection text as plain text (without newlines).
    pub fn get_plain_text(&self, buffer: &crate::text_buffer::TextBuffer) -> String {
        self.get_text(buffer).replace('\n', "")
    }
}

impl Default for Selection {
    fn default() -> Self {
        Self::new(0, 0, SelectionType::Regular)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_selection() {
        let sel = Selection::new(10, 20, SelectionType::Regular);
        assert_eq!(sel.anchor().row, 10);
        assert_eq!(sel.anchor().col, 20);
        let end = sel.end_point();
        assert_eq!(end.row, 10);
        assert_eq!(end.col, 20);
        assert!(sel.is_empty());
    }

    #[test]
    fn test_set_extent() {
        let mut sel = Selection::new(0, 0, SelectionType::Regular);
        sel.set_end(5, 10);
        let end = sel.end_point();
        assert_eq!(end.row, 5);
        assert_eq!(end.col, 10);
        assert!(!sel.is_empty());
    }

    #[test]
    fn test_bounds() {
        let sel = Selection::new(0, 0, SelectionType::Regular);
        let (top, left, bottom, right) = sel.bounds();
        assert_eq!(top, 0);
        assert_eq!(left, 0);
        assert_eq!(bottom, 0);
        assert_eq!(right, 0);

        let mut sel2 = Selection::new(5, 5, SelectionType::Regular);
        sel2.set_end(10, 15);
        let (top, left, bottom, right) = sel2.bounds();
        assert_eq!(top, 5);
        assert_eq!(left, 5);
        assert_eq!(bottom, 10);
        assert_eq!(right, 15);
    }

    #[test]
    fn test_extent_with_reversed_selection() {
        let mut sel = Selection::new(10, 10, SelectionType::Regular);
        sel.set_end(5, 5);
        let ((top, left), (bottom, right)) = sel.extent();
        assert_eq!(top, 5);
        assert_eq!(left, 5);
        assert_eq!(bottom, 10);
        assert_eq!(right, 10);
    }

    #[test]
    fn test_selection_dimensions() {
        let mut sel = Selection::new(0, 0, SelectionType::Regular);
        sel.set_end(0, 9);
        assert_eq!(sel.width(), 10);
        assert_eq!(sel.height(), 1);

        let mut sel2 = Selection::new(0, 0, SelectionType::Regular);
        sel2.set_end(4, 0);
        assert_eq!(sel2.width(), 1);
        assert_eq!(sel2.height(), 5);
    }

    #[test]
    fn test_selection_type_names() {
        assert_eq!(SelectionType::Regular.name(), "regular");
        assert_eq!(SelectionType::Rectangular.name(), "rectangular");
        assert_eq!(SelectionType::Word.name(), "word");
        assert_eq!(SelectionType::Line.name(), "line");
    }

    #[test]
    fn test_anchor_set() {
        let mut sel = Selection::new(0, 0, SelectionType::Regular);
        sel.set_anchor(5, 10);
        assert_eq!(sel.anchor().row, 5);
        assert_eq!(sel.anchor().col, 10);
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn test_bounds_are_consistent(row1 in 0i32..100i32, col1 in 0i32..100i32, row2 in 0i32..100i32, col2 in 0i32..100i32) {
            let mut sel = Selection::new(row1, col1, SelectionType::Regular);
            sel.set_end(row2, col2);
            let (top, left, bottom, right) = sel.bounds();
            prop_assert!(top <= bottom);
            prop_assert!(left <= right);
        }

        #[test]
        fn test_extent_matches_bounds(row1 in 0i32..100i32, col1 in 0i32..100i32, row2 in 0i32..100i32, col2 in 0i32..100i32) {
            let mut sel = Selection::new(row1, col1, SelectionType::Regular);
            sel.set_end(row2, col2);
            let ((top, left), (bottom, right)) = sel.extent();
            let (tb, lb, bb, rb) = sel.bounds();
            prop_assert_eq!(top, tb);
            prop_assert_eq!(left, lb);
            prop_assert_eq!(bottom, bb);
            prop_assert_eq!(right, rb);
        }

        #[test]
        fn test_dimensions_positive(row1 in 0i32..100i32, col1 in 0i32..100i32, row2 in 0i32..100i32, col2 in 0i32..100i32) {
            let mut sel = Selection::new(row1, col1, SelectionType::Regular);
            sel.set_end(row2, col2);
            prop_assert!(sel.width() > 0);
            prop_assert!(sel.height() > 0);
        }
    }
}
