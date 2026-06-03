//! `OutputCell` — the fundamental cell type in the terminal buffer.
//!
//! Ported from `OutputCell` in the Windows Terminal C++ buffer module.
//! Each cell stores a character (or sequence of characters) and its associated
//! text attributes. The `TextAttributeBehavior` determines how attributes are
//! stored and applied.

use types::TextColor;
use types::codepoint_width;

use crate::text_attribute::TextAttribute;
use crate::text_attribute_behavior::TextAttributeBehavior;

/// The behavior for storing text attributes in an OutputCell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellBehavior {
    /// Use the stored text attribute directly.
    #[default]
    Stored,
    /// Use the text attribute of the cell being written to.
    Current,
    /// Only use the stored text attribute and skip insertion of other attributes.
    StoredOnly,
}

impl CellBehavior {
    /// Returns whether this behavior stores attributes.
    pub const fn stores_attributes(&self) -> bool {
        matches!(self, Self::Stored | Self::StoredOnly)
    }
}

/// A single cell in the terminal buffer.
///
/// Ported from `OutputCell` in the Windows Terminal C++ buffer module.
/// Stores character data (as a fixed-size UTF-8 buffer) and associated
/// text attributes that determine how the cell is rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputCell {
    /// Character data stored as a fixed-size UTF-8 buffer (up to 4 chars).
    chars: [u8; 4],
    /// Length of valid character data in bytes.
    chars_len: u8,
    /// The text attributes for this cell.
    attributes: TextAttribute,
    /// How attributes should be applied when rendering.
    behavior: CellBehavior,
}

impl OutputCell {
    /// Maximum number of characters per cell.
    pub const MAX_CHARS: usize = 4;

    /// Maximum UTF-8 byte length for a cell (4 bytes for a single UTF-8 char).
    pub const MAX_CHARS_LEN: usize = 4;

    /// Creates a default (empty) output cell.
    pub fn new() -> Self {
        Self {
            chars: [0; Self::MAX_CHARS],
            chars_len: 0,
            attributes: TextAttribute::new(),
            behavior: CellBehavior::Stored,
        }
    }

    /// Creates a default (empty) output cell (const version).
    pub const fn new_const() -> Self {
        Self {
            chars: [0; Self::MAX_CHARS],
            chars_len: 0,
            attributes: TextAttribute::new(),
            behavior: CellBehavior::Stored,
        }
    }

    /// Creates an output cell from a string slice.
    pub fn from_str(s: &str) -> Self {
        let mut cell = Self::new();
        let bytes = s.as_bytes();
        let len = bytes.len().min(Self::MAX_CHARS);
        cell.chars[..len].copy_from_slice(&bytes[..len]);
        cell.chars_len = len as u8;
        cell
    }

    /// Creates an output cell from a single character.
    pub fn from_char(c: char) -> Self {
        let mut buf = [0u8; 4];
        let encoded = c.encode_utf8(&mut buf);
        let len = encoded.len();
        let mut cell = Self::new();
        cell.chars[..len].copy_from_slice(&buf[..len]);
        cell.chars_len = len as u8;
        cell
    }

    /// Creates an output cell with the given attributes.
    pub fn from_str_with_attrs(s: &str, attrs: TextAttribute) -> Self {
        let mut cell = Self::from_str(s);
        cell.set_attributes(attrs);
        cell
    }

    /// Creates an output cell with the given behavior.
    pub fn with_behavior(mut self, behavior: CellBehavior) -> Self {
        self.behavior = behavior;
        self
    }

    /// Returns the character data as a string slice.
    pub fn chars(&self) -> &str {
        std::str::from_utf8(&self.chars[..self.chars_len as usize]).unwrap_or("")
    }

    /// Returns the character data as a byte slice.
    pub fn chars_bytes(&self) -> &[u8] {
        &self.chars[..self.chars_len as usize]
    }

    /// Returns the number of display columns this cell occupies.
    ///
    /// For most characters this is 1. CJK and some emoji characters occupy 2 columns.
    pub fn column_width(&self) -> u8 {
        if self.chars_len == 0 {
            return 0;
        }
        // Get the first character and determine its width.
        if let Ok(s) = std::str::from_utf8(&self.chars[..self.chars_len as usize]) {
            if let Some(c) = s.chars().next() {
                return codepoint_width(c as u32);
            }
        }
        1
    }

    /// Returns the text attributes.
    pub const fn get_attributes(&self) -> &TextAttribute {
        &self.attributes
    }

    /// Returns a mutable reference to the text attributes.
    pub fn get_attributes_mut(&mut self) -> &mut TextAttribute {
        &mut self.attributes
    }

    /// Sets the text attributes.
    pub fn set_attributes(&mut self, attrs: TextAttribute) {
        self.attributes = attrs;
    }

    /// Returns the cell behavior.
    pub const fn get_behavior(&self) -> CellBehavior {
        self.behavior
    }

    /// Sets the cell behavior.
    pub fn set_behavior(&mut self, behavior: CellBehavior) {
        self.behavior = behavior;
    }

    /// Returns the color type used for foreground.
    pub fn foreground_color(&self) -> TextColor {
        self.attributes.get_foreground()
    }

    /// Returns the color type used for background.
    pub fn background_color(&self) -> TextColor {
        self.attributes.get_background()
    }

    /// Returns whether this cell is empty (no characters).
    pub fn is_empty(&self) -> bool {
        self.chars_len == 0
    }

    /// Returns whether this cell is a whitespace character.
    pub fn is_whitespace(&self) -> bool {
        let s = self.chars();
        s.chars().all(|c| c.is_whitespace())
    }

    /// Returns whether this cell contains a wide (double-width) character.
    pub fn is_wide_lead(&self) -> bool {
        self.column_width() == 2
    }

    /// Returns the behavior type for attribute application.
    pub fn get_attribute_behavior(&self) -> TextAttributeBehavior {
        match self.behavior {
            CellBehavior::Stored => TextAttributeBehavior::Replace,
            CellBehavior::Current => TextAttributeBehavior::Additive,
            CellBehavior::StoredOnly => TextAttributeBehavior::Subtractive,
        }
    }
}

impl Default for OutputCell {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for OutputCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.chars())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_cell_is_empty() {
        let cell = OutputCell::new();
        assert!(cell.is_empty());
        assert_eq!(cell.chars(), "");
        assert_eq!(cell.column_width(), 0);
    }

    #[test]
    fn test_from_char() {
        let cell = OutputCell::from_char('A');
        assert!(!cell.is_empty());
        assert_eq!(cell.chars(), "A");
        assert_eq!(cell.column_width(), 1);
    }

    #[test]
    fn test_from_str() {
        let cell = OutputCell::from_str("Hello");
        assert!(!cell.is_empty());
        assert_eq!(cell.chars(), "Hell"); // truncated to MAX_CHARS
        assert_eq!(cell.chars().len(), 4);
    }

    #[test]
    fn test_from_str_with_attrs() {
        let attrs = TextAttribute::new();
        let cell = OutputCell::from_str_with_attrs("test", attrs);
        assert_eq!(cell.chars(), "test");
    }

    #[test]
    fn test_cjk_width() {
        let cell = OutputCell::from_char('中');
        assert_eq!(cell.column_width(), 2);
        assert!(cell.is_wide_lead());
    }

    #[test]
    fn test_whitespace_detection() {
        let space = OutputCell::from_char(' ');
        assert!(space.is_whitespace());

        let tab = OutputCell::from_char('\t');
        assert!(tab.is_whitespace());

        let letter = OutputCell::from_char('a');
        assert!(!letter.is_whitespace());
    }

    #[test]
    fn test_behavior() {
        let cell = OutputCell::new().with_behavior(CellBehavior::Stored);
        assert!(cell.get_behavior().stores_attributes());

        // Current behavior does NOT store attributes (it uses current cell's attributes)
        let cell2 = OutputCell::new().with_behavior(CellBehavior::Current);
        assert!(!cell2.get_behavior().stores_attributes());

        let cell3 = OutputCell::new().with_behavior(CellBehavior::StoredOnly);
        assert!(cell3.get_behavior().stores_attributes());

        let cell4 = OutputCell::default();
        assert_eq!(cell4.get_behavior(), CellBehavior::Stored);
    }

    #[test]
    fn test_display_trait() {
        let cell = OutputCell::from_char('X');
        assert_eq!(format!("{}", cell), "X");
    }

    #[test]
    fn test_attribute_behavior_mapping() {
        let stored = OutputCell::new()
            .with_behavior(CellBehavior::Stored)
            .get_attribute_behavior();
        assert!(stored.is_replace());

        let current = OutputCell::new()
            .with_behavior(CellBehavior::Current)
            .get_attribute_behavior();
        assert!(current.is_additive());

        let stored_only = OutputCell::new()
            .with_behavior(CellBehavior::StoredOnly)
            .get_attribute_behavior();
        assert!(matches!(stored_only, TextAttributeBehavior::Subtractive));
    }

    #[test]
    fn test_combining_character() {
        // e + combining acute accent
        let cell = OutputCell::from_char('\u{0301}');
        assert_eq!(cell.column_width(), 0);
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::prop_compose;
    use super::*;

    prop_compose! {
        fn any_output_cell()(s in proptest::prelude::any::<String>()) -> OutputCell {
            OutputCell::from_str(&s)
        }
    }

    prop_compose! {
        fn any_cell_with_attrs()(s in proptest::prelude::any::<String>(), fg_rgb in proptest::prelude::any::<u32>(), bg_rgb in proptest::prelude::any::<u32>()) -> OutputCell {
            let attrs = TextAttribute::from_rgb(fg_rgb, bg_rgb, None);
            OutputCell::from_str_with_attrs(&s, attrs)
        }
    }

    #[test]
    fn test_chars_roundtrip() {
        // Test that creating a cell from a string preserves character data (within limits)
        let s = "Hell"; // 4 bytes, within MAX_CHARS
        let cell = OutputCell::from_str(s);
        assert_eq!(cell.chars(), s);

        // Test short strings
        let s = "Hi";
        let cell = OutputCell::from_str(s);
        assert_eq!(cell.chars(), s);

        // Test single char
        let s = "A";
        let cell = OutputCell::from_str(s);
        assert_eq!(cell.chars(), s);

        // Test very long strings (should be truncated to MAX_CHARS bytes)
        let long_s = "This is a very long string that should be truncated";
        let cell = OutputCell::from_str(long_s);
        assert!(cell.chars().len() <= OutputCell::MAX_CHARS);
    }

    #[test]
    fn test_from_char_preserves_char() {
        // Test ASCII
        let cell = OutputCell::from_char('A');
        assert_eq!(cell.chars(), "A");

        // Test non-ASCII (UTF-8 multi-byte)
        let cell = OutputCell::from_char('\u{4E2D}'); // 中
        assert_eq!(cell.chars(), "\u{4E2D}");

        // Test emoji
        let cell = OutputCell::from_char('\u{1F600}'); // 😠
        assert!(!cell.chars().is_empty());
    }

    #[test]
    fn test_equality_reflexive() {
        let cell = OutputCell::from_char('X');
        assert_eq!(cell, cell);

        let cell = OutputCell::from_str("test");
        assert_eq!(cell, cell);
    }

    #[test]
    fn test_equality_symmetric() {
        let c1 = OutputCell::from_str("test");
        let c2 = OutputCell::from_str("test");
        if c1 == c2 {
            assert_eq!(c2, c1);
        }

        // Test inequality
        let c1 = OutputCell::from_str("hello");
        let c2 = OutputCell::from_str("world");
        assert_ne!(c1, c2);
    }

    #[test]
    fn test_empty_cell_is_default() {
        let empty = OutputCell::new();
        let default = OutputCell::default();
        assert_eq!(empty, default);
        assert!(empty.is_empty());
    }

    #[test]
    fn test_truncation_at_max_chars() {
        let long_string = "abcdefghij"; // 10 chars = 10 bytes
        let cell = OutputCell::from_str(long_string);
        assert!(cell.chars_len <= OutputCell::MAX_CHARS_LEN as u8);

        // Test with multi-byte character
        let cell = OutputCell::from_char('\u{4E2D}'); // 中 (3 bytes in UTF-8)
        assert_eq!(cell.chars_len, 3);
        assert_eq!(cell.chars(), "\u{4E2D}");
    }

    #[test]
    fn test_column_width() {
        // ASCII
        let cell = OutputCell::from_char('A');
        assert_eq!(cell.column_width(), 1);

        // CJK (double-width)
        let cell = OutputCell::from_char('\u{4E2D}');
        assert_eq!(cell.column_width(), 2);

        // Combining mark (zero width)
        let cell = OutputCell::from_char('\u{0301}');
        assert_eq!(cell.column_width(), 0);
    }

    #[test]
    fn test_text_attribute_behavior() {
        let stored = OutputCell::new()
            .with_behavior(CellBehavior::Stored)
            .get_attribute_behavior();
        assert!(stored.is_replace());

        let current = OutputCell::new()
            .with_behavior(CellBehavior::Current)
            .get_attribute_behavior();
        assert!(current.is_additive());

        let stored_only = OutputCell::new()
            .with_behavior(CellBehavior::StoredOnly)
            .get_attribute_behavior();
        assert!(matches!(stored_only, TextAttributeBehavior::Subtractive));
    }

    #[test]
    fn test_whitespace_detection() {
        let space = OutputCell::from_char(' ');
        assert!(space.is_whitespace());

        let tab = OutputCell::from_char('\t');
        assert!(tab.is_whitespace());

        let letter = OutputCell::from_char('a');
        assert!(!letter.is_whitespace());
    }

    #[test]
    fn test_cell_clone_and_copy() {
        let cell = OutputCell::from_str("test");
        let cloned = cell.clone();
        assert_eq!(cell, cloned);

        let cell2 = OutputCell::from_str("test");
        let moved = cell2;
        assert_eq!(moved.chars(), "test");
    }

    #[test]
    fn test_display_trait() {
        let cell = OutputCell::from_char('X');
        assert_eq!(format!("{}", cell), "X");

        let cell = OutputCell::from_str("Hell"); // 4 bytes, within MAX_CHARS
        assert_eq!(format!("{}", cell), "Hell");
    }
}
