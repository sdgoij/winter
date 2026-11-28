//! The published registry is packaged from the extension directories beside it,
//! so the document and its archives must always agree (ADR 088).
//!
//! `extensions/registry.toml` is checked in with placeholder checksums; this
//! test runs the same packaging the Pages job runs, into a temporary directory,
//! and checks the result.

use std::io::Read;
use std::path::{Path, PathBuf};

use registry::Registry;
use sha2::{Digest, Sha256};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_published_registry_packages() {
    let source = repository().join("extensions");
    let out = tempfile::tempdir().expect("temp");
    let packaged = registry::package::build(&source, out.path()).expect("package the registry");
    let registry = Registry::load(&packaged.registry).expect("the output registry parses");
    assert_eq!(registry.schema, registry::SCHEMA);

    for extension in &registry.extensions {
        for release in &extension.versions {
            let Some(archive) = release.archive() else {
                continue;
            };
            let name = archive
                .url
                .strip_prefix(registry::package::SITE)
                .expect("every published archive is served from the site");
            let path = out.path().join(name);

            let bytes = std::fs::read(&path).expect("archive");
            let digest: String = Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(digest, archive.sha256, "{}", path.display());

            let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).expect("open")).expect("zip");
            let mut text = String::new();
            zip.by_name("extension.toml")
                .expect("the archive's manifest")
                .read_to_string(&mut text)
                .expect("read");
            let manifest = extensions::Manifest::parse(&text).expect("manifest");
            assert_eq!(manifest.id, extension.id);
            assert_eq!(manifest.version, release.version.to_string());
            assert_eq!(manifest.api, release.api);
        }
    }
}
