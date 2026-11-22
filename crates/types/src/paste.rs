//! `FilterStringForPaste` — cleaning clipboard text before it reaches a pty.
//!
//! Ported from `src/types/utils.cpp`. Pasted text is untrusted input: it can
//! carry control codes that would drive the terminal, including the `ESC[201~`
//! that closes a bracketed paste. The filter drops C0 and C1 control codes
//! (except HT, LF and CR) and rewrites every line ending to one carriage
//! return.

/// Which parts of the paste filter to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FilterOption(u8);

impl FilterOption {
    /// Replace line endings with a single carriage return.
    pub const CARRIAGE_RETURN_NEWLINE: Self = Self(1 << 0);
    /// Remove C0 and C1 control codes, keeping HT, LF and CR.
    pub const CONTROL_CODES: Self = Self(1 << 1);
    /// The set a clipboard paste uses.
    pub const PASTE: Self = Self(Self::CARRIAGE_RETURN_NEWLINE.0 | Self::CONTROL_CODES.0);

    const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Filters `text` for a paste, applying each selected rule.
pub fn filter_string_for_paste(text: &str, option: FilterOption) -> String {
    let mut filtered = String::with_capacity(text.len());
    let mut begin = 0;
    let mut previous = None;

    for (position, character) in text.char_indices() {
        if option.contains(FilterOption::CARRIAGE_RETURN_NEWLINE) && character == '\n' {
            // Copy up to but not including the newline, then keep the carriage
            // return that was already there or add one.
            filtered.push_str(&text[begin..position]);
            if previous != Some('\r') {
                filtered.push('\r');
            }
            begin = position + character.len_utf8();
        } else if option.contains(FilterOption::CONTROL_CODES) && is_control_code(character) {
            filtered.push_str(&text[begin..position]);
            begin = position + character.len_utf8();
        }
        previous = Some(character);
    }

    // `begin` stays zero only when nothing was removed or rewritten.
    if begin == 0 {
        return text.to_string();
    }
    filtered.push_str(&text[begin..]);
    filtered
}

/// Returns whether `character` is a C0 or C1 control code the filter removes.
fn is_control_code(character: char) -> bool {
    let code = character as u32;
    if (0x20..0x7f).contains(&code) {
        // Printable ASCII.
        return false;
    }
    if code > 0x9f {
        // Not a control code.
        return false;
    }
    // Horizontal tab, line feed and carriage return are kept.
    !matches!(code, 0x09 | 0x0a | 0x0d)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASTE: FilterOption = FilterOption::PASTE;

    #[test]
    fn control_codes_are_removed_and_line_endings_untouched() {
        let text = "Hello\r\nWorld\r\n\x01\x02\x03 123";
        assert_eq!(
            filter_string_for_paste(text, FilterOption::CONTROL_CODES),
            "Hello\r\nWorld\r\n 123"
        );
    }

    #[test]
    fn every_line_ending_becomes_a_carriage_return() {
        for text in ["Hello\rWorld\r\x01\x02\x03 123", "Hello\nWorld\n\x01\x02\x03 123"] {
            assert_eq!(filter_string_for_paste(text, PASTE), "Hello\rWorld\r 123");
        }
        assert_eq!(
            filter_string_for_paste("Hello\r\nWorld\r\n\x01\x02\x03 123", PASTE),
            "Hello\rWorld\r 123"
        );
    }

    #[test]
    fn backspaces_and_other_controls_are_dropped() {
        let text = "e\u{8}c\u{8}h\u{8}o\u{8} \u{8}'.\u{8}!\u{8}:\u{8}\u{8} \u{8}ke\u{8}S\u{8} \u{8}i3\u{8}l \u{8}ld\u{8}K\u{8}o\u{8} -1\u{8}+\u{8}9 +\u{8}2\u{8}-1'\u{8} >\u{8} \u{8}/\u{8}t\u{8}m\u{8}p\u{8}/\u{8}l\u{8}o\u{8}l\u{8}\r\nsleep 1\r\nmd5sum /tmp/lol";
        assert_eq!(
            filter_string_for_paste(text, PASTE),
            "echo '.!: keS i3l ldKo -1+9 +2-1' > /tmp/lol\rsleep 1\rmd5sum /tmp/lol"
        );
    }

    #[test]
    fn a_pasted_end_marker_cannot_close_the_block() {
        assert_eq!(filter_string_for_paste("echo\x1b[201~", PASTE), "echo[201~");
    }

    #[test]
    fn c1_control_codes_are_removed() {
        assert_eq!(filter_string_for_paste("echo\u{9c}", PASTE), "echo");
    }

    #[test]
    fn non_latin_text_survives() {
        assert_eq!(
            filter_string_for_paste("你好\r\n\x01世界\x02\r\n123", PASTE),
            "你好\r世界\r123"
        );
    }

    #[test]
    fn tabs_are_kept() {
        assert_eq!(filter_string_for_paste("a\tb", PASTE), "a\tb");
    }

    #[test]
    fn no_options_leaves_the_text_alone() {
        assert_eq!(
            filter_string_for_paste("a\r\n\x01b", FilterOption::default()),
            "a\r\n\x01b"
        );
    }

    #[test]
    fn an_empty_string_stays_empty() {
        assert_eq!(filter_string_for_paste("", PASTE), "");
    }
}
