//! The extension registry: schema, validation, fetching, installation and the
//! rows a shell searches.
//!
//! A registry is one static TOML document at an HTTPS URL (ADR 088). The schema
//! and its validation are pure; [`fetch`] is the only part that names a network
//! transport, and [`Job`] moves it off the UI thread with a channel polled from
//! the tick. [`install`] verifies an archive against the registry and places it
//! beside the settings file, and [`provenance`] says where a directory came
//! from. [`Screen`] is the Search tab's state: it fetches on first open, keeps
//! the picked version per row, and runs installs and uninstalls on workers. The
//! crate names no window and no engine, so an embedding can use the schema
//! without a registry view.
//!
//! The schema is strict — an unknown key is an error — so a typo is reported
//! instead of silently ignored, exactly as the extension manifest is. A source
//! type this build does not know is the one tolerated case: a future `git`
//! adapter must not make an older shell reject the whole registry.

use std::collections::HashSet;

mod fetch;
mod index;
mod install;
mod job;
mod schema;
mod screen;

pub use fetch::{DEFAULT_URL, Error as FetchError, Fetched, Fetcher, Network, cache_path, cached_at, load};
pub use index::{Index, Loaded, Row};
pub use install::{
    ARCHIVE_MAX, Error as InstallError, Installed, PROVENANCE, Provenance, install, provenance, uninstall,
    uninstall_any,
};
pub use job::{Job, State};
pub use schema::{
    Archive, DEPENDENCIES_MAX, DESCRIPTION_MAX, EXTENSIONS_MAX, Error, Extension, FILE_MAX, HOMEPAGE_MAX, LICENSE_MAX,
    RELEASES_MAX, Registry, Release, SCHEMA, Source, TAG_MAX, TAGS_MAX, URL_MAX,
};
pub use screen::{Changed, Local, Screen, SearchRow, Status};

/// The registry URLs a run reads.
///
/// `WINTER_REGISTRY` overrides the settings file for one run: `0` or `off`
/// disables registries, a URL replaces the list, and an empty value also means
/// none. Without it, `configured` names the list, and `None` means the built-in
/// default (ADR 088); `Some([])` means none.
pub fn configured_urls(configured: Option<&[String]>) -> Vec<String> {
    url_list(configured, std::env::var("WINTER_REGISTRY").ok().as_deref())
}

/// [`configured_urls`] with the environment value passed in, for tests.
pub fn url_list(configured: Option<&[String]>, override_value: Option<&str>) -> Vec<String> {
    let urls = match override_value {
        Some(value) if matches!(value.trim(), "0" | "off") => Vec::new(),
        Some(value) if !value.trim().is_empty() => vec![value.trim().to_string()],
        Some(_) => Vec::new(),
        None => configured.map_or_else(|| vec![DEFAULT_URL.to_string()], |list| list.to_vec()),
    };

    let mut seen = HashSet::new();
    urls.into_iter()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty() && seen.insert(url.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_registry_is_used_until_a_list_names_others() {
        assert_eq!(url_list(None, None), vec![DEFAULT_URL.to_string()]);
        assert!(url_list(Some(&[]), None).is_empty());

        let configured = [
            " https://one.example/registry.toml ".to_string(),
            "https://one.example/registry.toml".to_string(),
            String::new(),
            "https://two.example/registry.toml".to_string(),
        ];
        assert_eq!(
            url_list(Some(&configured), None),
            vec![
                "https://one.example/registry.toml".to_string(),
                "https://two.example/registry.toml".to_string()
            ]
        );
    }

    #[test]
    fn the_environment_overrides_the_file() {
        let configured = ["https://one.example/registry.toml".to_string()];
        assert!(url_list(Some(&configured), Some("off")).is_empty());
        assert!(url_list(Some(&configured), Some("0")).is_empty());
        assert!(url_list(None, Some("")).is_empty());
        assert_eq!(
            url_list(None, Some("file:///tmp/registry.toml")),
            vec!["file:///tmp/registry.toml".to_string()]
        );
    }
}
