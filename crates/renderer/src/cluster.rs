//! `Cluster` — a single glyph cluster drawn on the screen.
//!
//! Ported from `Cluster.hpp`. Upstream stores a UTF-16 `wstring_view`; this port
//! stores the UTF-8 `&str` the buffer's decoding path produces, so a cluster is
//! a slice of decoded text plus the number of grid columns it covers.

/// A glyph cluster and the number of grid columns it occupies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cluster<'a> {
    text: &'a str,
    columns: i32,
}

impl<'a> Cluster<'a> {
    /// Creates a cluster from `text` covering `columns` grid columns.
    pub const fn new(text: &'a str, columns: i32) -> Self {
        Self { text, columns }
    }

    /// Returns the cluster as a single character, or U+FFFD when it is not one.
    ///
    /// Upstream makes the same substitution when the text does not fit in one
    /// `wchar_t`; here the test is one Unicode scalar instead of one UTF-16
    /// unit, which is what a `&str` can guarantee.
    pub fn text_as_single(&self) -> char {
        let mut chars = self.text.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) => ch,
            _ => '\u{FFFD}',
        }
    }

    /// Returns the cluster's text.
    pub const fn text(&self) -> &'a str {
        self.text
    }

    /// Returns the number of grid columns the cluster covers.
    pub const fn columns(&self) -> i32 {
        self.columns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_character_is_returned_as_is() {
        let cluster = Cluster::new("A", 1);
        assert_eq!(cluster.text_as_single(), 'A');
        assert_eq!(cluster.text(), "A");
        assert_eq!(cluster.columns(), 1);
    }

    #[test]
    fn a_folded_cluster_returns_the_replacement_character() {
        let cluster = Cluster::new("e\u{0301}", 1);
        assert_eq!(cluster.text_as_single(), '\u{FFFD}');
        assert_eq!(cluster.text(), "e\u{0301}");
    }

    #[test]
    fn an_empty_cluster_returns_the_replacement_character() {
        assert_eq!(Cluster::default().text_as_single(), '\u{FFFD}');
        assert_eq!(Cluster::default().columns(), 0);
    }

    #[test]
    fn a_wide_character_keeps_its_column_count() {
        let cluster = Cluster::new("\u{4e2d}", 2);
        assert_eq!(cluster.text_as_single(), '\u{4e2d}');
        assert_eq!(cluster.columns(), 2);
    }
}
