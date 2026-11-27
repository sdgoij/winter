//! The extension manifest: `extension.json` and the caps that bound it.
//!
//! Parsing is strict — an unknown key is an error — so a typo is reported
//! instead of silently ignored. The identifier is the extension's identity
//! everywhere (in `loadAfter`, in the `winter.<module>` call it registers, in
//! the log line a failure is reported on), so it is validated here rather than
//! at each use.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The file that makes a directory an extension.
pub const FILE: &str = "extension.json";

/// The API major this build hosts.
pub const API: u32 = 1;

/// The largest manifest file, in bytes.
pub const MANIFEST_MAX: u64 = 256 * 1024;

/// The largest entry module, in bytes.
pub const ENTRY_MAX: u64 = 4 * 1024 * 1024;

/// The largest other module an extension may register, in bytes.
pub const SOURCE_MAX: u64 = 4 * 1024 * 1024;

/// The largest total of an extension's other modules, in bytes.
pub const SOURCES_MAX: u64 = 16 * 1024 * 1024;

/// The largest compiled module, in bytes.
pub const MODULE_MAX: u64 = 16 * 1024 * 1024;

/// The largest asset, in bytes.
pub const ASSET_MAX: u64 = 64 * 1024 * 1024;

/// How many other modules one extension may register.
pub const SOURCE_COUNT_MAX: usize = 64;

/// The longest identifier, in bytes.
pub const ID_MAX: usize = 64;

/// The longest asset name, in bytes.
pub const ASSET_NAME_MAX: usize = 64;

/// An extension's `extension.json`.
///
/// Every field is required unless marked optional. `api` is the extension's
/// major and is one integer, additive within a major, so a new event or module
/// does not bump it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Manifest {
    /// The extension's identity: a lowercase dotted name.
    pub id: String,
    /// The name a user sees.
    pub name: String,
    /// The extension's own version, free-form.
    pub version: String,
    /// The API major the extension was written for.
    pub api: u32,
    /// One line for the extensions screen.
    #[serde(default)]
    pub description: Option<String>,
    /// The author's name.
    #[serde(default)]
    pub author: Option<String>,
    /// Identifiers this extension must load after.
    #[serde(default)]
    pub load_after: Vec<String>,
    /// The ES module entry, relative to the extension directory.
    #[serde(default)]
    pub entry: Option<String>,
    /// The compiled module, relative to the extension directory.
    #[serde(default)]
    pub module: Option<String>,
    /// Assets, by the opaque name the extension asks for.
    #[serde(default)]
    pub assets: BTreeMap<String, String>,
}

impl Manifest {
    /// Read and validate `<dir>/extension.json`.
    pub fn load(dir: &Path) -> Result<Self, Error> {
        let path = dir.join(FILE);
        let bytes = read_capped(&path, MANIFEST_MAX)?;
        let text = String::from_utf8(bytes).map_err(|_| Error::NotUtf8 { path: path.clone() })?;
        Self::parse(&text)
    }

    /// Validate a manifest from text.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let manifest: Self = serde_json::from_str(text)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// The rules a parsed manifest must already satisfy.
    pub fn validate(&self) -> Result<(), Error> {
        if !is_id(&self.id) {
            return Err(Error::Id { id: self.id.clone() });
        }
        if self.name.trim().is_empty() {
            return Err(Error::Name);
        }
        if self.version.trim().is_empty() {
            return Err(Error::Version);
        }
        if self.api == 0 {
            return Err(Error::ApiZero);
        }
        if self.api > API {
            return Err(Error::ApiNewer {
                api: self.api,
                supported: API,
            });
        }
        for id in &self.load_after {
            if !is_id(id) {
                return Err(Error::LoadAfter { id: id.clone() });
            }
        }
        if let Some(entry) = &self.entry
            && !is_relative_path(entry)
        {
            return Err(Error::Path {
                field: "entry",
                path: entry.clone(),
            });
        }
        if let Some(module) = &self.module
            && !is_relative_path(module)
        {
            return Err(Error::Path {
                field: "module",
                path: module.clone(),
            });
        }
        for name in self.assets.keys() {
            if !is_asset_name(name) {
                return Err(Error::AssetName { name: name.clone() });
            }
        }
        for path in self.assets.values() {
            if !is_relative_path(path) {
                return Err(Error::Path {
                    field: "assets",
                    path: path.clone(),
                });
            }
        }
        if self.entry.is_none() && self.module.is_none() && self.assets.is_empty() {
            return Err(Error::Empty);
        }
        Ok(())
    }
}

/// Whether `id` is a lowercase dotted name: segments of `a–z`, `0–9`, `_` and
/// `-`, joined by `.`, starting and ending with an alphanumeric, at most
/// [`ID_MAX`] bytes. Case is refused rather than folded, so two extensions can
/// never differ only in case.
pub fn is_id(id: &str) -> bool {
    if id.is_empty() || id.len() > ID_MAX || id.contains("..") {
        return false;
    }
    if !id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return false;
    }
    if !id.ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return false;
    }
    id.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// Whether `path` is a relative path that stays inside the extension directory.
///
/// Absolute paths, `..`, empty segments, `.`, a backslash (a separator on
/// Windows) and a colon (a drive or URL scheme) are refused. The check is the
/// manifest's own; discovery re-checks the filesystem path it ends up with.
pub fn is_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// Whether `name` can be an asset's opaque name.
pub fn is_asset_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= ASSET_NAME_MAX
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Read at most `max` bytes, refusing a file that is longer.
///
/// The size is checked while reading rather than from the metadata, so a file
/// that grows between the check and the read is still capped.
pub(crate) fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, Error> {
    let file = File::open(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > max {
        return Err(Error::TooLarge {
            path: path.to_path_buf(),
            size: bytes.len() as u64,
            max,
        });
    }
    Ok(bytes)
}

/// What a manifest can be wrong about.
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
    /// The file is longer than [`MANIFEST_MAX`].
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
    /// The file is not the JSON it claims to be.
    #[error("the manifest is not valid: {0}")]
    Json(#[from] serde_json::Error),
    /// The identifier is not a lowercase dotted name.
    #[error("{id:?} is not a lowercase dotted identifier")]
    Id {
        /// The identifier as written.
        id: String,
    },
    /// The name is empty.
    #[error("`name` is empty")]
    Name,
    /// The version is empty.
    #[error("`version` is empty")]
    Version,
    /// The extension asks for a major that does not exist.
    #[error("`api` is 0; the first major is 1")]
    ApiZero,
    /// The extension asks for a major this build does not host.
    #[error("`api` is {api}, newer than the {supported} this build hosts")]
    ApiNewer {
        /// The major as written.
        api: u32,
        /// The major this build hosts, [`API`].
        supported: u32,
    },
    /// A `loadAfter` entry is not an identifier.
    #[error("`loadAfter` names {id:?}, which is not an identifier")]
    LoadAfter {
        /// The entry as written.
        id: String,
    },
    /// A path escapes the extension directory or is absolute.
    #[error("`{field}` is not a relative path inside the extension: {path:?}")]
    Path {
        /// The manifest field that carries it.
        field: &'static str,
        /// The path as written.
        path: String,
    },
    /// An asset name carries characters an opaque name may not.
    #[error("the asset name {name:?} may hold only letters, digits, `.`, `_` and `-`")]
    AssetName {
        /// The name as written.
        name: String,
    },
    /// The extension contributes nothing.
    #[error("the extension declares no entry, module or asset")]
    Empty,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{
        "id": "com.example.failed-command",
        "name": "Failed command",
        "version": "0.1.0",
        "api": 1,
        "description": "Says when a command exited non-zero.",
        "author": "example",
        "loadAfter": ["com.example.gridkit"],
        "entry": "extension.js",
        "module": "extension.wasm",
        "assets": { "icon": "assets/icon.png" }
    }"#;

    #[test]
    fn every_field_of_a_complete_manifest_is_read() {
        let manifest = Manifest::parse(FULL).expect("manifest");
        assert_eq!(manifest.id, "com.example.failed-command");
        assert_eq!(manifest.name, "Failed command");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.api, 1);
        assert_eq!(
            manifest.description.as_deref(),
            Some("Says when a command exited non-zero.")
        );
        assert_eq!(manifest.author.as_deref(), Some("example"));
        assert_eq!(manifest.load_after, ["com.example.gridkit"]);
        assert_eq!(manifest.entry.as_deref(), Some("extension.js"));
        assert_eq!(manifest.module.as_deref(), Some("extension.wasm"));
        assert_eq!(manifest.assets.get("icon").map(String::as_str), Some("assets/icon.png"));
    }

    #[test]
    fn an_unknown_key_is_an_error_rather_than_forward_compatibility() {
        let text = FULL.replace("\"api\": 1,", "\"api\": 1, \"loadAfterX\": [],");
        let error = Manifest::parse(&text).expect_err("a typo must be refused");
        assert!(matches!(error, Error::Json(_)), "{error}");
    }

    #[test]
    fn the_snake_case_spelling_of_a_camel_case_key_is_refused() {
        let text = FULL.replace("\"loadAfter\"", "\"load_after\"");
        assert!(Manifest::parse(&text).is_err());
    }

    #[test]
    fn a_data_only_extension_needs_an_asset() {
        let no_assets = r#"{"id": "a.b", "name": "B", "version": "1", "api": 1}"#;
        assert!(matches!(Manifest::parse(no_assets), Err(Error::Empty)));

        let data = r#"{"id": "a.b", "name": "B", "version": "1", "api": 1, "assets": {"icon": "icon.png"}}"#;
        assert!(Manifest::parse(data).is_ok());
    }

    #[test]
    fn the_api_major_bounds_what_loads() {
        let zero = FULL.replace("\"api\": 1", "\"api\": 0");
        assert!(matches!(Manifest::parse(&zero), Err(Error::ApiZero)));

        let newer = FULL.replace("\"api\": 1", "\"api\": 2");
        assert!(matches!(
            Manifest::parse(&newer),
            Err(Error::ApiNewer { api: 2, supported: 1 })
        ));
    }

    #[test]
    fn an_identifier_is_lowercase_dotted_and_bounded() {
        for id in ["a", "a.b", "com.example.failed-command", "com.example_2.x-y", "7z.pack"] {
            assert!(is_id(id), "{id} should be an identifier");
        }
        for id in [
            "",
            ".a",
            "a.",
            "a..b",
            "A.b",
            "a b",
            "a/b",
            "-a",
            "a-",
            "_a",
            "a\nb",
            &"a".repeat(ID_MAX + 1),
        ] {
            assert!(!is_id(id), "{id:?} should not be an identifier");
        }
    }

    #[test]
    fn a_dotted_identifier_may_not_be_all_dots_or_double_dots() {
        assert!(!is_id(".."));
        assert!(!is_id("a..b"));
    }

    #[test]
    fn a_path_may_not_escape_the_extension() {
        for path in ["extension.js", "lib/util.js", "assets/icon.png"] {
            assert!(is_relative_path(path), "{path} should be allowed");
        }
        for path in [
            "",
            "/etc/passwd",
            "../escape.js",
            "lib/../../escape.js",
            "lib//util.js",
            "./util.js",
            "lib\\util.js",
            "C:/escape.js",
            "https://example.com/x.js",
        ] {
            assert!(!is_relative_path(path), "{path:?} should be refused");
        }
    }

    #[test]
    fn an_asset_name_is_opaque_and_path_free() {
        assert!(is_asset_name("icon"));
        assert!(is_asset_name("icon.dark_2x"));
        assert!(!is_asset_name(""));
        assert!(!is_asset_name("a/b"));
        assert!(!is_asset_name("a b"));
        assert!(!is_asset_name(&"a".repeat(ASSET_NAME_MAX + 1)));
    }

    #[test]
    fn a_manifest_with_an_escaping_entry_is_refused_by_the_field_name() {
        let text = FULL.replace("\"entry\": \"extension.js\"", "\"entry\": \"../extension.js\"");
        match Manifest::parse(&text) {
            Err(Error::Path { field, path }) => {
                assert_eq!(field, "entry");
                assert_eq!(path, "../extension.js");
            }
            other => panic!("expected a path error, got {other:?}"),
        }
    }

    #[test]
    fn the_id_is_validated_as_an_id_not_only_as_a_path() {
        let text = FULL.replace("\"id\": \"com.example.failed-command\"", "\"id\": \"Com.Example\"");
        assert!(matches!(Manifest::parse(&text), Err(Error::Id { .. })));
    }

    #[test]
    fn a_load_after_entry_must_itself_be_an_identifier() {
        let text = FULL.replace("[\"com.example.gridkit\"]", "[\"Not An Id\"]");
        assert!(matches!(Manifest::parse(&text), Err(Error::LoadAfter { .. })));
    }
}
