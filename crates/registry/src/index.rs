//! Several registries merged into the rows a shell searches.
//!
//! A registry is identified by its URL, and two registries may list the same
//! extension identifier. Those are two rows, not one: the user picks which
//! registry to install from, and the provenance record keeps the answer.

use crate::{Extension, Registry};

/// One fetched registry and the URL it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The URL identity of the registry.
    pub url: String,
    /// The parsed document.
    pub document: Registry,
}

/// One search result: a registry and one extension it lists.
#[derive(Debug, Clone, Copy)]
pub struct Row<'a> {
    /// The registry the row came from.
    pub registry: &'a Loaded,
    /// The extension the registry lists.
    pub extension: &'a Extension,
}

/// The registries loaded this session, in the order they were added.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    registries: Vec<Loaded>,
}

impl Index {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a registry, replacing the document when its URL is already here.
    ///
    /// A refresh is the same registry with a newer document, not a second one.
    pub fn add(&mut self, url: impl Into<String>, document: Registry) {
        let url = url.into();
        if let Some(existing) = self.registries.iter_mut().find(|loaded| loaded.url == url) {
            existing.document = document;
        } else {
            self.registries.push(Loaded { url, document });
        }
    }

    /// The registries loaded, in order.
    pub fn registries(&self) -> &[Loaded] {
        &self.registries
    }

    /// Whether no registry has been added.
    pub fn is_empty(&self) -> bool {
        self.registries.is_empty()
    }

    /// Every row, registry by registry and in the document's order.
    pub fn rows(&self) -> impl Iterator<Item = Row<'_>> {
        self.registries.iter().flat_map(|loaded| {
            loaded.document.extensions.iter().map(move |extension| Row {
                registry: loaded,
                extension,
            })
        })
    }

    /// The rows whose identifier, name, description or tags contain `query`,
    /// case-insensitively.
    ///
    /// An empty query keeps every row, so the Search panel can show the whole
    /// catalogue before the user types.
    pub fn matching(&self, query: &str) -> Vec<Row<'_>> {
        let needle = query.trim().to_lowercase();
        self.rows()
            .filter(|row| needle.is_empty() || row.matches(&needle))
            .collect()
    }
}

impl Row<'_> {
    /// The registry's display name, which validation guarantees is not empty.
    pub fn registry_name(&self) -> &str {
        &self.registry.document.name
    }

    /// Whether `needle` (already lowercased) appears in the row's searchable
    /// text.
    fn matches(&self, needle: &str) -> bool {
        let extension = self.extension;
        extension.id.to_lowercase().contains(needle)
            || extension.name.to_lowercase().contains(needle)
            || extension
                .description
                .as_deref()
                .is_some_and(|description| description.to_lowercase().contains(needle))
            || extension.tags.iter().any(|tag| tag.to_lowercase().contains(needle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn document(name: &str, entries: &[(&str, &str, &[&str])]) -> Registry {
        let mut text = format!("schema = 1\nname = \"{name}\"\n");
        for (id, label, tags) in entries {
            let tags = tags
                .iter()
                .map(|tag| format!("\"{tag}\""))
                .collect::<Vec<_>>()
                .join(", ");
            text.push_str(&format!(
                "[[extensions]]\nid = \"{id}\"\nname = \"{label}\"\ndescription = \"notifies when a command fails\"\ntags = [{tags}]\n[[extensions.versions]]\nversion = \"1.0.0\"\napi = 1\nsource = {{ type = \"archive\", url = \"https://example.com/{id}.zip\", sha256 = \"{SHA}\" }}\n"
            ));
        }
        Registry::parse(&text).expect("registry")
    }

    #[test]
    fn rows_keep_duplicates_from_different_registries() {
        let mut index = Index::new();
        index.add(
            "https://one.example/registry.toml",
            document("One", &[("a.b", "Alpha", &[])]),
        );
        index.add(
            "https://two.example/registry.toml",
            document("Two", &[("a.b", "Alpha", &[])]),
        );

        let rows = index.rows().collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].registry.url, "https://one.example/registry.toml");
        assert_eq!(rows[1].registry.url, "https://two.example/registry.toml");
        assert_eq!(rows[0].extension.id, "a.b");
        assert_eq!(rows[0].registry_name(), "One");
    }

    #[test]
    fn matching_reads_the_identifier_name_description_and_tags() {
        let mut index = Index::new();
        index.add(
            "https://one.example/registry.toml",
            document("One", &[("a.b", "Alpha", &["shell"]), ("c.d", "Chatter", &["network"])]),
        );

        assert_eq!(index.matching("").len(), 2);
        assert_eq!(index.matching("a.b").len(), 1);
        assert_eq!(index.matching("ALPHA").len(), 1);
        assert_eq!(index.matching("command fails").len(), 2);
        assert_eq!(index.matching("shell").len(), 1);
        assert_eq!(index.matching("network").len(), 1);
        assert!(index.matching("zzz").is_empty());
    }

    #[test]
    fn adding_the_same_url_replaces_the_document() {
        let mut index = Index::new();
        index.add(
            "https://one.example/registry.toml",
            document("First", &[("a.b", "A", &[])]),
        );
        index.add(
            "https://one.example/registry.toml",
            document("Second", &[("c.d", "C", &[])]),
        );

        assert_eq!(index.registries().len(), 1);
        assert_eq!(index.registries()[0].document.name, "Second");
        assert_eq!(index.rows().count(), 1);
    }
}
