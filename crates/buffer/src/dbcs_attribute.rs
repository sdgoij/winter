//! `DbcsAttribute` — Double-Byte Character Support for terminal text.

/// Represents the DBCS (Double-Byte Character Set) attribute of a cell.
///
/// In DBCS encodings like Shift-JIS, GBK, or EUC-JP, some characters
/// occupy two cell positions. This type tracks whether a cell is the
/// first or second byte of a multi-byte character.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DbcsAttribute {
    /// Not a DBCS character.
    #[default]
    None,
    /// First byte of a DBCS character.
    FirstByte,
    /// Second byte of a DBCS character.
    SecondByte,
}

impl DbcsAttribute {
    /// Returns whether this is a valid DBCS first byte.
    pub const fn is_first_byte(&self) -> bool {
        matches!(self, Self::FirstByte)
    }

    /// Returns whether this is a valid DBCS second byte.
    pub const fn is_second_byte(&self) -> bool {
        matches!(self, Self::SecondByte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_none() {
        let attr = DbcsAttribute::default();
        assert!(!attr.is_first_byte());
        assert!(!attr.is_second_byte());
    }

    #[test]
    fn test_first_byte() {
        let attr = DbcsAttribute::FirstByte;
        assert!(attr.is_first_byte());
        assert!(!attr.is_second_byte());
    }

    #[test]
    fn test_second_byte() {
        let attr = DbcsAttribute::SecondByte;
        assert!(!attr.is_first_byte());
        assert!(attr.is_second_byte());
    }
}
