//! Portable data from the atlas engine's `common.h`.
//!
//! The D2D/D3D/DWrite members are left to the backend modules; these are the
//! value types the engine and its tests share.

use crate::render_frame_info::{GridLineSet, GridLines};
use types::LineRendition;

/// Converts a `COLORREF` (`0x00BBGGRR`) to `0xAARRGGBB`.
pub const fn argb_from_colorref(color: u32) -> u32 {
    0xFF00_0000 | ((color & 0x0000_00FF) << 16) | (color & 0x0000_FF00) | ((color & 0x00FF_0000) >> 16)
}

/// The position and height of a font decoration (underline, grid line, ...).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FontDecorationPosition {
    /// Vertical position within the cell, in pixels.
    pub position: u16,
    /// Thickness in pixels.
    pub height: u16,
}

/// A run of grid lines over a column range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridLineRange {
    /// The lines to draw.
    pub lines: GridLineSet,
    /// The first column of the run.
    pub from: u16,
    /// The column one past the run.
    pub to: u16,
}

impl GridLineRange {
    /// Returns whether the run is empty.
    pub const fn is_empty(&self) -> bool {
        self.from >= self.to
    }
}

/// Where an image bitmap is scaled into a row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bitmap {
    /// Matches `ImageSlice`'s revision; zero means the bitmap is empty.
    pub revision: u64,
    /// The source RGBA pixels, row-major.
    pub source: Vec<u32>,
    /// The source size in pixels.
    pub source_size: (i32, i32),
    /// The horizontal offset in columns.
    pub target_offset: i32,
    /// The width in columns; the height is always one cell.
    pub target_width: i32,
    /// Whether the bitmap was used this frame.
    pub active: bool,
}

impl Bitmap {
    /// Returns whether the bitmap holds an image.
    pub const fn is_empty(&self) -> bool {
        self.revision == 0
    }
}

/// The cursor colour and shape the atlas draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorSettings {
    /// The `COLORREF` the cursor is drawn with.
    pub cursor_color: u32,
    /// The `CursorType` as a raw value.
    pub cursor_type: u16,
    /// The height percentage for a legacy or underscore cursor.
    pub height_percentage: u16,
}

impl Default for CursorSettings {
    fn default() -> Self {
        Self {
            cursor_color: 0xFFFF_FFFF,
            cursor_type: 0,
            height_percentage: 20,
        }
    }
}

/// Colours and the retro-effect shader paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiscellaneousSettings {
    /// The default background, as a `COLORREF`.
    pub background_color: u32,
    /// The default foreground, as a `COLORREF`.
    pub foreground_color: u32,
    /// The selection background, as a `COLORREF`.
    pub selection_color: u32,
    /// The selection foreground, as a `COLORREF`.
    pub selection_foreground: u32,
    /// Optional custom pixel-shader source path.
    pub custom_pixel_shader_path: String,
    /// Optional custom pixel-shader image path.
    pub custom_pixel_shader_image_path: String,
    /// Whether the retro-terminal shader is enabled.
    pub use_retro_terminal_effect: bool,
}

impl Default for MiscellaneousSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl MiscellaneousSettings {
    /// Creates the upstream default colours.
    pub fn new() -> Self {
        Self {
            background_color: 0,
            foreground_color: 0,
            selection_color: 0xFFFF_FFFF,
            selection_foreground: 0xFF00_0000,
            custom_pixel_shader_path: String::new(),
            custom_pixel_shader_image_path: String::new(),
            use_retro_terminal_effect: false,
        }
    }
}

/// The text attributes that select a distinct font face.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FontRelevantAttributes(u8);

impl FontRelevantAttributes {
    /// No font-relevant attribute.
    pub const NONE: Self = Self(0);
    /// Bold selects the bold face.
    pub const BOLD: Self = Self(0b01);
    /// Italic selects the italic face.
    pub const ITALIC: Self = Self(0b10);

    /// Returns whether every attribute in `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Adds the attributes in `other`.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Returns the raw bits.
    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// Identifies a font face the shaping backend knows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FontId(pub usize);

/// A run of glyphs that use one font face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontMapping {
    /// The face for this run.
    pub font: FontId,
    /// The first glyph index in the row's glyph arrays.
    pub glyphs_from: usize,
    /// One past the last glyph index.
    pub glyphs_to: usize,
}

impl FontMapping {
    /// Returns the number of glyphs in the run.
    pub const fn len(&self) -> usize {
        self.glyphs_to.saturating_sub(self.glyphs_from)
    }

    /// Returns whether the run has no glyphs.
    pub const fn is_empty(&self) -> bool {
        self.glyphs_from >= self.glyphs_to
    }
}

/// A glyph's sub-cell offset, in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GlyphOffset {
    /// Horizontal offset from the pen position.
    pub advance_offset: f32,
    /// Vertical offset from the baseline.
    pub ascender_offset: f32,
}

/// One shaped glyph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedGlyph {
    /// The glyph index, or a code point for a built-in glyph.
    pub glyph_index: u16,
    /// The advance in pixels.
    pub advance: f32,
    /// The sub-cell offset.
    pub offset: (f32, f32),
    /// An explicit colour, if the glyph carries one (for example an emoji).
    pub color: Option<u32>,
}

/// One shaped row of the atlas, stored as parallel arrays.
///
/// Ported from `common.h`'s `ShapedRow`. The parallel arrays are grouped by
/// [`FontMapping`], and [`ShapedRow::clear`] resets them between frames.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShapedRow {
    /// The font runs, in order.
    pub mappings: Vec<FontMapping>,
    /// Glyph indices; when a mapping's font is unused, these are code points.
    pub glyph_indices: Vec<u16>,
    /// The starting column of each glyph, in cells.
    pub glyph_columns: Vec<u16>,
    /// Advances in pixels, one per glyph.
    pub glyph_advances: Vec<f32>,
    /// Sub-cell offsets, one per glyph.
    pub glyph_offsets: Vec<GlyphOffset>,
    /// Explicit glyph colours, one per glyph; zero means "use the run colour".
    pub colors: Vec<u32>,
    /// An image bitmap painted with the row.
    pub bitmap: Bitmap,
    /// The grid-line runs over the row.
    pub grid_line_ranges: Vec<GridLineRange>,
    /// The line rendition the row is drawn with.
    pub line_rendition: LineRendition,
    /// The first dirty pixel row.
    pub dirty_top: i32,
    /// The first clean pixel row below the dirty area.
    pub dirty_bottom: i32,
}

impl ShapedRow {
    /// Clears the row and marks its cell band dirty.
    pub fn clear(&mut self, y: i32, cell_height: i32) {
        self.mappings.clear();
        self.glyph_indices.clear();
        self.glyph_columns.clear();
        self.glyph_advances.clear();
        self.glyph_offsets.clear();
        self.colors.clear();
        self.bitmap.active = false;
        self.grid_line_ranges.clear();
        self.line_rendition = LineRendition::SingleWidth;
        self.dirty_top = y * cell_height;
        self.dirty_bottom = self.dirty_top + cell_height;
    }

    /// Appends shaped glyphs for `font` starting at `column`, extending the
    /// last run when it matches.
    pub fn push_glyphs(&mut self, font: FontId, glyphs: &[ShapedGlyph], column: u16) {
        if glyphs.is_empty() {
            return;
        }

        let from = self.glyph_indices.len();
        for glyph in glyphs {
            self.glyph_indices.push(glyph.glyph_index);
            self.glyph_columns.push(column);
            self.glyph_advances.push(glyph.advance);
            self.glyph_offsets.push(GlyphOffset {
                advance_offset: glyph.offset.0,
                ascender_offset: glyph.offset.1,
            });
            self.colors.push(glyph.color.unwrap_or(0));
        }
        let to = self.glyph_indices.len();

        if let Some(last) = self.mappings.last_mut()
            && last.font == font
            && last.glyphs_to == from
        {
            last.glyphs_to = to;
        } else {
            self.mappings.push(FontMapping {
                font,
                glyphs_from: from,
                glyphs_to: to,
            });
        }
    }

    /// Returns the number of glyphs in the row.
    pub const fn glyph_count(&self) -> usize {
        self.glyph_indices.len()
    }
}

/// The text antialiasing mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AntialiasingMode {
    /// Subpixel (ClearType) rendering.
    #[default]
    ClearType,
    /// Grayscale antialiasing.
    Grayscale,
    /// No antialiasing.
    Aliased,
}

/// Target settings that are not device-specific.
///
/// Device and surface selection belongs to the concrete backend; only the
/// cross-platform transparency flag lives here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TargetSettings {
    /// Whether the surface is rendered with an alpha channel.
    pub use_alpha: bool,
}

/// The metrics and decorations the atlas draws text with.
#[derive(Debug, Clone, PartialEq)]
pub struct FontSettings {
    /// The face the backend loaded.
    pub font: FontId,
    /// The face name, for diagnostics.
    pub font_name: String,
    /// The font size in points.
    pub font_size: f32,
    /// The cell size in pixels.
    pub cell_size: (u16, u16),
    /// The font weight.
    pub font_weight: u16,
    /// The advance width of `0` in pixels.
    pub advance_width: u16,
    /// The baseline offset from the cell top.
    pub baseline: u16,
    /// The descender depth in pixels.
    pub descender: u16,
    /// The thickness of a thin line (grid, underline).
    pub thin_line_width: u16,
    /// The top grid line.
    pub grid_top: FontDecorationPosition,
    /// The bottom grid line.
    pub grid_bottom: FontDecorationPosition,
    /// The left grid line.
    pub grid_left: FontDecorationPosition,
    /// The right grid line.
    pub grid_right: FontDecorationPosition,
    /// The underline.
    pub underline: FontDecorationPosition,
    /// The strikethrough.
    pub strikethrough: FontDecorationPosition,
    /// The two lines of a double underline.
    pub double_underline: [FontDecorationPosition; 2],
    /// The overline.
    pub overline: FontDecorationPosition,
    /// The DPI.
    pub dpi: u16,
    /// The antialiasing mode.
    pub antialiasing_mode: AntialiasingMode,
    /// Whether built-in box-drawing glyphs are enabled.
    pub builtin_glyphs: bool,
    /// Whether colour glyphs are enabled.
    pub color_glyphs: bool,
    /// The DECDLD soft-font bit pattern, if any.
    pub soft_font_pattern: Vec<u16>,
    /// The DECDLD soft-font cell size.
    pub soft_font_cell_size: (i32, i32),
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            font: FontId(0),
            font_name: String::new(),
            font_size: 0.0,
            cell_size: (0, 0),
            font_weight: 0,
            advance_width: 0,
            baseline: 0,
            descender: 0,
            thin_line_width: 0,
            grid_top: FontDecorationPosition::default(),
            grid_bottom: FontDecorationPosition::default(),
            grid_left: FontDecorationPosition::default(),
            grid_right: FontDecorationPosition::default(),
            underline: FontDecorationPosition::default(),
            strikethrough: FontDecorationPosition::default(),
            double_underline: [FontDecorationPosition::default(); 2],
            overline: FontDecorationPosition::default(),
            dpi: 96,
            antialiasing_mode: AntialiasingMode::ClearType,
            builtin_glyphs: false,
            color_glyphs: true,
            soft_font_pattern: Vec::new(),
            soft_font_cell_size: (0, 0),
        }
    }
}

/// The settings the atlas engine reads each frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// Target transparency.
    pub target: TargetSettings,
    /// Font metrics and decorations.
    pub font: FontSettings,
    /// Cursor colour and shape.
    pub cursor: CursorSettings,
    /// Colours and shader paths.
    pub misc: MiscellaneousSettings,
    /// The target size in pixels.
    pub target_size: (u16, u16),
    /// The viewport size in cells.
    pub viewport_cell_count: (u16, u16),
}

/// The per-frame data a backend renders.
///
/// Ported from `common.h`'s `RenderingPayload` without its D3D/DXGI handles;
/// those belong to the concrete backend. The colour bitmap holds three stacked
/// images (background, foreground and underline colours).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderingPayload {
    /// The settings for this frame.
    pub settings: Settings,
    /// The shaped rows, ordered from row 0.
    pub rows: Vec<ShapedRow>,
    /// The three colour bitmaps, back to back.
    pub color_bitmap: Vec<u32>,
    /// The bitmap stride in `u32` items.
    pub color_bitmap_row_stride: usize,
    /// The distance between the start of each of the three bitmaps.
    pub color_bitmap_depth_stride: usize,
    /// One generation per colour bitmap; one means "redraw it".
    pub color_bitmap_generations: [u64; 3],
    /// The cursor rectangle, in columns and rows.
    pub cursor_rect: (i32, i32, i32, i32),
    /// The dirty viewport area in pixels; may extend past the target.
    pub dirty_rect_in_px: (i32, i32, i32, i32),
    /// The invalidated rows, as a half-open range.
    pub invalidated_rows: (u16, u16),
    /// The horizontal scroll offset in columns.
    pub scroll_offset_x: i32,
    /// The vertical scroll delta in pixels.
    pub scroll_delta_y: i16,
}

impl Default for RenderingPayload {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            rows: Vec::new(),
            color_bitmap: Vec::new(),
            color_bitmap_row_stride: 0,
            color_bitmap_depth_stride: 0,
            // A generation of 1 forces the first Present to upload the bitmaps.
            color_bitmap_generations: [1, 1, 1],
            cursor_rect: (0, 0, 0, 0),
            dirty_rect_in_px: (0, 0, 0, 0),
            invalidated_rows: (0, 0),
            scroll_offset_x: 0,
            scroll_delta_y: 0,
        }
    }
}

impl RenderingPayload {
    /// Marks the whole target and every row dirty and clears the scroll delta.
    pub fn mark_all_as_dirty(&mut self) {
        let (width, height) = self.settings.target_size;
        self.dirty_rect_in_px = (0, 0, i32::from(width), i32::from(height));
        self.invalidated_rows = (0, self.settings.viewport_cell_count.1);
        self.scroll_delta_y = 0;
    }
}

/// The pixel rectangle of one cell, as `(left, top, right, bottom)`.
pub fn cell_rect(column: u16, row: u16, cell_width: u32, cell_height: u32) -> (i32, i32, i32, i32) {
    let left = i32::from(column) * cell_width as i32;
    let top = i32::from(row) * cell_height as i32;
    (left, top, left + cell_width as i32, top + cell_height as i32)
}

/// The `(background, foreground)` colours of one cell, as `0xAARRGGBB`.
///
/// A cell outside the colour bitmap keeps the engine's black-on-white default,
/// which is what a backend painted before this was shared.
pub fn cell_colors(payload: &RenderingPayload, column: u16, row: u16) -> (u32, u32) {
    const BLACK: u32 = 0xFF00_0000;
    const WHITE: u32 = 0xFFFF_FFFF;

    let stride = payload.color_bitmap_row_stride;
    let depth = payload.color_bitmap_depth_stride;
    let Some(index) = (row as usize)
        .checked_mul(stride)
        .and_then(|base| base.checked_add(column as usize))
    else {
        return (BLACK, WHITE);
    };
    if index >= depth {
        return (BLACK, WHITE);
    }
    let background = payload.color_bitmap.get(index).copied().unwrap_or(BLACK);
    let foreground = payload.color_bitmap.get(depth + index).copied().unwrap_or(WHITE);
    (background, foreground)
}

/// The text baseline in pixels from the top of a row.
pub fn baseline(payload: &RenderingPayload, cell_height: u32) -> i32 {
    if payload.settings.font.baseline == 0 {
        (cell_height * 3 / 4) as i32
    } else {
        i32::from(payload.settings.font.baseline)
    }
}

/// The pixel origin of a rasterised glyph inside a cell.
pub fn glyph_origin(
    column: u16,
    row: u16,
    cell_width: u32,
    cell_height: u32,
    baseline: i32,
    bitmap_left: i32,
    bitmap_top: i32,
) -> (i32, i32) {
    (
        i32::from(column) * cell_width as i32 + bitmap_left,
        i32::from(row) * cell_height as i32 + baseline + bitmap_top,
    )
}

/// A solid rectangle a backend fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolidRect {
    /// Left edge in pixels.
    pub left: i32,
    /// Top edge in pixels.
    pub top: i32,
    /// Right edge, exclusive.
    pub right: i32,
    /// Bottom edge, exclusive.
    pub bottom: i32,
    /// The fill, `0xAARRGGBB`.
    pub color: u32,
}

/// The grid lines of one row, as solid rectangles.
pub fn grid_line_rects(
    payload: &RenderingPayload,
    row_index: u16,
    cell_width: u32,
    cell_height: u32,
) -> Vec<SolidRect> {
    let top = i32::from(row_index) * cell_height as i32;
    let bottom = top + cell_height as i32 - 1;
    let underline = bottom - 1;
    let middle = top + cell_height as i32 / 2;
    let mut rects = Vec::new();

    let Some(row) = payload.rows.get(row_index as usize) else {
        return rects;
    };

    for range in &row.grid_line_ranges {
        let left = i32::from(range.from) * cell_width as i32;
        let right = i32::from(range.to) * cell_width as i32;
        if left >= right {
            continue;
        }

        let (_, foreground) = cell_colors(payload, range.from, row_index);
        let mut push = |l: i32, t: i32, r: i32, b: i32| {
            if l < r && t < b {
                rects.push(SolidRect {
                    left: l,
                    top: t,
                    right: r,
                    bottom: b,
                    color: foreground,
                });
            }
        };

        if range.lines.contains(GridLines::Top) {
            push(left, top, right, top + 1);
        }
        if range.lines.contains(GridLines::Bottom) {
            push(left, bottom, right, bottom + 1);
        }
        if range.lines.contains(GridLines::Strikethrough) {
            push(left, middle, right, middle + 1);
        }
        if range.lines.contains(GridLines::Underline)
            || range.lines.contains(GridLines::DoubleUnderline)
            || range.lines.contains(GridLines::CurlyUnderline)
            || range.lines.contains(GridLines::DottedUnderline)
            || range.lines.contains(GridLines::DashedUnderline)
            || range.lines.contains(GridLines::HyperlinkUnderline)
        {
            push(left, underline, right, underline + 1);
        }

        for column in range.from..range.to {
            let cell_left = i32::from(column) * cell_width as i32;
            let cell_right = cell_left + cell_width as i32;
            if range.lines.contains(GridLines::Left) {
                push(cell_left, top, cell_left + 1, bottom + 1);
            }
            if range.lines.contains(GridLines::Right) {
                push(cell_right - 1, top, cell_right, bottom + 1);
            }
        }
    }

    rects
}

/// A graphics backend for the atlas engine.
///
/// Ported from `common.h`'s `IBackend`. The backend owns its device and surface;
/// this trait only hands it the per-frame [`RenderingPayload`].
pub trait IBackend {
    /// Releases device and surface resources.
    fn release_resources(&mut self);

    /// Renders one frame.
    fn render(&mut self, payload: &mut RenderingPayload);

    /// Returns whether the backend needs an immediate second frame.
    fn requires_continuous_redraw(&mut self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoration_positions_default_to_zero() {
        let position = FontDecorationPosition::default();
        assert_eq!(position.position, 0);
        assert_eq!(position.height, 0);
    }

    #[test]
    fn a_grid_line_range_knows_when_it_is_empty() {
        let empty = GridLineRange {
            lines: GridLineSet::new(),
            from: 4,
            to: 4,
        };
        assert!(empty.is_empty());

        let mut lines = GridLineSet::new();
        lines.insert(crate::render_frame_info::GridLines::Underline);
        let range = GridLineRange { lines, from: 2, to: 6 };
        assert!(!range.is_empty());
        assert!(range.lines.contains(crate::render_frame_info::GridLines::Underline));
    }

    #[test]
    fn a_default_bitmap_is_empty() {
        let bitmap = Bitmap::default();
        assert!(bitmap.is_empty());
        assert!(bitmap.source.is_empty());
        assert_eq!(bitmap.source_size, (0, 0));
    }

    #[test]
    fn cursor_settings_match_upstream_defaults() {
        let cursor = CursorSettings::default();
        assert_eq!(cursor.cursor_color, 0xFFFF_FFFF);
        assert_eq!(cursor.cursor_type, 0);
        assert_eq!(cursor.height_percentage, 20);
    }

    #[test]
    fn miscellaneous_settings_match_upstream_defaults() {
        let misc = MiscellaneousSettings::new();
        assert_eq!(misc.background_color, 0);
        assert_eq!(misc.foreground_color, 0);
        assert_eq!(misc.selection_color, 0xFFFF_FFFF);
        assert_eq!(misc.selection_foreground, 0xFF00_0000);
        assert!(!misc.use_retro_terminal_effect);
    }

    #[test]
    fn font_relevant_attributes_combine() {
        let mut attributes = FontRelevantAttributes::NONE;
        assert!(!attributes.contains(FontRelevantAttributes::BOLD));

        attributes.insert(FontRelevantAttributes::BOLD);
        assert!(attributes.contains(FontRelevantAttributes::BOLD));
        assert!(!attributes.contains(FontRelevantAttributes::ITALIC));

        attributes.insert(FontRelevantAttributes::ITALIC);
        assert!(attributes.contains(FontRelevantAttributes::BOLD));
        assert!(attributes.contains(FontRelevantAttributes::ITALIC));
        assert_eq!(attributes.bits(), 0b11);
    }

    fn glyph(index: u16) -> ShapedGlyph {
        ShapedGlyph {
            glyph_index: index,
            advance: 1.0,
            offset: (0.0, 0.0),
            color: None,
        }
    }

    #[test]
    fn clearing_a_shaped_row_resets_it_and_marks_the_band_dirty() {
        let mut row = ShapedRow::default();
        row.push_glyphs(FontId(0), &[glyph(1), glyph(2)], 0);
        row.bitmap.active = true;

        row.clear(3, 16);

        assert_eq!(row.glyph_count(), 0);
        assert!(row.mappings.is_empty());
        assert!(row.glyph_columns.is_empty());
        assert!(!row.bitmap.active);
        assert_eq!(row.dirty_top, 48);
        assert_eq!(row.dirty_bottom, 64);
        assert_eq!(row.line_rendition, LineRendition::SingleWidth);
    }

    #[test]
    fn consecutive_glyphs_of_one_font_share_a_mapping() {
        let mut row = ShapedRow::default();
        row.push_glyphs(FontId(0), &[glyph(1), glyph(2)], 0);
        row.push_glyphs(FontId(0), &[glyph(3)], 2);

        assert_eq!(row.mappings.len(), 1);
        assert_eq!(row.mappings[0].font, FontId(0));
        assert_eq!(row.mappings[0].glyphs_from, 0);
        assert_eq!(row.mappings[0].glyphs_to, 3);
        assert_eq!(row.mappings[0].len(), 3);
    }

    #[test]
    fn a_font_change_starts_a_new_mapping() {
        let mut row = ShapedRow::default();
        row.push_glyphs(FontId(0), &[glyph(1)], 0);
        row.push_glyphs(FontId(1), &[glyph(2)], 1);

        assert_eq!(row.mappings.len(), 2);
        assert_eq!(row.mappings[0].glyphs_from, 0);
        assert_eq!(row.mappings[0].glyphs_to, 1);
        assert_eq!(row.mappings[1].glyphs_from, 1);
        assert_eq!(row.mappings[1].glyphs_to, 2);
    }

    #[test]
    fn glyph_columns_are_recorded() {
        let mut row = ShapedRow::default();
        row.push_glyphs(FontId(0), &[glyph(1), glyph(2)], 3);
        row.push_glyphs(FontId(0), &[glyph(3)], 5);

        assert_eq!(row.glyph_columns, [3, 3, 5]);
    }

    #[test]
    fn font_settings_defaults_match_upstream() {
        let font = FontSettings::default();
        assert_eq!(font.dpi, 96);
        assert!(font.color_glyphs);
        assert!(!font.builtin_glyphs);
        assert_eq!(font.antialiasing_mode, AntialiasingMode::ClearType);
        assert!(font.soft_font_pattern.is_empty());
    }

    #[test]
    fn settings_default_carries_the_misc_defaults() {
        let settings = Settings::default();
        assert_eq!(settings.misc.selection_color, 0xFFFF_FFFF);
        assert_eq!(settings.misc.selection_foreground, 0xFF00_0000);
        assert_eq!(settings.font.dpi, 96);
        assert_eq!(settings.target_size, (0, 0));
    }

    #[test]
    fn a_payload_starts_asking_for_a_bitmap_upload() {
        let payload = RenderingPayload::default();
        assert_eq!(payload.color_bitmap_generations, [1, 1, 1]);
        assert!(payload.rows.is_empty());
    }

    #[test]
    fn marking_all_dirty_covers_the_target_and_clears_the_scroll() {
        let mut payload = RenderingPayload::default();
        payload.settings.target_size = (120, 30);
        payload.settings.viewport_cell_count = (15, 3);
        payload.scroll_delta_y = -7;

        payload.mark_all_as_dirty();

        assert_eq!(payload.dirty_rect_in_px, (0, 0, 120, 30));
        assert_eq!(payload.invalidated_rows, (0, 3));
        assert_eq!(payload.scroll_delta_y, 0);
    }

    struct MockBackend {
        renders: usize,
        released: bool,
    }

    impl IBackend for MockBackend {
        fn release_resources(&mut self) {
            self.released = true;
        }
        fn render(&mut self, _payload: &mut RenderingPayload) {
            self.renders += 1;
        }
        fn requires_continuous_redraw(&mut self) -> bool {
            false
        }
    }

    #[test]
    fn the_backend_trait_is_object_safe() {
        let mut backend: Box<dyn IBackend> = Box::new(MockBackend {
            renders: 0,
            released: false,
        });
        let mut payload = RenderingPayload::default();

        backend.render(&mut payload);
        backend.release_resources();

        assert!(!backend.requires_continuous_redraw());
    }

    fn one_cell_payload(background: u32, foreground: u32) -> RenderingPayload {
        let mut payload = RenderingPayload::default();
        payload.settings.viewport_cell_count = (1, 1);
        payload.color_bitmap_row_stride = 1;
        payload.color_bitmap_depth_stride = 1;
        payload.color_bitmap = vec![background, foreground];
        payload.rows.push(ShapedRow::default());
        payload
    }

    #[test]
    fn a_cell_rect_is_the_cell_size_times_its_column_and_row() {
        assert_eq!(cell_rect(2, 3, 10, 20), (20, 60, 30, 80));
    }

    #[test]
    fn cell_colors_read_the_background_and_foreground_planes() {
        let payload = one_cell_payload(0xFF11_2233, 0xFF44_5566);
        assert_eq!(cell_colors(&payload, 0, 0), (0xFF11_2233, 0xFF44_5566));
    }

    #[test]
    fn a_cell_outside_the_colour_bitmap_keeps_the_default_colours() {
        let payload = one_cell_payload(0xFF11_2233, 0xFF44_5566);
        assert_eq!(cell_colors(&payload, 1, 0), (0xFF00_0000, 0xFFFF_FFFF));
    }

    #[test]
    fn an_underline_is_a_one_pixel_rect_at_the_bottom_of_the_cell() {
        let mut payload = one_cell_payload(0, 0xFF12_3456);
        let mut lines = GridLineSet::new();
        lines.insert(GridLines::Underline);
        payload.rows[0]
            .grid_line_ranges
            .push(GridLineRange { lines, from: 0, to: 1 });

        let rects = grid_line_rects(&payload, 0, 10, 20);

        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].left, 0);
        assert_eq!(rects[0].right, 10);
        assert_eq!(rects[0].bottom - rects[0].top, 1);
        assert_eq!(rects[0].color, 0xFF12_3456);
    }

    #[test]
    fn a_glyph_origin_sits_on_the_baseline() {
        assert_eq!(glyph_origin(2, 3, 10, 20, 15, 1, -8), (21, 67));
    }

    #[test]
    fn the_baseline_defaults_to_three_quarters_of_the_cell() {
        let payload = RenderingPayload::default();
        assert_eq!(baseline(&payload, 20), 15);
    }
}
