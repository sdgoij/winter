//! `ColorTable` — 16-color palette management.
//!
//! Ported from colorTable.cpp in the Windows Terminal C++ codebase.

use crate::text_color::TextColor;

/// A 16-color palette for terminal color selection.
///
/// Stores colors in the standard 16-color format (index 0-15)
/// with support for default, indexed, and RGB color types.
#[derive(Debug, Clone)]
pub struct ColorTable {
    colors: [TextColor; 16],
}

impl ColorTable {
    /// Creates a new color table with the standard dark palette.
    pub const fn default_dark() -> Self {
        ColorTable {
            colors: [
                TextColor::index16(0),  // Black
                TextColor::index16(1),  // Red
                TextColor::index16(2),  // Green
                TextColor::index16(3),  // Yellow
                TextColor::index16(4),  // Blue
                TextColor::index16(5),  // Magenta
                TextColor::index16(6),  // Cyan
                TextColor::index16(7),  // White
                TextColor::index16(8),  // Bright Black
                TextColor::index16(9),  // Bright Red
                TextColor::index16(10), // Bright Green
                TextColor::index16(11), // Bright Yellow
                TextColor::index16(12), // Bright Blue
                TextColor::index16(13), // Bright Magenta
                TextColor::index16(14), // Bright Cyan
                TextColor::index16(15), // Bright White
            ],
        }
    }

    /// Creates a new color table with the standard light palette.
    pub const fn default_light() -> Self {
        ColorTable {
            colors: [
                TextColor::index16(0),  // Black
                TextColor::index16(1),  // Red
                TextColor::index16(2),  // Green
                TextColor::index16(3),  // Yellow
                TextColor::index16(4),  // Blue
                TextColor::index16(5),  // Magenta
                TextColor::index16(6),  // Cyan
                TextColor::index16(7),  // White
                TextColor::index16(8),  // Bright Black
                TextColor::index16(9),  // Bright Red
                TextColor::index16(10), // Bright Green
                TextColor::index16(11), // Bright Yellow
                TextColor::index16(12), // Bright Blue
                TextColor::index16(13), // Bright Magenta
                TextColor::index16(14), // Bright Cyan
                TextColor::index16(15), // Bright White
            ],
        }
    }

    /// Returns the color at the given index.
    pub const fn get(&self, index: u8) -> TextColor {
        if index < 16 {
            self.colors[index as usize]
        } else {
            TextColor::new()
        }
    }

    /// Sets the color at the given index.
    pub fn set(&mut self, index: u8, color: TextColor) {
        if index < 16 {
            self.colors[index as usize] = color;
        }
    }

    /// Resets all colors to their default values.
    pub fn reset(&mut self) {
        *self = Self::default_dark();
    }

    /// Returns the number of colors in the table.
    pub const fn len(&self) -> usize {
        16
    }

    /// Returns whether the table is empty (always false).
    pub const fn is_empty(&self) -> bool {
        false
    }
}

impl Default for ColorTable {
    fn default() -> Self {
        Self::default_dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_dark() {
        let table = ColorTable::default_dark();
        assert_eq!(table.len(), 16);
        assert!(!table.is_empty());
    }

    #[test]
    fn test_get_color() {
        let table = ColorTable::default_dark();
        let black = table.get(0);
        let white = table.get(15);
        let default_color = table.get(255);

        assert!(black.is_index16());
        assert!(white.is_index16());
        assert!(default_color.is_default());
    }

    #[test]
    fn test_set_color() {
        let mut table = ColorTable::default_dark();
        let custom = TextColor::rgb(255, 128, 64);
        table.set(5, custom);

        assert_eq!(table.get(5), custom);
        // Other colors unchanged
        assert!(table.get(0).is_index16());
    }

    #[test]
    fn test_reset() {
        let mut table = ColorTable::default_dark();
        table.set(0, TextColor::rgb(1, 2, 3));
        assert_eq!(table.get(0), TextColor::rgb(1, 2, 3));

        table.reset();
        assert!(table.get(0).is_index16());
    }

    #[test]
    fn test_default_light() {
        let table = ColorTable::default_light();
        assert_eq!(table.len(), 16);
    }
}
