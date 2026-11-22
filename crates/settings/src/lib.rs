//! User settings: the file, the default profile and the rules that make a
//! partial file usable.
//!
//! The schema is the port's own (ADR 022). Windows Terminal's `settings.json`
//! is a large cascadia schema; this one starts with what the application
//! consumes — named profiles — and grows one consumer at a time.

mod parser;
mod profiles;

pub use parser::{load, parse, settings_path, to_json};
pub use profiles::{Profile, default_shell};

use serde::{Deserialize, Serialize};

/// The settings file's root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The named profiles this installation knows.
    pub profiles: Vec<Profile>,
    /// The profile a new tab uses, by name.
    pub default_profile: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let profile = Profile::default();
        Self {
            default_profile: Some(profile.name.clone()),
            profiles: vec![profile],
        }
    }
}

impl Settings {
    /// Returns the profile a new tab should use.
    pub fn profile(&self) -> Option<&Profile> {
        self.default_profile
            .as_deref()
            .and_then(|name| self.profiles.iter().find(|profile| profile.name == name))
            .or_else(|| self.profiles.first())
    }

    /// Fills in what a partial file leaves out.
    ///
    /// Called on every parse: an empty list gets the default profile, a blank
    /// command gets the platform shell, and a default that names nothing falls
    /// back to the first profile.
    pub(crate) fn normalize(mut self) -> Self {
        if self.profiles.is_empty() {
            self.profiles.push(Profile::default());
        }

        for profile in &mut self.profiles {
            if profile.name.trim().is_empty() {
                profile.name = "default".to_string();
            }
            if profile.command.trim().is_empty() {
                profile.command = profiles::default_shell();
            }
        }

        let known = self
            .default_profile
            .as_deref()
            .is_some_and(|name| self.profiles.iter().any(|profile| profile.name == name));
        if !known {
            self.default_profile = Some(self.profiles[0].name.clone());
        }

        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_settings_run_the_platform_shell() {
        let settings = Settings::default();
        assert_eq!(settings.default_profile.as_deref(), Some("default"));
        assert_eq!(settings.profile().map(|profile| profile.name.as_str()), Some("default"));
        assert!(!settings.profile().expect("profile").command.is_empty());
    }

    #[test]
    fn an_empty_object_gets_the_defaults() {
        let settings = parse("{}").expect("parse");
        assert_eq!(settings.profiles.len(), 1);
        assert_eq!(settings.profile().map(|profile| profile.name.as_str()), Some("default"));
        assert_eq!(settings.default_profile.as_deref(), Some("default"));
    }

    #[test]
    fn a_named_profile_is_read_from_json() {
        let json = r#"{
            "profiles": [
                { "name": "zsh", "command": "/bin/zsh", "args": ["-l"], "title": "Z" }
            ],
            "default_profile": "zsh"
        }"#;
        let settings = parse(json).expect("parse");
        let profile = settings.profile().expect("profile");

        assert_eq!(profile.name, "zsh");
        assert_eq!(profile.command, "/bin/zsh");
        assert_eq!(profile.args, vec!["-l".to_string()]);
        assert_eq!(profile.title.as_deref(), Some("Z"));
    }

    #[test]
    fn a_default_that_names_nothing_falls_back_to_the_first_profile() {
        let json = r#"{
            "profiles": [
                { "name": "one", "command": "sh" },
                { "name": "two", "command": "sh" }
            ],
            "default_profile": "missing"
        }"#;
        let settings = parse(json).expect("parse");
        assert_eq!(settings.default_profile.as_deref(), Some("one"));
        assert_eq!(settings.profile().map(|profile| profile.name.as_str()), Some("one"));
    }

    #[test]
    fn an_empty_list_or_command_is_filled_in() {
        let settings = parse(r#"{"profiles": []}"#).expect("parse");
        assert_eq!(settings.profiles.len(), 1);
        assert!(!settings.profile().expect("profile").command.is_empty());

        let settings = parse(r#"{"profiles": [{"name": "x", "command": "   "}]}"#).expect("parse");
        assert_eq!(settings.profile().expect("profile").command, default_shell());
    }

    #[test]
    fn unknown_fields_and_invalid_json() {
        let settings = parse(r#"{"profiles": [{"name": "x", "command": "sh", "colour": 1}], "extra": true}"#)
            .expect("unknown fields are ignored");
        assert_eq!(settings.profile().map(|profile| profile.name.as_str()), Some("x"));

        assert!(parse("{").is_err());
    }

    #[test]
    fn settings_round_trip_through_json() {
        let settings = Settings::default();
        let json = to_json(&settings);
        assert_eq!(parse(&json).expect("parse"), settings);
    }
}
