//! `CursorType` — cursor shape enumeration.
//!
//! Ported from `CursorType` in the Windows Terminal C++ buffer module.

/// Represents the shape/style of a terminal cursor.
///
/// Ported from `CursorType` in the Windows Terminal C++ `conattrs.hpp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum CursorType {
    /// Uses the cursor's height value to range from underscore-like to full box.
    Legacy = 0,
    /// A single vertical line, `|`.
    VerticalBar = 1,
    /// A single horizontal underscore, smaller than the minimum height legacy cursor.
    Underscore = 2,
    /// Just the outline of a full box.
    EmptyBox = 3,
    /// A full box, similar to legacy with height 100%.
    FullBox = 4,
    /// A double horizontal underscore.
    DoubleUnderscore = 5,
}

impl CursorType {
    /// Returns the name of this cursor type.
    pub const fn name(&self) -> &'static str {
        match self {
            CursorType::Legacy => "legacy",
            CursorType::VerticalBar => "vertical_bar",
            CursorType::Underscore => "underscore",
            CursorType::EmptyBox => "empty_box",
            CursorType::FullBox => "full_box",
            CursorType::DoubleUnderscore => "double_underscore",
        }
    }

    /// Returns the raw value.
    pub const fn as_u32(&self) -> u32 {
        *self as u32
    }

    /// Creates a CursorType from a raw u32 value.
    pub const fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(CursorType::Legacy),
            1 => Some(CursorType::VerticalBar),
            2 => Some(CursorType::Underscore),
            3 => Some(CursorType::EmptyBox),
            4 => Some(CursorType::FullBox),
            5 => Some(CursorType::DoubleUnderscore),
            _ => None,
        }
    }
}

impl std::fmt::Display for CursorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl Default for CursorType {
    fn default() -> Self {
        CursorType::Legacy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_type_names() {
        assert_eq!(CursorType::Legacy.name(), "legacy");
        assert_eq!(CursorType::VerticalBar.name(), "vertical_bar");
        assert_eq!(CursorType::Underscore.name(), "underscore");
        assert_eq!(CursorType::EmptyBox.name(), "empty_box");
        assert_eq!(CursorType::FullBox.name(), "full_box");
        assert_eq!(CursorType::DoubleUnderscore.name(), "double_underscore");
    }

    #[test]
    fn test_cursor_type_from_u32() {
        assert_eq!(CursorType::from_u32(0), Some(CursorType::Legacy));
        assert_eq!(CursorType::from_u32(1), Some(CursorType::VerticalBar));
        assert_eq!(CursorType::from_u32(5), Some(CursorType::DoubleUnderscore));
        assert_eq!(CursorType::from_u32(6), None);
        assert_eq!(CursorType::from_u32(100), None);
    }

    #[test]
    fn test_cursor_type_as_u32() {
        assert_eq!(CursorType::Legacy.as_u32(), 0);
        assert_eq!(CursorType::FullBox.as_u32(), 4);
    }

    #[test]
    fn test_cursor_type_display() {
        assert_eq!(format!("{}", CursorType::VerticalBar), "vertical_bar");
    }
}
