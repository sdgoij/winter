//! `CharToColumnMapper` — maps character offsets to display column positions.
//!
//! Ported from the character-to-column mapping logic in the Windows Terminal C++ buffer module.
//! Essential for handling wide (double-width) characters like CJK and emoji correctly.

/// Maps character offsets within a row to display column positions.
///
/// For rows containing wide characters (CJK, emoji, etc.), the mapping from
/// byte offset to display column is not linear. This type provides the mapping.
#[derive(Debug, Clone)]
pub struct CharToColumnMapper {
    /// For each cell, the starting column position.
    column_offsets: Vec<i32>,
    /// The total number of display columns.
    total_columns: i32,
}

impl CharToColumnMapper {
    /// Creates a new mapper for a row with the given width.
    pub fn new(width: i32) -> Self {
        let mut offsets = Vec::with_capacity(width as usize);
        let mut current_col: i32 = 0;

        for _ in 0..width {
            offsets.push(current_col);
            current_col += 1;
        }

        Self {
            column_offsets: offsets,
            total_columns: width,
        }
    }

    /// Updates the mapper with cell widths for the row.
    pub fn update(&mut self, cell_widths: &[u8]) {
        self.column_offsets.clear();

        let mut current_col: i32 = 0;
        for &width in cell_widths {
            self.column_offsets.push(current_col);
            current_col += width as i32;
        }
        self.total_columns = current_col;
    }

    /// Returns the display column for a given cell index.
    pub fn get_column_for_cell(&self, cell_index: usize) -> i32 {
        if cell_index >= self.column_offsets.len() {
            return self.total_columns;
        }
        self.column_offsets[cell_index]
    }

    /// Returns the cell index for a given display column.
    ///
    /// Returns the cell index whose column range contains the given column,
    /// or the last cell index if the column is beyond all cells.
    pub fn get_cell_for_column(&self, column: i32) -> usize {
        if column <= 0 {
            return 0;
        }
        if column >= self.total_columns {
            return self.column_offsets.len().saturating_sub(1);
        }

        // Find the rightmost cell whose start column is less than the target
        match self.column_offsets.iter().rposition(|&c| c < column) {
            Some(idx) => idx,
            None => 0,
        }
    }

    /// Returns the number of display columns.
    pub const fn total_columns(&self) -> i32 {
        self.total_columns
    }

    /// Returns the number of cells.
    pub const fn cell_count(&self) -> usize {
        self.column_offsets.len()
    }
}

impl Default for CharToColumnMapper {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_mapper() {
        let mapper = CharToColumnMapper::new(10);
        assert_eq!(mapper.total_columns(), 10);
        assert_eq!(mapper.cell_count(), 10);
    }

    #[test]
    fn test_get_column_for_cell() {
        let mapper = CharToColumnMapper::new(5);
        // Default: each cell takes 1 column
        assert_eq!(mapper.get_column_for_cell(0), 0);
        assert_eq!(mapper.get_column_for_cell(1), 1);
        assert_eq!(mapper.get_column_for_cell(2), 2);
        assert_eq!(mapper.get_column_for_cell(4), 4);
    }

    #[test]
    fn test_get_cell_for_column() {
        let mapper = CharToColumnMapper::new(5);
        assert_eq!(mapper.get_cell_for_column(0), 0);
        assert_eq!(mapper.get_cell_for_column(1), 0);
        assert_eq!(mapper.get_cell_for_column(2), 1);
        assert_eq!(mapper.get_cell_for_column(4), 3);
        assert_eq!(mapper.get_cell_for_column(10), 4); // beyond end
    }

    #[test]
    fn test_update_with_wide_chars() {
        let mut mapper = CharToColumnMapper::new(5);
        // Simulate a row: [1, 2, 1, 2, 1] = ASCII, CJK, ASCII, CJK, ASCII
        mapper.update(&[1, 2, 1, 2, 1]);
        assert_eq!(mapper.total_columns(), 7);
        assert_eq!(mapper.get_column_for_cell(0), 0); // ASCII at col 0
        assert_eq!(mapper.get_column_for_cell(1), 1); // CJK at col 1
        assert_eq!(mapper.get_column_for_cell(2), 3); // ASCII at col 3
        assert_eq!(mapper.get_column_for_cell(3), 4); // CJK at col 4
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn test_get_cell_for_column_within_bounds(width in 1i32..100i32, col in 0i32..100i32) {
            let mapper = CharToColumnMapper::new(width);
            let cell = mapper.get_cell_for_column(col);
            prop_assert!(cell < mapper.cell_count() || col >= mapper.total_columns());
        }

        #[test]
        fn test_cell_count_equals_width(width in 1i32..100i32) {
            let mapper = CharToColumnMapper::new(width);
            prop_assert_eq!(mapper.cell_count(), width as usize);
        }

        #[test]
        fn test_column_for_cell_within_bounds(width in 1i32..100i32, cell_idx in 0usize..100usize) {
            let mapper = CharToColumnMapper::new(width);
            if cell_idx < mapper.cell_count() {
                let col = mapper.get_column_for_cell(cell_idx);
                prop_assert!(col < mapper.total_columns());
            }
        }
    }
}
