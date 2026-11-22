//! `Renderer` — the orchestration over the engine and render data.
//!
//! Ported from `renderer.hpp/.cpp`. Per ADR 006 the host owns the loop: this
//! type paints one frame when asked, keeps no thread, and leaves wakeups and
//! cancellation to the caller.
//!
//! The frame follows upstream's order: start, default brushes, scroll, frame
//! info, background, buffer text, selection, cursor, title, end, present.
//! Composition previews, search-hover dashes and the soft-font range are the
//! pieces still to be wired in the paint path.

use std::sync::atomic::AtomicBool;

use buffer::{OutputCell, TextBufferCellIterator};
use types::color_table::SELECTION_BACKGROUND;
use types::{TextAttribute, UnderlineStyle, codepoint_width};
use utils::{Color, InclusiveRect, Point, Rect};

use crate::cluster::Cluster;
use crate::composition::Composition;
use crate::render_data::RenderData;
use crate::render_engine::{RenderEngine, RenderResult, StartPaint};
use crate::render_frame_info::{GridLineSet, GridLines, RenderFrameInfo};
use crate::render_settings::RenderSettings;

/// The first code point reserved for DECDLD soft-font glyphs.
pub const FIRST_SOFT_FONT_CHAR: usize = 0xEF20;

/// Maps a text attribute's borders and underline onto grid lines.
///
/// Ported from `Renderer::s_GetGridlines`. An underline style outside the enum
/// cannot reach here: `TextAttribute::get_underline_style` already maps it to
/// `NoUnderline`.
pub fn get_gridlines(attribute: &TextAttribute) -> GridLineSet {
    let mut lines = GridLineSet::new();

    if attribute.is_top_horizontal_displayed() {
        lines.insert(GridLines::Top);
    }
    if attribute.is_bottom_horizontal_displayed() {
        lines.insert(GridLines::Bottom);
    }
    if attribute.is_left_vertical_displayed() {
        lines.insert(GridLines::Left);
    }
    if attribute.is_right_vertical_displayed() {
        lines.insert(GridLines::Right);
    }
    if attribute.is_crossed_out() {
        lines.insert(GridLines::Strikethrough);
    }

    match attribute.get_underline_style() {
        UnderlineStyle::NoUnderline => {}
        UnderlineStyle::SinglyUnderlined => lines.insert(GridLines::Underline),
        UnderlineStyle::DoublyUnderlined => lines.insert(GridLines::DoubleUnderline),
        UnderlineStyle::CurlyUnderlined => lines.insert(GridLines::CurlyUnderline),
        UnderlineStyle::DottedUnderlined => lines.insert(GridLines::DottedUnderline),
        UnderlineStyle::DashedUnderlined => lines.insert(GridLines::DashedUnderline),
    }

    if attribute.is_hyperlink() {
        lines.insert(GridLines::HyperlinkUnderline);
    }

    lines
}

/// Returns whether `text` is a single glyph in the inclusive soft-font range.
///
/// Ported from `Renderer::s_IsSoftFontChar`. Upstream tests one UTF-16 unit;
/// this port tests one Unicode scalar, which is what the decoded text holds.
pub fn is_soft_font_char(text: &str, first: usize, last: usize) -> bool {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => (first..=last).contains(&(ch as usize)),
        _ => false,
    }
}

/// The renderer: the render data, the settings and the engines that draw it.
pub struct Renderer<D: RenderData> {
    data: D,
    settings: RenderSettings,
    engines: Vec<Box<dyn RenderEngine>>,
    hovered_hyperlink_id: u16,
}

impl<D: RenderData> Renderer<D> {
    /// Creates a renderer over `data` and `settings`.
    pub fn new(data: D, settings: RenderSettings) -> Self {
        Self {
            data,
            settings,
            engines: Vec::new(),
            hovered_hyperlink_id: 0,
        }
    }

    /// Returns the render data.
    pub const fn data(&self) -> &D {
        &self.data
    }

    /// Returns the render data mutably.
    pub const fn data_mut(&mut self) -> &mut D {
        &mut self.data
    }

    /// Returns the render settings.
    pub const fn settings(&self) -> &RenderSettings {
        &self.settings
    }

    /// Returns the render settings mutably.
    pub const fn settings_mut(&mut self) -> &mut RenderSettings {
        &mut self.settings
    }

    /// Adds an engine. Every frame is painted into every engine, in order.
    pub fn add_engine(&mut self, engine: Box<dyn RenderEngine>) {
        self.engines.push(engine);
    }

    /// Changes the size every engine paints for.
    ///
    /// `target_size` is the surface in pixels and `viewport` the grid in cells;
    /// the two describe the same resize, so they are applied together.
    pub fn resize(&mut self, target_size: (u16, u16), viewport: InclusiveRect) -> RenderResult<()> {
        for engine in &mut self.engines {
            engine.update_target_size(target_size)?;
            engine.update_viewport(viewport)?;
        }
        Ok(())
    }

    /// Changes the default background and foreground every engine paints with.
    pub fn set_default_colors(&mut self, background: u32, foreground: u32) -> RenderResult<()> {
        for engine in &mut self.engines {
            engine.update_default_colors(background, foreground)?;
        }
        Ok(())
    }

    /// Records which hyperlink is under the pointer.
    ///
    /// A hovered hyperlink is underlined solidly instead of with the normal
    /// hyperlink dash, matching `Renderer::_isHoveredHyperlink`.
    pub fn set_hovered_hyperlink_id(&mut self, id: u16) {
        self.hovered_hyperlink_id = id;
    }

    /// Returns the hyperlink id under the pointer.
    pub const fn hovered_hyperlink_id(&self) -> u16 {
        self.hovered_hyperlink_id
    }

    /// Returns whether any engine asked for an immediate second frame.
    pub fn requires_continuous_redraw(&mut self) -> bool {
        self.engines
            .iter_mut()
            .any(|engine| engine.requires_continuous_redraw())
    }

    /// Marks the whole viewport dirty, so the next frame repaints every row.
    ///
    /// The atlas engine paints only its dirty area; a shell that always presents
    /// the software buffer invalidates before each frame (ADR 006).
    pub fn invalidate_all(&mut self) {
        for engine in &mut self.engines {
            let _ = engine.invalidate(None);
        }
    }

    /// Paints one frame into every engine.
    pub fn paint_frame(&mut self) -> RenderResult<()> {
        let Renderer {
            data,
            settings,
            engines,
            hovered_hyperlink_id,
        } = self;

        for engine in engines {
            paint_frame_for_engine(engine.as_mut(), data, settings, *hovered_hyperlink_id)?;
        }

        Ok(())
    }
}

/// Paints one engine, ending the paint even when a draw step fails.
fn paint_frame_for_engine(
    engine: &mut dyn RenderEngine,
    data: &impl RenderData,
    settings: &RenderSettings,
    hovered_hyperlink_id: u16,
) -> RenderResult<()> {
    if engine.start_paint()? == StartPaint::Skip {
        return Ok(());
    }

    let result = paint_commands(engine, data, settings, hovered_hyperlink_id);

    // Upstream's scope guard always calls EndPaint, even after a failure; the
    // frame is handed to the backend only when painting succeeded.
    let end = engine.end_paint();
    result.and(end).and_then(|()| engine.present(&AtomicBool::new(false)))
}

fn paint_commands(
    engine: &mut dyn RenderEngine,
    data: &impl RenderData,
    settings: &RenderSettings,
    hovered_hyperlink_id: u16,
) -> RenderResult<()> {
    // A. Default brushes.
    engine.update_drawing_brushes(&TextAttribute::default(), settings, data, false, true)?;

    // B. Scroll the previous frame into place before repainting.
    engine.scroll_frame()?;

    // C. Frame info.
    let info = RenderFrameInfo {
        search_highlights: data.search_highlights().to_vec(),
        search_highlight_focused: data.search_highlight_focused(),
        selection_spans: data.selection_spans().to_vec(),
        selection_background: Color::from_color_ref(settings.color_table_entry(SELECTION_BACKGROUND)),
    };
    engine.prepare_render_info(info)?;

    // 1. Background.
    engine.paint_background()?;

    // 2. Text.
    paint_buffer_output(engine, data, settings, hovered_hyperlink_id)?;

    // 4. Selection.
    paint_selection(engine, data)?;

    // 5. Cursor.
    let cursor = data.cursor_options();
    if cursor.in_viewport && cursor.is_visible {
        engine.paint_cursor(&cursor)?;
    }

    // 6. Title.
    engine.update_title(data.console_title())?;

    Ok(())
}

fn paint_buffer_output(
    engine: &mut dyn RenderEngine,
    data: &impl RenderData,
    settings: &RenderSettings,
    hovered_hyperlink_id: u16,
) -> RenderResult<()> {
    let dirty: Vec<Rect> = engine.dirty_area().to_vec();
    if dirty.is_empty() {
        return Ok(());
    }

    let viewport = data.viewport();
    let buffer = data.text_buffer();
    let composition = data.active_composition();
    let composition_origin = data.composition_origin();
    let composition_active = composition_origin.is_some() && !composition.text.is_empty();

    let result = (|| {
        for rect in &dirty {
            if rect.right() <= rect.left() || rect.bottom() <= rect.top() {
                continue;
            }

            // Map the screen-relative dirty rows onto buffer rows.
            let top = rect.top().max(0);
            let bottom = rect.bottom().min(viewport.height());
            if top >= bottom {
                continue;
            }

            let left_screen = rect.left().max(0);
            let right_screen = rect.right().min(viewport.width());
            if left_screen >= right_screen {
                continue;
            }

            let buffer_left = viewport.left() + left_screen;
            let buffer_right = viewport.left() + right_screen - 1;

            for screen_row in top..bottom {
                let buffer_row = viewport.top() + screen_row;
                let screen_line = InclusiveRect::new(buffer_left, buffer_row, buffer_right, buffer_row);
                let target = Point::new(left_screen, screen_row);

                if TextBufferCellIterator::with_bounds(buffer, Point::new(buffer_left, buffer_row), screen_line)
                    .is_some()
                {
                    let rendition = buffer.get_line_rendition(buffer_row);
                    engine.prepare_line_transform(rendition, screen_row, viewport.left())?;

                    if composition_active && composition_origin.is_some_and(|origin| origin.y() == buffer_row) {
                        paint_composition_row(
                            engine,
                            data,
                            settings,
                            buffer_row,
                            buffer_left,
                            buffer_right,
                            composition_origin.expect("checked above"),
                            composition,
                            target,
                            hovered_hyperlink_id,
                        )?;
                    } else if let Some(iterator) =
                        TextBufferCellIterator::with_bounds(buffer, Point::new(buffer_left, buffer_row), screen_line)
                    {
                        let cells: Vec<&OutputCell> = iterator.collect();
                        paint_cells(engine, data, settings, &cells, target, hovered_hyperlink_id)?;
                    }
                }

                if let Some(image) = buffer
                    .get_row_at_screen_line(buffer_row)
                    .and_then(|row| row.image_slice())
                {
                    engine.paint_image_slice(image, screen_row, viewport.left())?;
                }
            }
        }

        Ok(())
    })();

    // Upstream resets the line transform even when a paint step failed.
    engine.reset_line_transform()?;
    result
}

/// Paints a row with the active IME composition overlaid on it.
#[allow(clippy::too_many_arguments)]
fn paint_composition_row(
    engine: &mut dyn RenderEngine,
    data: &impl RenderData,
    settings: &RenderSettings,
    buffer_row: i32,
    buffer_left: i32,
    buffer_right: i32,
    origin: Point,
    composition: &Composition,
    target: Point,
    hovered_hyperlink_id: u16,
) -> RenderResult<()> {
    let buffer = data.text_buffer();
    let Some(row) = buffer.get_row_at_screen_line(buffer_row) else {
        return Ok(());
    };

    let source: Vec<OutputCell> = (0..buffer.screen_width())
        .map(|column| row.get_cell(column).clone())
        .collect();
    let overlay = apply_composition(&source, origin.x(), composition);
    let cells: Vec<&OutputCell> = (buffer_left..=buffer_right)
        .filter_map(|column| overlay.get(column.max(0) as usize))
        .collect();

    paint_cells(engine, data, settings, &cells, target, hovered_hyperlink_id)
}

/// Overwrites the cells at `origin_x` with the composition and pushes the
/// overwritten non-space text to the right, absorbing whitespace.
///
/// Ported from `Renderer::_PaintBufferOutputComposition`. This works on a copy
/// of the row rather than mutating the buffer, so the composition never leaks
/// into the terminal's content.
fn apply_composition(source: &[OutputCell], origin_x: i32, composition: &Composition) -> Vec<OutputCell> {
    let mut result = source.to_vec();
    if origin_x < 0 || origin_x as usize >= source.len() {
        return result;
    }

    let origin = origin_x as usize;
    let base = *source[origin].get_attributes();

    let mut column = origin;
    let mut offset = 0usize;
    for range in &composition.attributes {
        let text: String = composition.text.chars().skip(offset).take(range.len).collect();

        let mut attr = range.attr;
        if attr.get_background().is_default() {
            attr.set_background(base.get_background());
        }
        if attr.get_foreground().is_default() {
            attr.set_foreground(base.get_foreground());
        }

        for ch in text.chars() {
            if column >= result.len() {
                break;
            }
            let width = usize::from(codepoint_width(ch as u32).max(1));
            let mut cell = OutputCell::from_char(ch);
            cell.set_attributes(attr);
            result[column] = cell;
            for trailing in 1..width {
                if column + trailing < result.len() {
                    result[column + trailing] = OutputCell::new();
                }
            }
            column += width;
        }

        offset += range.len;
    }

    let composition_width = column.saturating_sub(origin);
    if composition_width > 0 && column < source.len() {
        let mut src = origin;
        let mut dst = column;
        let mut remaining = composition_width;

        while src < source.len() && dst < result.len() {
            // Whitespace in the overwritten text is absorbed by the composition.
            while src < source.len() && source[src].is_whitespace() && remaining > 0 {
                remaining -= 1;
                src += 1;
            }
            if remaining == 0 {
                break;
            }

            let span_start = src;
            while src < source.len() && !source[src].is_whitespace() {
                src += 1;
            }
            let span_len = src - span_start;
            if span_len == 0 {
                break;
            }

            for index in 0..span_len {
                if dst + index >= result.len() {
                    break;
                }
                result[dst + index] = source[span_start + index].clone();
            }
            dst += span_len;
        }
    }

    result
}

/// Paints cells, grouping consecutive cells into attribute runs.
fn paint_cells<'a>(
    engine: &mut dyn RenderEngine,
    data: &impl RenderData,
    settings: &RenderSettings,
    cells: &[&'a OutputCell],
    target: Point,
    hovered_hyperlink_id: u16,
) -> RenderResult<()> {
    let mut index = 0usize;
    let mut screen_x = target.x();

    while index < cells.len() {
        let first: &'a OutputCell = cells[index];
        let run_attr = *first.get_attributes();
        let using_soft_font = uses_soft_font(data, first.chars());

        let mut clusters: Vec<Cluster<'a>> = Vec::new();
        let mut columns = 0i32;

        while index < cells.len() {
            let cell: &'a OutputCell = cells[index];
            if *cell.get_attributes() != run_attr || uses_soft_font(data, cell.chars()) != using_soft_font {
                break;
            }

            // A cell that reports no width still advances by one column, or the
            // walk would never end.
            let width = i32::from(cell.column_width()).max(1);
            clusters.push(Cluster::new(cell.chars(), width));
            index += usize::try_from(width).unwrap_or(1);
            columns += width;
        }

        if clusters.is_empty() {
            index += 1;
            continue;
        }

        engine.update_drawing_brushes(&run_attr, settings, data, using_soft_font, false)?;
        let run_start = Point::new(screen_x, target.y());
        engine.paint_buffer_line(&clusters, run_start, false)?;

        if data.is_grid_line_drawing_allowed() {
            let mut lines = get_gridlines(&run_attr);
            if hovered_hyperlink_id != 0 && run_attr.get_hyperlink_id() == hovered_hyperlink_id {
                lines.remove(GridLines::HyperlinkUnderline);
                lines.insert(GridLines::Underline);
            }
            if !lines.is_empty() {
                let (gridline_color, _) = settings.get_attribute_colors(&run_attr);
                let underline_color = settings.get_attribute_underline_color(&run_attr);
                engine.paint_buffer_grid_lines(
                    lines,
                    gridline_color,
                    underline_color,
                    usize::try_from(columns).unwrap_or(0),
                    run_start,
                )?;
            }
        }

        screen_x += columns;
    }

    Ok(())
}

fn uses_soft_font(data: &impl RenderData, text: &str) -> bool {
    data.soft_font_range()
        .is_some_and(|(first, last)| is_soft_font_char(text, first, last))
}

fn paint_selection(engine: &mut dyn RenderEngine, data: &impl RenderData) -> RenderResult<()> {
    let spans = data.selection_spans();
    if spans.is_empty() {
        return Ok(());
    }

    let dirty: Vec<Rect> = engine.dirty_area().to_vec();
    if dirty.is_empty() {
        return Ok(());
    }

    let viewport = data.viewport();
    let width = data.text_buffer().screen_width();

    for span in spans {
        span.iterate_rows(width, |row, left, right| {
            if row < viewport.top() || row > viewport.bottom() {
                return;
            }

            let screen_row = row - viewport.top();
            let screen_left = (left - viewport.left()).max(0);
            let screen_right = (right - viewport.left() + 1).min(viewport.width());
            if screen_left >= screen_right {
                return;
            }

            let rect = Rect::new(screen_left, screen_row, screen_right, screen_row + 1);
            if dirty.iter().any(|dirty| rects_intersect(dirty, &rect)) {
                // A failed selection paint is not fatal in upstream either.
                let _ = engine.paint_selection(rect);
            }
        });
    }

    Ok(())
}

fn rects_intersect(a: &Rect, b: &Rect) -> bool {
    a.left() < b.right() && b.left() < a.right() && a.top() < b.bottom() && b.top() < a.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use buffer::{ImageSlice, OutputCell, TextBuffer};
    use types::{LineRendition, TextAttribute, TextColor, UnderlineStyle};
    use utils::{PointSpan, Size};

    use crate::composition::{Composition, CompositionRange};
    use crate::cursor_options::CursorOptions;
    use crate::font_info::FontInfo;
    use crate::font_info_desired::FontInfoDesired;
    use crate::render_data::RenderData;
    use crate::render_engine::RenderEngine;
    use crate::render_frame_info::{GridLineSet, RenderFrameInfo};

    #[derive(Default)]
    struct Log {
        calls: Vec<&'static str>,
        lines: Vec<(String, Point)>,
        grid_lines: Vec<GridLineSet>,
        cursor_paints: usize,
        titles: Vec<String>,
        target_sizes: Vec<(u16, u16)>,
        viewports: Vec<InclusiveRect>,
        default_colors: Vec<(u32, u32)>,
    }

    struct RecordingEngine {
        log: Rc<RefCell<Log>>,
        dirty: Vec<Rect>,
        start: StartPaint,
    }

    impl RecordingEngine {
        fn new(log: Rc<RefCell<Log>>, dirty: Vec<Rect>) -> Self {
            Self {
                log,
                dirty,
                start: StartPaint::Continue,
            }
        }

        fn record(&self, call: &'static str) {
            self.log.borrow_mut().calls.push(call);
        }
    }

    impl RenderEngine for RecordingEngine {
        fn start_paint(&mut self) -> RenderResult<StartPaint> {
            self.record("start_paint");
            Ok(self.start)
        }
        fn end_paint(&mut self) -> RenderResult<()> {
            self.record("end_paint");
            Ok(())
        }
        fn requires_continuous_redraw(&mut self) -> bool {
            false
        }
        fn wait_until_can_render(&mut self, shutdown: &AtomicBool) -> bool {
            !shutdown.load(std::sync::atomic::Ordering::Relaxed)
        }
        fn present(&mut self, _shutdown: &AtomicBool) -> RenderResult<()> {
            Ok(())
        }
        fn scroll_frame(&mut self) -> RenderResult<()> {
            self.record("scroll_frame");
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
            self.record("prepare_render_info");
            Ok(())
        }
        fn reset_line_transform(&mut self) -> RenderResult<()> {
            self.record("reset_line_transform");
            Ok(())
        }
        fn prepare_line_transform(
            &mut self,
            _rendition: LineRendition,
            _target_row: i32,
            _viewport_left: i32,
        ) -> RenderResult<()> {
            self.record("prepare_line_transform");
            Ok(())
        }
        fn paint_background(&mut self) -> RenderResult<()> {
            self.record("paint_background");
            Ok(())
        }
        fn paint_buffer_line(&mut self, clusters: &[Cluster<'_>], coord: Point, _trim_left: bool) -> RenderResult<()> {
            self.record("paint_buffer_line");
            let text = clusters.iter().map(|cluster| cluster.text()).collect();
            self.log.borrow_mut().lines.push((text, coord));
            Ok(())
        }
        fn paint_buffer_grid_lines(
            &mut self,
            lines: GridLineSet,
            _gridline_color: u32,
            _underline_color: u32,
            _length: usize,
            _target: Point,
        ) -> RenderResult<()> {
            self.log.borrow_mut().grid_lines.push(lines);
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
            self.record("paint_selection");
            Ok(())
        }
        fn paint_cursor(&mut self, _options: &CursorOptions) -> RenderResult<()> {
            self.log.borrow_mut().cursor_paints += 1;
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
            self.record("update_drawing_brushes");
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
        fn update_viewport(&mut self, viewport: InclusiveRect) -> RenderResult<()> {
            self.log.borrow_mut().viewports.push(viewport);
            Ok(())
        }
        fn update_target_size(&mut self, target_size: (u16, u16)) -> RenderResult<()> {
            self.log.borrow_mut().target_sizes.push(target_size);
            Ok(())
        }
        fn update_default_colors(&mut self, background: u32, foreground: u32) -> RenderResult<()> {
            self.log.borrow_mut().default_colors.push((background, foreground));
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
            Size::new(1, 1)
        }
        fn is_glyph_wide_by_font(&self, _glyph: &str) -> bool {
            false
        }
        fn update_title(&mut self, new_title: &str) -> RenderResult<()> {
            self.log.borrow_mut().titles.push(new_title.to_string());
            Ok(())
        }
        fn update_hyperlink_hovered_id(&mut self, _hovered_id: u16) {}
    }

    struct TestData {
        buffer: TextBuffer,
        viewport: InclusiveRect,
        font: FontInfo,
        cursor: CursorOptions,
        selection: Vec<PointSpan>,
        highlights: Vec<PointSpan>,
        title: String,
        tsf: Composition,
        snippet: Composition,
        composition_origin: Option<Point>,
    }

    impl TestData {
        fn new(width: i32, height: i32) -> Self {
            Self {
                buffer: TextBuffer::with_dimensions(width, height, 0),
                viewport: InclusiveRect::new(0, 0, width - 1, height - 1),
                font: FontInfo::default(),
                cursor: CursorOptions::default(),
                selection: Vec::new(),
                highlights: Vec::new(),
                title: "winterm".to_string(),
                tsf: Composition::default(),
                snippet: Composition::default(),
                composition_origin: None,
            }
        }

        fn write(&mut self, row: i32, text: &str) {
            let line = self.buffer.get_row_at_screen_line_mut(row).expect("row exists");
            for (column, ch) in text.chars().enumerate() {
                line.write_cell(column as i32, OutputCell::from_char(ch));
            }
        }
    }

    impl RenderData for TestData {
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
            None
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
        fn attribute_colors(&self, _attr: &TextAttribute) -> (u32, u32) {
            (0, 0)
        }
        fn is_selection_active(&self) -> bool {
            !self.selection.is_empty()
        }
        fn is_block_selection(&self) -> bool {
            false
        }
        fn clear_selection(&mut self) {
            self.selection.clear();
        }
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
        fn composition_origin(&self) -> Option<Point> {
            self.composition_origin
        }
    }

    fn screen_rect(width: i32, height: i32) -> Rect {
        Rect::new(0, 0, width, height)
    }

    #[test]
    fn new_default_colours_reach_every_engine() {
        let log = Rc::new(RefCell::new(Log::default()));
        let mut renderer = Renderer::new(TestData::new(3, 2), RenderSettings::new());
        renderer.add_engine(Box::new(RecordingEngine::new(Rc::clone(&log), Vec::new())));

        renderer
            .set_default_colors(0x0010_2030, 0x0040_5060)
            .expect("default colours");

        assert_eq!(log.borrow().default_colors, vec![(0x0010_2030, 0x0040_5060)]);
    }

    #[test]
    fn a_resize_reaches_every_engine_as_a_size_and_a_viewport() {
        let log = Rc::new(RefCell::new(Log::default()));
        let mut renderer = Renderer::new(TestData::new(3, 2), RenderSettings::new());
        renderer.add_engine(Box::new(RecordingEngine::new(Rc::clone(&log), Vec::new())));

        renderer
            .resize((96, 64), InclusiveRect::new(0, 0, 11, 3))
            .expect("resize");

        let log = log.borrow();
        assert_eq!(log.target_sizes, vec![(96, 64)]);
        assert_eq!(log.viewports, vec![InclusiveRect::new(0, 0, 11, 3)]);
    }

    #[test]
    fn a_frame_paints_every_visible_row() {
        let mut data = TestData::new(3, 2);
        data.write(0, "abc");
        data.write(1, "def");

        let log = Rc::new(RefCell::new(Log::default()));
        let mut renderer = Renderer::new(data, RenderSettings::new());
        renderer.add_engine(Box::new(RecordingEngine::new(Rc::clone(&log), vec![screen_rect(3, 2)])));

        renderer.paint_frame().expect("frame");

        let log = log.borrow();
        assert_eq!(
            log.lines,
            vec![
                ("abc".to_string(), Point::new(0, 0)),
                ("def".to_string(), Point::new(0, 1))
            ]
        );
        assert_eq!(log.calls.first(), Some(&"start_paint"));
        assert_eq!(log.calls.last(), Some(&"end_paint"));
        assert!(log.calls.contains(&"paint_background"));
        assert!(log.calls.contains(&"prepare_render_info"));
    }

    #[test]
    fn a_skipped_start_paints_nothing() {
        let log = Rc::new(RefCell::new(Log::default()));
        let mut engine = RecordingEngine::new(Rc::clone(&log), vec![screen_rect(3, 2)]);
        engine.start = StartPaint::Skip;

        let mut renderer = Renderer::new(TestData::new(3, 2), RenderSettings::new());
        renderer.add_engine(Box::new(engine));

        renderer.paint_frame().expect("frame");

        let log = log.borrow();
        assert_eq!(log.calls, vec!["start_paint"]);
        assert!(log.lines.is_empty());
    }

    #[test]
    fn the_cursor_is_painted_only_when_visible_and_in_viewport() {
        let mut data = TestData::new(3, 2);
        data.cursor = CursorOptions {
            is_visible: true,
            in_viewport: true,
            ..CursorOptions::default()
        };

        let log = Rc::new(RefCell::new(Log::default()));
        let mut renderer = Renderer::new(data, RenderSettings::new());
        renderer.add_engine(Box::new(RecordingEngine::new(Rc::clone(&log), vec![screen_rect(3, 2)])));

        renderer.paint_frame().expect("frame");
        assert_eq!(log.borrow().cursor_paints, 1);
    }

    #[test]
    fn the_title_is_forwarded_every_frame() {
        let log = Rc::new(RefCell::new(Log::default()));
        let mut renderer = Renderer::new(TestData::new(3, 2), RenderSettings::new());
        renderer.add_engine(Box::new(RecordingEngine::new(Rc::clone(&log), vec![screen_rect(3, 2)])));

        renderer.paint_frame().expect("frame");

        assert_eq!(log.borrow().titles, vec!["winterm".to_string()]);
    }

    #[test]
    fn an_underlined_run_paints_grid_lines() {
        let mut data = TestData::new(3, 1);
        let line = data.buffer.get_row_at_screen_line_mut(0).expect("row exists");
        let mut attribute = TextAttribute::default();
        attribute.set_underline_style(UnderlineStyle::SinglyUnderlined);
        for column in 0..3 {
            let mut cell = OutputCell::from_char('x');
            cell.set_attributes(attribute);
            line.write_cell(column, cell);
        }

        let log = Rc::new(RefCell::new(Log::default()));
        let mut renderer = Renderer::new(data, RenderSettings::new());
        renderer.add_engine(Box::new(RecordingEngine::new(Rc::clone(&log), vec![screen_rect(3, 1)])));

        renderer.paint_frame().expect("frame");
        let log = log.borrow();
        assert_eq!(log.grid_lines.len(), 1);
        assert!(log.grid_lines[0].contains(GridLines::Underline));
    }

    fn write_hyperlink_row(data: &mut TestData, id: u16) {
        let line = data.buffer.get_row_at_screen_line_mut(0).expect("row exists");
        let mut attribute = TextAttribute::default();
        attribute.set_hyperlink_id(id);
        for column in 0..3 {
            let mut cell = OutputCell::from_char('x');
            cell.set_attributes(attribute);
            line.write_cell(column, cell);
        }
    }

    #[test]
    fn a_hovered_hyperlink_switches_to_a_solid_underline() {
        let log = Rc::new(RefCell::new(Log::default()));
        let mut data = TestData::new(3, 1);
        write_hyperlink_row(&mut data, 5);

        let mut renderer = Renderer::new(data, RenderSettings::new());
        renderer.add_engine(Box::new(RecordingEngine::new(Rc::clone(&log), vec![screen_rect(3, 1)])));
        renderer.set_hovered_hyperlink_id(5);

        renderer.paint_frame().expect("frame");

        let log = log.borrow();
        assert_eq!(log.grid_lines.len(), 1);
        assert!(log.grid_lines[0].contains(GridLines::Underline));
        assert!(!log.grid_lines[0].contains(GridLines::HyperlinkUnderline));
    }

    #[test]
    fn an_unhovered_hyperlink_keeps_its_dash() {
        let log = Rc::new(RefCell::new(Log::default()));
        let mut data = TestData::new(3, 1);
        write_hyperlink_row(&mut data, 5);

        let mut renderer = Renderer::new(data, RenderSettings::new());
        renderer.add_engine(Box::new(RecordingEngine::new(Rc::clone(&log), vec![screen_rect(3, 1)])));
        renderer.set_hovered_hyperlink_id(6);

        renderer.paint_frame().expect("frame");

        let log = log.borrow();
        assert_eq!(log.grid_lines.len(), 1);
        assert!(log.grid_lines[0].contains(GridLines::HyperlinkUnderline));
        assert!(!log.grid_lines[0].contains(GridLines::Underline));
    }

    fn row_from(text: &str) -> Vec<OutputCell> {
        text.chars().map(OutputCell::from_char).collect()
    }

    fn composition(text: &str, len: usize, attr: TextAttribute) -> Composition {
        Composition {
            text: text.to_string(),
            attributes: vec![CompositionRange { len, attr }],
            cursor_pos: 0,
        }
    }

    #[test]
    fn a_composition_overwrites_the_origin_and_pushes_text_right() {
        let source = row_from("abcd");
        let result = apply_composition(&source, 1, &composition("XY", 2, TextAttribute::default()));

        let text: String = result.iter().map(|cell| cell.chars()).collect();
        assert_eq!(text, "aXYb");
    }

    #[test]
    fn a_composition_absorbs_overwritten_whitespace() {
        let source = row_from("a  cd");
        let result = apply_composition(&source, 1, &composition("XY", 2, TextAttribute::default()));

        let text: String = result.iter().map(|cell| cell.chars()).collect();
        assert_eq!(text, "aXYcd");
    }

    #[test]
    fn a_default_composition_colour_inherits_the_cell_colour() {
        let mut base = TextAttribute::default();
        base.set_foreground(TextColor::index16(2));
        let mut cell = OutputCell::from_char('a');
        cell.set_attributes(base);

        let mut run = TextAttribute::default();
        run.set_background(TextColor::index16(4));

        let result = apply_composition(&[cell], 0, &composition("X", 1, run));
        let attrs = result[0].get_attributes();
        assert_eq!(attrs.get_foreground(), TextColor::index16(2));
        assert_eq!(attrs.get_background(), TextColor::index16(4));
    }

    #[test]
    fn a_frame_paints_the_composition_over_the_row() {
        let mut data = TestData::new(4, 1);
        data.write(0, "abcd");
        data.tsf = composition("XY", 2, TextAttribute::default());
        data.composition_origin = Some(Point::new(1, 0));

        let log = Rc::new(RefCell::new(Log::default()));
        let mut renderer = Renderer::new(data, RenderSettings::new());
        renderer.add_engine(Box::new(RecordingEngine::new(Rc::clone(&log), vec![screen_rect(4, 1)])));

        renderer.paint_frame().expect("frame");
        assert_eq!(log.borrow().lines, vec![("aXYb".to_string(), Point::new(0, 0))]);
    }

    #[test]
    fn a_selection_span_is_painted_per_row() {
        let mut data = TestData::new(3, 2);
        data.selection = vec![PointSpan::new(Point::new(0, 0), Point::new(1, 1))];

        let log = Rc::new(RefCell::new(Log::default()));
        let mut renderer = Renderer::new(data, RenderSettings::new());
        renderer.add_engine(Box::new(RecordingEngine::new(Rc::clone(&log), vec![screen_rect(3, 2)])));

        renderer.paint_frame().expect("frame");
        let paints = log
            .borrow()
            .calls
            .iter()
            .filter(|call| **call == "paint_selection")
            .count();
        assert_eq!(paints, 2);
    }

    #[test]
    fn a_plain_attribute_has_no_lines() {
        assert!(get_gridlines(&TextAttribute::default()).is_empty());
    }

    #[test]
    fn each_border_maps_to_its_line() {
        for (flag, line) in [
            (types::CharacterAttributes::TopGridline, GridLines::Top),
            (types::CharacterAttributes::BottomGridline, GridLines::Bottom),
            (types::CharacterAttributes::LeftGridline, GridLines::Left),
            (types::CharacterAttributes::RightGridline, GridLines::Right),
        ] {
            let mut attribute = TextAttribute::default();
            attribute.set_attrs(flag);
            let lines = get_gridlines(&attribute);
            assert!(lines.contains(line), "{flag:?}");
            assert!(!lines.contains(GridLines::Strikethrough));
        }
    }

    #[test]
    fn crossed_out_is_a_strikethrough() {
        let mut attribute = TextAttribute::default();
        attribute.set_crossed_out(true);
        assert!(get_gridlines(&attribute).contains(GridLines::Strikethrough));
    }

    #[test]
    fn every_underline_style_maps_to_its_line() {
        for (style, line) in [
            (UnderlineStyle::SinglyUnderlined, GridLines::Underline),
            (UnderlineStyle::DoublyUnderlined, GridLines::DoubleUnderline),
            (UnderlineStyle::CurlyUnderlined, GridLines::CurlyUnderline),
            (UnderlineStyle::DottedUnderlined, GridLines::DottedUnderline),
            (UnderlineStyle::DashedUnderlined, GridLines::DashedUnderline),
        ] {
            let mut attribute = TextAttribute::default();
            attribute.set_underline_style(style);
            assert!(get_gridlines(&attribute).contains(line), "{style:?}");
        }
    }

    #[test]
    fn a_hyperlink_adds_its_underline() {
        let mut attribute = TextAttribute::default();
        attribute.set_hyperlink_id(7);
        assert!(get_gridlines(&attribute).contains(GridLines::HyperlinkUnderline));
    }

    #[test]
    fn combined_attributes_produce_every_line() {
        let mut attribute = TextAttribute::default();
        attribute.set_attrs(types::CharacterAttributes::TopGridline);
        attribute.set_crossed_out(true);
        attribute.set_underline_style(UnderlineStyle::DoublyUnderlined);
        attribute.set_hyperlink_id(1);

        let lines = get_gridlines(&attribute);
        assert!(lines.contains(GridLines::Top));
        assert!(lines.contains(GridLines::Strikethrough));
        assert!(lines.contains(GridLines::DoubleUnderline));
        assert!(lines.contains(GridLines::HyperlinkUnderline));
    }

    #[test]
    fn only_single_glyphs_inside_the_range_are_soft_font_chars() {
        let first = FIRST_SOFT_FONT_CHAR;
        let last = first + 10;

        assert!(is_soft_font_char("\u{EF20}", first, last));
        assert!(is_soft_font_char("\u{EF2A}", first, last));
        assert!(!is_soft_font_char("\u{EF1F}", first, last));
        assert!(!is_soft_font_char("\u{EF2B}", first, last));
        assert!(!is_soft_font_char("", first, last));
        assert!(!is_soft_font_char("AB", first, last));
    }
}
