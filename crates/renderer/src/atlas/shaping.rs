//! Cross-platform text shaping for the atlas engine.
//!
//! Upstream shapes with DirectWrite (`DWriteTextAnalysis.cpp`). This module
//! defines the trait a cross-platform shaper implements and a per-cell fallback
//! for the software path and tests.

use crate::atlas::common::{FontId, ShapedGlyph};

/// Turns text into glyphs for one font face.
pub trait ShapingBackend {
    /// Shapes `text` for `font`, returning its glyphs in display order.
    fn shape(&self, text: &str, font: FontId) -> Vec<ShapedGlyph>;
}

/// A shaper that maps each scalar to itself with a one-cell advance.
///
/// This is the monospace fallback: it performs no ligature, combining-mark or
/// bidirectional processing, and glyphs are code points rather than font
/// indices. A real shaper implements [`ShapingBackend`] instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct CellShaping;

impl ShapingBackend for CellShaping {
    fn shape(&self, text: &str, _font: FontId) -> Vec<ShapedGlyph> {
        text.chars()
            .map(|ch| ShapedGlyph {
                glyph_index: u16::try_from(ch as u32).unwrap_or(0xFFFD),
                advance: 1.0,
                offset: (0.0, 0.0),
                color: None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::common::ShapedRow;

    #[test]
    fn ascii_shapes_to_its_code_points() {
        let glyphs = CellShaping.shape("AB", FontId(0));

        assert_eq!(glyphs.len(), 2);
        assert_eq!(glyphs[0].glyph_index, u16::from(b'A'));
        assert_eq!(glyphs[1].glyph_index, u16::from(b'B'));
        assert_eq!(glyphs[0].advance, 1.0);
        assert!(glyphs.iter().all(|glyph| glyph.color.is_none()));
    }

    #[test]
    fn bmp_text_keeps_its_scalar() {
        let glyphs = CellShaping.shape("\u{00E9}", FontId(0));
        assert_eq!(glyphs[0].glyph_index, 0x00E9);
    }

    #[test]
    fn supplementary_text_falls_back_to_the_replacement_character() {
        let glyphs = CellShaping.shape("\u{1F600}", FontId(0));
        assert_eq!(glyphs[0].glyph_index, 0xFFFD);
    }

    #[test]
    fn empty_text_shapes_to_nothing() {
        assert!(CellShaping.shape("", FontId(0)).is_empty());
    }

    #[test]
    fn shaped_glyphs_fill_a_row() {
        let mut row = ShapedRow::default();
        row.push_glyphs(FontId(2), &CellShaping.shape("AB", FontId(2)), 0);

        assert_eq!(row.glyph_count(), 2);
        assert_eq!(row.mappings.len(), 1);
        assert_eq!(row.mappings[0].font, FontId(2));
        assert_eq!(row.mappings[0].glyphs_to, 2);
    }
}
