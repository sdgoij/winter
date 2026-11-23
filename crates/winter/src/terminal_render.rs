//! The live terminal as a renderer's data.
//!
//! `Renderer` reads a [`RenderData`]: a buffer, a viewport, the font metrics,
//! the cursor and the selection. The terminal's buffer lives behind a lock, so
//! this module keeps a screen-sized copy that a frame can borrow; `refresh`
//! fills it from the control once per frame (ADR 040).

use std::time::Duration;

use buffer::TextBuffer;
use renderer::{Composition, CursorOptions, FontInfo, RenderData, RenderSettings};
use utils::{InclusiveRect, Point, PointSpan};

use crate::find::FindBar;
use crate::grid;
use crate::term_control::Control;
use crate::view::View;

/// The state a renderer paints one frame from.
pub struct TerminalData {
    /// The visible rows, copied out of the terminal's locked buffer.
    buffer: TextBuffer,
    viewport: InclusiveRect,
    font: FontInfo,
    settings: RenderSettings,
    cursor: CursorOptions,
    selection: Vec<PointSpan>,
    highlights: Vec<PointSpan>,
    focused_highlight: Option<PointSpan>,
    block: bool,
    title: String,
    tsf: Composition,
    snippet: Composition,
    composition_origin: Option<Point>,
    /// How many rows the document holds, history included.
    document_height: i32,
    /// The prompt marks the scrollbar draws.
    marks: Vec<grid::MarkTick>,
}

impl TerminalData {
    /// Creates data for a pane of `columns` by `rows` cells.
    pub fn new(font: FontInfo, settings: RenderSettings, columns: i32, rows: i32) -> Self {
        Self {
            buffer: TextBuffer::with_dimensions(columns.max(1), rows.max(1), 0),
            viewport: InclusiveRect::new(0, 0, columns.max(1) - 1, rows.max(1) - 1),
            font,
            settings,
            cursor: CursorOptions::default(),
            selection: Vec::new(),
            highlights: Vec::new(),
            focused_highlight: None,
            block: false,
            title: String::new(),
            tsf: Composition::default(),
            snippet: Composition::default(),
            composition_origin: None,
            document_height: rows,
            marks: Vec::new(),
        }
    }

    /// Refreshes the copy, the cursor, the selection and the find highlights.
    pub(crate) fn refresh(&mut self, control: &mut Control, view: &View, cursor_on: bool, find: Option<&FindBar>) {
        let screen = control.snapshot(view);
        let (columns, rows) = (screen.width.max(1), screen.height.max(1));

        if self.buffer.screen_width() != columns || self.buffer.screen_height() != rows {
            self.buffer = TextBuffer::with_dimensions(columns, rows, 0);
            self.viewport = InclusiveRect::new(0, 0, columns - 1, rows - 1);
        }
        control.copy_visible_into(&mut self.buffer, view.scroll());

        // The shell's table is the one a scheme was applied to, so the renderer
        // resolves attributes against the same colours the pane paints.
        if let Ok(table) = control.color_table().lock() {
            self.settings.set_color_table(table.clone());
        }

        self.cursor.coord_cursor = Point::new(screen.cursor.0, screen.cursor.1);
        self.cursor.cursor_type = screen.cursor_shape;
        self.cursor.is_visible = screen.cursor_visible;
        self.cursor.is_on = cursor_on;
        self.cursor.in_viewport = screen.cursor_visible;

        let history = control.max_scroll();
        let scroll = view.scroll();
        self.block = view.selection().is_some_and(|selection| selection.block);
        self.selection.clear();
        if let Some(selection) = view.selection() {
            for row in 0..rows {
                let document = crate::input::screen_to_document(history, scroll, row);
                if let Some((start, end)) = selection.row_span(document) {
                    let start = start.clamp(0, columns - 1);
                    let end = end.clamp(0, columns - 1);
                    if start <= end {
                        self.selection
                            .push(PointSpan::new(Point::new(start, row), Point::new(end, row)));
                    }
                }
            }
        }

        self.title.clear();
        if let Some(title) = control.title() {
            self.title.push_str(title);
        }

        self.highlights.clear();
        self.focused_highlight = None;
        if let Some(find) = find.filter(|find| find.open && !find.matches.is_empty()) {
            let (spans, focused) =
                crate::find::highlight_spans(&find.matches, find.current, history, scroll, columns, rows);
            self.highlights = spans;
            self.focused_highlight = focused;
        }

        self.document_height = screen.document_height;
        self.marks = screen.marks;
    }

    /// How many rows the document holds, history included.
    pub const fn document_height(&self) -> i32 {
        self.document_height
    }

    /// The prompt marks the scrollbar draws.
    pub fn marks(&self) -> &[grid::MarkTick] {
        &self.marks
    }
}

impl RenderData for TerminalData {
    fn viewport(&self) -> InclusiveRect {
        self.viewport
    }

    fn text_buffer_end_position(&self) -> Point {
        Point::new(self.buffer.screen_width(), self.buffer.screen_height())
    }

    fn text_buffer(&self) -> &TextBuffer {
        &self.buffer
    }

    fn font_info(&self) -> &FontInfo {
        &self.font
    }

    fn search_highlights(&self) -> &[PointSpan] {
        &self.highlights
    }

    fn search_highlight_focused(&self) -> Option<PointSpan> {
        self.focused_highlight
    }

    fn selection_spans(&self) -> &[PointSpan] {
        &self.selection
    }

    fn lock_console(&mut self) {}

    fn unlock_console(&mut self) {}

    fn blink_interval(&mut self) -> Duration {
        Duration::ZERO
    }

    fn cursor_options(&self) -> CursorOptions {
        self.cursor
    }

    fn is_grid_line_drawing_allowed(&self) -> bool {
        true
    }

    fn console_title(&self) -> &str {
        &self.title
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

    fn attribute_colors(&self, attr: &types::TextAttribute) -> (u32, u32) {
        self.settings.get_attribute_colors(attr)
    }

    fn is_selection_active(&self) -> bool {
        !self.selection.is_empty()
    }

    fn is_block_selection(&self) -> bool {
        self.block
    }

    fn clear_selection(&mut self) {
        self.selection.clear();
    }

    fn select_new_region(&mut self, _start: Point, _end: Point) {}

    fn selection_anchor(&self) -> Point {
        self.selection.first().map_or(Point::new(0, 0), |span| span.start)
    }

    fn selection_end(&self) -> Point {
        self.selection.last().map_or(Point::new(0, 0), |span| span.end)
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

    fn composition_origin(&self) -> Option<Point> {
        self.composition_origin
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderer::renderer::Renderer;
    use renderer::{
        AtlasEngine, CellSizeInDIP, FontId, GlyphBitmap, GlyphRasterizer, Settings, SharedBackend, SoftwareBackend,
    };
    use utils::Size;

    use crate::term_connection::tests::Buffer;

    /// A rasteriser that paints the whole cell, so any glyph is visible.
    struct SolidRasterizer;

    impl GlyphRasterizer for SolidRasterizer {
        fn rasterize(&mut self, _font: FontId, _glyph: u16) -> Option<GlyphBitmap> {
            Some(GlyphBitmap {
                width: 8,
                height: 16,
                left: 0,
                top: 0,
                coverage: vec![255; 8 * 16],
            })
        }
    }

    fn font() -> FontInfo {
        let mut font = FontInfo::default();
        font.set_from_engine(
            "Test".to_string(),
            0,
            400,
            65001,
            16.0,
            CellSizeInDIP {
                width: 8.0,
                height: 16.0,
            },
            Size::new(8, 16),
        );
        font
    }

    fn atlas_settings(columns: u16, rows: u16) -> Settings {
        Settings {
            font: renderer::FontSettings {
                cell_size: (8, 16),
                font_size: 16.0,
                advance_width: 8,
                baseline: 12,
                descender: 4,
                thin_line_width: 1,
                ..renderer::FontSettings::default()
            },
            misc: renderer::MiscellaneousSettings {
                background_color: 0x00_0000,
                foreground_color: 0xFF_FFFF,
                ..renderer::MiscellaneousSettings::default()
            },
            target_size: (columns * 8, rows * 16),
            viewport_cell_count: (columns, rows),
            ..Settings::default()
        }
    }

    #[test]
    fn a_frame_of_a_control_reaches_the_shared_pixels() {
        let mut control = Control::new(Box::new(Buffer::with_output(b"hi")), (4, 2));
        control.pump();

        let mut data = TerminalData::new(font(), RenderSettings::new(), 4, 2);
        let view = View::new();
        data.refresh(&mut control, &view, true, None);

        let shared = SharedBackend::new(SoftwareBackend::with_rasterizer(Box::new(SolidRasterizer)));
        let mut renderer = Renderer::new(data, RenderSettings::new());
        renderer.add_engine(Box::new(AtlasEngine::with_settings(
            atlas_settings(4, 2),
            Box::new(shared.clone()),
        )));
        renderer.invalidate_all();
        renderer.paint_frame().expect("a frame");

        assert_eq!((shared.width(), shared.height()), (32, 32));
        shared.with_pixels(|pixels| {
            let distinct = pixels.iter().collect::<std::collections::HashSet<_>>().len();
            assert!(distinct >= 2, "the frame is one flat colour: {distinct}");
            let light = pixels
                .iter()
                .filter(|&&pixel| pixel & 0x00FF_FFFF > 0x0080_8080)
                .count();
            assert!(light > 0, "no foreground pixels were painted");
        });
    }

    #[test]
    fn an_image_band_reaches_the_renderers_buffer() {
        let mut control = Control::new(Box::new(Buffer::with_output(b"\x1bP0;1;0q#1;2;0;100;0~\x1b\\")), (4, 2));
        control.pump();

        let mut data = TerminalData::new(font(), RenderSettings::new(), 4, 2);
        data.refresh(&mut control, &View::new(), true, None);

        let image = data
            .buffer
            .get_row_at_screen_line(0)
            .expect("the first row")
            .image_slice()
            .expect("the snapshot keeps the image band");
        assert_eq!(image.pixels()[0], 0xFF00_FF00);
    }

    #[test]
    fn a_selection_crossing_history_is_in_screen_rows() {
        let mut control = Control::new(Box::new(Buffer::with_output(b"one\r\ntwo\r\nthree")), (6, 2));
        control.pump();

        let mut view = View::new();
        // The first row has scrolled into history; the view is scrolled back.
        let max = control.max_scroll();
        view.set_scroll(1, max);
        view.select((0, 0), false);
        view.extend((2, 0));

        let mut data = TerminalData::new(font(), RenderSettings::new(), 6, 2);
        data.refresh(&mut control, &view, true, None);

        assert!(!data.selection.is_empty(), "the selection reached the screen");
        assert!(data.selection.iter().all(|span| span.start.y < 2));
    }
}
