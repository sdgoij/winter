//! Text search for the terminal buffer.
//!
//! Ported from the text search functionality in the Windows Terminal C++ buffer module.

/// Results of a text search operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    /// The starting row of the match (0-based, relative to search start).
    pub start_row: i32,
    /// The starting column of the match.
    pub start_col: i32,
    /// The length of the match in display columns.
    pub length: usize,
}

impl SearchMatch {
    /// Creates a new search match.
    pub const fn new(start_row: i32, start_col: i32, length: usize) -> Self {
        Self {
            start_row,
            start_col,
            length,
        }
    }

    /// Returns the ending column (exclusive).
    pub const fn end_col(&self) -> i32 {
        self.start_col + self.length as i32
    }

    /// Returns the ending row.
    pub const fn end_row(&self) -> i32 {
        let total = self.start_col + self.length as i32;
        if total > 0 {
            let shift = total / 80;
            if shift > 0 {
                self.start_row + shift
            } else {
                self.start_row
            }
        } else {
            self.start_row
        }
    }
}

/// Options for text search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchOptions {
    /// Whether the search is case-sensitive.
    pub case_sensitive: bool,
    /// Whether to use regex matching.
    pub use_regex: bool,
}

impl SearchOptions {
    /// Creates case-sensitive search options.
    pub const fn case_sensitive() -> Self {
        Self {
            case_sensitive: true,
            use_regex: false,
        }
    }

    /// Creates case-insensitive search options.
    pub const fn case_insensitive() -> Self {
        Self {
            case_sensitive: false,
            use_regex: false,
        }
    }

    /// Sets whether the search is case-sensitive.
    pub const fn with_case_sensitive(mut self, sensitive: bool) -> Self {
        self.case_sensitive = sensitive;
        self
    }

    /// Sets whether to use regex matching.
    pub const fn with_regex(mut self, use_regex: bool) -> Self {
        self.use_regex = use_regex;
        self
    }
}

/// Searches for text in a row string.
///
/// Returns the column offset of the first match, or `None` if not found.
pub fn search_in_row(row_text: &str, pattern: &str, options: SearchOptions) -> Option<usize> {
    let search_text = if options.case_sensitive {
        row_text.to_string()
    } else {
        row_text.to_lowercase()
    };

    let search_pattern = if options.case_sensitive {
        pattern.to_string()
    } else {
        pattern.to_lowercase()
    };

    if search_text.contains(&search_pattern) {
        Some(search_text.find(&search_pattern).unwrap())
    } else {
        None
    }
}

/// Searches for a pattern across multiple rows.
///
/// Returns all matching positions (row, column) where the pattern starts.
pub fn search_across_rows(rows: &[String], pattern: &str, options: SearchOptions) -> Vec<(i32, usize)> {
    let mut results = Vec::new();

    for (row_idx, row_text) in rows.iter().enumerate() {
        if let Some(col) = search_in_row(row_text, pattern, options) {
            results.push((row_idx as i32, col));
        }
    }

    results
}

/// Simple regex-like pattern matching for basic patterns.
///
/// Supports:
/// - `*` — matches any sequence of characters (including empty)
/// - `?` — matches any single character
pub fn matches_simple_regex(text: &str, pattern: &str) -> bool {
    if !pattern.contains('*') && !pattern.contains('?') {
        return text == pattern;
    }

    simple_match(text.as_bytes(), pattern.as_bytes())
}

/// Core simple regex matching.
fn simple_match(text: &[u8], pattern: &[u8]) -> bool {
    // Use dynamic programming approach
    let text_len = text.len();
    let pattern_len = pattern.len();

    // dp[i][j] = true if text[0..i] matches pattern[0..j]
    let mut dp = vec![vec![false; pattern_len + 1]; text_len + 1];
    dp[0][0] = true;

    // Handle leading * in pattern
    for j in 1..=pattern_len {
        if pattern[j - 1] == b'*' {
            dp[0][j] = dp[0][j - 1];
        }
    }

    for i in 1..=text_len {
        for j in 1..=pattern_len {
            if pattern[j - 1] == b'?' || pattern[j - 1] == text[i - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            } else if pattern[j - 1] == b'*' {
                // * matches empty or one or more chars
                dp[i][j] = dp[i][j - 1] || dp[i - 1][j];
            }
        }
    }

    dp[text_len][pattern_len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_found() {
        let result = search_in_row("Hello World", "World", SearchOptions::default());
        assert_eq!(result, Some(6));
    }

    #[test]
    fn test_search_not_found() {
        let result = search_in_row("Hello World", "xyz", SearchOptions::default());
        assert_eq!(result, None);
    }

    #[test]
    fn test_search_case_sensitive() {
        assert_eq!(search_in_row("Hello", "hello", SearchOptions::case_sensitive()), None);
        assert_eq!(
            search_in_row("Hello", "hello", SearchOptions::case_insensitive()),
            Some(0)
        );
    }

    #[test]
    fn test_search_empty_text() {
        assert_eq!(search_in_row("", "", SearchOptions::default()), Some(0));
        assert_eq!(search_in_row("", "a", SearchOptions::default()), None);
    }

    #[test]
    fn test_search_empty_pattern() {
        assert_eq!(search_in_row("Hello", "", SearchOptions::default()), Some(0));
    }

    #[test]
    fn test_search_across_rows() {
        let rows = vec![
            "Hello World".to_string(),
            "Goodbye World".to_string(),
            "Hello Again".to_string(),
        ];
        let results = search_across_rows(&rows, "World", SearchOptions::default());
        assert_eq!(results, vec![(0, 6), (1, 8)]);
    }

    #[test]
    fn test_search_across_rows_multiple() {
        let rows = vec!["test".to_string(), "TEST".to_string(), "test".to_string()];
        let results = search_across_rows(&rows, "test", SearchOptions::case_insensitive());
        assert_eq!(results, vec![(0, 0), (1, 0), (2, 0)]);
    }

    #[test]
    fn test_search_match_option() {
        let opts = SearchOptions::case_sensitive().with_regex(false);
        assert!(opts.case_sensitive);
        assert!(!opts.use_regex);
    }

    #[test]
    fn test_simple_regex_exact_match() {
        assert!(matches_simple_regex("abc", "abc"));
        assert!(!matches_simple_regex("abc", "xyz"));
    }

    #[test]
    fn test_simple_regex_star() {
        assert!(matches_simple_regex("abc", "a*c"));
        assert!(matches_simple_regex("abxc", "a*c"));
        assert!(matches_simple_regex("ac", "a*c"));
        assert!(!matches_simple_regex("abd", "a*c"));
    }

    #[test]
    fn test_simple_regex_question() {
        assert!(matches_simple_regex("abc", "a?c"));
        assert!(matches_simple_regex("axc", "a?c"));
        assert!(!matches_simple_regex("ac", "a?c"));
    }

    #[test]
    fn test_simple_regex_multiple_stars() {
        assert!(matches_simple_regex("abcdef", "a*c*f"));
        assert!(matches_simple_regex("acaf", "a*c*f"));
        assert!(matches_simple_regex("acdf", "a*c*f"));
        assert!(matches_simple_regex("abcef", "a*c*f")); // a*bc*ef -> a + b + c + e + f
        assert!(!matches_simple_regex("abc", "a*c*f"));
        assert!(!matches_simple_regex("abcde", "a*c*f"));
        assert!(!matches_simple_regex("aXf", "a*c*f")); // X doesn't match c
    }

    #[test]
    fn test_search_match_length() {
        let m = SearchMatch::new(0, 5, 10);
        assert_eq!(m.start_row, 0);
        assert_eq!(m.start_col, 5);
        assert_eq!(m.length, 10);
        assert_eq!(m.end_col(), 15);
    }
}
