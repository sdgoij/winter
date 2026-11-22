//! `AtlasEngine` — the `RenderEngine` that drives an atlas backend.
//!
//! Ported from `AtlasEngine.cpp`. This slice implements the frame plumbing:
//! invalidation, the colour bitmap, background and line painting, cursor and
//! selection recording, and `present` handing a [`RenderingPayload`] to the
//! backend. Glyph rasterisation and atlas packing live in the backend.

use std::sync::atomic::{AtomicBool, Ordering};

use buffer::ImageSlice;
use types::{LineRendition, TextAttribute};
use utils::{InclusiveRect, Point, Rect, Size};

use crate::cluster::Cluster;
use crate::cursor_options::CursorOptions;
use crate::font_info::FontInfo;
use crate::font_info_desired::FontInfoDesired;
use crate::render_data::RenderData;
use crate::render_engine::{RenderEngine, RenderResult, StartPaint};
use crate::render_frame_info::{GridLineSet, RenderFrameInfo};
use crate::render_settings::RenderSettings;

use super::common::{Bitmap, FontId, FontSettings, GridLineRange, RenderingPayload, Settings, ShapedRow};
use super::shaping::{CellShaping, ShapingBackend};

/// Converts a `COLORREF` (`0x00BBGGRR`) to the bitmap's `0xAARRGGBB`.
const fn argb_from_colorref(color: u32) -> u32 {
    0xFF00_0000 | ((color & 0x0000_00FF) << 16) | (color & 0x0000_FF00) | ((color & 0x00FF_0000) >> 16)
}

/// Converts an RGBA [`utils::Color`] to the bitmap's `0xAARRGGBB`.
const fn argb_from_color(color: utils::Color) -> u32 {
    ((color.a as u32) << 24) | ((color.r as u32) << 16) | ((color.g as u32) << 8) | (color.b as u32)
}

/// The atlas render engine.
pub struct AtlasEngine {
    settings: Settings,
    payload: RenderingPayload,
    backend: Box<dyn super::common::IBackend>,
    shaping: Box<dyn ShapingBackend>,
    /// The resolved background and foreground colours for the current run,
    /// as `0xAARRGGBB`.
    current_colors: (u32, u32),
    rendition: LineRendition,
    dirty: Vec<Rect>,
    frame_info: RenderFrameInfo,
}

impl AtlasEngine {
    /// Creates an engine with default settings and a per-cell shaper.
    pub fn new(backend: Box<dyn super::common::IBackend>) -> Self {
        Self::with_settings(Settings::default(), backend)
    }

    /// Creates an engine with the given settings.
    pub fn with_settings(settings: Settings, backend: Box<dyn super::common::IBackend>) -> Self {
        let mut engine = Self {
            settings,
            payload: RenderingPayload::default(),
            backend,
            shaping: Box::new(CellShaping),
            current_colors: (0, 0),
            rendition: LineRendition::SingleWidth,
            dirty: Vec::new(),
            frame_info: RenderFrameInfo::default(),
        };
        engine.current_colors = (
            argb_from_colorref(engine.settings.misc.background_color),
            argb_from_colorref(engine.settings.misc.foreground_color),
        );
        engine
    }

    /// Replaces the shaping backend.
    pub fn set_shaping(&mut self, shaping: Box<dyn ShapingBackend>) {
        self.shaping = shaping;
    }

    /// Returns the last produced payload.
    pub const fn payload(&self) -> &RenderingPayload {
        &self.payload
    }

    /// Returns the settings.
    pub const fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Returns the settings mutably.
    pub const fn settings_mut(&mut self) -> &mut Settings {
        &mut self.settings
    }

    fn viewport_size(&self) -> (u16, u16) {
        self.settings.viewport_cell_count
    }

    fn ensure_bitmap(&mut self) {
        let (columns, rows) = self.viewport_size();
        let cells = usize::from(columns) * usize::from(rows);
        // The payload describes the frame to the back end, so it carries the
        // engine's settings: a back end that falls back to its own defaults
        // paints the wrong cell size and leaves the rest of the target blank.
        self.payload.settings = self.settings.clone();
        self.payload.color_bitmap_row_stride = usize::from(columns);
        self.payload.color_bitmap_depth_stride = cells;
        if self.payload.color_bitmap.len() != cells * 3 {
            self.payload.color_bitmap = vec![0; cells * 3];
        }
    }

    fn set_cell_colors(&mut self, column: u16, row: u16, background: u32, foreground: u32) {
        let stride = self.payload.color_bitmap_row_stride;
        let depth = self.payload.color_bitmap_depth_stride;
        let Some(index) = usize::from(row)
            .checked_mul(stride)
            .and_then(|base| base.checked_add(usize::from(column)))
        else {
            return;
        };
        if index >= depth {
            return;
        }
        self.payload.color_bitmap[index] = background;
        self.payload.color_bitmap[index + depth] = foreground;
    }

    fn screen_rect(&self) -> Rect {
        let (columns, rows) = self.viewport_size();
        Rect::new(0, 0, i32::from(columns), i32::from(rows))
    }

    fn mark_all_dirty(&mut self) {
        self.payload.mark_all_as_dirty();
        let screen = self.screen_rect();
        if screen.right() > 0 && screen.bottom() > 0 {
            self.dirty.push(screen);
        }
    }

    fn ensure_rows(&mut self) {
        let (_, rows) = self.viewport_size();
        if self.payload.rows.len() != usize::from(rows) {
            self.payload.rows = vec![ShapedRow::default(); usize::from(rows)];
        }
    }
}

impl RenderEngine for AtlasEngine {
    fn start_paint(&mut self) -> RenderResult<StartPaint> {
        Ok(StartPaint::Continue)
    }

    fn end_paint(&mut self) -> RenderResult<()> {
        Ok(())
    }

    fn requires_continuous_redraw(&mut self) -> bool {
        false
    }

    fn wait_until_can_render(&mut self, shutdown: &AtomicBool) -> bool {
        !shutdown.load(Ordering::Relaxed)
    }

    fn present(&mut self, shutdown: &AtomicBool) -> RenderResult<()> {
        if shutdown.load(Ordering::Relaxed) {
            return Ok(());
        }

        self.backend.render(&mut self.payload);
        self.payload.invalidated_rows = (0, 0);
        self.payload.dirty_rect_in_px = (0, 0, 0, 0);
        self.dirty.clear();
        Ok(())
    }

    fn scroll_frame(&mut self) -> RenderResult<()> {
        Ok(())
    }

    fn invalidate(&mut self, region: Option<&Rect>) -> RenderResult<()> {
        match region {
            Some(region) if region.right() > region.left() && region.bottom() > region.top() => {
                self.dirty.push(*region);
            }
            Some(_) => {}
            None => self.mark_all_dirty(),
        }
        Ok(())
    }

    fn invalidate_cursor(&mut self, region: Option<&Rect>) -> RenderResult<()> {
        self.invalidate(region)
    }

    fn invalidate_system(&mut self, dirty: Option<&Rect>) -> RenderResult<()> {
        self.invalidate(dirty)
    }

    fn invalidate_selection(&mut self, selections: &[Rect]) -> RenderResult<()> {
        for selection in selections {
            if selection.right() > selection.left() && selection.bottom() > selection.top() {
                self.dirty.push(*selection);
            }
        }
        Ok(())
    }

    fn invalidate_highlight(
        &mut self,
        _highlights: &[utils::PointSpan],
        _buffer: &buffer::TextBuffer,
    ) -> RenderResult<()> {
        Ok(())
    }

    fn invalidate_scroll(&mut self, _delta: Point) -> RenderResult<()> {
        Ok(())
    }

    fn invalidate_all(&mut self) -> RenderResult<()> {
        self.mark_all_dirty();
        Ok(())
    }

    fn invalidate_title(&mut self, _proposed: &str) -> RenderResult<()> {
        Ok(())
    }

    fn notify_new_text(&mut self, _new_text: &str) -> RenderResult<()> {
        Ok(())
    }

    fn prepare_render_info(&mut self, info: RenderFrameInfo) -> RenderResult<()> {
        self.frame_info = info;
        Ok(())
    }

    fn reset_line_transform(&mut self) -> RenderResult<()> {
        self.rendition = LineRendition::SingleWidth;
        Ok(())
    }

    fn prepare_line_transform(
        &mut self,
        rendition: LineRendition,
        _target_row: i32,
        _viewport_left: i32,
    ) -> RenderResult<()> {
        self.rendition = rendition;
        Ok(())
    }

    fn paint_background(&mut self) -> RenderResult<()> {
        self.ensure_bitmap();
        let background = argb_from_colorref(self.settings.misc.background_color);
        let depth = self.payload.color_bitmap_depth_stride;
        for cell in &mut self.payload.color_bitmap[..depth] {
            *cell = background;
        }
        Ok(())
    }

    fn paint_buffer_line(&mut self, clusters: &[Cluster<'_>], coord: Point, _trim_left: bool) -> RenderResult<()> {
        self.ensure_rows();
        let row = usize::try_from(coord.y()).unwrap_or(usize::MAX);
        if row >= self.payload.rows.len() {
            return Ok(());
        }

        let (background, foreground) = self.current_colors;
        let mut column = u16::try_from(coord.x()).unwrap_or(0);

        for cluster in clusters {
            let glyphs = self.shaping.shape(cluster.text(), FontId(0));
            self.payload.rows[row].push_glyphs(FontId(0), &glyphs, column);
            self.payload.rows[row].line_rendition = self.rendition;

            for _ in 0..cluster.columns().max(0) {
                self.set_cell_colors(column, row as u16, background, foreground);
                column = column.saturating_add(1);
            }
        }

        Ok(())
    }

    fn paint_buffer_grid_lines(
        &mut self,
        lines: GridLineSet,
        _gridline_color: u32,
        _underline_color: u32,
        length: usize,
        target: Point,
    ) -> RenderResult<()> {
        self.ensure_rows();
        let row = usize::try_from(target.y()).unwrap_or(usize::MAX);
        if row >= self.payload.rows.len() {
            return Ok(());
        }

        let from = u16::try_from(target.x()).unwrap_or(0);
        let to = from.saturating_add(u16::try_from(length).unwrap_or(u16::MAX));
        self.payload.rows[row]
            .grid_line_ranges
            .push(GridLineRange { lines, from, to });
        Ok(())
    }

    fn paint_image_slice(&mut self, image: &ImageSlice, target_row: i32, viewport_left: i32) -> RenderResult<()> {
        self.ensure_rows();
        let row = usize::try_from(target_row).unwrap_or(usize::MAX);
        if row >= self.payload.rows.len() {
            return Ok(());
        }

        let width = image.pixel_width();
        let height = if width > 0 {
            i32::try_from(image.pixels().len()).unwrap_or(0) / width
        } else {
            0
        };
        self.payload.rows[row].bitmap = Bitmap {
            revision: image.revision(),
            source: image.pixels().to_vec(),
            source_size: (width, height),
            target_offset: image.column_begin() - viewport_left,
            target_width: image.column_end() - image.column_begin(),
            active: true,
        };
        Ok(())
    }

    fn paint_selection(&mut self, rect: Rect) -> RenderResult<()> {
        let background = argb_from_color(self.frame_info.selection_background);
        let foreground = argb_from_colorref(self.settings.misc.selection_foreground);

        for row in rect.top().max(0)..rect.bottom() {
            for column in rect.left().max(0)..rect.right() {
                if let (Ok(column), Ok(row)) = (u16::try_from(column), u16::try_from(row)) {
                    self.set_cell_colors(column, row, background, foreground);
                }
            }
        }

        Ok(())
    }

    fn paint_cursor(&mut self, options: &CursorOptions) -> RenderResult<()> {
        let x = options.coord_cursor.x();
        let y = options.coord_cursor.y();
        let width = if options.is_double_width { 2 } else { 1 };
        self.payload.cursor_rect = (x, y, x + width, y + 1);
        Ok(())
    }

    fn update_drawing_brushes(
        &mut self,
        attr: &TextAttribute,
        settings: &RenderSettings,
        _data: &dyn RenderData,
        _using_soft_font: bool,
        _setting_default_brushes: bool,
    ) -> RenderResult<()> {
        let (foreground, background) = settings.get_attribute_colors(attr);
        self.current_colors = (argb_from_colorref(background), argb_from_colorref(foreground));
        Ok(())
    }

    fn update_font(&mut self, desired: &FontInfoDesired, info: &mut FontInfo) -> RenderResult<()> {
        self.settings.font = FontSettings {
            font_size: desired.font_size_in_pt(),
            cell_size: (
                u16::try_from(info.cell_size_in_physical_px().width()).unwrap_or(0),
                u16::try_from(info.cell_size_in_physical_px().height()).unwrap_or(0),
            ),
            advance_width: u16::try_from(info.cell_size_in_physical_px().width()).unwrap_or(0),
            builtin_glyphs: desired.enable_builtin_glyphs(),
            color_glyphs: desired.enable_color_glyphs(),
            ..FontSettings::default()
        };
        Ok(())
    }

    fn update_soft_font(&mut self, bit_pattern: &[u16], cell_size: Size, _centering_hint: usize) -> RenderResult<()> {
        self.settings.font.soft_font_pattern = bit_pattern.to_vec();
        self.settings.font.soft_font_cell_size = (cell_size.width(), cell_size.height());
        Ok(())
    }

    fn update_dpi(&mut self, dpi: i32) -> RenderResult<()> {
        self.settings.font.dpi = u16::try_from(dpi).unwrap_or(96);
        Ok(())
    }

    fn update_viewport(&mut self, viewport: InclusiveRect) -> RenderResult<()> {
        self.settings.viewport_cell_count = (
            u16::try_from(viewport.width()).unwrap_or(0),
            u16::try_from(viewport.height()).unwrap_or(0),
        );
        self.ensure_rows();
        self.ensure_bitmap();
        self.mark_all_dirty();
        Ok(())
    }

    fn update_target_size(&mut self, target_size: (u16, u16)) -> RenderResult<()> {
        if self.settings.target_size != target_size {
            self.settings.target_size = target_size;
            self.ensure_bitmap();
            self.mark_all_dirty();
        }
        Ok(())
    }

    fn update_default_colors(&mut self, background: u32, foreground: u32) -> RenderResult<()> {
        if self.settings.misc.background_color != background || self.settings.misc.foreground_color != foreground {
            self.settings.misc.background_color = background;
            self.settings.misc.foreground_color = foreground;
            self.current_colors = (argb_from_colorref(background), argb_from_colorref(foreground));
            self.mark_all_dirty();
        }
        Ok(())
    }

    fn get_proposed_font(&mut self, _desired: &FontInfoDesired, _info: &mut FontInfo, _dpi: i32) -> RenderResult<()> {
        Ok(())
    }

    fn dirty_area(&self) -> &[Rect] {
        &self.dirty
    }

    fn font_size(&mut self) -> Size {
        Size::new(
            i32::from(self.settings.font.cell_size.0),
            i32::from(self.settings.font.cell_size.1),
        )
    }

    fn is_glyph_wide_by_font(&self, _glyph: &str) -> bool {
        false
    }

    fn update_title(&mut self, _new_title: &str) -> RenderResult<()> {
        Ok(())
    }

    fn update_hyperlink_hovered_id(&mut self, _hovered_id: u16) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::common::MiscellaneousSettings;
    use crate::render_frame_info::GridLines;
    use std::cell::RefCell;
    use std::rc::Rc;
    use utils::Color;

    #[derive(Default)]
    struct Log {
        renders: usize,
        released: bool,
        last_rows: usize,
    }

    struct RecordingBackend {
        log: Rc<RefCell<Log>>,
    }

    impl super::super::common::IBackend for RecordingBackend {
        fn release_resources(&mut self) {
            self.log.borrow_mut().released = true;
        }
        fn render(&mut self, payload: &mut RenderingPayload) {
            let mut log = self.log.borrow_mut();
            log.renders += 1;
            log.last_rows = payload.rows.len();
        }
        fn requires_continuous_redraw(&mut self) -> bool {
            false
        }
    }

    fn engine() -> (AtlasEngine, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log::default()));
        let backend = RecordingBackend { log: Rc::clone(&log) };

        let settings = Settings {
            viewport_cell_count: (4, 1),
            misc: MiscellaneousSettings {
                background_color: 0x0011_2233,
                foreground_color: 0x0044_5566,
                ..MiscellaneousSettings::default()
            },
            ..Settings::default()
        };

        let mut engine = AtlasEngine::with_settings(settings, Box::new(backend));
        engine
            .update_viewport(InclusiveRect::new(0, 0, 3, 0))
            .expect("viewport");
        (engine, log)
    }

    #[test]
    fn new_default_colours_repaint_the_background_layer() {
        let (mut engine, _log) = engine();

        engine
            .update_default_colors(0x0000_1234, 0x00AB_CDEF)
            .expect("default colours");
        engine.paint_background().expect("background");

        // COLORREF 0x001234 is R=0x34, G=0x12, B=0x00 -> 0xFF341200.
        let depth = engine.payload().color_bitmap_depth_stride;
        assert!(
            engine.payload().color_bitmap[..depth]
                .iter()
                .all(|cell| *cell == 0xFF34_1200),
            "the background layer kept the old colour"
        );
    }

    #[test]
    fn the_payload_carries_the_cell_size_the_engine_was_given() {
        let settings = Settings {
            font: FontSettings {
                cell_size: (9, 20),
                baseline: 16,
                ..FontSettings::default()
            },
            viewport_cell_count: (4, 1),
            target_size: (36, 20),
            ..Settings::default()
        };
        let mut engine = AtlasEngine::with_settings(
            settings,
            Box::new(RecordingBackend {
                log: Rc::new(RefCell::new(Log::default())),
            }),
        );

        engine.paint_background().expect("background");

        assert_eq!(engine.payload().settings.font.cell_size, (9, 20));
        assert_eq!(engine.payload().settings.font.baseline, 16);
        assert_eq!(engine.payload().settings.target_size, (36, 20));
    }

    #[test]
    fn a_target_resize_reaches_the_payload_and_dirties_everything() {
        let (mut engine, _log) = engine();
        engine
            .present(&std::sync::atomic::AtomicBool::new(false))
            .expect("present");
        assert!(engine.dirty_area().is_empty(), "the frame cleared the dirty area");

        engine.update_target_size((64, 32)).expect("target size");

        assert_eq!(engine.settings().target_size, (64, 32));
        assert_eq!(engine.payload().settings.target_size, (64, 32));
        assert_eq!(engine.payload().dirty_rect_in_px, (0, 0, 64, 32));
    }

    #[test]
    fn the_background_fills_the_background_layer() {
        let (mut engine, _log) = engine();
        engine.paint_background().expect("background");

        let depth = engine.payload().color_bitmap_depth_stride;
        assert!(depth > 0);
        // COLORREF 0x00112233 is R=0x33, G=0x22, B=0x11 -> 0xFF332211.
        assert!(
            engine.payload().color_bitmap[..depth]
                .iter()
                .all(|cell| *cell == 0xFF33_2211)
        );
    }

    #[test]
    fn a_line_of_text_fills_the_rows_and_cell_colours() {
        let (mut engine, _log) = engine();
        engine.paint_background().expect("background");
        engine
            .paint_buffer_line(&[Cluster::new("AB", 2)], Point::new(0, 0), false)
            .expect("line");

        let payload = engine.payload();
        assert_eq!(payload.rows.len(), 1);
        assert_eq!(payload.rows[0].glyph_count(), 2);

        let depth = payload.color_bitmap_depth_stride;
        assert_eq!(payload.color_bitmap[0], 0xFF33_2211);
        assert_eq!(payload.color_bitmap[1], 0xFF33_2211);
        assert_eq!(payload.color_bitmap[depth], 0xFF66_5544);
        assert_eq!(payload.color_bitmap[depth + 1], 0xFF66_5544);
    }

    #[test]
    fn grid_lines_are_recorded_on_the_row() {
        let (mut engine, _log) = engine();
        let mut lines = GridLineSet::new();
        lines.insert(GridLines::Underline);
        engine
            .paint_buffer_grid_lines(lines, 0, 0, 2, Point::new(0, 0))
            .expect("grid lines");

        let ranges = &engine.payload().rows[0].grid_line_ranges;
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].from, 0);
        assert_eq!(ranges[0].to, 2);
        assert!(ranges[0].lines.contains(GridLines::Underline));
    }

    #[test]
    fn present_hands_the_payload_to_the_backend_and_clears_dirty() {
        let (mut engine, log) = engine();
        engine.invalidate_all().expect("invalidate");
        engine.present(&AtomicBool::new(false)).expect("present");

        assert_eq!(log.borrow().renders, 1);
        assert_eq!(log.borrow().last_rows, 1);
        assert!(engine.dirty_area().is_empty());
    }

    #[test]
    fn a_signalled_shutdown_skips_present() {
        let (mut engine, log) = engine();
        engine.present(&AtomicBool::new(true)).expect("present");
        assert_eq!(log.borrow().renders, 0);
    }

    #[test]
    fn a_cursor_is_recorded_as_a_cell_rect() {
        let (mut engine, _log) = engine();
        let options = CursorOptions {
            coord_cursor: Point::new(2, 0),
            is_visible: true,
            in_viewport: true,
            ..CursorOptions::default()
        };
        engine.paint_cursor(&options).expect("cursor");
        assert_eq!(engine.payload().cursor_rect, (2, 0, 3, 1));
    }

    #[test]
    fn a_selection_paints_the_selection_colours() {
        let (mut engine, _log) = engine();
        engine.paint_background().expect("background");
        let info = RenderFrameInfo {
            selection_background: Color::new(0x10, 0x20, 0x30, 0xFF),
            ..RenderFrameInfo::default()
        };
        engine.prepare_render_info(info).expect("info");
        engine.paint_selection(Rect::new(0, 0, 1, 1)).expect("selection");

        let depth = engine.payload().color_bitmap_depth_stride;
        assert_eq!(engine.payload().color_bitmap[0], 0xFF10_2030);
        assert_eq!(engine.payload().color_bitmap[depth], 0xFF00_0000);
    }
}
