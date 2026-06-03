//! String replacement utilities.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::replace`.

/// Replaces the first occurrence of `from` with `to` in `s`.
pub fn replace(s: &str, from: &str, to: &str) -> String {
    if let Some(pos) = s.find(from) {
        let mut result = s[..pos].to_string();
        result.push_str(to);
        result.push_str(&s[pos + from.len()..]);
        result
    } else {
        s.to_string()
    }
}

/// Replaces all occurrences of `from` with `to` in `s`.
pub fn replace_all(s: &str, from: &str, to: &str) -> String {
    s.replace(from, to)
}

/// Replaces a substring at a specific position.
/// If `pos` is beyond the string length, appends `to` to the end.
/// If `pos + from.len() > s.len()`, replaces up to the end of `s`.
pub fn replace_at(s: &str, pos: usize, from: &str, to: &str) -> String {
    if from.is_empty() {
        // Insert at position
        let pos = pos.min(s.len());
        let mut result = String::with_capacity(s.len() + to.len());
        result.push_str(&s[..pos]);
        result.push_str(to);
        result.push_str(&s[pos..]);
        result
    } else {
        let pos = pos.min(s.len());
        let end = (pos + from.len()).min(s.len());
        let mut result = String::with_capacity(s.len() - (end - pos) + to.len());
        result.push_str(&s[..pos]);
        result.push_str(to);
        result.push_str(&s[end..]);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace() {
        assert_eq!(replace("hello world", "world", "rust"), "hello rust");
        assert_eq!(replace("hello", "xyz", "abc"), "hello");
    }

    #[test]
    fn test_replace_all() {
        assert_eq!(replace_all("aaa", "a", "b"), "bbb");
        assert_eq!(replace_all("hello", "x", "y"), "hello");
    }

    #[test]
    fn test_replace_at() {
        assert_eq!(replace_at("hello", 0, "", "xx"), "xxhello");
        assert_eq!(replace_at("hello", 3, "", "xx"), "helxxlo");
        assert_eq!(replace_at("hello", 100, "", "xx"), "helloxx");
    }
}
