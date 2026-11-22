//! `Renderer` — the orchestration over the engine and render data.
//!
//! Ported from `renderer.hpp/.cpp`. This first slice carries the two
//! self-contained helpers the paint paths call; the frame loop and the paint
//! orchestration land next.

use types::{TextAttribute, UnderlineStyle};

use crate::render_frame_info::{GridLineSet, GridLines};

/// The first code point reserved for DECDLD soft-font glyphs.
pub const FIRST_SOFT_FONT_CHAR: usize = 0xEF20;

/// Maps a text attribute's borders and underline onto grid lines.
///
/// Ported from `Renderer::s_GetGridlines`. An underline style outside the enum
/// cannot reach here: `TextAttribute::get_underline_style` already maps it to
/// `NoUnderline`.
pub fn get_gridlines(attribute: &TextAttribute) -> GridLineSet {
    let mut lines = GridLineSet::new();

    if attribute.is_top_horizontal_displayed() {
        lines.insert(GridLines::Top);
    }
    if attribute.is_bottom_horizontal_displayed() {
        lines.insert(GridLines::Bottom);
    }
    if attribute.is_left_vertical_displayed() {
        lines.insert(GridLines::Left);
    }
    if attribute.is_right_vertical_displayed() {
        lines.insert(GridLines::Right);
    }
    if attribute.is_crossed_out() {
        lines.insert(GridLines::Strikethrough);
    }

    match attribute.get_underline_style() {
        UnderlineStyle::NoUnderline => {}
        UnderlineStyle::SinglyUnderlined => lines.insert(GridLines::Underline),
        UnderlineStyle::DoublyUnderlined => lines.insert(GridLines::DoubleUnderline),
        UnderlineStyle::CurlyUnderlined => lines.insert(GridLines::CurlyUnderline),
        UnderlineStyle::DottedUnderlined => lines.insert(GridLines::DottedUnderline),
        UnderlineStyle::DashedUnderlined => lines.insert(GridLines::DashedUnderline),
    }

    if attribute.is_hyperlink() {
        lines.insert(GridLines::HyperlinkUnderline);
    }

    lines
}

/// Returns whether `text` is a single glyph in the inclusive soft-font range.
///
/// Ported from `Renderer::s_IsSoftFontChar`. Upstream tests one UTF-16 unit;
/// this port tests one Unicode scalar, which is what the decoded text holds.
pub fn is_soft_font_char(text: &str, first: usize, last: usize) -> bool {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => (first..=last).contains(&(ch as usize)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{CharacterAttributes, TextAttribute, UnderlineStyle};

    #[test]
    fn a_plain_attribute_has_no_lines() {
        assert!(get_gridlines(&TextAttribute::default()).is_empty());
    }

    #[test]
    fn each_border_maps_to_its_line() {
        for (flag, line) in [
            (CharacterAttributes::TopGridline, GridLines::Top),
            (CharacterAttributes::BottomGridline, GridLines::Bottom),
            (CharacterAttributes::LeftGridline, GridLines::Left),
            (CharacterAttributes::RightGridline, GridLines::Right),
        ] {
            let mut attribute = TextAttribute::default();
            attribute.set_attrs(flag);
            let lines = get_gridlines(&attribute);
            assert!(lines.contains(line), "{flag:?}");
            assert!(!lines.contains(GridLines::Strikethrough));
        }
    }

    #[test]
    fn crossed_out_is_a_strikethrough() {
        let mut attribute = TextAttribute::default();
        attribute.set_crossed_out(true);
        assert!(get_gridlines(&attribute).contains(GridLines::Strikethrough));
    }

    #[test]
    fn every_underline_style_maps_to_its_line() {
        for (style, line) in [
            (UnderlineStyle::SinglyUnderlined, GridLines::Underline),
            (UnderlineStyle::DoublyUnderlined, GridLines::DoubleUnderline),
            (UnderlineStyle::CurlyUnderlined, GridLines::CurlyUnderline),
            (UnderlineStyle::DottedUnderlined, GridLines::DottedUnderline),
            (UnderlineStyle::DashedUnderlined, GridLines::DashedUnderline),
        ] {
            let mut attribute = TextAttribute::default();
            attribute.set_underline_style(style);
            assert!(get_gridlines(&attribute).contains(line), "{style:?}");
        }
    }

    #[test]
    fn a_hyperlink_adds_its_underline() {
        let mut attribute = TextAttribute::default();
        attribute.set_hyperlink_id(7);
        assert!(get_gridlines(&attribute).contains(GridLines::HyperlinkUnderline));
    }

    #[test]
    fn combined_attributes_produce_every_line() {
        let mut attribute = TextAttribute::default();
        attribute.set_attrs(CharacterAttributes::TopGridline);
        attribute.set_crossed_out(true);
        attribute.set_underline_style(UnderlineStyle::DoublyUnderlined);
        attribute.set_hyperlink_id(1);

        let lines = get_gridlines(&attribute);
        assert!(lines.contains(GridLines::Top));
        assert!(lines.contains(GridLines::Strikethrough));
        assert!(lines.contains(GridLines::DoubleUnderline));
        assert!(lines.contains(GridLines::HyperlinkUnderline));
    }

    #[test]
    fn only_single_glyphs_inside_the_range_are_soft_font_chars() {
        let first = FIRST_SOFT_FONT_CHAR;
        let last = first + 10;

        assert!(is_soft_font_char("\u{EF20}", first, last));
        assert!(is_soft_font_char("\u{EF2A}", first, last));
        assert!(!is_soft_font_char("\u{EF1F}", first, last));
        assert!(!is_soft_font_char("\u{EF2B}", first, last));
        assert!(!is_soft_font_char("", first, last));
        assert!(!is_soft_font_char("AB", first, last));
    }
}
