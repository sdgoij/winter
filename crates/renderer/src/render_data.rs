//! `RenderData` — what a render engine reads from the terminal.
//!
//! Ported from `IRenderData.hpp`. The host owns the buffer, the font info, the
//! viewport, the selection and the composition; the engine only reads them.
//! `std::chrono` durations become [`std::time::Duration`], and a string result
//! becomes an owned `String` because the trait cannot borrow from `&self` while
//! the caller also holds it mutably.

use std::time::Duration;

use buffer::TextBuffer;
use types::TextAttribute;
use utils::{InclusiveRect, Point, PointSpan};

use crate::composition::Composition;
use crate::cursor_options::CursorOptions;
use crate::font_info::FontInfo;

/// The terminal state a render engine draws.
pub trait RenderData {
    /// Returns the visible viewport.
    fn viewport(&self) -> InclusiveRect;

    /// Returns the exclusive end position of the text buffer.
    fn text_buffer_end_position(&self) -> Point;

    /// Returns the text buffer.
    fn text_buffer(&self) -> &TextBuffer;

    /// Returns the measured font info.
    fn font_info(&self) -> &FontInfo;

    /// Returns every search highlight.
    fn search_highlights(&self) -> &[PointSpan];

    /// Returns the focused search highlight, if any.
    fn search_highlight_focused(&self) -> Option<PointSpan>;

    /// Returns the selection as spans.
    fn selection_spans(&self) -> &[PointSpan];

    /// Locks the console so the buffer cannot change mid-frame.
    fn lock_console(&mut self);

    /// Releases the console lock.
    fn unlock_console(&mut self);

    /// Returns the blink interval; [`Duration::ZERO`] or [`Duration::MAX`]
    /// means the cursor does not blink.
    fn blink_interval(&mut self) -> Duration;

    /// Returns the cursor state to paint, if any.
    fn cursor_options(&self) -> CursorOptions;

    /// Returns whether grid lines may be drawn.
    fn is_grid_line_drawing_allowed(&self) -> bool;

    /// Returns the console title.
    fn console_title(&self) -> &str;

    /// Returns the URI registered for a hyperlink id.
    fn hyperlink_uri(&self, id: u16) -> String;

    /// Returns the custom id registered for a hyperlink id.
    fn hyperlink_custom_id(&self, id: u16) -> String;

    /// Returns the pattern ids that cover a cell.
    fn pattern_id(&self, location: Point) -> Vec<usize>;

    /// Returns the resolved foreground and background colours for an attribute.
    fn attribute_colors(&self, attr: &TextAttribute) -> (u32, u32);

    /// Returns whether a selection is active.
    fn is_selection_active(&self) -> bool;

    /// Returns whether the selection is a block selection.
    fn is_block_selection(&self) -> bool;

    /// Clears the selection.
    fn clear_selection(&mut self);

    /// Selects a new region between two points.
    fn select_new_region(&mut self, start: Point, end: Point);

    /// Returns the selection anchor.
    fn selection_anchor(&self) -> Point;

    /// Returns the selection end.
    fn selection_end(&self) -> Point;

    /// Returns whether the accessibility data is initialized.
    fn is_uia_data_initialized(&self) -> bool;

    /// Returns the IME composition preview.
    fn tsf_preview(&self) -> &Composition;

    /// Returns the snippet-preview composition.
    fn snippet_preview(&self) -> &Composition;

    /// Returns the composition to draw, preferring the snippet preview.
    fn active_composition(&self) -> &Composition {
        let snippet = self.snippet_preview();
        if snippet.text.is_empty() {
            self.tsf_preview()
        } else {
            snippet
        }
    }

    /// Returns the inclusive soft-font code-point range, if one is loaded.
    fn soft_font_range(&self) -> Option<(usize, usize)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_info::FontInfo;

    struct MockData {
        buffer: TextBuffer,
        font: FontInfo,
        tsf: Composition,
        snippet: Composition,
    }

    impl MockData {
        fn new() -> Self {
            Self {
                buffer: TextBuffer::new(),
                font: FontInfo::default(),
                tsf: Composition::default(),
                snippet: Composition::default(),
            }
        }
    }

    impl RenderData for MockData {
        fn viewport(&self) -> InclusiveRect {
            InclusiveRect::new(0, 0, 0, 0)
        }
        fn text_buffer_end_position(&self) -> Point {
            Point::new(0, 0)
        }
        fn text_buffer(&self) -> &TextBuffer {
            &self.buffer
        }
        fn font_info(&self) -> &FontInfo {
            &self.font
        }
        fn search_highlights(&self) -> &[PointSpan] {
            &[]
        }
        fn search_highlight_focused(&self) -> Option<PointSpan> {
            None
        }
        fn selection_spans(&self) -> &[PointSpan] {
            &[]
        }
        fn lock_console(&mut self) {}
        fn unlock_console(&mut self) {}
        fn blink_interval(&mut self) -> Duration {
            Duration::ZERO
        }
        fn cursor_options(&self) -> CursorOptions {
            CursorOptions::default()
        }
        fn is_grid_line_drawing_allowed(&self) -> bool {
            true
        }
        fn console_title(&self) -> &str {
            "winterm"
        }
        fn hyperlink_uri(&self, _id: u16) -> String {
            String::new()
        }
        fn hyperlink_custom_id(&self, _id: u16) -> String {
            String::new()
        }
        fn pattern_id(&self, _location: Point) -> Vec<usize> {
            Vec::new()
        }
        fn attribute_colors(&self, _attr: &TextAttribute) -> (u32, u32) {
            (0, 0)
        }
        fn is_selection_active(&self) -> bool {
            false
        }
        fn is_block_selection(&self) -> bool {
            false
        }
        fn clear_selection(&mut self) {}
        fn select_new_region(&mut self, _start: Point, _end: Point) {}
        fn selection_anchor(&self) -> Point {
            Point::new(0, 0)
        }
        fn selection_end(&self) -> Point {
            Point::new(0, 0)
        }
        fn is_uia_data_initialized(&self) -> bool {
            false
        }
        fn tsf_preview(&self) -> &Composition {
            &self.tsf
        }
        fn snippet_preview(&self) -> &Composition {
            &self.snippet
        }
    }

    #[test]
    fn an_empty_snippet_preview_falls_back_to_the_ime_preview() {
        let mut data = MockData::new();
        data.tsf.text = "tsf".to_string();
        data.tsf.cursor_pos = 3;

        assert_eq!(data.active_composition().text, "tsf");
        assert_eq!(data.active_composition().cursor_pos, 3);
    }

    #[test]
    fn a_non_empty_snippet_preview_wins() {
        let mut data = MockData::new();
        data.tsf.text = "tsf".to_string();
        data.snippet.text = "snippet".to_string();

        assert_eq!(data.active_composition().text, "snippet");
    }

    #[test]
    fn the_trait_is_object_safe() {
        let data: Box<dyn RenderData> = Box::new(MockData::new());
        assert_eq!(data.cursor_options(), CursorOptions::default());
        assert!(data.is_grid_line_drawing_allowed());
    }
}
