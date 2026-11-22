//! Text search for the terminal buffer.
//!
//! Ported from `search.cpp`, `TextBuffer::SearchText` and the UText adapter: the
//! rows are joined into logical lines, a line break follows only a row that was
//! not wrapped, and matches are reported in buffer coordinates as half-open
//! spans.
//!
//! Not ported: the ICU regular-expression mode. `find_in_rows` is a literal
//! match that ignores case with Unicode lowercasing.

use crate::row::Row;

/// A match in document coordinates.
///
/// The range is half-open: `start` is the first cell and `end` is one cell past
/// the last. A match can run through an autowrap, in which case it starts on one
/// document row and ends on another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextMatch {
    /// The document row the first cell is on.
    pub start_row: i32,
    /// The column of the first cell.
    pub start_column: i32,
    /// The document row the last cell is on.
    pub end_row: i32,
    /// The column one past the last cell on `end_row`.
    pub end_column: i32,
}

impl TextMatch {
    /// The cells the match covers on `document_row`, as `(first column,
    /// columns covered)`, or `None` when it covers none.
    ///
    /// `width` is the screen width, which is how far a row that continues on the
    /// next one extends.
    pub fn row_span(&self, document_row: i32, width: i32) -> Option<(i32, i32)> {
        if document_row < self.start_row || document_row > self.end_row {
            return None;
        }

        let start = if document_row == self.start_row {
            self.start_column
        } else {
            0
        };
        let end = if document_row == self.end_row {
            self.end_column
        } else {
            width
        };
        (end > start).then_some((start, end - start))
    }
}

/// One document row as the search reads it.
struct RowSpan {
    /// The index into the rows the caller passed.
    index: usize,
    /// The document row number.
    row: i32,
    /// Where the row's text starts in the joined text.
    start: usize,
}

/// Finds every occurrence of `needle` in `rows`, in document order.
///
/// The rows are joined the way the buffer reads them: each contributes its
/// readable cells, and a row that was not wrapped contributes a line break after
/// them, so a match may run through an autowrap but never through a real line
/// break. A whitespace-only needle finds nothing, because it would match the
/// buffer's unwritten cells. Matching ignores case and does not overlap.
pub fn find_in_rows(rows: &[(i32, &Row)], needle: &str) -> Vec<TextMatch> {
    let mut matches = Vec::new();
    if needle.chars().all(|character| character.is_whitespace()) {
        return matches;
    }

    let mut text = Vec::new();
    let mut spans = Vec::with_capacity(rows.len());
    for (index, (row_number, row)) in rows.iter().enumerate() {
        let start = text.len();
        text.extend(row.get_text_range(0, row.measure_right()).chars());
        spans.push(RowSpan {
            index,
            row: *row_number,
            start,
        });
        if !row.is_wrap() {
            text.push('\n');
        }
    }

    let (folded, owners) = fold(&text);
    let wanted = fold_str(needle);
    if wanted.is_empty() || wanted.len() > folded.len() {
        return matches;
    }

    let mut start = 0;
    while start + wanted.len() <= folded.len() {
        let hit = folded[start..start + wanted.len()]
            .iter()
            .zip(&wanted)
            .all(|(found, wanted)| found == wanted);
        if !hit {
            start += 1;
            continue;
        }

        // Folded offsets map back through the characters that produced them, so
        // a match's ends land on cell boundaries.
        let end = start + wanted.len();
        let first = owners[start];
        let last = owners[end - 1];
        let (start_row, start_column) = position_of(&spans, rows, first);
        let (end_row, end_column) = position_of(&spans, rows, last + 1);
        matches.push(TextMatch {
            start_row,
            start_column,
            end_row,
            end_column,
        });
        start = end;
    }

    matches
}

/// Lowercases every character, keeping the index of the character it came from.
fn fold(text: &[char]) -> (Vec<char>, Vec<usize>) {
    let mut folded = Vec::with_capacity(text.len());
    let mut owners = Vec::with_capacity(text.len());
    for (index, character) in text.iter().enumerate() {
        for lower in character.to_lowercase() {
            folded.push(lower);
            owners.push(index);
        }
    }
    (folded, owners)
}

fn fold_str(text: &str) -> Vec<char> {
    text.chars().flat_map(|character| character.to_lowercase()).collect()
}

/// The document position of the `offset`th character of the joined text.
fn position_of(spans: &[RowSpan], rows: &[(i32, &Row)], offset: usize) -> (i32, i32) {
    let index = spans.partition_point(|span| span.start <= offset).saturating_sub(1);
    let Some(span) = spans.get(index) else {
        return (0, 0);
    };

    let row = rows[span.index].1;
    let inner = offset.saturating_sub(span.start);
    (span.row, column_at_char_offset(row, inner))
}

/// The column the `offset`th character of `row` starts in.
///
/// A wide character's trailing half holds no character of its own, so the
/// offset of the character after it skips a column.
fn column_at_char_offset(row: &Row, offset: usize) -> i32 {
    let content = row.measure_right();
    let mut seen = 0;
    for column in 0..content {
        if row.is_trailing_half(column) {
            continue;
        }
        let count = row.get_cell(column).chars().chars().count();
        if offset < seen + count {
            return column;
        }
        seen += count;
    }
    content
}

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

    fn row_of(text: &str, width: i32) -> Row {
        use crate::output_cell::OutputCell;

        let mut row = Row::new(width);
        let mut column = 0;
        for character in text.chars() {
            let cell = OutputCell::from_char(character);
            let advance = i32::from(cell.column_width().max(1));
            row.write_cell(column, cell);
            column += advance;
        }
        row
    }

    fn wrapped(mut row: Row) -> Row {
        row.force_wrap(1);
        row
    }

    #[test]
    fn a_match_runs_through_an_autowrap() {
        let first = wrapped(row_of("abcde", 5));
        let second = row_of("fgh", 5);

        assert_eq!(
            find_in_rows(&[(0, &first), (1, &second)], "def"),
            vec![TextMatch {
                start_row: 0,
                start_column: 3,
                end_row: 1,
                end_column: 1,
            }]
        );
    }

    #[test]
    fn a_line_break_stops_a_match() {
        let first = row_of("abcde", 5);
        let second = row_of("fgh", 5);

        assert!(find_in_rows(&[(0, &first), (1, &second)], "def").is_empty());
    }

    #[test]
    fn a_match_ends_one_past_its_last_cell() {
        let row = row_of("abcde", 8);

        assert_eq!(
            find_in_rows(&[(0, &row)], "cde"),
            vec![TextMatch {
                start_row: 0,
                start_column: 2,
                end_row: 0,
                end_column: 5,
            }]
        );
    }

    #[test]
    fn a_wide_character_covers_two_columns() {
        let row = row_of("\u{4e16}a", 6);

        assert_eq!(
            find_in_rows(&[(0, &row)], "\u{4e16}"),
            vec![TextMatch {
                start_row: 0,
                start_column: 0,
                end_row: 0,
                end_column: 2,
            }]
        );
        // The character after it starts at column two, not one.
        assert_eq!(
            find_in_rows(&[(0, &row)], "a"),
            vec![TextMatch {
                start_row: 0,
                start_column: 2,
                end_row: 0,
                end_column: 3,
            }]
        );
    }

    #[test]
    fn a_whitespace_needle_finds_nothing() {
        let row = row_of("a b", 5);

        assert!(find_in_rows(&[(0, &row)], "").is_empty());
        assert!(find_in_rows(&[(0, &row)], " ").is_empty());
    }

    #[test]
    fn matching_ignores_case() {
        let row = row_of("Hello HELLO", 12);
        let found = find_in_rows(&[(0, &row)], "hello");

        assert_eq!(found.iter().map(|found| found.start_column).collect::<Vec<_>>(), [0, 6]);
    }

    #[test]
    fn matches_do_not_overlap() {
        let row = row_of("aaaa", 6);

        assert_eq!(find_in_rows(&[(0, &row)], "aa").len(), 2);
    }

    #[test]
    fn a_match_reports_the_cells_it_covers_on_each_row() {
        let found = TextMatch {
            start_row: 0,
            start_column: 3,
            end_row: 1,
            end_column: 1,
        };

        assert_eq!(found.row_span(0, 5), Some((3, 2)));
        assert_eq!(found.row_span(1, 5), Some((0, 1)));
        assert_eq!(found.row_span(2, 5), None);
    }
}
