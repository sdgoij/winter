//! `RenderFrameInfo` and the grid-line flags.
//!
//! Ported from the data types in `IRenderEngine.hpp`. The `RenderEngine` trait
//! itself lands with the engine; this module carries the values it passes
//! around each frame.

use utils::{Color, EnumSet, PointSpan};

/// A line the renderer can draw over a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GridLines {
    /// No line.
    None = 0,
    /// The top border.
    Top = 1,
    /// The bottom border.
    Bottom = 2,
    /// The left border.
    Left = 3,
    /// The right border.
    Right = 4,
    /// A single underline.
    Underline = 5,
    /// A double underline.
    DoubleUnderline = 6,
    /// A curly underline.
    CurlyUnderline = 7,
    /// A dotted underline.
    DottedUnderline = 8,
    /// A dashed underline.
    DashedUnderline = 9,
    /// A strikethrough.
    Strikethrough = 10,
    /// The underline shown under a hyperlink.
    HyperlinkUnderline = 11,
}

/// A set of [`GridLines`], ported from `til::enumset<GridLines>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GridLineSet(EnumSet);

impl GridLineSet {
    /// Creates an empty set.
    pub const fn new() -> Self {
        Self(EnumSet::new())
    }

    /// Adds one line to the set.
    pub fn insert(&mut self, line: GridLines) {
        self.0.set(line as u64);
    }

    /// Removes one line from the set.
    pub fn remove(&mut self, line: GridLines) {
        self.0.unset(line as u64);
    }

    /// Returns whether the set contains a line.
    pub const fn contains(&self, line: GridLines) -> bool {
        self.0.contains(line as u64)
    }

    /// Returns whether the set is empty.
    pub const fn is_empty(&self) -> bool {
        !self.0.is_any()
    }
}

/// Per-frame data the renderer needs beyond the buffer contents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderFrameInfo {
    /// Every search result to highlight.
    pub search_highlights: Vec<PointSpan>,
    /// The search result that currently has focus, if any.
    pub search_highlight_focused: Option<PointSpan>,
    /// The selection rectangles, as spans.
    pub selection_spans: Vec<PointSpan>,
    /// The colour to paint the selection with.
    pub selection_background: Color,
}

#[cfg(test)]
mod tests {
    use super::*;
    use utils::Point;

    #[test]
    fn grid_line_sets_are_independent() {
        let mut set = GridLineSet::new();
        assert!(set.is_empty());

        set.insert(GridLines::Underline);
        assert!(set.contains(GridLines::Underline));
        assert!(!set.contains(GridLines::DoubleUnderline));
        assert!(!set.is_empty());

        set.insert(GridLines::CurlyUnderline);
        assert!(set.contains(GridLines::Underline));
        assert!(set.contains(GridLines::CurlyUnderline));

        set.remove(GridLines::Underline);
        assert!(!set.contains(GridLines::Underline));
        assert!(set.contains(GridLines::CurlyUnderline));
    }

    #[test]
    fn every_grid_line_has_its_own_bit() {
        let mut set = GridLineSet::new();
        set.insert(GridLines::Top);
        set.insert(GridLines::Bottom);
        set.insert(GridLines::Left);
        set.insert(GridLines::Right);
        set.insert(GridLines::Strikethrough);
        set.insert(GridLines::HyperlinkUnderline);

        for line in [
            GridLines::Top,
            GridLines::Bottom,
            GridLines::Left,
            GridLines::Right,
            GridLines::Strikethrough,
            GridLines::HyperlinkUnderline,
        ] {
            assert!(set.contains(line), "{line:?}");
        }
        assert!(!set.contains(GridLines::Underline));
    }

    #[test]
    fn frame_info_defaults_to_empty() {
        let info = RenderFrameInfo::default();
        assert!(info.search_highlights.is_empty());
        assert!(info.search_highlight_focused.is_none());
        assert!(info.selection_spans.is_empty());
        assert_eq!(info.selection_background, Color::default());
    }

    #[test]
    fn frame_info_carries_spans_and_colour() {
        let span = PointSpan::new(Point::new(1, 2), Point::new(3, 4));
        let info = RenderFrameInfo {
            search_highlights: vec![span],
            search_highlight_focused: Some(span),
            selection_spans: vec![span],
            selection_background: Color::new(1, 2, 3, 255),
        };

        assert_eq!(info.search_highlights, vec![span]);
        assert_eq!(info.search_highlight_focused, Some(span));
        assert_eq!(info.selection_spans, vec![span]);
        assert_eq!(info.selection_background, Color::new(1, 2, 3, 255));
    }
}
