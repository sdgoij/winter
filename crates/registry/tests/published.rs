//! The registry served from GitHub Pages, and the archives beside it, must be
//! what the default URL promises (ADR 088).
//!
//! `extensions/registry.toml` is checked in rather than fetched, so this test is
//! both the author's pre-flight and CI's validation of what the `pages` job
//! publishes.

use std::path::{Path, PathBuf};

use registry::Registry;
use sha2::{Digest, Sha256};

/// The prefix the Pages job serves the `extensions` directory under.
const SITE: &str = "https://sdgoij.github.io/winter/extensions/";

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn published() -> Registry {
    let path = repository().join("extensions/registry.toml");
    Registry::load(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn the_published_registry_is_valid() {
    let registry = published();
    assert_eq!(registry.schema, registry::SCHEMA);
    assert!(!registry.name.trim().is_empty());
}

#[test]
fn every_archive_beside_the_registry_matches_its_checksum() {
    for extension in &published().extensions {
        for release in &extension.versions {
            let Some(archive) = release.archive() else {
                continue;
            };
            // An archive may live on any host; only the ones this job serves are
            // checked against a file in the tree.
            let Some(name) = archive.url.strip_prefix(SITE) else {
                continue;
            };
            let path = repository().join("extensions").join(name);
            let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let digest: String = Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(
                digest,
                archive.sha256,
                "{} does not match the registry's checksum",
                path.display()
            );
        }
    }
}
