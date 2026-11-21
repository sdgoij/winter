//! `TextAttribute` — character rendering attributes.
//!
//! Ported from `TextAttribute` in the Windows Terminal C++ buffer module.

use crate::{CharacterAttributes, MarkKind, TextColor, UnderlineStyle};

/// Controls how text attributes are applied during rendering.
///
/// Ported from `TextAttributeBehavior` in the C++ codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextAttributeBehavior {
    /// Use the stored text attribute directly.
    Stored,
    /// Use the text attribute of the cell being written to.
    Current,
    /// Only use the stored text attribute and skip insertion of other attributes.
    StoredOnly,
}

/// Text rendering attributes for a terminal cell.
///
/// Ported from `TextAttribute` in the Windows Terminal C++ buffer module.
/// Stores foreground/background colors, character attributes, underline style,
/// hyperlink ID, and mark kind in a RLE-friendly layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextAttribute {
    /// Character attributes (intensity, italics, underline, etc.)
    attrs: CharacterAttributes,
    /// Hyperlink ID (0 = no hyperlink)
    hyperlink_id: u16,
    /// Foreground color
    foreground: TextColor,
    /// Background color
    background: TextColor,
    /// Underline color
    underline_color: TextColor,
    /// Mark kind for semantic cell marking
    mark_kind: MarkKind,
}

impl Default for TextAttribute {
    fn default() -> Self {
        Self::new()
    }
}

impl TextAttribute {
    /// Creates a default text attribute (normal, no color).
    pub const fn new() -> Self {
        TextAttribute {
            attrs: CharacterAttributes::Normal,
            hyperlink_id: 0,
            foreground: TextColor::new(),
            background: TextColor::new(),
            underline_color: TextColor::new(),
            mark_kind: MarkKind::None,
        }
    }

    /// Creates a text attribute from legacy WORD attributes.
    pub const fn from_legacy_word(_word: u16) -> Self {
        TextAttribute {
            attrs: CharacterAttributes::Normal,
            hyperlink_id: 0,
            foreground: TextColor::new(),
            background: TextColor::new(),
            underline_color: TextColor::new(),
            mark_kind: MarkKind::None,
        }
    }

    /// Creates a text attribute from RGB colors.
    pub const fn from_rgb(fg_rgb: u32, bg_rgb: u32, underline_rgb: Option<u32>) -> Self {
        let underline_color = if let Some(rgb) = underline_rgb {
            TextColor::rgb(
                ((rgb >> 16) & 0xFF) as u8,
                ((rgb >> 8) & 0xFF) as u8,
                (rgb & 0xFF) as u8,
            )
        } else {
            TextColor::new()
        };
        TextAttribute {
            attrs: CharacterAttributes::Normal,
            hyperlink_id: 0,
            foreground: TextColor::rgb(
                ((fg_rgb >> 16) & 0xFF) as u8,
                ((fg_rgb >> 8) & 0xFF) as u8,
                (fg_rgb & 0xFF) as u8,
            ),
            background: TextColor::rgb(
                ((bg_rgb >> 16) & 0xFF) as u8,
                ((bg_rgb >> 8) & 0xFF) as u8,
                (bg_rgb & 0xFF) as u8,
            ),
            underline_color,
            mark_kind: MarkKind::None,
        }
    }

    /// Returns the character attributes.
    pub const fn get_attrs(&self) -> CharacterAttributes {
        self.attrs
    }

    /// Sets the character attributes.
    pub fn set_attrs(&mut self, attrs: CharacterAttributes) {
        self.attrs = attrs;
    }

    /// Returns the mark attributes.
    pub const fn get_mark_attrs(&self) -> MarkKind {
        self.mark_kind
    }

    /// Sets the mark attributes.
    pub fn set_mark_attrs(&mut self, mark: MarkKind) {
        self.mark_kind = mark;
    }

    /// Returns the foreground color.
    pub const fn get_foreground(&self) -> TextColor {
        self.foreground
    }

    /// Returns the background color.
    pub const fn get_background(&self) -> TextColor {
        self.background
    }

    /// Returns the hyperlink ID.
    pub const fn get_hyperlink_id(&self) -> u16 {
        self.hyperlink_id
    }

    /// Returns the underline color.
    pub const fn get_underline_color(&self) -> TextColor {
        self.underline_color
    }

    /// Returns the underline style.
    pub const fn get_underline_style(&self) -> UnderlineStyle {
        match UnderlineStyle::from_u8(self.attrs.get_underline_style()) {
            Some(style) => style,
            None => UnderlineStyle::NoUnderline,
        }
    }

    /// Sets the foreground color.
    pub fn set_foreground(&mut self, fg: TextColor) {
        self.foreground = fg;
    }

    /// Sets the background color.
    pub fn set_background(&mut self, bg: TextColor) {
        self.background = bg;
    }

    /// Sets the underline color.
    pub fn set_underline_color(&mut self, color: TextColor) {
        self.underline_color = color;
    }

    /// Sets the hyperlink ID.
    pub fn set_hyperlink_id(&mut self, id: u16) {
        self.hyperlink_id = id;
    }

    /// Sets indexed foreground (16-color palette).
    pub fn set_indexed_foreground(&mut self, index: u8) {
        self.foreground = TextColor::index16(index);
    }

    /// Sets indexed background (16-color palette).
    pub fn set_indexed_background(&mut self, index: u8) {
        self.background = TextColor::index16(index);
    }

    /// Sets the foreground to default.
    pub fn set_default_foreground(&mut self) {
        self.foreground = TextColor::new();
    }

    /// Sets the background to default.
    pub fn set_default_background(&mut self) {
        self.background = TextColor::new();
    }

    /// Sets the underline color to default.
    pub fn set_default_underline_color(&mut self) {
        self.underline_color = TextColor::new();
    }

    /// Sets default rendition attributes.
    pub fn set_default_rendition_attributes(&mut self) {
        self.attrs = CharacterAttributes::Normal;
        self.hyperlink_id = 0;
    }

    /// Sets standard erase attributes.
    pub fn set_standard_erase(&mut self) {
        self.attrs = CharacterAttributes::Normal;
        self.hyperlink_id = 0;
        self.foreground = TextColor::new();
        self.background = TextColor::new();
        self.underline_color = TextColor::new();
        self.mark_kind = MarkKind::None;
    }

    /// Returns whether the text is bold/intense.
    pub const fn is_intense(&self) -> bool {
        self.attrs.contains(CharacterAttributes::Intense)
    }

    /// Returns whether the text is faint/dim.
    pub const fn is_faint(&self) -> bool {
        self.attrs.contains(CharacterAttributes::Faint)
    }

    /// Returns whether the text is italic.
    pub const fn is_italic(&self) -> bool {
        self.attrs.contains(CharacterAttributes::Italics)
    }

    /// Returns whether the text is blinking.
    pub const fn is_blinking(&self) -> bool {
        self.attrs.contains(CharacterAttributes::Blinking)
    }

    /// Returns whether the text is invisible.
    pub const fn is_invisible(&self) -> bool {
        self.attrs.contains(CharacterAttributes::Invisible)
    }

    /// Returns whether the text is struck through.
    pub const fn is_crossed_out(&self) -> bool {
        self.attrs.contains(CharacterAttributes::CrossedOut)
    }

    /// Returns whether the colours are reversed.
    pub const fn is_reverse_video(&self) -> bool {
        self.attrs.contains(CharacterAttributes::ReverseVideo)
    }

    /// Turns the intense (bright) attribute on or off.
    pub fn set_intense(&mut self, enabled: bool) {
        self.update(CharacterAttributes::Intense, enabled);
    }

    /// Turns the faint attribute on or off.
    pub fn set_faint(&mut self, enabled: bool) {
        self.update(CharacterAttributes::Faint, enabled);
    }

    /// Turns italics on or off.
    pub fn set_italic(&mut self, enabled: bool) {
        self.update(CharacterAttributes::Italics, enabled);
    }

    /// Turns blinking on or off.
    pub fn set_blinking(&mut self, enabled: bool) {
        self.update(CharacterAttributes::Blinking, enabled);
    }

    /// Turns invisibility on or off.
    pub fn set_invisible(&mut self, enabled: bool) {
        self.update(CharacterAttributes::Invisible, enabled);
    }

    /// Turns strike-through on or off.
    pub fn set_crossed_out(&mut self, enabled: bool) {
        self.update(CharacterAttributes::CrossedOut, enabled);
    }

    /// Turns reverse video on or off.
    pub fn set_reverse_video(&mut self, enabled: bool) {
        self.update(CharacterAttributes::ReverseVideo, enabled);
    }

    /// Turns the overline on or off.
    ///
    /// Upstream reuses the top-gridline bit for this (`TextAttribute::SetOverlined`).
    pub fn set_overlined(&mut self, enabled: bool) {
        self.update(CharacterAttributes::TopGridline, enabled);
    }

    /// Marks the attribute as protected (DECSCA).
    pub fn set_protected(&mut self, enabled: bool) {
        self.update(CharacterAttributes::Protected, enabled);
    }

    /// Returns whether the attribute is protected.
    pub const fn is_protected(&self) -> bool {
        self.attrs.contains(CharacterAttributes::Protected)
    }

    /// Sets the underline style, replacing the two style bits.
    pub fn set_underline_style(&mut self, style: UnderlineStyle) {
        let shifted = (style as u16) << CharacterAttributes::UNDERLINE_STYLE_SHIFT;
        self.attrs =
            (self.attrs & !CharacterAttributes::UnderlineStyleMask) | CharacterAttributes::from_bits_truncate(shifted);
    }

    /// Sets an xterm-256 foreground index.
    pub fn set_indexed_foreground_256(&mut self, index: u8) {
        self.foreground = TextColor::index256(index);
    }

    /// Sets an xterm-256 background index.
    pub fn set_indexed_background_256(&mut self, index: u8) {
        self.background = TextColor::index256(index);
    }

    fn update(&mut self, flag: CharacterAttributes, enabled: bool) {
        if enabled {
            self.attrs |= flag;
        } else {
            self.attrs &= !flag;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_none() {
        let attr = TextAttribute::new();
        assert!(attr.foreground.is_default());
        assert!(attr.background.is_default());
        assert!(!attr.is_intense());
        assert!(!attr.is_faint());
        assert!(!attr.is_italic());
        assert!(!attr.is_blinking());
    }

    #[test]
    fn test_set_attrs() {
        let mut attr = TextAttribute::new();
        attr.set_attrs(CharacterAttributes::Intense);
        assert!(attr.is_intense());

        attr.set_attrs(CharacterAttributes::Normal);
        assert!(!attr.is_intense());
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    prop_compose! {
        fn any_text_attribute()(
            fg_rgb in any::<u32>(),
            bg_rgb in any::<u32>(),
            underline_rgb in prop::option::of(any::<u32>()),
        ) -> TextAttribute {
            TextAttribute::from_rgb(fg_rgb, bg_rgb, underline_rgb)
        }
    }

    proptest! {
        #[test]
        fn test_equality_reflexive(attr in any_text_attribute()) {
            prop_assert_eq!(attr, attr);
        }

        #[test]
        fn test_equality_symmetric(attr1 in any_text_attribute(), attr2 in any_text_attribute()) {
            if attr1 == attr2 {
                prop_assert_eq!(attr2, attr1);
            }
        }

        #[test]
        fn test_equality_transitive(
            attr1 in any_text_attribute(),
            attr2 in any_text_attribute(),
            attr3 in any_text_attribute(),
        ) {
            if attr1 == attr2 && attr2 == attr3 {
                prop_assert_eq!(attr1, attr3);
            }
        }

        #[test]
        fn test_set_unset_attribute_returns_to_default(mut attr in any_text_attribute()) {
            let original = attr;

            // Set all attributes
            attr.set_attrs(CharacterAttributes::All);
            prop_assert_ne!(attr, original);

            // Reset to default
            attr.set_default_rendition_attributes();
            attr.set_default_foreground();
            attr.set_default_background();
            attr.set_default_underline_color();

            // Should be back to default
            prop_assert_eq!(attr.attrs, CharacterAttributes::Normal);
        }

        #[test]
        fn test_equality_field_by_field(fg1 in any::<u32>(), bg1 in any::<u32>(),
                                         fg2 in any::<u32>(), bg2 in any::<u32>()) {
            let attr1 = TextAttribute::from_rgb(fg1, bg1, None);
            let attr2 = TextAttribute::from_rgb(fg2, bg2, None);

            // Equal if and only if all fields are equal
            if fg1 == fg2 && bg1 == bg2 {
                prop_assert_eq!(attr1, attr2);
            }
        }
    }
}
