//! Portable data from the atlas engine's `common.h`.
//!
//! The D2D/D3D/DWrite members are left to the backend modules; these are the
//! value types the engine and its tests share.

use crate::render_frame_info::GridLineSet;

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
}
