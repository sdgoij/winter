//! Fetching one registry document, with a last-good cache.
//!
//! The fetcher is the only part of the registry that names a transport. It reads
//! `https` through `ureq` and `file` through the filesystem (for a mirror and for
//! tests); everything else in this crate stays pure. A fetch that fails or
//! returns a document that does not parse falls back to the last good copy, so an
//! offline machine still searches what it saw last time.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use ureq::ResponseExt;

use crate::schema::{Error as SchemaError, FILE_MAX, Registry};

/// The registry a fresh installation reads until the settings file names others.
pub const DEFAULT_URL: &str = "https://sdgoij.github.io/winter/extensions/registry.toml";

/// Where a registry comes from.
///
/// Tests implement this without a network; [`Network`] is the production one.
pub trait Fetcher {
    /// Reads `url`, capped at [`FILE_MAX`].
    fn fetch(&self, url: &str) -> Result<Vec<u8>, Error>;
}

/// The platform fetcher: `https` through `ureq`, `file` through the filesystem.
pub struct Network;

impl Fetcher for Network {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, Error> {
        if let Some(rest) = url.strip_prefix("file://") {
            return read_file(url, rest);
        }
        if !url.starts_with("https://") {
            return Err(Error::Unsupported { url: url.to_string() });
        }
        fetch_https(url)
    }
}

/// One document, and whether it came from the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The parsed document.
    pub document: Registry,
    /// Whether the network failed and the last good copy was used.
    pub cached: bool,
    /// Why the fresh copy was not used, when it was not.
    pub note: Option<String>,
}

/// Fetches `url`, or falls back to the cached copy for it.
///
/// A fresh document that parses is written to the cache. A network failure, or a
/// fresh document that does not parse, answers the cached copy with `cached` set
/// and the reason in `note`; with no cache, the failure is returned.
pub fn load(url: &str, cache: &Path, fetcher: &dyn Fetcher) -> Result<Fetched, Error> {
    match fetcher.fetch(url) {
        Ok(bytes) => match parse(url, &bytes) {
            Ok(document) => {
                write_cache(cache, url, &bytes);
                Ok(Fetched {
                    document,
                    cached: false,
                    note: None,
                })
            }
            Err(error) => match read_cache(cache, url) {
                Some(document) => Ok(Fetched {
                    document,
                    cached: true,
                    note: Some(error.to_string()),
                }),
                None => Err(error),
            },
        },
        Err(error) => match read_cache(cache, url) {
            Some(document) => Ok(Fetched {
                document,
                cached: true,
                note: Some(error.to_string()),
            }),
            None => Err(error),
        },
    }
}

/// The cache file for `url` under `cache`.
pub fn cache_path(cache: &Path, url: &str) -> PathBuf {
    let digest = Sha256::digest(url.as_bytes());
    let name: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    cache.join(format!("{name}.toml"))
}

/// When the cache file for `url` was last written, if it exists.
pub fn cached_at(cache: &Path, url: &str) -> Option<SystemTime> {
    std::fs::metadata(cache_path(cache, url))
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn fetch_https(url: &str) -> Result<Vec<u8>, Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .timeout_connect(Some(Duration::from_secs(5)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("Accept", "application/toml, text/plain")
        .header("User-Agent", concat!("Winter/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|error| Error::Http {
            url: url.to_string(),
            message: error.to_string(),
        })?;
    if response.get_uri().scheme_str() != Some("https") {
        return Err(Error::Redirected {
            url: url.to_string(),
            to: response.get_uri().to_string(),
        });
    }

    // Ask for one byte over the cap: a body that reached the cap is refused by
    // the length check rather than silently truncated.
    let cap = FILE_MAX.saturating_add(1);
    let bytes = response
        .body_mut()
        .with_config()
        .limit(cap)
        .read_to_vec()
        .map_err(|error| Error::Http {
            url: url.to_string(),
            message: error.to_string(),
        })?;
    if bytes.len() as u64 > FILE_MAX {
        return Err(Error::TooLarge {
            url: url.to_string(),
            size: bytes.len() as u64,
            max: FILE_MAX,
        });
    }
    Ok(bytes)
}

fn read_file(url: &str, rest: &str) -> Result<Vec<u8>, Error> {
    let path = file_path(rest);
    let file = File::open(&path).map_err(|source| Error::Io {
        url: url.to_string(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(FILE_MAX.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| Error::Io {
            url: url.to_string(),
            source,
        })?;
    if bytes.len() as u64 > FILE_MAX {
        return Err(Error::TooLarge {
            url: url.to_string(),
            size: bytes.len() as u64,
            max: FILE_MAX,
        });
    }
    Ok(bytes)
}

/// The path a `file://` URL names.
///
/// `file:///C:/x` is a Windows path with a leading slash the filesystem does not
/// want; `file:///tmp/x` on Unix keeps its slash.
fn file_path(rest: &str) -> PathBuf {
    let path = if cfg!(windows) && rest.starts_with('/') && rest.as_bytes().get(2) == Some(&b':') {
        &rest[1..]
    } else {
        rest
    };
    PathBuf::from(path)
}

fn parse(url: &str, bytes: &[u8]) -> Result<Registry, Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::NotUtf8 { url: url.to_string() })?;
    Registry::parse(text).map_err(|source| Error::Document {
        url: url.to_string(),
        source,
    })
}

fn write_cache(cache: &Path, url: &str, bytes: &[u8]) {
    // The cache is an optimization: a read-only settings directory must not make
    // a fetched registry fail.
    if std::fs::create_dir_all(cache).is_err() {
        return;
    }
    let path = cache_path(cache, url);
    let part = path.with_extension("part");
    if std::fs::write(&part, bytes).is_err() {
        return;
    }
    // Windows refuses to rename over an existing file, and a cache is not worth
    // an atomic replace: remove the old copy and try again.
    if std::fs::rename(&part, &path).is_err() {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::rename(&part, &path);
    }
}

fn read_cache(cache: &Path, url: &str) -> Option<Registry> {
    let bytes = std::fs::read(cache_path(cache, url)).ok()?;
    if bytes.len() as u64 > FILE_MAX {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    Registry::parse(text).ok()
}

/// What a fetch can be wrong about.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The URL's scheme is not one this build reads.
    #[error("{url}: only https and file are read")]
    Unsupported {
        /// The URL as written.
        url: String,
    },
    /// A `file` URL could not be read.
    #[error("{url}: {source}")]
    Io {
        /// The URL as written.
        url: String,
        /// The operating system's reason.
        #[source]
        source: std::io::Error,
    },
    /// The HTTP request failed, timed out, or answered an error status.
    #[error("{url}: {message}")]
    Http {
        /// The URL as written.
        url: String,
        /// The client's reason.
        message: String,
    },
    /// A redirect left `https` for another scheme.
    #[error("{url} redirected to {to}, which is not https")]
    Redirected {
        /// The URL as written.
        url: String,
        /// The final URL.
        to: String,
    },
    /// The body is over [`FILE_MAX`].
    #[error("{url} is {size} bytes, over the {max}-byte cap")]
    TooLarge {
        /// The URL as written.
        url: String,
        /// The body's length.
        size: u64,
        /// The cap.
        max: u64,
    },
    /// The body is not UTF-8.
    #[error("{url} is not UTF-8")]
    NotUtf8 {
        /// The URL as written.
        url: String,
    },
    /// The body is not a valid registry.
    #[error("{url}: {source}")]
    Document {
        /// The URL as written.
        url: String,
        /// The schema's reason.
        #[source]
        source: SchemaError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn document() -> Vec<u8> {
        document_named("Test registry")
    }

    fn document_named(name: &str) -> Vec<u8> {
        format!(
            "schema = 1\nname = \"{name}\"\nupdated = \"2026-11-28T12:00:00Z\"\n\n[[extensions]]\nid = \"a.b\"\nname = \"B\"\n[[extensions.versions]]\nversion = \"1.0.0\"\napi = 1\nsource = {{ type = \"archive\", url = \"https://example.com/b.zip\", sha256 = \"{SHA}\" }}\n"
        )
        .into_bytes()
    }

    struct Fake(Result<Vec<u8>, String>);

    impl Fetcher for Fake {
        fn fetch(&self, url: &str) -> Result<Vec<u8>, Error> {
            match &self.0 {
                Ok(bytes) => Ok(bytes.clone()),
                Err(message) => Err(Error::Http {
                    url: url.to_string(),
                    message: message.clone(),
                }),
            }
        }
    }

    fn file_url(path: &Path) -> String {
        let text = path.display().to_string().replace('\\', "/");
        if text.starts_with('/') {
            format!("file://{text}")
        } else {
            format!("file:///{text}")
        }
    }

    #[test]
    fn a_fresh_document_is_parsed_and_cached() {
        let cache = tempfile::tempdir().expect("cache");
        let fetched = load("https://one.example/registry.toml", cache.path(), &Fake(Ok(document()))).expect("fetch");

        assert!(!fetched.cached);
        assert!(fetched.note.is_none());
        assert_eq!(fetched.document.name, "Test registry");
        assert!(cache_path(cache.path(), "https://one.example/registry.toml").is_file());
        assert!(cached_at(cache.path(), "https://one.example/registry.toml").is_some());
    }

    #[test]
    fn a_second_fetch_replaces_the_cache() {
        let cache = tempfile::tempdir().expect("cache");
        let url = "https://one.example/registry.toml";
        load(url, cache.path(), &Fake(Ok(document_named("One")))).expect("first");
        load(url, cache.path(), &Fake(Ok(document_named("Two")))).expect("second");

        let fetched = load(url, cache.path(), &Fake(Err("offline".to_string()))).expect("cached");
        assert_eq!(fetched.document.name, "Two");
    }

    #[test]
    fn a_failed_fetch_answers_the_cache() {
        let cache = tempfile::tempdir().expect("cache");
        let url = "https://one.example/registry.toml";
        load(url, cache.path(), &Fake(Ok(document()))).expect("prime the cache");

        let fetched = load(url, cache.path(), &Fake(Err("offline".to_string()))).expect("fall back");
        assert!(fetched.cached);
        assert_eq!(fetched.document.name, "Test registry");
        assert!(fetched.note.as_deref().is_some_and(|note| note.contains("offline")));
    }

    #[test]
    fn a_malformed_fresh_document_answers_the_cache() {
        let cache = tempfile::tempdir().expect("cache");
        let url = "https://one.example/registry.toml";
        load(url, cache.path(), &Fake(Ok(document()))).expect("prime the cache");

        let fetched = load(url, cache.path(), &Fake(Ok(b"not a registry".to_vec()))).expect("fall back");
        assert!(fetched.cached);
        assert!(fetched.note.as_deref().is_some_and(|note| !note.is_empty()));
    }

    #[test]
    fn a_failure_with_no_cache_is_the_failure() {
        let cache = tempfile::tempdir().expect("cache");
        let error = load(
            "https://one.example/registry.toml",
            cache.path(),
            &Fake(Err("offline".to_string())),
        )
        .expect_err("no cache");
        assert!(matches!(error, Error::Http { .. }));
    }

    #[test]
    fn file_urls_are_read_without_a_network() {
        let directory = tempfile::tempdir().expect("dir");
        let path = directory.path().join("registry.toml");
        std::fs::write(&path, document()).expect("write");
        let url = file_url(&path);

        let bytes = Network.fetch(&url).expect("file fetch");
        assert_eq!(bytes, document());

        let cache = tempfile::tempdir().expect("cache");
        let fetched = load(&url, cache.path(), &Network).expect("load");
        assert_eq!(fetched.document.name, "Test registry");
    }

    #[test]
    fn anything_but_https_and_file_is_refused() {
        let error = Network.fetch("http://example.com/registry.toml").expect_err("http");
        assert!(matches!(error, Error::Unsupported { .. }));
    }

    #[test]
    fn a_file_over_the_cap_is_refused() {
        let directory = tempfile::tempdir().expect("dir");
        let path = directory.path().join("big.toml");
        std::fs::write(&path, "x".repeat(FILE_MAX as usize + 1)).expect("write");

        let error = Network.fetch(&file_url(&path)).expect_err("too large");
        assert!(matches!(error, Error::TooLarge { .. }));
    }

    #[test]
    fn the_cache_name_depends_only_on_the_url() {
        let cache = Path::new("/tmp/cache");
        assert_eq!(
            cache_path(cache, "https://one.example/registry.toml"),
            cache_path(cache, "https://one.example/registry.toml")
        );
        assert_ne!(
            cache_path(cache, "https://one.example/registry.toml"),
            cache_path(cache, "https://two.example/registry.toml")
        );
    }
}
