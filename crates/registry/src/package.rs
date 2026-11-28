//! Building the published registry and its archives from the extension
//! directories beside it.
//!
//! The repository keeps each extension unpackaged under `extensions/<id>/` and
//! one source `registry.toml` whose checksums are placeholders. This module zips
//! each directory, hashes it, and rewrites the registry with the real digests;
//! the Pages job runs it into the site directory, so the published document and
//! the archives it names are produced together and cannot disagree (ADR 088).

use std::fs;
use std::path::{Component, Path, PathBuf};

use extensions::manifest::is_relative_path;
use semver::Version;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::schema::{Archive, Registry, Source};

/// The site prefix the GitHub Pages job serves `extensions/` under.
pub const SITE: &str = "https://sdgoij.github.io/winter/extensions/";

/// What a package run wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packaged {
    /// The rewritten registry.
    pub registry: PathBuf,
    /// The archives it names.
    pub archives: Vec<PathBuf>,
}

/// Packages every extension under `extensions` into `out`, rewriting the
/// registry's checksums.
///
/// Each row's archive is built from `extensions/<id>/` and must live under
/// [`SITE`], so the published registry never names a file this job cannot make.
/// The archive's manifest must repeat the row's id, version and api, exactly as
/// an install checks them.
pub fn build(extensions: &Path, out: &Path) -> Result<Packaged, Error> {
    let mut registry = Registry::load(&extensions.join("registry.toml")).map_err(Error::Registry)?;
    fs::create_dir_all(out).map_err(|source| Error::Io {
        path: out.to_path_buf(),
        source,
    })?;

    let mut archives = Vec::new();
    for extension in &mut registry.extensions {
        let dir = extensions.join(&extension.id);
        let manifest = extensions::Manifest::load(&dir).map_err(|source| Error::Manifest {
            dir: dir.clone(),
            source,
        })?;
        if manifest.id != extension.id {
            return Err(Error::IdMismatch {
                found: manifest.id,
                wanted: extension.id.clone(),
            });
        }

        for release in &mut extension.versions {
            let (url, path) = match &release.source {
                Source::Archive(archive) => (archive.url.clone(), archive.path.clone()),
                Source::Unsupported => {
                    return Err(Error::Unsupported {
                        id: extension.id.clone(),
                        version: release.version.clone(),
                    });
                }
            };

            let wanted = release.version.to_string();
            if manifest.version != wanted {
                return Err(Error::VersionMismatch {
                    id: extension.id.clone(),
                    found: manifest.version.clone(),
                    wanted,
                });
            }
            if manifest.api != release.api {
                return Err(Error::ApiMismatch {
                    id: extension.id.clone(),
                    found: manifest.api,
                    wanted: release.api,
                });
            }

            let Some(name) = url.strip_prefix(SITE) else {
                return Err(Error::External { url });
            };
            if !is_relative_path(name) {
                return Err(Error::ArchiveName { name: name.to_string() });
            }

            let target = out.join(name);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            zip_dir(&dir, &target)?;
            let sha256 = digest(&target)?;
            release.source = Source::Archive(Archive { url, sha256, path });
            archives.push(target);
        }
    }

    let registry_path = out.join("registry.toml");
    let text = toml::to_string_pretty(&registry).map_err(|error| Error::Encode {
        message: error.to_string(),
    })?;
    fs::write(&registry_path, text).map_err(|source| Error::Io {
        path: registry_path.clone(),
        source,
    })?;
    Ok(Packaged {
        registry: registry_path,
        archives,
    })
}

/// Zips the contents of `dir` into `target`, at the archive's root.
///
/// Entries are sorted and carry one fixed timestamp, so the same directory
/// always produces the same bytes and therefore the same checksum.
fn zip_dir(dir: &Path, target: &Path) -> Result<(), Error> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort_by(|left, right| left.1.cmp(&right.1));

    let file = fs::File::create(target).map_err(|source| Error::Io {
        path: target.to_path_buf(),
        source,
    })?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());

    for (path, name) in files {
        writer.start_file(name, options).map_err(|source| Error::Zip {
            path: target.to_path_buf(),
            source,
        })?;
        let mut reader = fs::File::open(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        std::io::copy(&mut reader, &mut writer).map_err(|source| Error::Io {
            path: target.to_path_buf(),
            source,
        })?;
    }
    writer.finish().map_err(|source| Error::Zip {
        path: target.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Every file under `dir`, with its `/`-separated name from `root`.
fn collect(root: &Path, dir: &Path, files: &mut Vec<(PathBuf, String)>) -> Result<(), Error> {
    let entries = fs::read_dir(dir).map_err(|source| Error::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        if kind.is_symlink() {
            return Err(Error::Symlink { path });
        }
        if kind.is_dir() {
            collect(root, &path, files)?;
        } else if kind.is_file() {
            let Some(name) = archive_name(root, &path) else {
                return Err(Error::ArchiveName {
                    name: path.display().to_string(),
                });
            };
            files.push((path, name));
        }
    }
    Ok(())
}

fn archive_name(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        parts.push(part.to_string_lossy());
    }
    Some(parts.join("/"))
}

fn digest(path: &Path) -> Result<String, Error> {
    let bytes = fs::read(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// What packaging a registry can be wrong about.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The source registry is not a valid document.
    #[error(transparent)]
    Registry(#[from] crate::schema::Error),
    /// The row's directory has no readable manifest.
    #[error("{dir}: {source}")]
    Manifest {
        /// The directory the row names.
        dir: PathBuf,
        /// The manifest's reason.
        #[source]
        source: extensions::manifest::Error,
    },
    /// A filesystem operation failed.
    #[error("{path}: {source}")]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The operating system's reason.
        #[source]
        source: std::io::Error,
    },
    /// A zip could not be written.
    #[error("{path}: {source}")]
    Zip {
        /// The archive being written.
        path: PathBuf,
        /// The zip layer's reason.
        #[source]
        source: zip::result::ZipError,
    },
    /// The row's source is not one this job packages.
    #[error("{id} {version} does not have a packageable archive source")]
    Unsupported {
        /// The extension's identifier.
        id: String,
        /// The release's version.
        version: Version,
    },
    /// The archive URL is not served from this site.
    #[error("{url} is not served from {SITE}")]
    External {
        /// The URL as the registry wrote it.
        url: String,
    },
    /// A file's name is not a relative archive path.
    #[error("{name} is not a relative archive name")]
    ArchiveName {
        /// The name as written.
        name: String,
    },
    /// A symbolic link would be packed.
    #[error("{path} is a symbolic link")]
    Symlink {
        /// The link's path.
        path: PathBuf,
    },
    /// The directory's manifest names a different extension.
    #[error("the archive's manifest is {found}, not {wanted}")]
    IdMismatch {
        /// The identifier in the manifest.
        found: String,
        /// The identifier the registry listed.
        wanted: String,
    },
    /// The directory's manifest is a different version.
    #[error("{id}: the archive's manifest is version {found}, not {wanted}")]
    VersionMismatch {
        /// The extension's identifier.
        id: String,
        /// The version in the manifest.
        found: String,
        /// The version the registry listed.
        wanted: String,
    },
    /// The directory's manifest needs a different api major.
    #[error("{id}: the archive's manifest needs api {found}, not {wanted}")]
    ApiMismatch {
        /// The extension's identifier.
        id: String,
        /// The api major in the manifest.
        found: u32,
        /// The api major the registry listed.
        wanted: u32,
    },
    /// The rewritten registry could not be encoded.
    #[error("cannot encode the registry: {message}")]
    Encode {
        /// The encoder's reason.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn write(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, text).expect("write");
    }

    fn tree() -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("temp");
        let extensions = temp.path().join("extensions");
        write(
            &extensions.join("a.b"),
            "extension.toml",
            "id = \"a.b\"\nname = \"B\"\nversion = \"1.0.0\"\napi = 1\nentry = \"main.js\"\n",
        );
        write(&extensions.join("a.b"), "main.js", "export {};\n");
        write(&extensions.join("a.b"), "lib/util.js", "export {};\n");
        write(
            &extensions,
            "registry.toml",
            &format!(
                "schema = 1\nname = \"T\"\nupdated = \"2026-11-28T12:00:00Z\"\n\n[[extensions]]\nid = \"a.b\"\nname = \"B\"\n[[extensions.versions]]\nversion = \"1.0.0\"\napi = 1\nsource = {{ type = \"archive\", url = \"{SITE}a.b-1.0.0.zip\", sha256 = \"{}\" }}\n",
                "0".repeat(64)
            ),
        );
        temp
    }

    #[test]
    fn a_directory_becomes_a_hashed_archive_and_the_registry_names_it() {
        let temp = tree();
        let out = temp.path().join("site");
        let packaged = build(&temp.path().join("extensions"), &out).expect("package");

        let registry = Registry::load(&packaged.registry).expect("the output parses");
        let archive = registry.extensions[0].versions[0].archive().expect("archive");
        assert_eq!(archive.sha256, digest(&out.join("a.b-1.0.0.zip")).expect("digest"));

        let file = fs::File::open(out.join("a.b-1.0.0.zip")).expect("open");
        let mut zip = zip::ZipArchive::new(file).expect("zip");
        let mut manifest = String::new();
        zip.by_name("extension.toml")
            .expect("manifest")
            .read_to_string(&mut manifest)
            .expect("read");
        assert!(manifest.contains("id = \"a.b\""));
        zip.by_name("lib/util.js").expect("nested module");
    }

    #[test]
    fn the_same_directory_produces_the_same_archive() {
        let temp = tree();
        let one = temp.path().join("one");
        let two = temp.path().join("two");
        build(&temp.path().join("extensions"), &one).expect("first");
        build(&temp.path().join("extensions"), &two).expect("second");

        assert_eq!(
            fs::read(one.join("a.b-1.0.0.zip")).expect("one"),
            fs::read(two.join("a.b-1.0.0.zip")).expect("two")
        );
    }

    #[test]
    fn an_archive_from_another_host_is_refused() {
        let temp = tree();
        let path = temp.path().join("extensions/registry.toml");
        let text = fs::read_to_string(&path)
            .expect("read")
            .replace(&format!("{SITE}a.b-1.0.0.zip"), "https://example.com/a.zip");
        fs::write(&path, text).expect("write");

        let error = build(&temp.path().join("extensions"), &temp.path().join("site")).expect_err("external");
        assert!(matches!(error, Error::External { .. }));
    }

    #[test]
    fn a_manifest_for_another_version_is_refused() {
        let temp = tree();
        let path = temp.path().join("extensions/registry.toml");
        let text = fs::read_to_string(&path).expect("read").replace(
            "version = \"1.0.0\"\napi = 1\nsource",
            "version = \"2.0.0\"\napi = 1\nsource",
        );
        fs::write(&path, text).expect("write");

        let error = build(&temp.path().join("extensions"), &temp.path().join("site")).expect_err("version");
        assert!(matches!(error, Error::VersionMismatch { .. }));
    }
}
