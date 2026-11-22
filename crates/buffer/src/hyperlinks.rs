//! `Hyperlinks` — the buffer's OSC 8 hyperlink registry.
//!
//! Ported from the hyperlink bookkeeping in `TextBuffer`
//! (`_hyperlinkMap`, `_hyperlinkCustomIdMap`, `_currentHyperlinkId`). A cell
//! carries the numeric id in its text attribute (`TextAttribute::hyperlink_id`);
//! this map resolves it back to a URI.
//!
//! Upstream keys a caller-supplied id with a hash of the URI so the same custom
//! id used for a different target is a different link. The key here is the
//! `(custom id, uri)` pair, which has the same effect without needing the hash.

use std::collections::HashMap;

/// A hyperlink registered in the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hyperlink {
    /// The URI the hyperlink points to.
    pub uri: String,
    /// The caller-supplied id from OSC 8; empty when the terminal allocated one.
    pub custom_id: String,
}

impl Hyperlink {
    /// Creates a new hyperlink.
    pub fn new(uri: String, custom_id: String) -> Self {
        Self { uri, custom_id }
    }
}

/// Resolves the numeric hyperlink ids that cells carry back to their URIs.
#[derive(Debug, Clone)]
pub struct HyperlinkMap {
    /// Maps the numeric id to the hyperlink.
    links: HashMap<u16, Hyperlink>,
    /// Maps a `(custom id, uri)` pair to the numeric id, so a repeated OSC 8
    /// start reuses the same link instead of allocating a new one.
    custom_ids: HashMap<(String, String), u16>,
    /// The next id to hand out. Zero means "no hyperlink", so it is skipped.
    next_id: u16,
}

impl HyperlinkMap {
    /// Creates a new empty hyperlink map.
    pub fn new() -> Self {
        Self {
            links: HashMap::new(),
            custom_ids: HashMap::new(),
            next_id: 1,
        }
    }

    /// Registers a hyperlink and returns the numeric id cells should carry.
    ///
    /// Ported from `TextBuffer::GetHyperlinkId` plus `AddHyperlinkToMap`: every
    /// upstream call site registers and stores together, so they are one step
    /// here. An empty `custom_id` always allocates; a repeated custom id for
    /// the same URI returns the id allocated the first time.
    pub fn register(&mut self, uri: &str, custom_id: &str) -> u16 {
        let id = if custom_id.is_empty() {
            self.allocate()
        } else {
            let key = (custom_id.to_string(), uri.to_string());
            match self.custom_ids.get(&key) {
                Some(&id) => id,
                None => {
                    let id = self.allocate();
                    self.custom_ids.insert(key, id);
                    id
                }
            }
        };

        self.links.insert(
            id,
            Hyperlink {
                uri: uri.to_string(),
                custom_id: custom_id.to_string(),
            },
        );
        id
    }

    /// Allocates a fresh numeric id, skipping the zero sentinel.
    fn allocate(&mut self) -> u16 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        if self.next_id == 0 {
            self.next_id = 1;
        }
        id
    }

    /// Gets a hyperlink by its numeric id.
    pub fn get(&self, id: u16) -> Option<&Hyperlink> {
        self.links.get(&id)
    }

    /// Resolves a numeric id to its URI, which is what the renderer needs.
    pub fn uri(&self, id: u16) -> Option<&str> {
        self.links.get(&id).map(|link| link.uri.as_str())
    }

    /// Removes a hyperlink by its numeric id.
    pub fn remove(&mut self, id: u16) {
        self.links.remove(&id);
        self.custom_ids.retain(|_, value| *value != id);
    }

    /// Removes all hyperlinks.
    pub fn clear(&mut self) {
        self.links.clear();
        self.custom_ids.clear();
    }

    /// Returns the number of registered hyperlinks.
    pub fn len(&self) -> usize {
        self.links.len()
    }

    /// Returns whether there are any hyperlinks.
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// Returns an iterator over all hyperlinks.
    pub fn iter(&self) -> impl Iterator<Item = &Hyperlink> {
        self.links.values()
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
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn ids_start_at_one_and_resolve_to_the_uri() {
        let mut map = HyperlinkMap::new();
        let id = map.register("https://example.com", "");

        assert_eq!(id, 1);
        assert_eq!(map.uri(id), Some("https://example.com"));
        assert_eq!(map.get(id).unwrap().custom_id, "");
        assert!(!map.is_empty());
    }

    #[test]
    fn repeated_custom_ids_for_the_same_uri_reuse_the_link() {
        let mut map = HyperlinkMap::new();
        let first = map.register("https://example.com", "custom");
        let second = map.register("https://example.com", "custom");

        assert_eq!(first, second);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn the_same_custom_id_for_a_different_uri_allocates_a_new_link() {
        let mut map = HyperlinkMap::new();
        let first = map.register("https://one.example", "custom");
        let second = map.register("https://two.example", "custom");

        assert_ne!(first, second);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn an_empty_custom_id_always_allocates() {
        let mut map = HyperlinkMap::new();
        let first = map.register("https://example.com", "");
        let second = map.register("https://example.com", "");

        assert_ne!(first, second);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn removing_a_link_frees_its_custom_id_for_a_new_allocation() {
        let mut map = HyperlinkMap::new();
        let first = map.register("https://example.com", "custom");
        map.remove(first);

        assert!(map.is_empty());
        assert_eq!(map.uri(first), None);

        let second = map.register("https://example.com", "custom");
        assert_ne!(second, first);
    }

    #[test]
    fn clearing_removes_links_and_custom_ids() {
        let mut map = HyperlinkMap::new();
        map.register("https://a.example", "a");
        map.register("https://b.example", "b");

        map.clear();

        assert!(map.is_empty());
        let id = map.register("https://a.example", "a");
        assert_eq!(map.len(), 1);
        assert_eq!(map.uri(id), Some("https://a.example"));
    }

    #[test]
    fn allocation_skips_zero_when_the_id_wraps() {
        let mut map = HyperlinkMap::new();
        map.next_id = u16::MAX;

        let last = map.register("https://last.example", "");
        let wrapped = map.register("https://wrapped.example", "");

        assert_eq!(last, u16::MAX);
        assert_eq!(wrapped, 1);
        assert!(map.get(0).is_none());
    }

    #[test]
    fn iter_visits_every_link() {
        let mut map = HyperlinkMap::new();
        map.register("https://a.example", "");
        map.register("https://b.example", "");

        let uris: Vec<&str> = map.iter().map(|link| link.uri.as_str()).collect();
        assert!(uris.contains(&"https://a.example"));
        assert!(uris.contains(&"https://b.example"));
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn test_add_and_retrieve(domain in "\\w{1,10}", path in "\\w{1,20}") {
            let uri = format!("https://{}.{}", domain, path);
            let mut map = HyperlinkMap::new();
            let id = map.register(&uri, "");
            prop_assert_eq!(map.uri(id), Some(uri.as_str()));
        }

        #[test]
        fn allocated_ids_are_never_zero(count in 0usize..1024) {
            let mut map = HyperlinkMap::new();
            for _ in 0..count {
                prop_assert_ne!(map.register("https://example.com", ""), 0);
            }
            prop_assert_eq!(map.len(), count);
        }
    }
}
