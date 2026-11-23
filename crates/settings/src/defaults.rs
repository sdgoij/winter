//! The settings a fresh installation runs with.
//!
//! Upstream ships `defaults.json` beside the executable and layers the user's
//! file over it. This port has one file: [`Settings::default`] is the base
//! layer, normalization lays the user's document over it, and the application
//! writes the base out when it finds no file, so a fresh installation has
//! something to edit (ADR 034).

use std::path::Path;

use crate::{CURRENT_VERSION, DEFAULT_KEYBINDINGS, Font, Settings, default_profiles, migration, parser};

/// The settings in force before any file is read.
pub fn settings() -> Settings {
    let profiles = default_profiles();
    Settings {
        version: CURRENT_VERSION,
        default_profile: profiles.first().map(|profile| profile.name.clone()),
        default_scheme: None,
        profiles,
        keybindings: DEFAULT_KEYBINDINGS.to_vec(),
        schemes: Vec::new(),
        theme: None,
        themes: Vec::new(),
        font: Font::default(),
    }
}

/// Writes the default document when this platform has no settings file.
///
/// This is the one write the application makes that nobody asked for, and it
/// happens once: a file that exists — even an empty or broken one — is left
/// alone, and so is a directory holding an earlier build's `settings.json`,
/// which the migration reads instead.
pub fn ensure_file() {
    let Some(path) = parser::settings_path() else {
        return;
    };
    ensure_file_at(&path);
}

fn ensure_file_at(path: &Path) {
    if path.exists() || migration::legacy_path(path).exists() || parser::has_legacy_settings(path) {
        return;
    }
    // A directory the process cannot write is not an error: the defaults are
    // in force either way.
    let _ = parser::save_to(path, &settings());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_written_defaults_read_back_as_the_defaults() {
        let text = crate::to_toml(&settings());
        let (parsed, warnings) = crate::parse_with_warnings(&text).expect("the defaults parse");

        assert_eq!(parsed, settings());
        assert!(warnings.is_empty(), "the defaults need no explanation: {warnings:?}");
    }

    #[test]
    fn the_defaults_name_a_profile_that_exists() {
        let settings = settings();
        let default = settings.default_profile.as_deref().expect("a default profile");

        assert!(settings.profile_named(default).is_some(), "{default}");
        assert_eq!(settings.profile(), settings.profiles.first());
    }

    #[test]
    fn a_missing_file_is_written_once_and_loads_as_the_defaults() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("winter").join("settings.toml");

        ensure_file_at(&path);
        assert!(path.exists(), "a fresh installation gets a file to edit");
        assert_eq!(parser::load_from_path(Some(path.clone())).settings, settings());

        // A file that exists is the user's, even an empty one.
        std::fs::write(&path, "version = 1\n\n[font]\nsize = 30\n").expect("write");
        ensure_file_at(&path);
        assert_eq!(parser::load_from_path(Some(path)).settings.font.size, 30.0);
    }

    #[test]
    fn a_file_in_the_old_directory_is_not_shadowed_by_the_defaults() {
        let directory = tempfile::tempdir().expect("temp dir");
        let old = directory.path().join("winterm").join("settings.toml");
        std::fs::create_dir_all(old.parent().expect("parent")).expect("create");
        std::fs::write(&old, "version = 1\n").expect("write");
        let path = directory.path().join("winter").join("settings.toml");

        ensure_file_at(&path);
        assert!(!path.exists(), "the migration reads the old file instead");
    }

    #[test]
    fn an_earlier_file_is_not_shadowed_by_the_defaults() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        std::fs::write(directory.path().join("settings.json"), "{}").expect("write");

        ensure_file_at(&path);
        assert!(!path.exists(), "the migration reads the old file instead");
    }
}
