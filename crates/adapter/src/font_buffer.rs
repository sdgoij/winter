//! `FontBuffer` — Font tracking for terminal rendering.
//!
//! Ported from `FontBuffer.hpp/cpp` in the Windows Terminal C++ codebase.
//! Manages font information and caches for terminal display.

/// Maximum number of fonts to track.
const MAX_FONTS: usize = 16;

/// Information about a single font.
#[derive(Debug, Clone)]
pub struct FontInfo {
    /// Font face name.
    pub face_name: String,
    /// Font size in points.
    pub size: f32,
    /// Font weight (100-900).
    pub weight: u16,
    /// Whether the font is italic.
    pub italic: bool,
    /// Character set.
    pub charset: u32,
    /// Pitch and family.
    pub pitch_and_family: u32,
}

impl FontInfo {
    /// Creates a new FontInfo.
    pub fn new(
        face_name: impl Into<String>,
        size: f32,
        weight: u16,
        italic: bool,
        charset: u32,
        pitch_and_family: u32,
    ) -> Self {
        Self {
            face_name: face_name.into(),
            size,
            weight,
            italic,
            charset,
            pitch_and_family,
        }
    }

    /// Returns whether this font info is equal to another (ignoring identity).
    pub fn matches(&self, other: &Self) -> bool {
        self.face_name == other.face_name
            && (self.size - other.size).abs() < f32::EPSILON
            && self.weight == other.weight
            && self.italic == other.italic
            && self.charset == other.charset
            && self.pitch_and_family == other.pitch_and_family
    }
}

/// Font buffer for tracking and caching fonts.
#[derive(Debug)]
pub struct FontBuffer {
    /// Tracked fonts.
    fonts: Vec<FontInfo>,
    /// Current font index.
    current_index: usize,
}

impl FontBuffer {
    /// Creates a new FontBuffer.
    pub fn new() -> Self {
        Self {
            fonts: Vec::new(),
            current_index: 0,
        }
    }

    /// Adds a font to the buffer.
    /// Returns the index of the font (or existing index if it matches).
    pub fn add_font(&mut self, font_info: FontInfo) -> usize {
        // Check if font already exists
        for (i, existing) in self.fonts.iter().enumerate() {
            if existing.matches(&font_info) {
                return i;
            }
        }

        // Add new font if we haven't reached the limit
        if self.fonts.len() < MAX_FONTS {
            let index = self.fonts.len();
            self.fonts.push(font_info);
            index
        } else {
            // Replace the oldest font (FIFO)
            self.fonts[self.current_index] = font_info;
            let index = self.current_index;
            self.current_index = (self.current_index + 1) % MAX_FONTS;
            index
        }
    }

    /// Gets a font by index.
    pub fn get_font(&self, index: usize) -> Option<&FontInfo> {
        self.fonts.get(index)
    }

    /// Sets the current font.
    pub fn set_current_font(&mut self, index: usize) -> Result<(), FontError> {
        if index < self.fonts.len() {
            self.current_index = index;
            Ok(())
        } else {
            Err(FontError::IndexOutOfBounds {
                index,
                max: self.fonts.len(),
            })
        }
    }

    /// Gets the current font.
    pub fn current_font(&self) -> Option<&FontInfo> {
        if self.fonts.is_empty() {
            None
        } else {
            self.fonts.get(self.current_index)
        }
    }

    /// Returns the number of fonts in the buffer.
    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    /// Returns whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    /// Clears all fonts.
    pub fn clear(&mut self) {
        self.fonts.clear();
        self.current_index = 0;
    }

    /// Returns the current font index.
    pub fn current_index(&self) -> usize {
        self.current_index
    }
}

impl Default for FontBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Font-related errors.
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    /// Index out of bounds.
    #[error("Font index {index} out of bounds (max: {max})")]
    IndexOutOfBounds { index: usize, max: usize },
    /// Font not found.
    #[error("Font not found")]
    NotFound,
    /// Font buffer is empty.
    #[error("Font buffer is empty")]
    Empty,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_font_buffer_new() {
        let buffer = FontBuffer::new();
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert!(buffer.current_font().is_none());
    }

    #[test]
    fn test_add_font() {
        let mut buffer = FontBuffer::new();
        let font = FontInfo::new("Consolas", 12.0, 400, false, 0, 0);
        let index = buffer.add_font(font);
        assert_eq!(index, 0);
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn test_add_duplicate_font() {
        let mut buffer = FontBuffer::new();
        let font1 = FontInfo::new("Consolas", 12.0, 400, false, 0, 0);
        let index1 = buffer.add_font(font1);

        let font2 = FontInfo::new("Consolas", 12.0, 400, false, 0, 0);
        let index2 = buffer.add_font(font2);

        // Should return same index for duplicate
        assert_eq!(index1, index2);
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn test_set_current_font() {
        let mut buffer = FontBuffer::new();
        let font1 = FontInfo::new("Font1", 12.0, 400, false, 0, 0);
        let font2 = FontInfo::new("Font2", 14.0, 400, false, 0, 0);
        buffer.add_font(font1);
        buffer.add_font(font2);

        buffer.set_current_font(1).unwrap();
        assert_eq!(buffer.current_index(), 1);
        assert_eq!(buffer.current_font().unwrap().face_name, "Font2");
    }

    #[test]
    fn test_set_current_font_out_of_bounds() {
        let mut buffer = FontBuffer::new();
        let result = buffer.set_current_font(5);
        assert!(result.is_err());
    }

    #[test]
    fn test_clear() {
        let mut buffer = FontBuffer::new();
        buffer.add_font(FontInfo::new("Font1", 12.0, 400, false, 0, 0));
        buffer.add_font(FontInfo::new("Font2", 14.0, 400, false, 0, 0));
        assert_eq!(buffer.len(), 2);

        buffer.clear();
        assert!(buffer.is_empty());
        assert_eq!(buffer.current_index(), 0);
    }
}
