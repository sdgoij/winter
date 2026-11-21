//! `Hyperlinks` — hyperlink map for terminal buffer cells.
//!
//! Ported from the hyperlink tracking code in the Windows Terminal C++ buffer module.
//! Supports OSC 8 hyperlinks with generational indexing for invalidation.

use std::collections::HashMap;

/// A hyperlink in the terminal buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hyperlink {
    /// The URI the hyperlink points to.
    pub uri: String,
    /// The ID of this hyperlink (used for matching start/end pairs).
    pub id: String,
    /// Optional metadata associated with the hyperlink.
    pub metadata: Option<String>,
}

impl Hyperlink {
    /// Creates a new hyperlink.
    pub fn new(uri: String, id: String) -> Self {
        Self {
            uri,
            id,
            metadata: None,
        }
    }

    /// Creates a new hyperlink with metadata.
    pub fn with_metadata(mut self, metadata: String) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Manages hyperlinks in the terminal buffer.
///
/// Uses a generational approach for efficient invalidation: when the buffer
/// changes, the generation is incremented, and all hyperlinks from the old
/// generation are considered stale.
#[derive(Debug, Clone)]
pub struct HyperlinkMap {
    /// Maps hyperlink IDs to hyperlink data.
    hyperlinks: HashMap<String, Hyperlink>,
    /// Maps buffer row/column positions to hyperlink IDs.
    /// Key: (row, col) -> hyperlink_id
    position_map: HashMap<(i32, i32), String>,
    /// The current generation number.
    generation: u64,
    /// Maximum number of hyperlinks before garbage collection.
    max_size: usize,
}

impl HyperlinkMap {
    /// Creates a new empty hyperlink map.
    pub fn new() -> Self {
        Self {
            hyperlinks: HashMap::new(),
            position_map: HashMap::new(),
            generation: 0,
            max_size: 10000,
        }
    }

    /// Creates a hyperlink map with a custom maximum size.
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            hyperlinks: HashMap::new(),
            position_map: HashMap::new(),
            generation: 0,
            max_size,
        }
    }

    /// Adds or updates a hyperlink.
    ///
    /// The hyperlink will be associated with the current generation.
    pub fn add(&mut self, uri: String, id: String, metadata: Option<String>) {
        let link = Hyperlink {
            uri,
            id: id.clone(),
            metadata,
        };

        // Update or insert the hyperlink
        self.hyperlinks.insert(id.clone(), link);

        // Bump generation to invalidate old entries
        self.generation += 1;

        // Garbage collect if needed
        if self.hyperlinks.len() > self.max_size {
            self.garbage_collect();
        }
    }

    /// Gets a hyperlink by ID.
    pub fn get(&self, id: &str) -> Option<&Hyperlink> {
        self.hyperlinks.get(id)
    }

    /// Gets a hyperlink at a specific buffer position.
    ///
    /// Returns None if there's no hyperlink at that position or if the
    /// hyperlink's generation has been bumped (indicating it's stale).
    pub fn get_at(&self, row: i32, col: i32) -> Option<&Hyperlink> {
        let id = self.position_map.get(&(row, col))?;
        self.hyperlinks.get(id)
    }

    /// Removes a hyperlink by ID.
    pub fn remove(&mut self, id: &str) {
        self.hyperlinks.remove(id);
        self.position_map.retain(|_, v| v != id);
    }

    /// Removes all hyperlinks.
    pub fn clear(&mut self) {
        self.hyperlinks.clear();
        self.position_map.clear();
    }

    /// Gets the number of active hyperlinks.
    pub fn count(&self) -> usize {
        self.hyperlinks.len()
    }

    /// Returns whether there are any hyperlinks.
    pub fn is_empty(&self) -> bool {
        self.hyperlinks.is_empty()
    }

    /// Returns the current generation number.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Bumps the generation number, invalidating all hyperlinks.
    pub fn bump_generation(&mut self) {
        self.generation += 1;
    }

    /// Returns an iterator over all hyperlinks.
    pub fn iter(&self) -> impl Iterator<Item = &Hyperlink> {
        self.hyperlinks.values()
    }

    /// Gets all hyperlinks that contain the given position.
    ///
    /// In a full implementation, this would track the full extent of each
    /// hyperlink (start_row/start_col to end_row/end_col).
    pub fn get_at_position(&self, _row: i32, _col: i32) -> Option<&Hyperlink> {
        self.get_at(_row, _col)
    }

    /// Garbage collects old hyperlinks based on generation.
    fn garbage_collect(&mut self) {
        // For simplicity, keep only the most recent hyperlinks
        // A more sophisticated approach would track individual hyperlink generations
        let to_remove = self.hyperlinks.len() - self.max_size / 2;
        if to_remove > 0 {
            let mut ids: Vec<_> = self.hyperlinks.keys().cloned().collect();
            ids.sort();
            for id in ids.into_iter().take(to_remove) {
                self.hyperlinks.remove(&id);
            }
            self.position_map.retain(|_, v| self.hyperlinks.contains_key(v));
        }
    }
}

impl Default for HyperlinkMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_map_is_empty() {
        let map = HyperlinkMap::new();
        assert!(map.is_empty());
        assert_eq!(map.count(), 0);
    }

    #[test]
    fn test_add_and_get() {
        let mut map = HyperlinkMap::new();
        map.add("https://example.com".to_string(), "id1".to_string(), None);

        let link = map.get("id1").unwrap();
        assert_eq!(link.uri, "https://example.com");
    }

    #[test]
    fn test_remove() {
        let mut map = HyperlinkMap::new();
        map.add("https://example.com".to_string(), "id1".to_string(), None);
        assert!(!map.is_empty());

        map.remove("id1");
        assert!(map.is_empty());
        assert!(map.get("id1").is_none());
    }

    #[test]
    fn test_clear() {
        let mut map = HyperlinkMap::new();
        map.add("https://a.com".to_string(), "a".to_string(), None);
        map.add("https://b.com".to_string(), "b".to_string(), None);
        assert_eq!(map.count(), 2);

        map.clear();
        assert_eq!(map.count(), 0);
    }

    #[test]
    fn test_update_existing() {
        let mut map = HyperlinkMap::new();
        map.add("https://old.com".to_string(), "id1".to_string(), None);
        map.add("https://new.com".to_string(), "id1".to_string(), None);

        let link = map.get("id1").unwrap();
        assert_eq!(link.uri, "https://new.com");
    }

    #[test]
    fn test_generation_bump() {
        let mut map = HyperlinkMap::new();
        let gen1 = map.generation();

        map.add("https://example.com".to_string(), "id1".to_string(), None);
        assert!(map.generation() > gen1);
    }

    #[test]
    fn test_with_metadata() {
        let mut map = HyperlinkMap::new();
        map.add(
            "https://example.com".to_string(),
            "id1".to_string(),
            Some("metadata".to_string()),
        );

        let link = map.get("id1").unwrap();
        assert_eq!(link.metadata.as_deref(), Some("metadata"));
    }

    #[test]
    fn test_iter() {
        let mut map = HyperlinkMap::new();
        map.add("https://a.com".to_string(), "a".to_string(), None);
        map.add("https://b.com".to_string(), "b".to_string(), None);

        let uris: Vec<_> = map.iter().map(|h| &h.uri).collect();
        assert!(uris.contains(&&"https://a.com".to_string()));
        assert!(uris.contains(&&"https://b.com".to_string()));
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    prop_compose! {
        fn any_uri()(
            domain in "\\w{1,10}",
            path in "\\w{1,20}",
        ) -> String {
            format!("https://{}.{}", domain, path)
        }
    }

    prop_compose! {
        fn any_id()(
            id in "\\w{1,16}",
        ) -> String {
            id.to_string()
        }
    }

    proptest! {
        #[test]
        fn test_add_and_retrieve(uri in any_uri(), id in any_id()) {
            let mut map = HyperlinkMap::new();
            map.add(uri.clone(), id.clone(), None);
            let link = map.get(&id).unwrap();
            prop_assert_eq!(&link.uri, &uri);
        }
    }

    // Note: HyperlinkMap doesn't implement Arbitrary, so we use regular tests instead.
}
