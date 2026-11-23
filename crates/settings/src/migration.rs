//! Reading the settings an earlier build wrote.
//!
//! ADR 022's file was `settings.json`, a JSON document of two fields: the
//! profile list and the name of the default profile. That schema has no
//! version of its own — it is version 0, and the first document this build
//! writes is version 1. A `settings.json` is still read when `settings.toml`
//! does not exist, and the migrated result is written out as TOML so the
//! migration happens once. The old file is left alone: deleting a user's file
//! is not a migration (ADR 034).

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::validation::Warning;
use crate::{Profile, Settings};

/// The version 0 document.
///
/// Unknown fields are ignored, as ADR 022 promised: a file a later build wrote
/// still reads as well as this schema can.
#[derive(Debug, Default, Deserialize)]
struct Legacy {
    #[serde(default)]
    profiles: Vec<LegacyProfile>,
    #[serde(default)]
    default_profile: Option<String>,
}

/// One profile in the version 0 document.
#[derive(Debug, Default, Deserialize)]
struct LegacyProfile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    title: Option<String>,
}

/// Why an earlier file could not be migrated.
#[derive(Debug)]
pub struct MigrationError(String);

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MigrationError {}

/// The earlier file that sits beside the current one.
pub fn legacy_path(toml: &Path) -> PathBuf {
    toml.with_extension("json")
}

/// Reads the version 0 document and returns it as today's settings, normalized.
///
/// The two fields the old schema held are the two this build still knows, so
/// nothing is dropped. A file with no profiles keeps the built-in ones: an
/// empty profile list meant "use the defaults" in version 0 too.
pub fn from_json(text: &str) -> Result<(Settings, Vec<Warning>), MigrationError> {
    let legacy: Legacy = serde_json::from_str(text).map_err(|error| MigrationError(error.to_string()))?;

    let mut settings = Settings::default();
    if !legacy.profiles.is_empty() {
        settings.profiles = legacy
            .profiles
            .into_iter()
            .map(|profile| Profile {
                name: profile.name,
                command: profile.command,
                args: profile.args,
                title: profile.title,
            })
            .collect();
        // `None` is not a guess: normalization names the first profile without
        // a warning, which is what a version 0 file that never chose one means.
        settings.default_profile = legacy.default_profile;
    }

    Ok(settings.normalize())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const LEGACY: &str = r#"{
        "profiles": [
            { "name": "work", "command": "cmd.exe", "args": ["/k", "echo hi"], "title": "Work" },
            { "name": "shell", "command": "bash.exe", "args": [], "title": null }
        ],
        "default_profile": "work"
    }"#;

    #[test]
    fn the_earlier_file_becomes_todays_settings() {
        let (settings, warnings) = from_json(LEGACY).expect("migrate");

        assert_eq!(settings.profiles.len(), 2);
        assert_eq!(settings.default_profile.as_deref(), Some("work"));
        let work = settings.profile().expect("the default profile");
        assert_eq!(work.command, "cmd.exe");
        assert_eq!(work.args, ["/k", "echo hi"]);
        assert_eq!(work.title.as_deref(), Some("Work"));
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn the_settings_the_old_file_never_held_get_their_defaults() {
        let (settings, _) = from_json(LEGACY).expect("migrate");
        let defaults = Settings::default();

        assert_eq!(settings.keybindings, defaults.keybindings);
        assert_eq!(settings.font, defaults.font);
        assert!(settings.schemes.is_empty());
    }

    #[test]
    fn a_file_with_no_profiles_keeps_the_built_in_ones() {
        let (settings, warnings) = from_json("{ \"default_profile\": \"work\" }").expect("migrate");

        assert_eq!(settings, Settings::default());
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_profile_with_no_command_gets_the_platform_shell() {
        let (settings, warnings) = from_json(r#"{ "profiles": [{ "name": "a" }] }"#).expect("migrate");

        assert!(!settings.profiles[0].command.is_empty());
        assert!(
            warnings
                .iter()
                .any(|warning| matches!(warning, Warning::MissingCommand { .. })),
            "{warnings:?}"
        );
    }

    #[test]
    fn a_default_that_names_nothing_falls_back_to_the_first_profile() {
        let (settings, warnings) =
            from_json(r#"{ "profiles": [{ "name": "a", "command": "sh" }], "default_profile": "gone" }"#)
                .expect("migrate");

        assert_eq!(settings.default_profile.as_deref(), Some("a"));
        assert!(
            warnings
                .iter()
                .any(|warning| matches!(warning, Warning::UnknownDefaultProfile { .. })),
            "{warnings:?}"
        );
    }

    #[test]
    fn migrating_twice_is_migrating_once() {
        // The result is written as TOML; reading it back is the identity.
        let (first, _) = from_json(LEGACY).expect("migrate");
        let written = crate::to_toml(&first);
        let (second, warnings) = crate::parse_with_warnings(&written).expect("parse the migration");

        assert_eq!(second, first);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_broken_file_says_so() {
        let error = from_json("{ not json").expect_err("a broken file");
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn the_earlier_file_sits_beside_the_current_one() {
        assert_eq!(
            legacy_path(Path::new("/tmp/config/winter/settings.toml")),
            PathBuf::from("/tmp/config/winter/settings.json")
        );
    }

    proptest! {
        /// Whatever a version 0 file says, the migration is a fixed point: the
        /// document written for it reads back as the settings that were
        /// written, which is what makes a second start a no-op.
        #[test]
        fn whatever_the_old_file_says_the_migration_settles(
            profiles in prop::collection::vec(
                ("[a-zA-Z0-9 ]{0,8}", "[a-zA-Z0-9 .\\/]{0,12}", prop::collection::vec("[a-z]{0,4}", 0..3), prop::option::of("[A-Za-z ]{0,6}")),
                0..4,
            ),
            default in prop::option::of("[a-zA-Z0-9 ]{0,8}"),
        ) {
            let profiles: Vec<String> = profiles
                .into_iter()
                .map(|(name, command, args, title)| {
                    let args: Vec<String> = args.iter().map(|arg| format!("\"{arg}\"")).collect();
                    let title = title.map(|title| format!("\"{title}\"")).unwrap_or_else(|| "null".to_string());
                    format!(
                        "{{ \"name\": \"{name}\", \"command\": \"{command}\", \"args\": [{}], \"title\": {title} }}",
                        args.join(", ")
                    )
                })
                .collect();
            let default = default.map(|name| format!("\"{name}\"")).unwrap_or_else(|| "null".to_string());
            let document = format!(
                "{{ \"profiles\": [{}], \"default_profile\": {default} }}",
                profiles.join(", ")
            );

            let (migrated, _) = from_json(&document).expect("a document this shape parses");
            let written = crate::to_toml(&migrated);
            let (again, _) = crate::parse_with_warnings(&written).expect("the migration is a valid document");

            prop_assert_eq!(again, migrated);
        }
    }
}
