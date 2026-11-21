//! String utilities.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::string`.

/// Trims whitespace from both ends of a string.
#[inline]
pub fn trim(s: &str) -> &str {
    s.trim()
}

/// Splits a string by a separator.
#[inline]
pub fn split<'a>(s: &'a str, separator: &str) -> Vec<&'a str> {
    s.split(separator).collect()
}

/// Replaces all occurrences of `from` with `to`.
#[inline]
pub fn replace_all(s: &str, from: &str, to: &str) -> String {
    s.replace(from, to)
}

/// Truncates a string to `max_len` characters and adds ellipsis if truncated.
pub fn truncate_ellipsis(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let truncated = &s[..max_len.saturating_sub(3)];
        format!("{}...", truncated)
    }
}

/// Returns whether the string starts with the given prefix.
#[inline]
pub fn starts_with(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
}

/// Returns whether the string ends with the given suffix.
#[inline]
pub fn ends_with(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}

/// Returns whether two strings are equal (case-insensitive ASCII).
pub fn equals_insensitive_ascii(s1: &str, s2: &str) -> bool {
    if s1.len() != s2.len() {
        return false;
    }
    s1.eq_ignore_ascii_case(s2)
}

/// Returns whether a string starts with the given prefix (case-insensitive ASCII).
pub fn starts_with_insensitive_ascii(s: &str, prefix: &str) -> bool {
    s.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase())
}

/// Returns whether a string ends with the given suffix (case-insensitive ASCII).
pub fn ends_with_insensitive_ascii(s: &str, suffix: &str) -> bool {
    s.to_ascii_lowercase().ends_with(&suffix.to_ascii_lowercase())
}

/// Converts a string to lowercase ASCII.
pub fn tolower_ascii(s: &str) -> String {
    s.bytes()
        .map(|b: u8| {
            if b.is_ascii_lowercase() {
                b
            } else if b.is_ascii_uppercase() {
                b + 32
            } else {
                b
            }
        })
        .map(|b| b as char)
        .collect()
}

/// Converts a string to uppercase ASCII.
pub fn toupper_ascii(s: &str) -> String {
    s.bytes()
        .map(|b: u8| {
            if b.is_ascii_uppercase() {
                b
            } else if b.is_ascii_lowercase() {
                b - 32
            } else {
                b
            }
        })
        .map(|b| b as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim() {
        assert_eq!(trim("  hello  "), "hello");
        assert_eq!(trim("hello"), "hello");
        assert_eq!(trim(""), "");
    }

    #[test]
    fn test_split() {
        let result = split("a,b,c", ",");
        assert_eq!(result, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_replace_all() {
        assert_eq!(replace_all("hello world", "world", "rust"), "hello rust");
    }

    #[test]
    fn test_truncate_ellipsis() {
        assert_eq!(truncate_ellipsis("hello", 10), "hello");
        assert_eq!(truncate_ellipsis("hello world", 8), "hello...");
    }

    #[test]
    fn test_starts_with() {
        assert!(starts_with("hello world", "hello"));
        assert!(!starts_with("hello world", "world"));
    }

    #[test]
    fn test_ends_with() {
        assert!(ends_with("hello world", "world"));
        assert!(!ends_with("hello world", "hello"));
    }

    #[test]
    fn test_equals_insensitive_ascii() {
        assert!(equals_insensitive_ascii("Hello", "HELLO"));
        assert!(equals_insensitive_ascii("hello", "hello"));
        assert!(!equals_insensitive_ascii("Hello", "World"));
    }

    #[test]
    fn test_starts_with_insensitive_ascii() {
        assert!(starts_with_insensitive_ascii("Hello World", "hello"));
        assert!(!starts_with_insensitive_ascii("Hello", "world"));
    }

    #[test]
    fn test_ends_with_insensitive_ascii() {
        assert!(ends_with_insensitive_ascii("Hello World", "WORLD"));
        assert!(!ends_with_insensitive_ascii("Hello", "WORLD"));
    }

    #[test]
    fn test_tolower_ascii() {
        assert_eq!(tolower_ascii("Hello WORLD"), "hello world");
    }

    #[test]
    fn test_toupper_ascii() {
        assert_eq!(toupper_ascii("hello world"), "HELLO WORLD");
    }
}
