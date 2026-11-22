//! `RenderEngine` — the surface a graphics back end implements.
//!
//! Ported from `IRenderEngine.hpp` and `RenderEngineBase.hpp/.cpp`. The
//! Windows-typed parts are adapted: `HRESULT` becomes [`RenderResult`],
//! `S_FALSE` becomes `Ok(false)` or [`StartPaint::Skip`], a `HANDLE` shutdown
//! event becomes an [`AtomicBool`], and the `til` rectangles keep their
//! exclusive (`Rect`) or inclusive (`InclusiveRect`) meaning.

use std::fmt;
use std::sync::atomic::AtomicBool;

use buffer::{ImageSlice, TextBuffer};
use types::{LineRendition, TextAttribute};
use utils::{InclusiveRect, Point, PointSpan, Rect, Size};

use crate::cluster::Cluster;
use crate::cursor_options::CursorOptions;
use crate::font_info::FontInfo;
use crate::font_info_desired::FontInfoDesired;
use crate::render_data::RenderData;
use crate::render_frame_info::{GridLineSet, RenderFrameInfo};
use crate::render_settings::RenderSettings;

/// A render back end failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError {
    message: String,
}

impl RenderError {
    /// Creates an error with a human-readable message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RenderError {}

/// The result of a render operation.
pub type RenderResult<T> = std::result::Result<T, RenderError>;

/// Whether [`RenderEngine::start_paint`] produced a frame to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartPaint {
    /// Draw and present the frame.
    Continue,
    /// Nothing to draw; skip the rest of the frame.
    Skip,
}

/// A graphics back end.
///
/// Every method mirrors a virtual on upstream's `IRenderEngine`; a back end
/// that cannot support one returns `Ok(false)` or a no-op rather than
/// pretending it succeeded.
pub trait RenderEngine {
    /// Begins a frame.
    fn start_paint(&mut self) -> RenderResult<StartPaint>;

    /// Ends a frame.
    fn end_paint(&mut self) -> RenderResult<()>;

    /// Returns whether the engine must keep painting without new input.
    fn requires_continuous_redraw(&mut self) -> bool;

    /// Waits until the engine can render; returns false when shutting down.
    fn wait_until_can_render(&mut self, shutdown: &AtomicBool) -> bool;

    /// Presents the finished frame.
    fn present(&mut self, shutdown: &AtomicBool) -> RenderResult<()>;

    /// Scrolls the frame instead of repainting it.
    fn scroll_frame(&mut self) -> RenderResult<()>;

    /// Invalidates a region, or the whole surface when `None`.
    fn invalidate(&mut self, region: Option<&Rect>) -> RenderResult<()>;

    /// Invalidates the cursor.
    fn invalidate_cursor(&mut self, region: Option<&Rect>) -> RenderResult<()>;

    /// Invalidates a system-wide region.
    fn invalidate_system(&mut self, dirty: Option<&Rect>) -> RenderResult<()>;

    /// Invalidates the selection rectangles.
    fn invalidate_selection(&mut self, selections: &[Rect]) -> RenderResult<()>;

    /// Invalidates the search highlights.
    fn invalidate_highlight(&mut self, highlights: &[PointSpan], buffer: &TextBuffer) -> RenderResult<()>;

    /// Invalidates after a scroll by `delta`.
    fn invalidate_scroll(&mut self, delta: Point) -> RenderResult<()>;

    /// Invalidates everything.
    fn invalidate_all(&mut self) -> RenderResult<()>;

    /// Records a proposed title for the next frame.
    fn invalidate_title(&mut self, proposed: &str) -> RenderResult<()>;

    /// Notifies the engine of new text.
    fn notify_new_text(&mut self, new_text: &str) -> RenderResult<()>;

    /// Supplies the per-frame render info.
    fn prepare_render_info(&mut self, info: RenderFrameInfo) -> RenderResult<()>;

    /// Resets any line transform.
    fn reset_line_transform(&mut self) -> RenderResult<()>;

    /// Sets up the transform for a line rendition.
    fn prepare_line_transform(
        &mut self,
        rendition: LineRendition,
        target_row: i32,
        viewport_left: i32,
    ) -> RenderResult<()>;

    /// Paints the background.
    fn paint_background(&mut self) -> RenderResult<()>;

    /// Paints a run of clusters.
    fn paint_buffer_line(&mut self, clusters: &[Cluster<'_>], coord: Point, trim_left: bool) -> RenderResult<()>;

    /// Paints the grid lines for a run.
    fn paint_buffer_grid_lines(
        &mut self,
        lines: GridLineSet,
        gridline_color: u32,
        underline_color: u32,
        length: usize,
        target: Point,
    ) -> RenderResult<()>;

    /// Paints an image slice.
    fn paint_image_slice(&mut self, image: &ImageSlice, target_row: i32, viewport_left: i32) -> RenderResult<()>;

    /// Paints the selection.
    fn paint_selection(&mut self, rect: Rect) -> RenderResult<()>;

    /// Paints the cursor.
    fn paint_cursor(&mut self, options: &CursorOptions) -> RenderResult<()>;

    /// Selects the brushes for an attribute.
    fn update_drawing_brushes(
        &mut self,
        attr: &TextAttribute,
        settings: &RenderSettings,
        data: &dyn RenderData,
        using_soft_font: bool,
        setting_default_brushes: bool,
    ) -> RenderResult<()>;

    /// Loads a font and reports the metrics it produced.
    fn update_font(&mut self, desired: &FontInfoDesired, info: &mut FontInfo) -> RenderResult<()>;

    /// Updates the DECDLD soft font.
    fn update_soft_font(&mut self, bit_pattern: &[u16], cell_size: Size, centering_hint: usize) -> RenderResult<()>;

    /// Changes the DPI.
    fn update_dpi(&mut self, dpi: i32) -> RenderResult<()>;

    /// Changes the viewport.
    fn update_viewport(&mut self, viewport: InclusiveRect) -> RenderResult<()>;

    /// Changes the pixel size of the target.
    ///
    /// The default is a no-op for a back end whose target is fixed. The atlas
    /// engine caches the size, so a shell whose window can be resized must call
    /// this whenever the target changes.
    fn update_target_size(&mut self, _target_size: (u16, u16)) -> RenderResult<()> {
        Ok(())
    }

    /// Measures the font a desired font would produce without loading it.
    fn get_proposed_font(&mut self, desired: &FontInfoDesired, info: &mut FontInfo, dpi: i32) -> RenderResult<()>;

    /// Returns the regions that need repainting.
    fn dirty_area(&self) -> &[Rect];

    /// Returns the current cell size.
    fn font_size(&mut self) -> Size;

    /// Returns whether a glyph is wide by font metrics.
    fn is_glyph_wide_by_font(&self, glyph: &str) -> bool;

    /// Applies a title change.
    fn update_title(&mut self, new_title: &str) -> RenderResult<()>;

    /// Records which hyperlink is under the pointer.
    fn update_hyperlink_hovered_id(&mut self, hovered_id: u16);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use utils::Point;

    #[derive(Default)]
    struct MockEngine {
        dirty: Vec<Rect>,
        painted_clusters: usize,
        last_coord: Option<Point>,
    }

    impl RenderEngine for MockEngine {
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
        fn present(&mut self, _shutdown: &AtomicBool) -> RenderResult<()> {
            Ok(())
        }
        fn scroll_frame(&mut self) -> RenderResult<()> {
            Ok(())
        }
        fn invalidate(&mut self, _region: Option<&Rect>) -> RenderResult<()> {
            Ok(())
        }
        fn invalidate_cursor(&mut self, _region: Option<&Rect>) -> RenderResult<()> {
            Ok(())
        }
        fn invalidate_system(&mut self, _dirty: Option<&Rect>) -> RenderResult<()> {
            Ok(())
        }
        fn invalidate_selection(&mut self, _selections: &[Rect]) -> RenderResult<()> {
            Ok(())
        }
        fn invalidate_highlight(&mut self, _highlights: &[PointSpan], _buffer: &TextBuffer) -> RenderResult<()> {
            Ok(())
        }
        fn invalidate_scroll(&mut self, _delta: Point) -> RenderResult<()> {
            Ok(())
        }
        fn invalidate_all(&mut self) -> RenderResult<()> {
            Ok(())
        }
        fn invalidate_title(&mut self, _proposed: &str) -> RenderResult<()> {
            Ok(())
        }
        fn notify_new_text(&mut self, _new_text: &str) -> RenderResult<()> {
            Ok(())
        }
        fn prepare_render_info(&mut self, _info: RenderFrameInfo) -> RenderResult<()> {
            Ok(())
        }
        fn reset_line_transform(&mut self) -> RenderResult<()> {
            Ok(())
        }
        fn prepare_line_transform(
            &mut self,
            _rendition: LineRendition,
            _target_row: i32,
            _viewport_left: i32,
        ) -> RenderResult<()> {
            Ok(())
        }
        fn paint_background(&mut self) -> RenderResult<()> {
            Ok(())
        }
        fn paint_buffer_line(&mut self, clusters: &[Cluster<'_>], coord: Point, _trim_left: bool) -> RenderResult<()> {
            self.painted_clusters = clusters.len();
            self.last_coord = Some(coord);
            Ok(())
        }
        fn paint_buffer_grid_lines(
            &mut self,
            _lines: GridLineSet,
            _gridline_color: u32,
            _underline_color: u32,
            _length: usize,
            _target: Point,
        ) -> RenderResult<()> {
            Ok(())
        }
        fn paint_image_slice(
            &mut self,
            _image: &ImageSlice,
            _target_row: i32,
            _viewport_left: i32,
        ) -> RenderResult<()> {
            Ok(())
        }
        fn paint_selection(&mut self, _rect: Rect) -> RenderResult<()> {
            Ok(())
        }
        fn paint_cursor(&mut self, _options: &CursorOptions) -> RenderResult<()> {
            Ok(())
        }
        fn update_drawing_brushes(
            &mut self,
            _attr: &TextAttribute,
            _settings: &RenderSettings,
            _data: &dyn RenderData,
            _using_soft_font: bool,
            _setting_default_brushes: bool,
        ) -> RenderResult<()> {
            Ok(())
        }
        fn update_font(&mut self, _desired: &FontInfoDesired, _info: &mut FontInfo) -> RenderResult<()> {
            Ok(())
        }
        fn update_soft_font(
            &mut self,
            _bit_pattern: &[u16],
            _cell_size: Size,
            _centering_hint: usize,
        ) -> RenderResult<()> {
            Ok(())
        }
        fn update_dpi(&mut self, _dpi: i32) -> RenderResult<()> {
            Ok(())
        }
        fn update_viewport(&mut self, _viewport: InclusiveRect) -> RenderResult<()> {
            Ok(())
        }
        fn get_proposed_font(
            &mut self,
            _desired: &FontInfoDesired,
            _info: &mut FontInfo,
            _dpi: i32,
        ) -> RenderResult<()> {
            Ok(())
        }
        fn dirty_area(&self) -> &[Rect] {
            &self.dirty
        }
        fn font_size(&mut self) -> Size {
            Size::new(8, 16)
        }
        fn is_glyph_wide_by_font(&self, _glyph: &str) -> bool {
            false
        }
        fn update_title(&mut self, _new_title: &str) -> RenderResult<()> {
            Ok(())
        }
        fn update_hyperlink_hovered_id(&mut self, _hovered_id: u16) {}
    }

    #[test]
    fn the_trait_is_object_safe_and_dispatches() {
        let mut engine: Box<dyn RenderEngine> = Box::new(MockEngine::default());

        assert_eq!(engine.start_paint().expect("start"), StartPaint::Continue);
        engine
            .paint_buffer_line(&[Cluster::new("A", 1)], Point::new(2, 3), false)
            .expect("paint");
        assert_eq!(engine.font_size(), Size::new(8, 16));
        engine.end_paint().expect("end");
        engine.present(&AtomicBool::new(false)).expect("present");
    }

    #[test]
    fn a_frame_walks_from_start_to_present() {
        let mut engine = MockEngine::default();
        let shutdown = AtomicBool::new(false);

        let action = engine.start_paint().expect("start");
        assert_eq!(action, StartPaint::Continue);
        engine.paint_background().expect("background");
        engine
            .paint_buffer_line(&[Cluster::new("A", 1), Cluster::new("B", 1)], Point::new(0, 0), false)
            .expect("line");
        engine.paint_cursor(&CursorOptions::default()).expect("cursor");
        engine.present(&shutdown).expect("present");

        assert_eq!(engine.painted_clusters, 2);
        assert_eq!(engine.last_coord, Some(Point::new(0, 0)));
    }
}
