//! `CSSLengthPercentage` — a CSS length resolved against a reference frame.
//!
//! Ported from `CSSLengthPercentage.h/.cpp`. A length is stored either as an
//! absolute CSS inch measurement or as a fraction of the font size, the `0`
//! advance width, or the containing block.

/// What a length is measured against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReferenceFrame {
    /// Unset or empty; [`CSSLengthPercentage::resolve`] returns the fallback.
    #[default]
    None,
    /// Absolute CSS inches; multiply by the target DPI.
    Absolute,
    /// Relative to the font size.
    FontSize,
    /// Relative to the advance width of `0`.
    AdvanceWidth,
}

/// A CSS length, stored in CSS inches or as a fraction of a reference frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct CSSLengthPercentage {
    value: f32,
    reference_frame: ReferenceFrame,
}

impl PartialEq for CSSLengthPercentage {
    fn eq(&self, other: &Self) -> bool {
        self.reference_frame == other.reference_frame && (self.value - other.value).abs() < 0.0001
    }
}

impl CSSLengthPercentage {
    /// Creates an absolute length from a pixel value.
    pub const fn from_pixel(px: f32) -> Self {
        Self {
            value: px / 96.0,
            reference_frame: ReferenceFrame::Absolute,
        }
    }

    /// Parses a CSS length: a number with an optional `%`, `px`, `pt` or `ch`
    /// suffix.
    ///
    /// This mirrors upstream's `wcstof` walk: the longest numeric prefix is the
    /// value, an unknown suffix zeroes it, and a value that is not positive
    /// (including NaN) becomes the empty frame.
    pub fn from_string(text: &str) -> Self {
        let text = text.trim_start();

        let mut boundaries: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
        boundaries.push(text.len());

        for &end in boundaries.iter().rev() {
            let Ok(number) = text[..end].parse::<f32>() else {
                continue;
            };

            let suffix = &text[end..];
            let (value, reference_frame) = match suffix {
                "" => (number, ReferenceFrame::FontSize),
                "%" => (number / 100.0, ReferenceFrame::FontSize),
                "px" => (number / 96.0, ReferenceFrame::Absolute),
                "pt" => (number / 72.0, ReferenceFrame::Absolute),
                "ch" => (number, ReferenceFrame::AdvanceWidth),
                _ => (0.0, ReferenceFrame::FontSize),
            };

            if value > 0.0 {
                return Self { value, reference_frame };
            }
            return Self::default();
        }

        Self::default()
    }

    /// Returns the frame this length resolves against.
    pub const fn reference(&self) -> ReferenceFrame {
        self.reference_frame
    }

    /// Resolves the length, or returns `fallback` when the frame is unset.
    pub fn resolve(&self, fallback: f32, dpi: f32, font_size: f32, advance_width: f32) -> f32 {
        match self.reference_frame {
            ReferenceFrame::Absolute => self.value * dpi,
            ReferenceFrame::FontSize => self.value * font_size,
            ReferenceFrame::AdvanceWidth => self.value * advance_width,
            ReferenceFrame::None => fallback,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_number_is_font_size_relative() {
        let length = CSSLengthPercentage::from_string("1.5");
        assert_eq!(length.reference(), ReferenceFrame::FontSize);
        assert_eq!(length.resolve(0.0, 96.0, 10.0, 0.0), 15.0);
    }

    #[test]
    fn a_percentage_is_font_size_relative() {
        let length = CSSLengthPercentage::from_string("50%");
        assert_eq!(length.reference(), ReferenceFrame::FontSize);
        assert_eq!(length.resolve(0.0, 96.0, 12.0, 0.0), 6.0);
    }

    #[test]
    fn pixels_and_points_are_absolute() {
        let pixels = CSSLengthPercentage::from_string("96px");
        assert_eq!(pixels.reference(), ReferenceFrame::Absolute);
        assert_eq!(pixels.resolve(0.0, 96.0, 12.0, 0.0), 96.0);

        let points = CSSLengthPercentage::from_string("12pt");
        assert_eq!(points.reference(), ReferenceFrame::Absolute);
        assert_eq!(points.resolve(0.0, 96.0, 12.0, 0.0), 16.0);
    }

    #[test]
    fn ch_is_advance_width_relative() {
        let length = CSSLengthPercentage::from_string("2ch");
        assert_eq!(length.reference(), ReferenceFrame::AdvanceWidth);
        assert_eq!(length.resolve(0.0, 96.0, 12.0, 5.0), 10.0);
    }

    #[test]
    fn invalid_input_is_empty() {
        for text in ["", "abc", "0", "-5px", "1e", "2em", "  "] {
            let length = CSSLengthPercentage::from_string(text);
            assert_eq!(length.reference(), ReferenceFrame::None, "{text:?}");
            assert_eq!(length.resolve(7.0, 96.0, 12.0, 8.0), 7.0, "{text:?}");
        }
    }

    #[test]
    fn leading_whitespace_is_ignored_and_trailing_garbage_zeroes() {
        assert_eq!(
            CSSLengthPercentage::from_string("  10px").resolve(0.0, 96.0, 0.0, 0.0),
            10.0
        );
        assert!(CSSLengthPercentage::from_string("10px ").reference() == ReferenceFrame::None);
    }

    #[test]
    fn pixel_and_string_forms_compare_equal() {
        assert_eq!(
            CSSLengthPercentage::from_pixel(96.0),
            CSSLengthPercentage::from_string("96px")
        );
        assert_ne!(CSSLengthPercentage::from_pixel(0.0), CSSLengthPercentage::default());
    }
}
