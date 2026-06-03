//! `CharacterAttributes` — terminal character styling bitflags.
//!
//! Ported from `CharacterAttributes` in the Windows Terminal C++ `conattrs.hpp`.

bitflags::bitflags! {
    /// Character attributes for terminal text rendering.
    ///
    /// Ported from `CharacterAttributes` enum in the Windows Terminal C++ codebase.
    /// These bitflags represent foreground/background colors, intensity, underline
    /// styles, grid lines, and other visual attributes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct CharacterAttributes: u16 {
        /// Normal (no attributes).
        const Normal = 0x0000;

        /// Intense (bright) foreground/background.
        const Intense = 0x0001;

        /// Italics.
        const Italics = 0x0002;

        /// Blinking text.
        const Blinking = 0x0004;

        /// Invisible text.
        const Invisible = 0x0008;

        /// Crossed out text.
        const CrossedOut = 0x0010;

        /// Faint (dim) text.
        const Faint = 0x0020;

        /// Underline style bits (bits 6-8).
        /// Values correspond to `UnderlineStyle` enum values shifted left by 6.
        const UnderlineStyleMask = 0x01C0;

        /// Unused bit.
        const Unused1 = 0x0200;

        /// Top gridline (box drawing).
        const TopGridline = 0x0400;

        /// Left gridline (box drawing).
        const LeftGridline = 0x0800;

        /// Right gridline (box drawing).
        const RightGridline = 0x1000;

        /// Protected cell.
        const Protected = 0x2000;

        /// Reverse video (foreground/background swap).
        const ReverseVideo = 0x4000;

        /// Bottom gridline (box drawing).
        const BottomGridline = 0x8000;

        /// All character attributes.
        const All = 0xFFFF;

        /// All rendition attributes (everything except Protected).
        const Rendition = Self::All.bits() & !Self::Protected.bits();
    }
}

impl CharacterAttributes {
    /// The shift amount for underline style bits.
    pub const UNDERLINE_STYLE_SHIFT: u32 = 6;

    /// Returns the underline style value (0-5).
    pub const fn get_underline_style(&self) -> u8 {
        ((self.bits() & Self::UnderlineStyleMask.bits()) >> Self::UNDERLINE_STYLE_SHIFT) as u8
    }

    /// Returns whether any grid line is enabled.
    pub const fn has_any_gridline(&self) -> bool {
        self.bits()
            & (Self::TopGridline.bits()
                | Self::LeftGridline.bits()
                | Self::RightGridline.bits()
                | Self::BottomGridline.bits())
            != 0
    }

    /// Returns whether the cell has any visual attributes beyond color.
    pub const fn has_any_visual_attributes(&self) -> bool {
        self.bits() != Self::Normal.bits()
    }
}

impl std::fmt::Display for CharacterAttributes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{:04X}", self.bits())
    }
}

impl Default for CharacterAttributes {
    fn default() -> Self {
        CharacterAttributes::Normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_normal() {
        assert_eq!(CharacterAttributes::default(), CharacterAttributes::Normal);
    }

    #[test]
    fn test_individual_flags() {
        let mut attrs = CharacterAttributes::Normal;
        attrs |= CharacterAttributes::Intense;
        attrs |= CharacterAttributes::Italics;

        assert!(attrs.contains(CharacterAttributes::Intense));
        assert!(attrs.contains(CharacterAttributes::Italics));
        assert!(!attrs.contains(CharacterAttributes::CrossedOut));
    }

    #[test]
    fn test_all_and_rendition() {
        assert_eq!(CharacterAttributes::All, CharacterAttributes::all());
        assert!(CharacterAttributes::All.contains(CharacterAttributes::Protected));
        assert!(!CharacterAttributes::Rendition.contains(CharacterAttributes::Protected));
    }

    #[test]
    fn test_underline_style_bits() {
        let mut attrs = CharacterAttributes::Normal;
        attrs.insert(CharacterAttributes::UnderlineStyleMask);
        assert_eq!(attrs.get_underline_style(), 7); // bits 6-8 all set = 0b111 = 7
    }

    #[test]
    fn test_has_any_gridline() {
        let mut attrs = CharacterAttributes::Normal;
        assert!(!attrs.has_any_gridline());

        attrs |= CharacterAttributes::TopGridline;
        assert!(attrs.has_any_gridline());
    }

    #[test]
    fn test_has_any_visual_attributes() {
        assert!(!CharacterAttributes::Normal.has_any_visual_attributes());
        assert!(CharacterAttributes::Intense.has_any_visual_attributes());
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", CharacterAttributes::Normal), "0x0000");
        assert_eq!(format!("{}", CharacterAttributes::Intense), "0x0001");
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    prop_compose! {
        fn any_attrs()(bits in any::<u16>()) -> CharacterAttributes {
            CharacterAttributes::from_bits_retain(bits)
        }
    }

    proptest! {
        #[test]
        fn test_union_contains_both_flags(a in any_attrs(), b in any_attrs()) {
            let combined = a | b;
            prop_assert!(combined.contains(a), "union should contain a");
            prop_assert!(combined.contains(b), "union should contain b");
        }

        #[test]
        fn test_idempotent_union(a in any_attrs()) {
            let combined = a | a;
            prop_assert_eq!(combined, a);
        }

        #[test]
        fn test_idempotent_intersection(a in any_attrs(), b in any_attrs()) {
            let intersected = a & b;
            // Intersection is idempotent: (a & b) & (a & b) == a & b
            prop_assert_eq!(intersected & intersected, intersected);
        }

        #[test]
        fn test_intersection_contains_both(a in any_attrs(), b in any_attrs()) {
            let intersected = a & b;
            prop_assert!(intersected.contains(intersected & a));
            prop_assert!(intersected.contains(intersected & b));
        }
    }
}
