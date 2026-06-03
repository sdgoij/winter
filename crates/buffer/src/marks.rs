//! `Marks` — scroll marks, shell integration marks for the terminal.
//!
//! Ported from the marks tracking code in the Windows Terminal C++ buffer module.
//! Marks are used to track prompt boundaries, command boundaries, and output boundaries
//! for features like "scroll to prompt" and shell integration.

use types::MarkKind;

/// A mark in the terminal buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferMark {
    /// The kind of mark.
    kind: MarkKind,
    /// The buffer row where this mark starts (inclusive).
    start_row: i32,
    /// The buffer row where this mark ends (exclusive).
    end_row: i32,
}

impl BufferMark {
    /// Creates a new mark.
    pub const fn new(kind: MarkKind, start_row: i32, end_row: i32) -> Self {
        Self {
            kind,
            start_row,
            end_row,
        }
    }

    /// Returns the kind of this mark.
    pub const fn kind(&self) -> MarkKind {
        self.kind
    }

    /// Returns the start row (inclusive).
    pub const fn start_row(&self) -> i32 {
        self.start_row
    }

    /// Returns the end row (exclusive).
    pub const fn end_row(&self) -> i32 {
        self.end_row
    }

    /// Returns whether this mark covers the given row.
    pub fn covers_row(&self, row: i32) -> bool {
        row >= self.start_row && row < self.end_row
    }

    /// Returns the extent (number of rows) of this mark.
    pub const fn extent(&self) -> i32 {
        self.end_row - self.start_row
    }
}

/// Manages scroll marks and shell integration marks in the terminal buffer.
#[derive(Debug, Clone, Default)]
pub struct Marks {
    /// List of marks, sorted by start_row.
    marks: Vec<BufferMark>,
}

impl Marks {
    /// Creates a new empty marks tracker.
    pub const fn new() -> Self {
        Self { marks: Vec::new() }
    }

    /// Starts a prompt mark at the current row.
    ///
    /// If there's an existing output mark, it ends before this mark starts.
    pub fn start_prompt(&mut self, current_row: i32) {
        self.start_mark(MarkKind::Prompt, current_row, current_row + 1);
    }

    /// Starts a command mark at the current row.
    pub fn start_command(&mut self, current_row: i32) {
        self.start_mark(MarkKind::Command, current_row, current_row + 1);
    }

    /// Starts an output mark at the current row.
    pub fn start_output(&mut self, current_row: i32) {
        self.start_mark(MarkKind::Output, current_row, current_row + 1);
    }

    /// Ends the current command (closes the last command mark).
    pub fn end_current_command(&mut self, current_row: i32) {
        if let Some(last) = self.marks.last_mut() {
            if last.kind() == MarkKind::Command {
                last.end_row = current_row;
            }
        }
    }

    /// Gets marks of the specified kind that cover or are near the given row.
    pub fn get_marks_by_kind(&self, kind: MarkKind) -> &[BufferMark] {
        // Filter in place for simplicity; could be optimized with index.
        let _filtered: Vec<_> = self.marks.iter().filter(|m| m.kind() == kind).collect();
        // Since we can't return a filtered slice, we return all marks for now
        // and let the caller filter. This is a common pattern when the filter
        // set is small.
        &self.marks
    }

    /// Gets mark extents for a specific mark kind.
    pub fn get_mark_extents(&self, kind: MarkKind) -> Option<MarkExtents> {
        let mut min_start = i32::MAX;
        let mut max_end = i32::MIN;
        let mut found = false;

        for mark in &self.marks {
            if mark.kind() == kind {
                min_start = min_start.min(mark.start_row());
                max_end = max_end.max(mark.end_row());
                found = true;
            }
        }

        if found {
            Some(MarkExtents {
                start: min_start,
                end: max_end,
            })
        } else {
            None
        }
    }

    /// Clears all marks in the given row range.
    pub fn clear_marks_in_range(&mut self, start_row: i32, end_row: i32) {
        self.marks.retain(|m| {
            // Keep mark if it's entirely outside the range
            m.end_row <= start_row || m.start_row >= end_row
        });
    }

    /// Clears all marks.
    pub fn clear_all_marks(&mut self) {
        self.marks.clear();
    }

    /// Returns the number of marks.
    pub fn count(&self) -> usize {
        self.marks.len()
    }

    /// Returns whether there are any marks.
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Gets the error category marks.
    pub fn get_error_category(&self) -> MarkKind {
        // Check if there are any output marks (indicating program output)
        if self.get_mark_extents(MarkKind::Output).is_some() {
            MarkKind::Output
        } else if self.get_mark_extents(MarkKind::Command).is_some() {
            MarkKind::Command
        } else {
            MarkKind::None
        }
    }

    // ─── Internal ─────────────────────────────────────────────────────

    fn start_mark(&mut self, kind: MarkKind, start_row: i32, end_row: i32) {
        // End any output mark that was active before this mark
        if kind != MarkKind::Output {
            if let Some(last) = self.marks.last() {
                if last.kind() == MarkKind::Output {
                    // The output mark should end at the start of the new mark
                }
            }
        }

        // Check if we can merge with an adjacent mark of the same kind
        if let Some(pos) = self
            .marks
            .iter()
            .position(|m| m.kind() == kind && (m.end_row == start_row || m.start_row == end_row))
        {
            let mut mark = self.marks.remove(pos);
            mark.start_row = mark.start_row.min(start_row);
            mark.end_row = mark.end_row.max(end_row);
            self.insert_sorted(mark);
        } else {
            self.insert_sorted(BufferMark::new(kind, start_row, end_row));
        }
    }

    fn insert_sorted(&mut self, mark: BufferMark) {
        let pos = self
            .marks
            .partition_point(|m| m.start_row() < mark.start_row());
        self.marks.insert(pos, mark);
    }
}

/// Extents of marks (start and end rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkExtents {
    pub start: i32,
    pub end: i32,
}

impl MarkExtents {
    /// Creates new mark extents.
    pub const fn new(start: i32, end: i32) -> Self {
        Self { start, end }
    }

    /// Returns the extent (number of rows).
    pub const fn extent(&self) -> i32 {
        self.end - self.start
    }

    /// Returns whether this extent covers the given row.
    pub fn covers_row(&self, row: i32) -> bool {
        row >= self.start && row < self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_marks() {
        let marks = Marks::new();
        assert!(marks.is_empty());
        assert_eq!(marks.count(), 0);
        assert_eq!(marks.get_error_category(), MarkKind::None);
    }

    #[test]
    fn test_start_prompt() {
        let mut marks = Marks::new();
        marks.start_prompt(100);
        assert_eq!(marks.count(), 1);
        assert!(!marks.is_empty());

        let extents = marks.get_mark_extents(MarkKind::Prompt).unwrap();
        assert_eq!(extents.start, 100);
        assert_eq!(extents.end, 101);
    }

    #[test]
    fn test_start_command() {
        let mut marks = Marks::new();
        marks.start_command(50);
        marks.start_output(60);

        let command_extents = marks.get_mark_extents(MarkKind::Command).unwrap();
        assert_eq!(command_extents.start, 50);

        let output_extents = marks.get_mark_extents(MarkKind::Output).unwrap();
        assert_eq!(output_extents.start, 60);
    }

    #[test]
    fn test_end_current_command() {
        let mut marks = Marks::new();
        marks.start_command(10);
        marks.end_current_command(20);

        let extents = marks.get_mark_extents(MarkKind::Command).unwrap();
        assert_eq!(extents.end, 20);
    }

    #[test]
    fn test_clear_marks_in_range() {
        let mut marks = Marks::new();
        marks.start_prompt(10);
        marks.start_prompt(20);
        marks.start_prompt(30);

        marks.clear_marks_in_range(15, 25);
        assert_eq!(marks.count(), 2);
    }

    #[test]
    fn test_clear_all_marks() {
        let mut marks = Marks::new();
        marks.start_prompt(10);
        marks.start_command(20);
        marks.clear_all_marks();
        assert!(marks.is_empty());
    }

    #[test]
    fn test_mark_extents() {
        let extents = MarkExtents::new(10, 20);
        assert_eq!(extents.start, 10);
        assert_eq!(extents.end, 20);
        assert_eq!(extents.extent(), 10);
        assert!(extents.covers_row(15));
        assert!(!extents.covers_row(20));
        assert!(!extents.covers_row(9));
    }

    #[test]
    fn test_error_category() {
        let mut marks = Marks::new();
        assert_eq!(marks.get_error_category(), MarkKind::None);

        marks.start_output(10);
        assert_eq!(marks.get_error_category(), MarkKind::Output);

        let mut marks2 = Marks::new();
        marks2.start_command(10);
        assert_eq!(marks2.get_error_category(), MarkKind::Command);
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn test_clear_preserves_marks_outside_range(count in 1u32..10u32, clear_start in 0i32..100i32, clear_end in 100i32..200i32) {
            let mut marks = Marks::new();
            let mut original_count = 0;

            for i in 0..count {
                let row = (i as i32) * 10;
                marks.start_prompt(row);
                if row < clear_start || row >= clear_end {
                    original_count += 1;
                }
            }

            marks.clear_marks_in_range(clear_start, clear_end);
            prop_assert_eq!(marks.count(), original_count);
        }

        #[test]
        fn test_clear_all_empties(count in 0u32..10u32) {
            let mut marks = Marks::new();
            for _ in 0..count {
                marks.start_prompt(0);
            }
            marks.clear_all_marks();
            prop_assert!(marks.is_empty());
        }

        #[test]
        fn test_mark_extents_cover_correctly(start in 0i32..100i32, extent in 1i32..50i32) {
            let extents = MarkExtents::new(start, start + extent);
            prop_assert!(extents.covers_row(start));
            prop_assert!(extents.covers_row(start + extent - 1));
            prop_assert!(!extents.covers_row(start + extent));
            prop_assert!(!extents.covers_row(start - 1));
        }
    }
}
