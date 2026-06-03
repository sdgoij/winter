//! `codepoint_width` — Unicode character width detection for terminal rendering.
//!
//! Ported from CodepointWidthDetector and GlyphWidth in the Windows Terminal C++ codebase.

/// Returns the display width of a Unicode scalar value (code point).
///
/// Terminal characters typically have a width of 0 (non-spacing marks),
/// 1 (most Latin, CJK, etc.), or 2 (some CJK and emoji characters).
///
/// Uses the unicode-width algorithm as a reference:
/// - Control characters, combining marks: width 0
/// - Most Latin, Greek, Cyrillic, Arabic: width 1
/// - Most CJK, Korean, Japanese: width 2
/// - Certain emoji: width 0, 1, or 2
///
/// This is a simplified implementation. For production use, consider
/// using the `unicode-width` crate directly.
pub fn codepoint_width(cp: u32) -> u8 {
    match cp {
        // CJK Unified Ideographs and extensions
        0x1100..=0x115F |   // Hangul Jamo
        0x2E80..=0x2EFF |   // CJK Radicals Supplement
        0x2F00..=0x2FDF |   // Kangxi Radicals
        0x3000..=0x303F |   // CJK Symbols and Punctuation (includes fullwidth)
        0x3040..=0x309F |   // Hiragana
        0x30A0..=0x30FF |   // Katakana
        0x3100..=0x312F |   // Bopomofo
        0x3130..=0x318F |   // Hangul Compatibility Jamo
        0x3190..=0x319F |   // Ideographic Description Characters
        0x31A0..=0x31BF |   // Bopomofo Extended
        0x3200..=0x32FF |   // Enclosed CJK Letters and Months
        0x3300..=0x33FF |   // CJK Compatibility
        0x3400..=0x4DBF |   // CJK Unified Ideographs Extension A
        0x4E00..=0x9FFF |   // CJK Unified Ideographs
        0xF900..=0xFAFF |   // CJK Compatibility Ideographs
        0x20000..=0x2FFFF => // CJK Unified Ideographs Extension B
        2,

        // Halfwidth Katakana (fullwidth variants)
        0xFF00..=0xFF60 => 2,

        // Other wide characters
        0x1100..=0x11FF |   // Hangul Jamo (repeated for clarity)
        0xA000..=0xA48F |   // Yi Syllables
        0xAC00..=0xD7AF |   // Hangul Syllables
        0xF0000..=0xFFFFF => 2,

        // Combining marks and zero-width characters
        0x0300..=0x036F |   // Combining Diacritical Marks
        0x1AB0..=0x1AFF |   // Combining Diacritical Marks Extended
        0x1DC0..=0x1DFF |   // Combining Diacritical Marks Supplement
        0x20D0..=0x20FF |   // Combining Diacritical Marks for Symbols
        0xFE00..=0xFE0F |   // Variation Selectors
        0x1F000..=0x1FFFF => 0,

        // Control characters and space
        0x0000..=0x001F |   // C0 Controls
        0x007F..=0x009F |   // C1 Controls
        0x200B..=0x200F |   // Zero-width spaces
        0x2028..=0x202F |   // Directional formatting
        0x2060..=0x206F => 0,

        // Most other characters are width 1
        _ => 1,
    }
}

/// Returns the display width of a `char`.
pub fn char_width(c: char) -> u8 {
    codepoint_width(c as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_width() {
        assert_eq!(codepoint_width('A' as u32), 1);
        assert_eq!(codepoint_width('a' as u32), 1);
        assert_eq!(codepoint_width('0' as u32), 1);
        assert_eq!(codepoint_width(' ' as u32), 1);
    }

    #[test]
    fn test_cjk_width() {
        assert_eq!(codepoint_width('中' as u32), 2);
        assert_eq!(codepoint_width('日' as u32), 2);
        assert_eq!(codepoint_width('あ' as u32), 2);
    }

    #[test]
    fn test_combining_mark_width() {
        // Combining acute accent
        assert_eq!(codepoint_width(0x0301), 0);
    }

    #[test]
    fn test_control_width() {
        assert_eq!(codepoint_width(0x0000), 0);
        assert_eq!(codepoint_width(0x001F), 0);
    }

    #[test]
    fn test_char_width() {
        assert_eq!(char_width('A'), 1);
        assert_eq!(char_width('中'), 2);
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    /// Well-known codepoints from the unicode-width crate for testing.
    /// Each tuple is (codepoint, expected width).
    const UNICODE_WIDTH_SAMPLES: &[(u32, u8)] = &[
        // ASCII
        (0x0041, 1), // 'A'
        (0x0061, 1), // 'a'
        (0x0030, 1), // '0'
        (0x0020, 1), // ' '
        // CJK
        (0x4E2D, 2), // '中'
        (0x65E5, 2), // '日'
        (0x3042, 2), // 'あ'
        (0x30A1, 2), // 'ア'
        // Hangul
        (0xAC00, 2), // '가'
        // Combining marks (width 0)
        (0x0301, 0), // Combining acute accent
        (0x0300, 0), // Combining grave accent
        // Control characters (width 0)
        (0x0000, 0), // NUL
        (0x001F, 0), // Unit separator
        (0x007F, 0), // DEL
        // Zero-width spaces (width 0)
        (0x200B, 0), // Zero-width space
        (0x200C, 0), // Zero-width non-joiner
        // Other wide characters
        (0xFF41, 2), // Fullwidth 'A'
        (0xFF01, 2), // Fullwidth '！'
        // Latin extended (width 1)
        (0x00E9, 1), // 'é'
        (0x00DF, 1), // 'ß'
        // Greek (width 1)
        (0x0391, 1), // 'Α'
        (0x03B1, 1), // 'α'
        // Cyrillic (width 1)
        (0x0410, 1), // 'А'
        (0x0430, 1), // 'а'
    ];

    #[test]
    fn test_matches_unicode_width_samples() {
        // Test against known unicode-width crate reference values
        for &(cp, expected) in UNICODE_WIDTH_SAMPLES {
            assert_eq!(
                codepoint_width(cp),
                expected,
                "codepoint U+{:04X} should have width {}, got {}",
                cp,
                expected,
                codepoint_width(cp)
            );
        }
    }

    proptest! {
        #[test]
        fn test_width_is_valid(cp in any::<u32>()) {
            // Width should always be 0, 1, or 2
            let w = codepoint_width(cp);
            prop_assert!(w == 0 || w == 1 || w == 2);
        }

        #[test]
        fn test_control_chars_have_zero_width(cp in any::<u32>()) {
            // C0 and C1 control characters should have width 0
            if (0x0000..=0x001F).contains(&cp) || (0x007F..=0x009F).contains(&cp) {
                prop_assert_eq!(codepoint_width(cp), 0u8);
            }
        }
    }
}
