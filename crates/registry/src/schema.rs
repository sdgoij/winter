//! The registry document: the types, the caps and the rules a published file
//! must satisfy.
//!
//! The schema is one integer (ADR 088). Within a major a new release field is a
//! strict error here, so the schema is bumped when the shape changes; the one
//! forward-compatible case is a source `type` this build does not know, which is
//! listed but not installable.

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use extensions::manifest::{is_id, is_relative_path};
use serde::Deserialize;

/// The registry schema this build reads.
pub const SCHEMA: u32 = 1;

/// The largest registry document, in bytes.
pub const FILE_MAX: u64 = 1024 * 1024;

/// How many extensions one registry may list.
pub const EXTENSIONS_MAX: usize = 1024;

/// How many releases one extension may list.
pub const RELEASES_MAX: usize = 64;

/// The longest extension description.
pub const DESCRIPTION_MAX: usize = 1024;

/// The longest homepage URL.
pub const HOMEPAGE_MAX: usize = 2048;

/// The longest license name.
pub const LICENSE_MAX: usize = 64;

/// How many tags one extension may carry.
pub const TAGS_MAX: usize = 16;

/// The longest tag.
pub const TAG_MAX: usize = 32;

/// How many dependencies one extension may declare.
pub const DEPENDENCIES_MAX: usize = 32;

/// The longest source URL.
pub const URL_MAX: usize = 2048;

/// One published registry document.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    /// The schema this document was written for.
    pub schema: u32,
    /// The name a shell shows beside the URL.
    pub name: String,
    /// When the document was published, for the reader's benefit.
    #[serde(default)]
    pub updated: Option<String>,
    /// The extensions the registry lists.
    #[serde(default)]
    pub extensions: Vec<Extension>,
}

/// One extension a registry offers, with every release it publishes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extension {
    /// The manifest identifier, so the row and the installed directory agree.
    pub id: String,
    /// The name a user sees.
    pub name: String,
    /// One line for the search row.
    #[serde(default)]
    pub description: Option<String>,
    /// The project's page.
    #[serde(default)]
    pub homepage: Option<String>,
    /// The declared license.
    #[serde(default)]
    pub license: Option<String>,
    /// Words the search matches.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Identifiers this extension expects beside it.
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// The published releases, newest first by convention but read in file
    /// order. An empty list is caught by validation with a clearer message than
    /// a missing key would give.
    #[serde(default)]
    pub versions: Vec<Release>,
}

/// One release of an extension.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    /// The semver this release ships, which the manifest must repeat.
    pub version: semver::Version,
    /// The host API major it needs.
    pub api: u32,
    /// Where its bytes come from.
    pub source: Source,
}

/// Where a release's files come from.
///
/// A `type` this build does not know becomes [`Source::Unsupported`], so a
/// registry published for a newer shell still parses here and the row says the
/// release cannot be installed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Source {
    /// A `.zip` at `url`, checked against `sha256`.
    Archive(Archive),
    /// A source adapter this build does not have.
    #[serde(other)]
    Unsupported,
}

/// An archive a release is installed from.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    /// The `.zip`'s URL: `https`, or `file` for a local mirror and tests.
    pub url: String,
    /// The archive's SHA-256, as 64 lowercase hex digits.
    pub sha256: String,
    /// The directory inside the archive that holds `extension.toml`; the root
    /// when absent or empty.
    #[serde(default)]
    pub path: Option<String>,
}

impl Archive {
    /// The directory inside the archive that holds the extension, or `None`
    /// when the archive's root is the extension.
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref().filter(|path| !path.is_empty())
    }
}

impl Release {
    /// Whether this build can install the release.
    pub fn is_installable(&self) -> bool {
        matches!(&self.source, Source::Archive(_))
    }

    /// The archive, when the source is one.
    pub fn archive(&self) -> Option<&Archive> {
        match &self.source {
            Source::Archive(archive) => Some(archive),
            Source::Unsupported => None,
        }
    }
}

impl Extension {
    /// The releases this build can install and host.
    pub fn installable(&self, api: u32) -> impl Iterator<Item = &Release> + '_ {
        self.versions
            .iter()
            .filter(move |release| release.api == api && release.is_installable())
    }

    /// The highest installable release whose `api` this build hosts.
    pub fn latest(&self, api: u32) -> Option<&Release> {
        self.installable(api)
            .max_by(|left, right| left.version.cmp(&right.version))
    }

    /// The release with exactly `version`, if it publishes one.
    pub fn release(&self, version: &semver::Version) -> Option<&Release> {
        self.versions.iter().find(|release| &release.version == version)
    }
}

impl Registry {
    /// Read and validate a registry file.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let file = File::open(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut bytes = Vec::new();
        file.take(FILE_MAX.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if bytes.len() as u64 > FILE_MAX {
            return Err(Error::TooLarge {
                path: path.to_path_buf(),
                size: bytes.len() as u64,
                max: FILE_MAX,
            });
        }
        let text = String::from_utf8(bytes).map_err(|_| Error::NotUtf8 {
            path: path.to_path_buf(),
        })?;
        Self::parse(&text)
    }

    /// Validate a registry from text.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let registry: Self = toml::from_str(text)?;
        registry.validate()?;
        Ok(registry)
    }

    /// The rules a parsed registry must already satisfy.
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != SCHEMA {
            return Err(Error::Schema {
                found: self.schema,
                supported: SCHEMA,
            });
        }
        if self.name.trim().is_empty() {
            return Err(Error::Name);
        }
        if self.extensions.len() > EXTENSIONS_MAX {
            return Err(Error::TooManyExtensions {
                count: self.extensions.len(),
                max: EXTENSIONS_MAX,
            });
        }

        let mut ids: HashSet<&str> = HashSet::new();
        for extension in &self.extensions {
            extension.validate()?;
            if !ids.insert(extension.id.as_str()) {
                return Err(Error::DuplicateId {
                    id: extension.id.clone(),
                });
            }
        }
        Ok(())
    }
}

impl Extension {
    fn validate(&self) -> Result<(), Error> {
        if !is_id(&self.id) {
            return Err(Error::Id { id: self.id.clone() });
        }
        if self.name.trim().is_empty() {
            return Err(Error::ExtensionName { id: self.id.clone() });
        }
        for (field, text, max) in [
            ("description", self.description.as_deref(), DESCRIPTION_MAX),
            ("homepage", self.homepage.as_deref(), HOMEPAGE_MAX),
            ("license", self.license.as_deref(), LICENSE_MAX),
        ] {
            if text.is_some_and(|text| text.len() > max) {
                return Err(Error::Field {
                    id: self.id.clone(),
                    field,
                    max,
                });
            }
        }

        if self.tags.len() > TAGS_MAX {
            return Err(Error::Tags {
                id: self.id.clone(),
                count: self.tags.len(),
                max: TAGS_MAX,
            });
        }
        for tag in &self.tags {
            if tag.trim().is_empty() || tag.len() > TAG_MAX {
                return Err(Error::Tag {
                    id: self.id.clone(),
                    tag: tag.clone(),
                    max: TAG_MAX,
                });
            }
        }

        if self.dependencies.len() > DEPENDENCIES_MAX {
            return Err(Error::Dependencies {
                id: self.id.clone(),
                count: self.dependencies.len(),
                max: DEPENDENCIES_MAX,
            });
        }
        let mut dependencies: HashSet<&str> = HashSet::new();
        for dependency in &self.dependencies {
            if !is_id(dependency) {
                return Err(Error::Dependency {
                    id: self.id.clone(),
                    dependency: dependency.clone(),
                });
            }
            if dependency == &self.id {
                return Err(Error::SelfDependency { id: self.id.clone() });
            }
            if !dependencies.insert(dependency.as_str()) {
                return Err(Error::DuplicateDependency {
                    id: self.id.clone(),
                    dependency: dependency.clone(),
                });
            }
        }

        if self.versions.is_empty() {
            return Err(Error::NoVersions { id: self.id.clone() });
        }
        if self.versions.len() > RELEASES_MAX {
            return Err(Error::TooManyVersions {
                id: self.id.clone(),
                count: self.versions.len(),
                max: RELEASES_MAX,
            });
        }
        let mut versions: HashSet<String> = HashSet::new();
        for release in &self.versions {
            release.validate(&self.id)?;
            if !versions.insert(release.version.to_string()) {
                return Err(Error::DuplicateVersion {
                    id: self.id.clone(),
                    version: release.version.to_string(),
                });
            }
        }
        Ok(())
    }
}

impl Release {
    fn validate(&self, id: &str) -> Result<(), Error> {
        if self.api == 0 {
            return Err(Error::ApiZero {
                id: id.to_string(),
                version: self.version.to_string(),
            });
        }
        if let Source::Archive(archive) = &self.source {
            archive.validate(id, &self.version)?;
        }
        Ok(())
    }
}

impl Archive {
    fn validate(&self, id: &str, version: &semver::Version) -> Result<(), Error> {
        if self.url.len() > URL_MAX
            || !(self.url.starts_with("https://") || self.url.starts_with("file://"))
            || self
                .url
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(Error::ArchiveUrl {
                id: id.to_string(),
                version: version.to_string(),
                max: URL_MAX,
            });
        }
        if !is_sha256(&self.sha256) {
            return Err(Error::ArchiveSha256 {
                id: id.to_string(),
                version: version.to_string(),
            });
        }
        if let Some(path) = self.path()
            && !is_relative_path(path)
        {
            return Err(Error::ArchivePath {
                id: id.to_string(),
                version: version.to_string(),
                path: path.to_string(),
            });
        }
        Ok(())
    }
}

/// Whether `text` is 64 lowercase hexadecimal digits.
fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// What a registry can be wrong about.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The file that failed.
        path: PathBuf,
        /// The operating system's reason.
        #[source]
        source: std::io::Error,
    },
    /// The file is longer than [`FILE_MAX`].
    #[error("{path} is {size} bytes, over the {max}-byte cap")]
    TooLarge {
        /// The file that is too long.
        path: PathBuf,
        /// Its length.
        size: u64,
        /// The cap.
        max: u64,
    },
    /// The file is not UTF-8.
    #[error("{path} is not UTF-8")]
    NotUtf8 {
        /// The file that failed.
        path: PathBuf,
    },
    /// The file is not the TOML it claims to be.
    #[error("the registry is not valid: {0}")]
    Parse(#[from] toml::de::Error),
    /// The document targets a schema this build does not read.
    #[error("the registry schema is {found}; this build reads {supported}")]
    Schema {
        /// The schema as written.
        found: u32,
        /// The schema this build reads, [`SCHEMA`].
        supported: u32,
    },
    /// The registry has no display name.
    #[error("the registry has no `name`")]
    Name,
    /// The registry lists more extensions than the cap allows.
    #[error("the registry lists {count} extensions, over the {max} cap")]
    TooManyExtensions {
        /// How many it lists.
        count: usize,
        /// The cap.
        max: usize,
    },
    /// An extension identifier is not a lowercase dotted name.
    #[error("{id:?} is not a lowercase dotted identifier")]
    Id {
        /// The identifier as written.
        id: String,
    },
    /// Two rows share an identifier.
    #[error("the registry lists {id:?} twice")]
    DuplicateId {
        /// The identifier as written.
        id: String,
    },
    /// An extension has no name.
    #[error("{id}: `name` is empty")]
    ExtensionName {
        /// The extension that failed.
        id: String,
    },
    /// A text field is over its cap.
    #[error("{id}: `{field}` is over the {max}-byte cap")]
    Field {
        /// The extension that failed.
        id: String,
        /// The manifest field.
        field: &'static str,
        /// The cap.
        max: usize,
    },
    /// An extension has more tags than the cap allows.
    #[error("{id}: {count} tags, over the {max} cap")]
    Tags {
        /// The extension that failed.
        id: String,
        /// How many it has.
        count: usize,
        /// The cap.
        max: usize,
    },
    /// A tag is empty or too long.
    #[error("{id}: the tag {tag:?} is empty or over the {max}-byte cap")]
    Tag {
        /// The extension that failed.
        id: String,
        /// The tag as written.
        tag: String,
        /// The cap.
        max: usize,
    },
    /// An extension has more dependencies than the cap allows.
    #[error("{id}: {count} dependencies, over the {max} cap")]
    Dependencies {
        /// The extension that failed.
        id: String,
        /// How many it has.
        count: usize,
        /// The cap.
        max: usize,
    },
    /// A dependency is not an identifier.
    #[error("{id}: the dependency {dependency:?} is not an identifier")]
    Dependency {
        /// The extension that failed.
        id: String,
        /// The dependency as written.
        dependency: String,
    },
    /// An extension depends on itself.
    #[error("{id}: it depends on itself")]
    SelfDependency {
        /// The extension that failed.
        id: String,
    },
    /// A dependency is listed twice.
    #[error("{id}: the dependency {dependency:?} is listed twice")]
    DuplicateDependency {
        /// The extension that failed.
        id: String,
        /// The dependency as written.
        dependency: String,
    },
    /// An extension publishes no releases.
    #[error("{id}: no versions")]
    NoVersions {
        /// The extension that failed.
        id: String,
    },
    /// An extension publishes more releases than the cap allows.
    #[error("{id}: {count} versions, over the {max} cap")]
    TooManyVersions {
        /// The extension that failed.
        id: String,
        /// How many it has.
        count: usize,
        /// The cap.
        max: usize,
    },
    /// The same version is published twice.
    #[error("{id}: version {version} is listed twice")]
    DuplicateVersion {
        /// The extension that failed.
        id: String,
        /// The version as written.
        version: String,
    },
    /// A release asks for API major zero.
    #[error("{id} {version}: `api` is 0; the first major is 1")]
    ApiZero {
        /// The extension that failed.
        id: String,
        /// The release that failed.
        version: String,
    },
    /// An archive URL is not `https`/`file`, too long, or carries whitespace.
    #[error("{id} {version}: the archive URL is not https or file, or is over the {max}-byte cap")]
    ArchiveUrl {
        /// The extension that failed.
        id: String,
        /// The release that failed.
        version: String,
        /// The cap.
        max: usize,
    },
    /// An archive checksum is not 64 lowercase hex digits.
    #[error("{id} {version}: the archive checksum is not 64 lowercase hex digits")]
    ArchiveSha256 {
        /// The extension that failed.
        id: String,
        /// The release that failed.
        version: String,
    },
    /// An archive path escapes the archive.
    #[error("{id} {version}: `path` is not a relative path inside the archive: {path:?}")]
    ArchivePath {
        /// The extension that failed.
        id: String,
        /// The release that failed.
        version: String,
        /// The path as written.
        path: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn head() -> &'static str {
        "schema = 1\nname = \"Winter extensions\"\n"
    }

    fn archive(url: &str, sha: &str, path: Option<&str>) -> String {
        match path {
            Some(path) => {
                format!("source = {{ type = \"archive\", url = \"{url}\", sha256 = \"{sha}\", path = \"{path}\" }}")
            }
            None => format!("source = {{ type = \"archive\", url = \"{url}\", sha256 = \"{sha}\" }}"),
        }
    }

    fn release(version: &str, api: u32, source: &str) -> String {
        format!("[[extensions.versions]]\nversion = \"{version}\"\napi = {api}\n{source}\n")
    }

    fn extension(id: &str, releases: &[String]) -> String {
        let mut text = format!(
            "[[extensions]]\nid = \"{id}\"\nname = \"{id}\"\ndescription = \"d\"\ntags = [\"t\"]\ndependencies = [\"x.y\"]\n"
        );
        for release in releases {
            text.push_str(release);
        }
        text
    }

    fn document() -> String {
        format!(
            "{}{}",
            head(),
            extension(
                "winter.failed-command",
                &[release(
                    "0.1.0",
                    1,
                    &archive("https://example.com/x.zip", SHA, Some("bundle"))
                )]
            )
        )
    }

    #[test]
    fn a_complete_registry_parses() {
        let registry = Registry::parse(&document()).expect("registry");
        assert_eq!(registry.schema, SCHEMA);
        assert_eq!(registry.name, "Winter extensions");

        let extension = &registry.extensions[0];
        assert_eq!(extension.id, "winter.failed-command");
        assert_eq!(extension.tags, ["t"]);
        assert_eq!(extension.dependencies, ["x.y"]);

        let release = &extension.versions[0];
        assert_eq!(release.version.to_string(), "0.1.0");
        assert_eq!(release.api, 1);
        let archive = release.archive().expect("an archive source");
        assert_eq!(archive.url, "https://example.com/x.zip");
        assert_eq!(archive.path(), Some("bundle"));
        assert!(extension.latest(1).is_some());
    }

    #[test]
    fn an_older_or_newer_schema_is_refused() {
        for schema in ["0", "2"] {
            let text = document().replace("schema = 1", &format!("schema = {schema}"));
            assert!(matches!(Registry::parse(&text), Err(Error::Schema { .. })), "{schema}");
        }
        assert!(Registry::parse("schema = 1\n").is_err(), "a name is required");
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let text = document().replace(
            "name = \"Winter extensions\"",
            "name = \"Winter extensions\"\nsurprise = true",
        );
        assert!(matches!(Registry::parse(&text), Err(Error::Parse(_))));

        let text = document().replace("path = \"bundle\"", "path = \"bundle\", branch = \"main\"");
        assert!(matches!(Registry::parse(&text), Err(Error::Parse(_))));
    }

    #[test]
    fn an_unknown_source_type_is_listed_but_not_installable() {
        let text = document().replace("type = \"archive\"", "type = \"git\"");
        let registry = Registry::parse(&text).expect("a future adapter must not fail the registry");

        let release = &registry.extensions[0].versions[0];
        assert_eq!(release.source, Source::Unsupported);
        assert!(!release.is_installable());
        assert!(release.archive().is_none());
        assert!(registry.extensions[0].latest(1).is_none());
    }

    #[test]
    fn an_archive_url_must_be_https_or_file() {
        for url in [
            "http://example.com/x.zip",
            "ftp://example.com/x.zip",
            "https://example.com/a b.zip",
        ] {
            let text = format!(
                "{}{}",
                head(),
                extension("a.b", &[release("1.0.0", 1, &archive(url, SHA, None))])
            );
            assert!(matches!(Registry::parse(&text), Err(Error::ArchiveUrl { .. })), "{url}");
        }

        let text = format!(
            "{}{}",
            head(),
            extension("a.b", &[release("1.0.0", 1, &archive("file:///tmp/x.zip", SHA, None))])
        );
        assert!(Registry::parse(&text).is_ok());
    }

    #[test]
    fn a_checksum_is_64_lowercase_hex_digits() {
        for sha in ["abc", &"A".repeat(64), &"g".repeat(64)] {
            let text = format!(
                "{}{}",
                head(),
                extension(
                    "a.b",
                    &[release("1.0.0", 1, &archive("https://example.com/x.zip", sha, None))]
                )
            );
            assert!(matches!(Registry::parse(&text), Err(Error::ArchiveSha256 { .. })));
        }
    }

    #[test]
    fn an_archive_path_may_not_escape() {
        for path in ["..", "../evil", "/etc", "a/../../b", "a\\\\b", "a//b"] {
            let text = format!(
                "{}{}",
                head(),
                extension(
                    "a.b",
                    &[release(
                        "1.0.0",
                        1,
                        &archive("https://example.com/x.zip", SHA, Some(path))
                    )]
                )
            );
            assert!(
                matches!(Registry::parse(&text), Err(Error::ArchivePath { .. })),
                "{path}"
            );
        }
    }

    #[test]
    fn a_version_must_be_semver() {
        let text = document().replace("version = \"0.1.0\"", "version = \"1\"");
        assert!(matches!(Registry::parse(&text), Err(Error::Parse(_))));

        let text = document().replace("version = \"0.1.0\"", "version = \"1.0.0-rc.1+build\"");
        assert!(Registry::parse(&text).is_ok());
    }

    #[test]
    fn the_highest_compatible_release_wins() {
        let text = format!(
            "{}{}",
            head(),
            extension(
                "a.b",
                &[
                    release("0.1.0", 1, &archive("https://example.com/a.zip", SHA, None)),
                    release("1.0.0", 1, &archive("https://example.com/b.zip", SHA, None)),
                    release("1.1.0", 2, &archive("https://example.com/c.zip", SHA, None)),
                    release("0.9.0", 1, &archive("https://example.com/d.zip", SHA, None)),
                ]
            )
        );
        let registry = Registry::parse(&text).expect("registry");
        let extension = &registry.extensions[0];

        assert_eq!(
            extension
                .latest(1)
                .map(|release| release.version.to_string())
                .as_deref(),
            Some("1.0.0")
        );
        assert_eq!(
            extension
                .latest(2)
                .map(|release| release.version.to_string())
                .as_deref(),
            Some("1.1.0")
        );
        assert!(extension.latest(3).is_none());
        assert!(extension.release(&semver::Version::new(0, 9, 0)).is_some());
    }

    #[test]
    fn a_prerelease_sorts_below_its_stable_release() {
        let text = format!(
            "{}{}",
            head(),
            extension(
                "a.b",
                &[
                    release("1.0.0-rc.1", 1, &archive("https://example.com/a.zip", SHA, None)),
                    release("1.0.0", 1, &archive("https://example.com/b.zip", SHA, None)),
                ]
            )
        );
        let registry = Registry::parse(&text).expect("registry");
        assert_eq!(
            registry.extensions[0]
                .latest(1)
                .map(|release| release.version.to_string())
                .as_deref(),
            Some("1.0.0")
        );
    }

    #[test]
    fn duplicate_identifiers_and_versions_are_refused() {
        let one = release("1.0.0", 1, &archive("https://example.com/a.zip", SHA, None));
        let text = format!(
            "{}{}{}",
            head(),
            extension("a.b", std::slice::from_ref(&one)),
            extension("a.b", &[one])
        );
        assert!(matches!(Registry::parse(&text), Err(Error::DuplicateId { .. })));

        let text = format!(
            "{}{}",
            head(),
            extension(
                "a.b",
                &[
                    release("1.0.0", 1, &archive("https://example.com/a.zip", SHA, None)),
                    release("1.0.0", 1, &archive("https://example.com/b.zip", SHA, None)),
                ]
            )
        );
        assert!(matches!(Registry::parse(&text), Err(Error::DuplicateVersion { .. })));
    }

    #[test]
    fn an_extension_needs_a_valid_identity_and_release() {
        let text = format!(
            "{}{}",
            head(),
            extension(
                "Bad/Id",
                &[release("1.0.0", 1, &archive("https://example.com/a.zip", SHA, None))]
            )
        );
        assert!(matches!(Registry::parse(&text), Err(Error::Id { .. })));

        let text = format!("{}{}", head(), extension("a.b", &[]));
        assert!(matches!(Registry::parse(&text), Err(Error::NoVersions { .. })));

        let text = format!(
            "{}{}",
            head(),
            extension(
                "a.b",
                &[release("1.0.0", 0, &archive("https://example.com/a.zip", SHA, None))]
            )
        );
        assert!(matches!(Registry::parse(&text), Err(Error::ApiZero { .. })));
    }

    #[test]
    fn the_file_is_read_with_the_caps() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("registry.toml");
        std::fs::write(&path, document()).expect("write");

        let registry = Registry::load(&path).expect("load");
        assert_eq!(registry.name, "Winter extensions");

        let big = directory.path().join("big.toml");
        std::fs::write(&big, "x".repeat(FILE_MAX as usize + 1)).expect("write");
        assert!(matches!(Registry::load(&big), Err(Error::TooLarge { .. })));
    }

    #[test]
    fn the_shipped_fixture_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/registry.toml");
        let registry = Registry::load(&path).expect("the fixture is a valid registry");

        let extension = &registry.extensions[0];
        assert_eq!(extension.id, "winter.failed-command");
        assert_eq!(
            extension
                .latest(1)
                .map(|release| release.version.to_string())
                .as_deref(),
            Some("0.2.0")
        );
        assert_eq!(
            extension.versions[1].archive().and_then(Archive::path),
            Some("failed-command")
        );
    }
}
