//! `UnderlineStyle` — terminal text underline style enumeration.
//!
//! Ported from `UnderlineStyle` in the Windows Terminal C++ buffer module.

/// Represents the style of text underlining in a terminal.
///
/// Ported from `UnderlineStyle` enum in the Windows Terminal C++ `TextAttribute.hpp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
#[derive(Default)]
pub enum UnderlineStyle {
    /// No underline.
    #[default]
    NoUnderline = 0,
    /// Singly underlined.
    SinglyUnderlined = 1,
    /// Doubly underlined.
    DoublyUnderlined = 2,
    /// Curly (wavy) underlined.
    CurlyUnderlined = 3,
    /// Dotted underline.
    DottedUnderlined = 4,
    /// Dashed underline.
    DashedUnderlined = 5,
}

impl UnderlineStyle {
    /// Maximum underline style value.
    pub const MAX: Self = UnderlineStyle::DashedUnderlined;

    /// Returns the name of this underline style.
    pub const fn name(&self) -> &'static str {
        match self {
            UnderlineStyle::NoUnderline => "no_underline",
            UnderlineStyle::SinglyUnderlined => "singly_underlined",
            UnderlineStyle::DoublyUnderlined => "doubly_underlined",
            UnderlineStyle::CurlyUnderlined => "curly_underlined",
            UnderlineStyle::DottedUnderlined => "dotted_underlined",
            UnderlineStyle::DashedUnderlined => "dashed_underlined",
        }
    }

    /// Returns the raw u8 value.
    pub const fn as_u8(&self) -> u8 {
        *self as u8
    }

    /// Creates an UnderlineStyle from a u8 value.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(UnderlineStyle::NoUnderline),
            1 => Some(UnderlineStyle::SinglyUnderlined),
            2 => Some(UnderlineStyle::DoublyUnderlined),
            3 => Some(UnderlineStyle::CurlyUnderlined),
            4 => Some(UnderlineStyle::DottedUnderlined),
            5 => Some(UnderlineStyle::DashedUnderlined),
            _ => None,
        }
    }
}


impl std::fmt::Display for UnderlineStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_underline_names() {
        assert_eq!(UnderlineStyle::NoUnderline.name(), "no_underline");
        assert_eq!(UnderlineStyle::SinglyUnderlined.name(), "singly_underlined");
        assert_eq!(UnderlineStyle::DoublyUnderlined.name(), "doubly_underlined");
        assert_eq!(UnderlineStyle::CurlyUnderlined.name(), "curly_underlined");
        assert_eq!(UnderlineStyle::DottedUnderlined.name(), "dotted_underlined");
        assert_eq!(UnderlineStyle::DashedUnderlined.name(), "dashed_underlined");
    }

    #[test]
    fn test_from_u8() {
        for i in 0..=5 {
            assert!(UnderlineStyle::from_u8(i).is_some());
        }
        assert!(UnderlineStyle::from_u8(6).is_none());
    }

    #[test]
    fn test_as_u8() {
        assert_eq!(UnderlineStyle::NoUnderline.as_u8(), 0);
        assert_eq!(UnderlineStyle::DashedUnderlined.as_u8(), 5);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", UnderlineStyle::CurlyUnderlined), "curly_underlined");
    }
}
