//! `TextColor` — terminal text color with default/indexed/RGB support.
//!
//! Ported from `TextColor` in the Windows Terminal C++ buffer module.

use crate::color_table::ColorTable;

/// Represents the type of a color in a TextColor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorType {
    /// Use the terminal's default color for this component.
    Default,
    /// 16-color palette index.
    Index16(u8),
    /// 256-color palette index (XTerm compatible).
    Index256(u8),
    /// Direct RGB color.
    Rgb { r: u8, g: u8, b: u8 },
}

impl std::fmt::Display for ColorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ColorType::Default => write!(f, "default"),
            ColorType::Index16(i) => write!(f, "index16:{}", i),
            ColorType::Index256(i) => write!(f, "index256:{}", i),
            ColorType::Rgb { r, g, b } => write!(f, "rgb:{:02x}{:02x}{:02x}", r, g, b),
        }
    }
}

/// A terminal text color that can be default, indexed (16-color or 256-color), or RGB.
///
/// Ported from `TextColor` in the Windows Terminal C++ buffer module.
/// Stores color information compactly with a union-like layout matching the C++ implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextColor {
    /// The color type discriminator.
    color_type: ColorType,
}

impl TextColor {
    // Standard 16-color palette constants (dark palette indices)
    pub const DARK_BLACK: Self = Self::index16(0);
    pub const DARK_RED: Self = Self::index16(1);
    pub const DARK_GREEN: Self = Self::index16(2);
    pub const DARK_YELLOW: Self = Self::index16(3);
    pub const DARK_BLUE: Self = Self::index16(4);
    pub const DARK_MAGENTA: Self = Self::index16(5);
    pub const DARK_CYAN: Self = Self::index16(6);
    pub const DARK_WHITE: Self = Self::index16(7);
    pub const BRIGHT_BLACK: Self = Self::index16(8);
    pub const BRIGHT_RED: Self = Self::index16(9);
    pub const BRIGHT_GREEN: Self = Self::index16(10);
    pub const BRIGHT_YELLOW: Self = Self::index16(11);
    pub const BRIGHT_BLUE: Self = Self::index16(12);
    pub const BRIGHT_MAGENTA: Self = Self::index16(13);
    pub const BRIGHT_CYAN: Self = Self::index16(14);
    pub const BRIGHT_WHITE: Self = Self::index16(15);

    // Special color indices (> 256)
    pub const DEFAULT_FOREGROUND: Self = Self::default_color(261);
    pub const DEFAULT_BACKGROUND: Self = Self::default_color(262);
    pub const FRAME_FOREGROUND: Self = Self::default_color(263);
    pub const FRAME_BACKGROUND: Self = Self::default_color(264);
    pub const CURSOR_COLOR: Self = Self::default_color(265);
    pub const SELECTION_BACKGROUND: Self = Self::default_color(266);

    /// Size of the color table (16 standard + 240 indexed + special colors).
    pub const TABLE_SIZE: usize = 267;

    /// Creates a default color.
    pub const fn new() -> Self {
        TextColor {
            color_type: ColorType::Default,
        }
    }

    /// Creates a 16-color indexed color.
    pub const fn index16(index: u8) -> Self {
        TextColor {
            color_type: ColorType::Index16(index),
        }
    }

    /// Creates a 256-color indexed color.
    pub const fn index256(index: u8) -> Self {
        TextColor {
            color_type: ColorType::Index256(index),
        }
    }

    /// Creates an RGB color.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        TextColor {
            color_type: ColorType::Rgb { r, g, b },
        }
    }

    /// Creates a special default color with the given index.
    pub const fn default_color(index: usize) -> Self {
        // Use Index256 range for special colors (256+)
        TextColor {
            color_type: ColorType::Index256((index & 0xFF) as u8),
        }
    }

    /// Returns the color type.
    pub const fn get_type(&self) -> ColorType {
        self.color_type
    }

    /// Returns true if this is a default color.
    pub const fn is_default(&self) -> bool {
        matches!(self.color_type, ColorType::Default)
    }

    /// Returns true if this is a 16-color indexed color.
    pub const fn is_index16(&self) -> bool {
        matches!(self.color_type, ColorType::Index16(_))
    }

    /// Returns true if this is a 256-color indexed color.
    pub const fn is_index256(&self) -> bool {
        matches!(self.color_type, ColorType::Index256(_))
    }

    /// Returns true if this is an RGB color.
    pub const fn is_rgb(&self) -> bool {
        matches!(self.color_type, ColorType::Rgb { .. })
    }

    /// Returns true if this is either default or indexed (not RGB).
    pub const fn is_default_or_legacy(&self) -> bool {
        !self.is_rgb()
    }

    /// Returns true if the color can be brightened.
    pub const fn can_be_brightened(&self) -> bool {
        matches!(self.color_type, ColorType::Index16(_))
    }

    /// Returns the color index (for indexed colors).
    pub const fn get_index(&self) -> u8 {
        match self.color_type {
            ColorType::Index16(i) | ColorType::Index256(i) => i,
            _ => 0,
        }
    }

    /// Returns the RGB components (for RGB colors).
    pub const fn get_r(&self) -> u8 {
        match self.color_type {
            ColorType::Rgb { r, .. } => r,
            _ => 0,
        }
    }

    pub const fn get_g(&self) -> u8 {
        match self.color_type {
            ColorType::Rgb { g, .. } => g,
            _ => 0,
        }
    }

    pub const fn get_b(&self) -> u8 {
        match self.color_type {
            ColorType::Rgb { b, .. } => b,
            _ => 0,
        }
    }

    /// Returns the RGB color as a u32 (0x00RRGGBB).
    pub const fn get_rgb(&self) -> u32 {
        match self.color_type {
            ColorType::Rgb { r, g, b } => ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
            _ => 0,
        }
    }

    /// Returns the RGB color in upstream's `COLORREF` packing (`0x00BBGGRR`).
    pub const fn get_colorref(&self) -> u32 {
        match self.color_type {
            ColorType::Rgb { r, g, b } => (r as u32) | ((g as u32) << 8) | ((b as u32) << 16),
            _ => 0,
        }
    }

    /// Builds an RGB color from a `COLORREF` (`0x00BBGGRR`).
    pub const fn from_colorref(color: u32) -> Self {
        Self::rgb(color as u8, (color >> 8) as u8, (color >> 16) as u8)
    }

    /// Resolves this color against `table`, returning a `COLORREF`.
    ///
    /// Ported from `TextColor::GetColor`. A default color reads the entry its
    /// alias points at; when `brighten` is set and that entry matches one of the
    /// eight dark entries, its bright counterpart is used instead.
    pub fn get_color(&self, table: &ColorTable, default_index: usize, brighten: bool) -> u32 {
        let entry = |index: usize| table.get(index).map_or(0, |color| color.get_colorref());

        if self.is_default() {
            let default_color = entry(default_index);
            if brighten {
                for index in 0..8 {
                    if entry(index) == default_color {
                        return entry(index + 8);
                    }
                }
            }
            default_color
        } else if self.is_rgb() {
            self.get_colorref()
        } else if self.is_index16() && brighten {
            entry(usize::from(self.get_index() | 8))
        } else {
            entry(usize::from(self.get_index()))
        }
    }

    /// Transposes a legacy index from Windows order to ANSI order.
    /// Swaps bits 0 and 2 for compatibility.
    pub const fn transpose_legacy_index(index: u8) -> u8 {
        let one_bit_set = (index ^ (index >> 2)) & 1;
        index ^ one_bit_set ^ (one_bit_set << 2)
    }
}

impl Default for TextColor {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TextColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.color_type {
            ColorType::Default => write!(f, "{{default}}"),
            ColorType::Index16(i) => write!(f, "{{index16:{:02x}}}", i),
            ColorType::Index256(i) => write!(f, "{{index256:{:02x}}}", i),
            ColorType::Rgb { r, g, b } => write!(f, "{{rgb:{:02x}{:02x}{:02x}}}", r, g, b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_color() {
        let c = TextColor::new();
        assert!(c.is_default());
        assert_eq!(c.get_type(), ColorType::Default);
    }

    #[test]
    fn test_index16_color() {
        let c = TextColor::index16(7);
        assert!(c.is_index16());
        assert_eq!(c.get_index(), 7);
    }

    #[test]
    fn test_index256_color() {
        let c = TextColor::index256(200);
        assert!(c.is_index256());
        assert_eq!(c.get_index(), 200);
    }

    #[test]
    fn test_rgb_color() {
        let c = TextColor::rgb(255, 128, 64);
        assert!(c.is_rgb());
        assert_eq!(c.get_r(), 255);
        assert_eq!(c.get_g(), 128);
        assert_eq!(c.get_b(), 64);
        assert_eq!(c.get_rgb(), 0xFF8040);
    }

    #[test]
    fn test_colorref_packing_round_trips() {
        let color = TextColor::rgb(0x12, 0x34, 0x56);
        assert_eq!(color.get_colorref(), 0x0056_3412);
        assert_eq!(TextColor::from_colorref(0x0056_3412), color);
    }

    #[test]
    fn test_get_color_resolves_each_kind() {
        let table = ColorTable::new();

        let rgb = TextColor::rgb(1, 2, 3);
        assert_eq!(rgb.get_color(&table, 0, false), 0x0003_0201);

        let dark_red = table.get(1).unwrap().get_colorref();
        assert_eq!(TextColor::index16(1).get_color(&table, 0, false), dark_red);

        let bright_red = table.get(9).unwrap().get_colorref();
        assert_eq!(TextColor::index16(1).get_color(&table, 0, true), bright_red);

        let dark_white = table.get(7).unwrap().get_colorref();
        assert_eq!(TextColor::new().get_color(&table, 7, false), dark_white);
    }

    #[test]
    fn test_get_color_brightens_an_alias_that_matches_a_dark_entry() {
        let mut table = ColorTable::new();
        let dark = table.get(0).unwrap();
        table.set(261, dark);

        let bright = table.get(8).unwrap().get_colorref();
        assert_eq!(TextColor::new().get_color(&table, 261, true), bright);
    }

    #[test]
    fn test_color_constants() {
        assert!(TextColor::DARK_BLACK.is_index16());
        assert!(TextColor::BRIGHT_WHITE.is_index16());
        assert_eq!(TextColor::DARK_BLACK.get_index(), 0);
        assert_eq!(TextColor::BRIGHT_WHITE.get_index(), 15);
    }

    #[test]
    fn test_transpose_legacy_index() {
        // Index 0 should stay 0
        assert_eq!(TextColor::transpose_legacy_index(0), 0);
        // Index 1 should become 4 (swap bit 0 and 2)
        assert_eq!(TextColor::transpose_legacy_index(1), 4);
        // Index 4 should become 1
        assert_eq!(TextColor::transpose_legacy_index(4), 1);
    }

    #[test]
    fn test_display() {
        let c1 = TextColor::new();
        assert!(format!("{}", c1).contains("default"));

        let c2 = TextColor::rgb(255, 0, 128);
        assert!(format!("{}", c2).contains("rgb"));
    }
}
