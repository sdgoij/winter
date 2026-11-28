//! The extension registry: schema, validation and the rows a shell searches.
//!
//! A registry is one static TOML document at an HTTPS URL (ADR 088). This crate
//! is pure: it parses and validates a document and merges several into one
//! searchable index. It names no window, no engine and no network transport;
//! fetching and installing are the shell's job.
//!
//! The schema is strict — an unknown key is an error — so a typo is reported
//! instead of silently ignored, exactly as the extension manifest is. A source
//! type this build does not know is the one tolerated case: a future `git`
//! adapter must not make an older shell reject the whole registry.

mod index;
mod schema;

pub use index::{Index, Loaded, Row};
pub use schema::{
    Archive, DEPENDENCIES_MAX, DESCRIPTION_MAX, EXTENSIONS_MAX, Error, Extension, FILE_MAX, HOMEPAGE_MAX, LICENSE_MAX,
    RELEASES_MAX, Registry, Release, SCHEMA, Source, TAG_MAX, TAGS_MAX, URL_MAX,
};
