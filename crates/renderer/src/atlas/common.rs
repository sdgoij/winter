//! Portable data from the atlas engine's `common.h`.
//!
//! The D2D/D3D/DWrite members are left to the backend modules; these are the
//! value types the engine and its tests share.

use crate::render_frame_info::GridLineSet;
use types::LineRendition;

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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
        self.glyph_advances.clear();
        self.glyph_offsets.clear();
        self.colors.clear();
        self.bitmap.active = false;
        self.grid_line_ranges.clear();
        self.line_rendition = LineRendition::SingleWidth;
        self.dirty_top = y * cell_height;
        self.dirty_bottom = self.dirty_top + cell_height;
    }

    /// Appends shaped glyphs for `font`, extending the last run when it matches.
    pub fn push_glyphs(&mut self, font: FontId, glyphs: &[ShapedGlyph]) {
        if glyphs.is_empty() {
            return;
        }

        let from = self.glyph_indices.len();
        for glyph in glyphs {
            self.glyph_indices.push(glyph.glyph_index);
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
        row.push_glyphs(FontId(0), &[glyph(1), glyph(2)]);
        row.bitmap.active = true;

        row.clear(3, 16);

        assert_eq!(row.glyph_count(), 0);
        assert!(row.mappings.is_empty());
        assert!(!row.bitmap.active);
        assert_eq!(row.dirty_top, 48);
        assert_eq!(row.dirty_bottom, 64);
        assert_eq!(row.line_rendition, LineRendition::SingleWidth);
    }

    #[test]
    fn consecutive_glyphs_of_one_font_share_a_mapping() {
        let mut row = ShapedRow::default();
        row.push_glyphs(FontId(0), &[glyph(1), glyph(2)]);
        row.push_glyphs(FontId(0), &[glyph(3)]);

        assert_eq!(row.mappings.len(), 1);
        assert_eq!(row.mappings[0].font, FontId(0));
        assert_eq!(row.mappings[0].glyphs_from, 0);
        assert_eq!(row.mappings[0].glyphs_to, 3);
        assert_eq!(row.mappings[0].len(), 3);
    }

    #[test]
    fn a_font_change_starts_a_new_mapping() {
        let mut row = ShapedRow::default();
        row.push_glyphs(FontId(0), &[glyph(1)]);
        row.push_glyphs(FontId(1), &[glyph(2)]);

        assert_eq!(row.mappings.len(), 2);
        assert_eq!(row.mappings[0].glyphs_from, 0);
        assert_eq!(row.mappings[0].glyphs_to, 1);
        assert_eq!(row.mappings[1].glyphs_from, 1);
        assert_eq!(row.mappings[1].glyphs_to, 2);
    }
}
