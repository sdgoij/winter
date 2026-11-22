//! A `fontdue` glyph rasteriser.
//!
//! This is the cross-platform replacement for the DirectWrite rasteriser in
//! `src/renderer/atlas/`. It implements both [`ShapingBackend`] (code point to
//! glyph index and advance) and [`GlyphRasterizer`] (glyph index to coverage),
//! so the software backend can draw real text without a GPU or a C toolchain.
//!
//! The font file is supplied by the caller; nothing is bundled here. Use
//! [`FontDueRasterizer::from_file`] for a system font or [`from_bytes`] for an
//! embedded one.

use std::path::Path;

use fontdue::{Font, FontSettings, Metrics};

use crate::atlas::common::{FontId, ShapedGlyph};
use crate::atlas::shaping::ShapingBackend;
use crate::atlas::software::{GlyphBitmap, GlyphRasterizer};

/// Rasterises glyphs with `fontdue`.
pub struct FontDueRasterizer {
    font: Font,
    size: f32,
}

impl FontDueRasterizer {
    /// Creates a rasteriser from font bytes at `size` pixels per em.
    pub fn from_bytes(bytes: Vec<u8>, size: f32) -> Result<Self, String> {
        let font = Font::from_bytes(bytes, FontSettings::default()).map_err(|error| error.to_string())?;
        Ok(Self { font, size })
    }

    /// Creates a rasteriser from a font file at `size` pixels per em.
    pub fn from_file(path: impl AsRef<Path>, size: f32) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(bytes, size).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }

    /// Returns the glyph index for a character, or zero when absent.
    pub fn glyph_index(&self, character: char) -> u16 {
        self.font.lookup_glyph_index(character)
    }

    /// Returns the advance width for a character at this size.
    pub fn advance_width(&self, character: char) -> f32 {
        self.font.metrics(character, self.size).advance_width
    }

    /// Returns the face name, when the font has one.
    pub fn name(&self) -> Option<&str> {
        self.font.name()
    }
}

impl ShapingBackend for FontDueRasterizer {
    fn shape(&self, text: &str, _font: FontId) -> Vec<ShapedGlyph> {
        text.chars()
            .map(|character| {
                let metrics = self.font.metrics(character, self.size);
                ShapedGlyph {
                    glyph_index: self.font.lookup_glyph_index(character),
                    advance: metrics.advance_width,
                    offset: (0.0, 0.0),
                    color: None,
                }
            })
            .collect()
    }
}

impl GlyphRasterizer for FontDueRasterizer {
    fn rasterize(&mut self, _font: FontId, glyph_index: u16) -> Option<GlyphBitmap> {
        let (metrics, coverage) = self.font.rasterize_indexed(glyph_index, self.size);
        Some(to_glyph_bitmap(metrics, coverage))
    }
}

/// Converts fontdue metrics into a [`GlyphBitmap`].
///
/// `metrics.ymin` is the bitmap's bottom edge relative to the baseline, and
/// `height` grows downward, so the top edge relative to the baseline is
/// `-(ymin + height)`. The backend adds the cell baseline before painting.
fn to_glyph_bitmap(metrics: Metrics, coverage: Vec<u8>) -> GlyphBitmap {
    GlyphBitmap {
        width: metrics.width as u32,
        height: metrics.height as u32,
        left: metrics.xmin,
        top: -(metrics.ymin + metrics.height as i32),
        coverage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_become_a_top_relative_bitmap() {
        let metrics = Metrics {
            xmin: 1,
            ymin: 2,
            width: 5,
            height: 7,
            advance_width: 6.0,
            advance_height: 0.0,
            ..Metrics::default()
        };

        let bitmap = to_glyph_bitmap(metrics, vec![0; 35]);
        assert_eq!(bitmap.width, 5);
        assert_eq!(bitmap.height, 7);
        assert_eq!(bitmap.left, 1);
        // Top edge = -(bottom offset + height) relative to the baseline.
        assert_eq!(bitmap.top, -9);
        assert_eq!(bitmap.coverage.len(), 35);
    }

    #[test]
    fn a_glyph_below_the_baseline_keeps_its_offset() {
        let metrics = Metrics {
            xmin: 0,
            ymin: -4,
            width: 2,
            height: 6,
            ..Metrics::default()
        };

        let bitmap = to_glyph_bitmap(metrics, vec![0; 12]);
        assert_eq!(bitmap.top, -2);
    }
}
