//! `cmdline.cpp` — word delimiters.
//!
//! Ported from `src/host/cmdline.cpp`. Upstream keeps the user's delimiter
//! list in `globals.WordDelimiters`, filled from the registry; [`crate::registry`]
//! owns the defaults and the key that replaces them. Space is always a
//! delimiter and is deliberately not part of the list, because the list holds
//! only the configurable ones.

/// The console's default word delimiters, from `registry.cpp`.
pub use crate::registry::DEFAULT_WORD_DELIMITERS;

/// Returns whether `character` separates words.
pub fn is_word_delim(character: char) -> bool {
    character == ' ' || DEFAULT_WORD_DELIMITERS.contains(&character)
}

/// Returns whether a cell's text is a single delimiter.
///
/// A cell holding several characters, or none, is not a delimiter, which is
/// what upstream's `wstring_view` overload checks.
pub fn is_word_delim_text(text: &str) -> bool {
    let mut characters = text.chars();
    matches!((characters.next(), characters.next()), (Some(character), None) if is_word_delim(character))
}

/// Returns the class upstream's `DelimiterClass` assigns: 0 for a regular
/// character, 1 for a space and 2 for a configured delimiter.
///
/// The distinction between whitespace and other delimiters is what makes the
/// classic Windows word-wise movement inconsistent.
pub fn delimiter_class(character: char) -> u8 {
    if character == ' ' {
        1
    } else if DEFAULT_WORD_DELIMITERS.contains(&character) {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_and_every_configured_delimiter_break_words() {
        assert!(is_word_delim(' '));
        for delimiter in DEFAULT_WORD_DELIMITERS {
            assert!(is_word_delim(*delimiter), "expected {delimiter:?} to be a delimiter");
        }
        assert!(!is_word_delim('a'));
        assert!(!is_word_delim('\u{00e9}'));
    }

    #[test]
    fn a_delimiter_text_is_exactly_one_delimiter() {
        assert!(is_word_delim_text(" "));
        assert!(is_word_delim_text("/"));
        assert!(!is_word_delim_text(""));
        assert!(!is_word_delim_text("a"));
        assert!(!is_word_delim_text("/a"));
    }

    #[test]
    fn the_classes_separate_space_from_other_delimiters() {
        assert_eq!(delimiter_class('a'), 0);
        assert_eq!(delimiter_class(' '), 1);
        assert_eq!(delimiter_class('/'), 2);
    }
}
