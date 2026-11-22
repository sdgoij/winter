//! Unicode helpers.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::unicode`.

/// Returns whether a Unicode scalar value is a surrogate (D800–DFFF).
#[inline]
pub fn is_surrogate(cp: u32) -> bool {
    (cp & 0xF800) == 0xD800
}

/// Returns whether a Unicode scalar value is a leading surrogate (D800–DBFF).
#[inline]
pub fn is_leading_surrogate(cp: u32) -> bool {
    (cp & 0xFC00) == 0xD800
}

/// Returns whether a Unicode scalar value is a trailing surrogate (DC00–DFFF).
#[inline]
pub fn is_trailing_surrogate(cp: u32) -> bool {
    (cp & 0xFC00) == 0xDC00
}

/// Combines a leading and trailing surrogate pair into a Unicode scalar value.
#[inline]
pub fn combine_surrogates(lead: u32, trail: u32) -> u32 {
    // Correct surrogate pair decoding formula
    ((lead - 0xD800) << 10) + (trail - 0xDC00) + 0x10000
}

/// Returns whether a character is alphanumeric.
pub fn is_alphanumeric(cp: u32) -> bool {
    // Use Unicode properties for accurate detection
    let c = char::from_u32(cp).unwrap_or('\u{FFFD}');
    c.is_alphanumeric()
}

/// Returns whether a character is whitespace.
pub fn is_whitespace(cp: u32) -> bool {
    let c = char::from_u32(cp).unwrap_or(' ');
    c.is_whitespace()
}

/// Returns whether a character is a CJK character.
pub fn is_cjk(cp: u32) -> bool {
    matches!(cp,
        0x1100..=0x11FF |  // Hangul Jamo
        0x2E80..=0x2EFF |  // CJK Radicals Supplement
        0x2F00..=0x2FFF |  // Kangxi Radicals
        0x3000..=0x303F |  // CJK Symbols and Punctuation
        0x3040..=0x309F |  // Hiragana
        0x30A0..=0x30FF |  // Katakana
        0x3100..=0x312F |  // Bopomofo
        0x3130..=0x318F |  // Hangul Compatibility Jamo
        0x3190..=0x319F |  // Ideographic Description Characters
        0x31A0..=0x31BF |  // Bopomofo Extended
        0x3200..=0x32FF |  // Enclosed CJK Letters and Months
        0x3300..=0x33FF |  // CJK Compatibility
        0x3400..=0x4DBF |  // CJK Unified Ideographs Extension A
        0x4DC0..=0x4DFF |  // Yijing Hexagram Symbols
        0x4E00..=0x9FFF |  // CJK Unified Ideographs
        0xA000..=0xA48F |  // Yi Syllables
        0xA490..=0xA4CF |  // Yi Radicals
        0xF900..=0xFAFF |  // CJK Compatibility Ideographs
        0xFE10..=0xFE1F |  // Vertical Forms
        0xFE30..=0xFE4F |  // CJK Compatibility Forms and Special Items
        0xFF00..=0xFFEF |  // Halfwidth and Fullwidth Forms
        0x1B000..=0x1B0FF | // Kana Supplement
        0x1B100..=0x1B12F | // Kana Extended-A
        0x1B130..=0x1B16F | // Small Kana Extension
        0x1F200..=0x1F2FF | // Enclosed Ideographic Supplement
        0x20000..=0x2FFFF | // CJK Unified Ideographs Extension B
        0x30000..=0x3FFFF | // CJK Unified Ideographs Extension C
        0x40000..=0x4FFFF | // CJK Unified Ideographs Extension D
        0xE0100..=0xE01EF, // Variation Selectors Supplement
    )
}

/// Returns whether there is a Unicode grapheme break between two scalar values.
pub fn is_grapheme_boundary(cp1: u32, cp2: u32) -> bool {
    // Simplified grapheme break detection
    // In a full implementation, this would use the Unicode Grapheme Break Property
    let _c1 = char::from_u32(cp1).unwrap_or('\u{FFFD}');
    let _c2 = char::from_u32(cp2).unwrap_or('\u{FFFD}');

    // Combining marks (category Mn, Mc, Me) don't break graphemes
    if is_combining_mark(cp2) {
        return false;
    }

    // CrLf is a grapheme break
    if cp1 == 0x000D && cp2 == 0x000A {
        return true;
    }

    true
}

/// An incremental UTF-8 decoder with an explicit replacement policy.
///
/// The parsers hand over bytes; this is the one place they become `char`s.
/// Invalid sequences become U+FFFD, and a sequence interrupted by a control
/// character becomes one replacement character when the owner calls
/// [`clear`](Self::clear) and emits the replacement.
pub struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    /// Creates a decoder with no pending bytes.
    pub const fn new() -> Self {
        Self { pending: Vec::new() }
    }

    /// Returns whether a partial sequence is waiting.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Drops the pending bytes without emitting anything.
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Feeds `bytes`, calling `print` for every complete character.
    ///
    /// An invalid sequence becomes U+FFFD; an incomplete tail stays pending
    /// until more bytes arrive or the owner flushes it.
    pub fn emit(&mut self, bytes: &[u8], mut print: impl FnMut(char)) {
        for &byte in bytes {
            self.pending.push(byte);

            loop {
                if self.pending.is_empty() {
                    break;
                }

                match std::str::from_utf8(&self.pending) {
                    Ok(text) => {
                        for ch in text.chars() {
                            print(ch);
                        }
                        self.pending.clear();
                        break;
                    }
                    Err(error) => {
                        let valid = error.valid_up_to();
                        if valid > 0 {
                            let text = std::str::from_utf8(&self.pending[..valid]).expect("valid prefix");
                            for ch in text.chars() {
                                print(ch);
                            }
                        }

                        match error.error_len() {
                            Some(length) => {
                                print('\u{FFFD}');
                                self.pending.drain(..valid + length);
                            }
                            None => {
                                // Incomplete sequence; keep the tail waiting.
                                if valid > 0 {
                                    self.pending.drain(..valid);
                                }
                                // A UTF-8 character is at most four bytes, so a
                                // longer tail cannot become valid.
                                if self.pending.len() > 3 {
                                    print('\u{FFFD}');
                                    self.pending.clear();
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Default for Utf8Decoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns whether a Unicode scalar value is a combining mark.
fn is_combining_mark(cp: u32) -> bool {
    // General Category Mn (Nonspacing Mark), Mc (Spacing Combining Mark), Me (Enclosing Mark)
    matches!(cp, 0x0300..=0x036F | // Combining Diacritical Marks
                0x1AB0..=0x1AFF | // Combining Diacritical Marks Extended
                0x1DC0..=0x1DFF | // Combining Diacritical Marks Supplement
                0x20D0..=0x20FF | // Combining Diacritical Marks for Symbols
                0xFE20..=0xFE2F | // Combining Half Marks
                0xE0001 | // Tagging Close
                0xE0100..=0xE01EF) // Variation Selectors Supplement
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_surrogate() {
        assert!(is_surrogate(0xD800));
        assert!(is_surrogate(0xDFFF));
        assert!(!is_surrogate(0x0041));
        assert!(!is_surrogate(0x20000));
    }

    #[test]
    fn test_is_leading_surrogate() {
        assert!(is_leading_surrogate(0xD800));
        assert!(is_leading_surrogate(0xDBFF));
        assert!(!is_leading_surrogate(0xDC00));
    }

    #[test]
    fn test_is_trailing_surrogate() {
        assert!(is_trailing_surrogate(0xDC00));
        assert!(is_trailing_surrogate(0xDFFF));
        assert!(!is_trailing_surrogate(0xD800));
    }

    #[test]
    fn test_combine_surrogates() {
        // Test U+1DE00 (Latin Extended-D character)
        // Correct surrogate pair for U+1DE00: lead=0xD837, trail=0xDE00
        let lead = 0xD837;
        let trail = 0xDE00;
        let cp = combine_surrogates(lead, trail);
        assert_eq!(cp, 0x1DE00);

        // Test with a real emoji surrogate pair: U+1F600 (grinning face)
        let lead2 = 0xD83D;
        let trail2 = 0xDE00;
        let cp2 = combine_surrogates(lead2, trail2);
        assert_eq!(cp2, 0x1F600);
        assert!(char::from_u32(cp2).is_some());
    }

    #[test]
    fn test_is_alphanumeric() {
        assert!(is_alphanumeric('A' as u32));
        assert!(is_alphanumeric('a' as u32));
        assert!(is_alphanumeric('0' as u32));
        assert!(!is_alphanumeric(' ' as u32));
        assert!(!is_alphanumeric('!' as u32));
    }

    #[test]
    fn test_is_whitespace() {
        assert!(is_whitespace(' ' as u32));
        assert!(is_whitespace('\n' as u32));
        assert!(is_whitespace('\t' as u32));
        assert!(!is_whitespace('a' as u32));
    }

    #[test]
    fn test_is_cjk() {
        assert!(is_cjk('中' as u32));
        assert!(is_cjk('日' as u32));
        assert!(is_cjk('あ' as u32));
        assert!(!is_cjk('A' as u32));
    }

    #[test]
    fn test_is_grapheme_boundary() {
        assert!(is_grapheme_boundary('A' as u32, 'B' as u32));
        // Combining acute accent after 'A' should NOT be a boundary
        assert!(!is_grapheme_boundary('A' as u32, 0x0301u32));
    }

    #[test]
    fn a_decoder_emits_a_whole_sequence_once() {
        let mut decoder = Utf8Decoder::new();
        let mut printed = String::new();
        decoder.emit("é".as_bytes(), |ch| printed.push(ch));
        assert_eq!(printed, "é");
        assert!(decoder.is_empty());
    }

    #[test]
    fn a_decoder_holds_an_incomplete_sequence() {
        let mut decoder = Utf8Decoder::new();
        let mut printed = String::new();
        decoder.emit(&[0xE2, 0x82], |ch| printed.push(ch));
        assert_eq!(printed, "");
        assert!(!decoder.is_empty());

        decoder.emit(&[0xAC], |ch| printed.push(ch));
        assert_eq!(printed, "€");
        assert!(decoder.is_empty());
    }

    #[test]
    fn an_invalid_sequence_becomes_one_replacement() {
        let mut decoder = Utf8Decoder::new();
        let mut printed = String::new();
        decoder.emit(&[b'a', 0xFF, b'b'], |ch| printed.push(ch));
        assert_eq!(printed, "a\u{FFFD}b");
    }

    #[test]
    fn an_overlong_encoding_is_rejected_byte_by_byte() {
        let mut decoder = Utf8Decoder::new();
        let mut printed = String::new();
        decoder.emit(&[0xF0, 0x80, 0x80, 0x80], |ch| printed.push(ch));
        assert_eq!(printed, "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}");
        assert!(decoder.is_empty());
    }
}
