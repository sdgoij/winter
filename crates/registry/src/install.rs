//! Installing a release's archive beside the settings file.
//!
//! An install is verify-then-place (ADR 088): the archive is fetched under a
//! cap, hashed against the registry's checksum, unpacked into a hidden staging
//! directory under `extensions/`, read back through [`Manifest`], and only then
//! renamed into `extensions/<id>/`. Nothing in a half-finished install is
//! visible to discovery, and a failure leaves the previous copy alone.
//!
//! A directory the registry did not write — one with no [`PROVENANCE`] file —
//! is never replaced, and neither is one from another registry. The record the
//! install writes is what makes that distinction possible later.

use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use extensions::manifest::{Manifest, is_id, is_relative_path};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::fetch::Fetcher;
use crate::schema::{Extension, Release};

/// The largest archive this build downloads, in bytes.
pub const ARCHIVE_MAX: u64 = 64 * 1024 * 1024;

/// The file a registry install leaves beside the manifest.
pub const PROVENANCE: &str = "winter-installed.toml";

/// The largest provenance file, in bytes.
const PROVENANCE_MAX: u64 = 64 * 1024;

/// The hidden directory an install unpacks into before it is moved into place.
///
/// Discovery skips a hidden directory, so a failed install cannot be loaded.
const STAGING_DIR: &str = ".staging";

/// The largest single unpacked entry, in bytes.
const ENTRY_MAX: u64 = 64 * 1024 * 1024;

/// The largest total an archive may unpack to, in bytes.
///
/// The download cap bounds the compressed size; this bounds the tree it
/// expands to, so a small archive cannot fill the disk.
const UNPACKED_MAX: u64 = 128 * 1024 * 1024;

/// How many entries one archive may hold.
const ENTRIES_MAX: usize = 10_000;

/// The comment written above the provenance record.
const PROVENANCE_HEADER: &str = "\
# Written by Winter when it installed this extension. A directory without this
# file was installed by hand, and an install never replaces one.

";

/// A completed install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The extension's directory under the settings root.
    pub dir: PathBuf,
    /// Whether an older copy from the same registry was replaced.
    pub replaced: bool,
    /// The record written beside the manifest.
    pub provenance: Provenance,
}

/// What a registry install left beside the manifest.
///
/// Unknown keys are tolerated: a later build may add one, and an older build
/// must still see the directory as a registry install rather than a manual one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The registry document's URL, which is the registry's identity.
    pub registry: String,
    /// The release the directory holds.
    pub version: Version,
    /// The archive it came from.
    pub archive: String,
    /// The archive's SHA-256, as the registry published it.
    pub sha256: String,
    /// Seconds since the Unix epoch, when the install finished.
    pub installed: u64,
}

/// Fetches, verifies and installs `release` as `extensions/<extension.id>/`.
///
/// The archive is refused unless its digest matches the registry's checksum and
/// the manifest inside repeats the row's id, version and `api`. A directory
/// already at the target is replaced only when this same registry installed it;
/// a hand-made directory or one from another registry is an error.
pub fn install(
    extensions: &Path,
    registry_url: &str,
    extension: &Extension,
    release: &Release,
    fetcher: &dyn Fetcher,
) -> Result<Installed, Error> {
    if !is_id(&extension.id) {
        return Err(Error::Id {
            id: extension.id.clone(),
        });
    }
    let archive = release.archive().ok_or_else(|| Error::Unsupported {
        id: extension.id.clone(),
        version: release.version.clone(),
    })?;

    let target = extensions.join(&extension.id);
    let replaced = may_replace(&target, registry_url, &extension.id)?;

    let bytes = fetcher.fetch(archive.url.as_str(), ARCHIVE_MAX)?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != archive.sha256 {
        return Err(Error::Checksum {
            url: archive.url.clone(),
            expected: archive.sha256.clone(),
            actual,
        });
    }

    let mut staging = Staging::new(extensions)?;
    extract(&archive.url, &bytes, &staging.payload)?;

    let root = match archive.path() {
        Some(path) if is_relative_path(path) => staging.payload.join(path),
        Some(path) => {
            return Err(Error::Outside {
                entry: path.to_string(),
            });
        }
        None => staging.payload.clone(),
    };
    if !root.is_dir() {
        return Err(Error::MissingRoot { dir: root });
    }

    let manifest = Manifest::load(&root).map_err(|source| Error::Manifest {
        dir: root.clone(),
        source,
    })?;
    if manifest.id != extension.id {
        return Err(Error::IdMismatch {
            found: manifest.id,
            wanted: extension.id.clone(),
        });
    }
    let wanted = release.version.to_string();
    if manifest.version != wanted {
        return Err(Error::VersionMismatch {
            id: extension.id.clone(),
            found: manifest.version,
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

    let record = Provenance {
        registry: registry_url.to_string(),
        version: release.version.clone(),
        archive: archive.url.clone(),
        sha256: archive.sha256.clone(),
        installed: seconds_now(),
    };
    write_provenance(&root, &record)?;

    // Windows will not rename over an existing directory, so an update moves the
    // old copy into the staging tree first. The guard then deletes it, and the
    // download is the only thing that can fail afterwards.
    let previous = staging.root.join("previous");
    if replaced {
        std::fs::rename(&target, &previous).map_err(|source| Error::Move {
            from: target.clone(),
            to: previous.clone(),
            source,
        })?;
    }
    if let Err(source) = std::fs::rename(&root, &target) {
        if replaced && let Err(source) = std::fs::rename(&previous, &target) {
            // Keep the old copy rather than let the staging guard delete the
            // only one left.
            staging.keep();
            return Err(Error::Stranded {
                target,
                previous,
                source,
            });
        }
        return Err(Error::Move {
            from: root,
            to: target,
            source,
        });
    }

    Ok(Installed {
        dir: target,
        replaced,
        provenance: record,
    })
}

/// Removes a registry-installed extension.
///
/// A directory with no provenance record was installed by hand and is refused;
/// [`uninstall_any`] removes one when the user explicitly asks.
pub fn uninstall(extensions: &Path, id: &str) -> Result<(), Error> {
    remove(extensions, id, false)
}

/// Removes an extension directory whatever installed it.
pub fn uninstall_any(extensions: &Path, id: &str) -> Result<(), Error> {
    remove(extensions, id, true)
}

/// The provenance record in `dir`, when the install wrote one.
pub fn provenance(dir: &Path) -> Option<Provenance> {
    let path = dir.join(PROVENANCE);
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(PROVENANCE_MAX.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > PROVENANCE_MAX {
        return None;
    }
    toml::from_str(std::str::from_utf8(&bytes).ok()?).ok()
}

/// Whether `target` may be replaced, refusing a directory this registry did not
/// write. Returns whether there is an old copy to replace.
fn may_replace(target: &Path, registry_url: &str, id: &str) -> Result<bool, Error> {
    let metadata = match std::fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(Error::Io {
                path: target.to_path_buf(),
                source,
            });
        }
    };
    let manual = || Error::InstalledByHand {
        id: id.to_string(),
        dir: target.to_path_buf(),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(manual());
    }
    let Some(found) = provenance(target) else {
        return Err(manual());
    };
    if found.registry != registry_url {
        return Err(Error::OtherRegistry {
            id: id.to_string(),
            version: found.version,
            found: found.registry,
            wanted: registry_url.to_string(),
        });
    }
    Ok(true)
}

/// Removes `extensions/<id>/`, refusing a manual install unless `allow_manual`.
fn remove(extensions: &Path, id: &str, allow_manual: bool) -> Result<(), Error> {
    if !is_id(id) {
        return Err(Error::Id { id: id.to_string() });
    }
    let dir = extensions.join(id);
    match std::fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(Error::NotInstalled { id: id.to_string() }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NotInstalled { id: id.to_string() });
        }
        Err(source) => {
            return Err(Error::Io { path: dir, source });
        }
    }
    if !allow_manual && provenance(&dir).is_none() {
        return Err(Error::InstalledByHand {
            id: id.to_string(),
            dir,
        });
    }
    std::fs::remove_dir_all(&dir).map_err(|source| Error::Io { path: dir, source })
}

/// Unpacks a zip into `into`, refusing names that leave it.
fn extract(url: &str, bytes: &[u8], into: &Path) -> Result<(), Error> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|source| Error::Zip {
        url: url.to_string(),
        source,
    })?;
    if archive.len() > ENTRIES_MAX {
        return Err(Error::TooManyEntries {
            count: archive.len(),
            max: ENTRIES_MAX,
        });
    }

    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|source| Error::Zip {
            url: url.to_string(),
            source,
        })?;
        let entry_name = entry
            .name()
            .map_err(|source| Error::Zip {
                url: url.to_string(),
                source,
            })?
            .into_owned();
        let name = entry.enclosed_name().ok_or_else(|| Error::Outside {
            entry: entry_name.clone(),
        })?;
        if entry.is_symlink() {
            return Err(Error::Symlink { entry: entry_name });
        }
        // `enclosed_name` allows a `..` that stays inside (`a/../b`); ADR 088
        // refuses every `..`, so only plain path segments are kept.
        if !name
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(Error::Outside { entry: entry_name });
        }

        let destination = into.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&destination).map_err(|source| Error::Io {
                path: destination,
                source,
            })?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut file = File::create(&destination).map_err(|source| Error::Io {
            path: destination.clone(),
            source,
        })?;

        // Cap the bytes actually written, not the entry's declared size: a
        // header can lie about how much it expands to.
        let mut written = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = entry.read(&mut buffer).map_err(|source| Error::Io {
                path: destination.clone(),
                source,
            })?;
            if read == 0 {
                break;
            }
            written += read as u64;
            total += read as u64;
            if written > ENTRY_MAX {
                return Err(Error::EntryTooLarge {
                    entry: entry_name,
                    max: ENTRY_MAX,
                });
            }
            if total > UNPACKED_MAX {
                return Err(Error::UnpackedTooLarge { max: UNPACKED_MAX });
            }
            file.write_all(&buffer[..read]).map_err(|source| Error::Io {
                path: destination.clone(),
                source,
            })?;
        }
    }
    Ok(())
}

/// The hidden staging tree one install owns.
///
/// Dropping it removes the tree, so a failure at any point after the download
/// leaves nothing behind. The tree holds the unpacked `payload` and, during an
/// update, the displaced `previous` copy.
struct Staging {
    /// The shared `.staging` directory under `extensions`.
    parent: PathBuf,
    /// This install's own directory in it.
    root: PathBuf,
    /// Where the archive is unpacked.
    payload: PathBuf,
    /// Whether to leave the tree for a later inspection.
    keep: bool,
}

impl Staging {
    fn new(extensions: &Path) -> Result<Self, Error> {
        let parent = extensions.join(STAGING_DIR);
        let name = format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let root = parent.join(name);
        let payload = root.join("payload");
        std::fs::create_dir_all(&payload).map_err(|source| Error::Io {
            path: payload.clone(),
            source,
        })?;
        Ok(Self {
            parent,
            root,
            payload,
            keep: false,
        })
    }

    fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        let _ = std::fs::remove_dir_all(&self.root);
        // Only removes `.staging` when no other install is using it.
        let _ = std::fs::remove_dir(&self.parent);
    }
}

/// Distinguishes two installs in one process.
static NEXT: AtomicU64 = AtomicU64::new(0);

fn write_provenance(dir: &Path, record: &Provenance) -> Result<(), Error> {
    let body = toml::to_string(record).map_err(|error| Error::Encode {
        message: error.to_string(),
    })?;
    let path = dir.join(PROVENANCE);
    std::fs::write(&path, format!("{PROVENANCE_HEADER}{body}")).map_err(|source| Error::Io { path, source })
}

fn seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// What an install can be wrong about.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The release's source is not one this build installs.
    #[error("{id} {version} has no archive source")]
    Unsupported {
        /// The extension's identifier.
        id: String,
        /// The release's version.
        version: Version,
    },
    /// The identifier cannot name a directory under the extensions root.
    #[error("{id} is not a valid extension identifier")]
    Id {
        /// The identifier as the registry wrote it.
        id: String,
    },
    /// A directory with this identifier was installed by hand.
    #[error("{dir} was installed by hand; remove it before installing {id}")]
    InstalledByHand {
        /// The extension's identifier.
        id: String,
        /// The directory that is in the way.
        dir: PathBuf,
    },
    /// A directory with this identifier came from another registry.
    #[error("{id} {version} came from {found}, not {wanted}")]
    OtherRegistry {
        /// The extension's identifier.
        id: String,
        /// The version already installed.
        version: Version,
        /// The registry that installed it.
        found: String,
        /// The registry the user asked.
        wanted: String,
    },
    /// The archive could not be downloaded.
    #[error(transparent)]
    Fetch(#[from] crate::fetch::Error),
    /// The archive's bytes do not match the registry's checksum.
    #[error("{url}: sha256 is {actual}, not {expected}")]
    Checksum {
        /// The archive's URL.
        url: String,
        /// The checksum the registry published.
        expected: String,
        /// The checksum of what was downloaded.
        actual: String,
    },
    /// The download is not a readable zip.
    #[error("{url}: {source}")]
    Zip {
        /// The archive's URL.
        url: String,
        /// The archive's reason.
        #[source]
        source: zip::result::ZipError,
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
    /// An entry's name would leave the staging directory.
    #[error("the archive names {entry}, which is outside it")]
    Outside {
        /// The entry as the archive spells it.
        entry: String,
    },
    /// An entry is a symbolic link.
    #[error("the archive's entry {entry} is a symbolic link")]
    Symlink {
        /// The entry as the archive spells it.
        entry: String,
    },
    /// One unpacked entry is over its cap.
    #[error("the archive's entry {entry} unpacks to over {max} bytes")]
    EntryTooLarge {
        /// The entry as the archive spells it.
        entry: String,
        /// The per-entry cap.
        max: u64,
    },
    /// The archive as a whole unpacks to over its cap.
    #[error("the archive unpacks to over {max} bytes")]
    UnpackedTooLarge {
        /// The cap.
        max: u64,
    },
    /// The archive holds more entries than the cap allows.
    #[error("the archive holds {count} entries, over the {max}-entry cap")]
    TooManyEntries {
        /// How many entries it holds.
        count: usize,
        /// The cap.
        max: usize,
    },
    /// The staged tree does not hold the extension's manifest.
    #[error("{dir} does not hold an extension.toml")]
    MissingRoot {
        /// The directory the registry pointed at.
        dir: PathBuf,
    },
    /// The staged manifest is not readable.
    #[error("{dir}: {source}")]
    Manifest {
        /// The directory the manifest was read from.
        dir: PathBuf,
        /// The manifest's reason.
        #[source]
        source: extensions::manifest::Error,
    },
    /// The staged manifest names a different extension.
    #[error("the archive's manifest is {found}, not {wanted}")]
    IdMismatch {
        /// The identifier in the archive.
        found: String,
        /// The identifier the registry listed.
        wanted: String,
    },
    /// The staged manifest is a different version.
    #[error("{id}: the archive's manifest is version {found}, not {wanted}")]
    VersionMismatch {
        /// The extension's identifier.
        id: String,
        /// The version in the archive.
        found: String,
        /// The version the registry listed.
        wanted: String,
    },
    /// The staged manifest needs a different api major.
    #[error("{id}: the archive's manifest needs api {found}, not {wanted}")]
    ApiMismatch {
        /// The extension's identifier.
        id: String,
        /// The api major in the archive.
        found: u32,
        /// The api major the registry listed.
        wanted: u32,
    },
    /// The extension directory could not be moved into place.
    #[error("cannot move {from} to {to}: {source}")]
    Move {
        /// Where it is.
        from: PathBuf,
        /// Where it belongs.
        to: PathBuf,
        /// The operating system's reason.
        #[source]
        source: std::io::Error,
    },
    /// An update failed and the previous copy could not be restored.
    #[error("cannot restore {target}; the previous copy is at {previous}: {source}")]
    Stranded {
        /// The directory the copy belongs at.
        target: PathBuf,
        /// Where the previous copy was left.
        previous: PathBuf,
        /// The operating system's reason.
        #[source]
        source: std::io::Error,
    },
    /// The extension is not installed.
    #[error("{id} is not installed")]
    NotInstalled {
        /// The identifier asked for.
        id: String,
    },
    /// The provenance record could not be encoded.
    #[error("cannot write the provenance record: {message}")]
    Encode {
        /// The encoder's reason.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Network;
    use crate::schema::Source;
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    fn manifest(id: &str, version: &str) -> String {
        format!("id = \"{id}\"\nname = \"B\"\nversion = \"{version}\"\napi = 1\nentry = \"main.js\"\n")
    }

    fn zip_at(directory: &Path, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = directory.join(name);
        let file = File::create(&path).expect("create");
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (entry, body) in entries {
            writer.start_file(*entry, options).expect("start");
            writer.write_all(body).expect("write");
        }
        writer.finish().expect("finish");
        path
    }

    fn file_url(path: &Path) -> String {
        let text = path.display().to_string().replace('\\', "/");
        if text.starts_with('/') {
            format!("file://{text}")
        } else {
            format!("file:///{text}")
        }
    }

    fn sha256(path: &Path) -> String {
        let bytes = std::fs::read(path).expect("read");
        Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn release(url: &str, sha256: &str, version: &str, path: Option<&str>) -> Release {
        Release {
            version: Version::parse(version).expect("version"),
            api: 1,
            source: Source::Archive(crate::schema::Archive {
                url: url.to_string(),
                sha256: sha256.to_string(),
                path: path.map(str::to_string),
            }),
        }
    }

    fn extension(id: &str, release: Release) -> Extension {
        Extension {
            id: id.to_string(),
            name: "B".to_string(),
            description: None,
            homepage: None,
            license: None,
            tags: Vec::new(),
            dependencies: Vec::new(),
            versions: vec![release],
        }
    }

    fn standard(root: &Path, name: &str) -> (PathBuf, Release) {
        let path = zip_at(
            root,
            name,
            &[
                ("extension.toml", manifest("a.b", "0.1.0").as_bytes()),
                ("main.js", b"export {};\n"),
            ],
        );
        let release = release(&file_url(&path), &sha256(&path), "0.1.0", None);
        (path, release)
    }

    #[test]
    fn an_archive_is_verified_unpacked_and_recorded() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let (path, release) = standard(root.path(), "one.zip");
        let extension = extension("a.b", release.clone());

        let installed = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect("install");

        assert!(!installed.replaced);
        assert_eq!(installed.dir, home.join("a.b"));
        assert!(installed.dir.join("extension.toml").is_file());
        assert_eq!(
            std::fs::read_to_string(installed.dir.join("main.js")).expect("read"),
            "export {};\n"
        );
        assert!(!home.join(".staging").exists());

        let record = provenance(&installed.dir).expect("provenance");
        assert_eq!(record.registry, "https://one.example/registry.toml");
        assert_eq!(record.version, Version::parse("0.1.0").expect("version"));
        assert_eq!(record.archive, file_url(&path));
        assert_eq!(record.sha256, sha256(&path));
        assert!(record.installed > 0);

        let report = extensions::discover(&extensions::Options {
            roots: vec![home.clone()],
            disabled: Vec::new(),
        });
        assert_eq!(report.loaded.len(), 1);
        assert_eq!(report.loaded[0].manifest.id, "a.b");
    }

    #[test]
    fn a_bad_checksum_installs_nothing() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let (path, _) = standard(root.path(), "one.zip");
        let release = release(&file_url(&path), &"0".repeat(64), "0.1.0", None);
        let extension = extension("a.b", release.clone());

        let error = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect_err("checksum");

        assert!(matches!(error, Error::Checksum { .. }));
        assert!(!home.join("a.b").exists());
        assert!(!home.join(".staging").exists());
    }

    #[test]
    fn a_manifest_for_another_identifier_is_refused() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let path = zip_at(
            root.path(),
            "one.zip",
            &[
                ("extension.toml", manifest("x.y", "0.1.0").as_bytes()),
                ("main.js", b"export {};\n"),
            ],
        );
        let release = release(&file_url(&path), &sha256(&path), "0.1.0", None);
        let extension = extension("a.b", release.clone());

        let error = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect_err("identifier");

        assert!(matches!(error, Error::IdMismatch { .. }));
        assert!(!home.join("a.b").exists());
    }

    #[test]
    fn a_manifest_for_another_version_is_refused() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let path = zip_at(
            root.path(),
            "one.zip",
            &[
                ("extension.toml", manifest("a.b", "0.2.0").as_bytes()),
                ("main.js", b"export {};\n"),
            ],
        );
        let release = release(&file_url(&path), &sha256(&path), "0.1.0", None);
        let extension = extension("a.b", release.clone());

        let error = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect_err("version");

        assert!(matches!(error, Error::VersionMismatch { .. }));
        assert!(!home.join("a.b").exists());
    }

    #[test]
    fn a_manifest_for_another_api_is_refused() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let (path, _) = standard(root.path(), "one.zip");
        let mut release = release(&file_url(&path), &sha256(&path), "0.1.0", None);
        release.api = 2;
        let extension = extension("a.b", release.clone());

        let error = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect_err("api");

        assert!(matches!(error, Error::ApiMismatch { .. }));
    }

    #[test]
    fn an_archive_with_an_escaping_entry_is_refused() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let path = zip_at(
            root.path(),
            "evil.zip",
            &[
                ("extension.toml", manifest("a.b", "0.1.0").as_bytes()),
                ("../escape.txt", b"x"),
            ],
        );
        let release = release(&file_url(&path), &sha256(&path), "0.1.0", None);
        let extension = extension("a.b", release.clone());

        let error = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect_err("traversal");

        assert!(matches!(error, Error::Outside { .. }));
        assert!(!root.path().join("escape.txt").exists());
        assert!(!home.join("a.b").exists());
        assert!(!home.join(".staging").exists());
    }

    #[test]
    fn an_archive_with_a_symlink_is_refused() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let path = root.path().join("link.zip");
        let file = File::create(&path).expect("create");
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        writer.start_file("extension.toml", options).expect("start");
        writer.write_all(manifest("a.b", "0.1.0").as_bytes()).expect("write");
        writer.add_symlink("main.js", "/etc/passwd", options).expect("symlink");
        writer.finish().expect("finish");
        let release = release(&file_url(&path), &sha256(&path), "0.1.0", None);
        let extension = extension("a.b", release.clone());

        let error = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect_err("symlink");

        assert!(matches!(error, Error::Symlink { .. }));
        assert!(!home.join("a.b").exists());
    }

    #[test]
    fn an_archive_path_selects_the_nested_root() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let path = zip_at(
            root.path(),
            "nested.zip",
            &[
                ("pkg/extension.toml", manifest("a.b", "0.1.0").as_bytes()),
                ("pkg/main.js", b"export {};\n"),
            ],
        );
        let release = release(&file_url(&path), &sha256(&path), "0.1.0", Some("pkg"));
        let extension = extension("a.b", release.clone());

        let installed = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect("nested");
        assert!(installed.dir.join("extension.toml").is_file());
    }

    #[test]
    fn a_missing_archive_path_is_refused() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let path = zip_at(
            root.path(),
            "nested.zip",
            &[("extension.toml", manifest("a.b", "0.1.0").as_bytes())],
        );
        let release = release(&file_url(&path), &sha256(&path), "0.1.0", Some("nope"));
        let extension = extension("a.b", release.clone());

        let error = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect_err("missing root");

        assert!(matches!(error, Error::MissingRoot { .. }));
    }

    #[test]
    fn a_manual_install_is_not_replaced() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let dir = home.join("a.b");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("extension.toml"), manifest("a.b", "0.1.0")).expect("write");
        let (path, release) = standard(root.path(), "one.zip");
        let extension = extension("a.b", release.clone());

        let error = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect_err("manual");

        assert!(matches!(error, Error::InstalledByHand { .. }));
        assert_eq!(
            std::fs::read_to_string(dir.join("extension.toml")).expect("read"),
            manifest("a.b", "0.1.0")
        );
        assert!(provenance(&dir).is_none());
        drop(path);
    }

    #[test]
    fn an_install_from_another_registry_is_not_replaced() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let (_, release) = standard(root.path(), "one.zip");
        let extension = extension("a.b", release.clone());
        install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect("first");

        let error = install(
            &home,
            "https://two.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect_err("other registry");

        assert!(matches!(error, Error::OtherRegistry { .. }));
        let record = provenance(&home.join("a.b")).expect("provenance");
        assert_eq!(record.registry, "https://one.example/registry.toml");
    }

    #[test]
    fn an_update_replaces_the_previous_copy() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let registry = "https://one.example/registry.toml";

        let (one, first) = standard(root.path(), "one.zip");
        let first_extension = extension("a.b", first.clone());
        install(&home, registry, &first_extension, &first, &Network).expect("first");
        drop(one);

        let two = zip_at(
            root.path(),
            "two.zip",
            &[
                ("extension.toml", manifest("a.b", "0.2.0").as_bytes()),
                ("main.js", b"export const two = true;\n"),
            ],
        );
        let second = release(&file_url(&two), &sha256(&two), "0.2.0", None);
        let second_extension = extension("a.b", second.clone());

        let installed = install(&home, registry, &second_extension, &second, &Network).expect("update");

        assert!(installed.replaced);
        assert_eq!(installed.provenance.version, Version::parse("0.2.0").expect("version"));
        assert_eq!(
            std::fs::read_to_string(home.join("a.b").join("main.js")).expect("read"),
            "export const two = true;\n"
        );
        assert!(!home.join(".staging").exists());
        let report = extensions::discover(&extensions::Options {
            roots: vec![home.clone()],
            disabled: Vec::new(),
        });
        assert_eq!(report.loaded.len(), 1);
    }

    #[test]
    fn a_source_this_build_does_not_install_is_refused() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let release = Release {
            version: Version::parse("0.1.0").expect("version"),
            api: 1,
            source: Source::Unsupported,
        };
        let extension = extension("a.b", release.clone());

        let error = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect_err("unsupported");

        assert!(matches!(error, Error::Unsupported { .. }));
    }

    #[test]
    fn an_escaping_identifier_is_refused_before_any_path_is_used() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let (path, release) = standard(root.path(), "one.zip");
        let extension = extension("../escape", release.clone());

        let error = install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect_err("identifier");

        assert!(matches!(error, Error::Id { .. }));
        drop(path);
    }

    #[test]
    fn uninstall_removes_a_registry_install() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let (_, release) = standard(root.path(), "one.zip");
        let extension = extension("a.b", release.clone());
        install(
            &home,
            "https://one.example/registry.toml",
            &extension,
            &release,
            &Network,
        )
        .expect("install");

        uninstall(&home, "a.b").expect("uninstall");

        assert!(!home.join("a.b").exists());
        assert!(matches!(uninstall(&home, "a.b"), Err(Error::NotInstalled { .. })));
    }

    #[test]
    fn uninstall_leaves_a_manual_install_unless_forced() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");
        let dir = home.join("a.b");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("extension.toml"), manifest("a.b", "0.1.0")).expect("write");

        assert!(matches!(uninstall(&home, "a.b"), Err(Error::InstalledByHand { .. })));
        assert!(dir.exists());

        uninstall_any(&home, "a.b").expect("forced");
        assert!(!dir.exists());
    }

    #[test]
    fn an_escaping_identifier_is_refused_by_uninstall() {
        let root = tempfile::tempdir().expect("dir");
        let home = root.path().join("extensions");

        assert!(matches!(uninstall(&home, "../escape"), Err(Error::Id { .. })));
    }
}
