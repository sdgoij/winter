//! User settings: the file, the default profile and the rules that make a
//! partial file usable.
//!
//! The schema is the port's own (ADRs 022–026). Windows Terminal's
//! `settings.json` is a large cascadia schema; this one holds what the
//! application consumes — named profiles, keybindings, colour schemes and the
//! font size — and grows one consumer at a time. The document is TOML with a
//! `version`, so a future change can refuse an old file instead of misreading
//! it (ADR 026).

mod actions;
mod fonts;
mod keybindings;
mod palette;
mod parser;
mod profiles;
mod watch;

pub use actions::Action;
pub use fonts::Font;
pub use keybindings::{Chord, ChordError, DEFAULT_KEYBINDINGS, Key, Keybinding, Modifiers, resolve};
pub use palette::{Color, ColorError, Scheme};
pub use parser::{LoadError, Loaded, SettingsError, load, load_with_report, parse, settings_path, to_toml};
pub use profiles::{Profile, default_shell};
pub use watch::{Stamp, stamp};

use serde::{Deserialize, Serialize};

/// The schema version this build writes and understands.
///
/// A document that names a higher version is refused: the fields whose meaning
/// changed are exactly the ones an older build would misread.
pub const CURRENT_VERSION: u32 = 1;

/// The settings file's root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The schema version the document is written for.
    pub version: u32,
    /// The profile a new tab uses, by name.
    pub default_profile: Option<String>,
    /// The scheme the window paints with, by name.
    pub default_scheme: Option<String>,
    /// The named profiles this installation knows.
    pub profiles: Vec<Profile>,
    /// The chords the window claims before the shell sees them.
    pub keybindings: Vec<Keybinding>,
    /// The colour schemes this installation knows.
    pub schemes: Vec<Scheme>,
    /// The grid's type.
    pub font: Font,
}

impl Default for Settings {
    fn default() -> Self {
        let profile = Profile::default();
        Self {
            version: CURRENT_VERSION,
            default_profile: Some(profile.name.clone()),
            default_scheme: None,
            profiles: vec![profile],
            keybindings: DEFAULT_KEYBINDINGS.to_vec(),
            schemes: Vec::new(),
            font: Font::default(),
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

    /// Returns the scheme the window should paint with, or None for the
    /// built-in palette.
    ///
    /// A name that matches nothing is not replaced by another scheme: a typo
    /// should leave the built-in colours visible, not paint an arbitrary
    /// palette.
    pub fn scheme(&self) -> Option<&Scheme> {
        self.default_scheme
            .as_deref()
            .and_then(|name| self.schemes.iter().find(|scheme| scheme.name == name))
    }

    /// Fills in what a partial file leaves out.
    ///
    /// Called on every parse: a version below 1 is read as 1, an empty list
    /// gets the default profile, a blank command gets the platform shell, a
    /// default that names nothing falls back to the first profile, the user's
    /// chords are applied over the defaults, a scheme with no name is dropped
    /// because nothing can select it, and a font size outside the usable range
    /// is clamped.
    pub(crate) fn normalize(mut self) -> Self {
        self.version = self.version.max(CURRENT_VERSION);

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

        self.keybindings = keybindings::resolve(&self.keybindings);
        self.schemes.retain(|scheme| !scheme.name.trim().is_empty());
        self.font = self.font.normalize();

        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_settings_run_the_platform_shell() {
        let settings = Settings::default();
        assert_eq!(settings.version, CURRENT_VERSION);
        assert_eq!(settings.default_profile.as_deref(), Some("default"));
        assert_eq!(settings.profile().map(|profile| profile.name.as_str()), Some("default"));
        assert!(!settings.profile().expect("profile").command.is_empty());
    }

    #[test]
    fn an_empty_document_gets_the_defaults() {
        let settings = parse("").expect("parse");
        assert_eq!(settings.profiles.len(), 1);
        assert_eq!(settings.profile().map(|profile| profile.name.as_str()), Some("default"));
        assert_eq!(settings.default_profile.as_deref(), Some("default"));
        assert_eq!(settings.version, CURRENT_VERSION);
    }

    #[test]
    fn a_named_profile_is_read_from_toml() {
        let toml = r#"
            default_profile = "zsh"

            [[profiles]]
            name = "zsh"
            command = "/bin/zsh"
            args = ["-l"]
            title = "Z"
        "#;
        let settings = parse(toml).expect("parse");
        let profile = settings.profile().expect("profile");

        assert_eq!(profile.name, "zsh");
        assert_eq!(profile.command, "/bin/zsh");
        assert_eq!(profile.args, vec!["-l".to_string()]);
        assert_eq!(profile.title.as_deref(), Some("Z"));
    }

    #[test]
    fn a_default_that_names_nothing_falls_back_to_the_first_profile() {
        let toml = r#"
            default_profile = "missing"

            [[profiles]]
            name = "one"
            command = "sh"

            [[profiles]]
            name = "two"
            command = "sh"
        "#;
        let settings = parse(toml).expect("parse");
        assert_eq!(settings.default_profile.as_deref(), Some("one"));
        assert_eq!(settings.profile().map(|profile| profile.name.as_str()), Some("one"));
    }

    #[test]
    fn an_empty_list_or_command_is_filled_in() {
        let settings = parse("").expect("parse");
        assert_eq!(settings.profiles.len(), 1);
        assert!(!settings.profile().expect("profile").command.is_empty());

        let settings = parse("[[profiles]]\nname = \"x\"\ncommand = \"   \"\n").expect("parse");
        assert_eq!(settings.profile().expect("profile").command, default_shell());
    }

    #[test]
    fn unknown_fields_and_invalid_toml() {
        let toml = r#"
            extra = true

            [[profiles]]
            name = "x"
            command = "sh"
            colour = 1
        "#;
        let settings = parse(toml).expect("unknown fields are ignored");
        assert_eq!(settings.profile().map(|profile| profile.name.as_str()), Some("x"));

        assert!(parse("[").is_err());
    }

    #[test]
    fn a_missing_version_is_current_and_a_newer_one_is_refused() {
        assert_eq!(parse("").expect("parse").version, CURRENT_VERSION);
        assert_eq!(parse("version = 0").expect("parse").version, CURRENT_VERSION);
        assert_eq!(parse("version = 1").expect("parse").version, CURRENT_VERSION);

        assert!(matches!(
            parse("version = 2"),
            Err(SettingsError::NewerVersion {
                found: 2,
                supported: CURRENT_VERSION
            })
        ));
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let toml = r##"
            default_profile = "zsh"
            default_scheme = "muted"

            [[profiles]]
            name = "zsh"
            command = "/bin/zsh"
            args = ["-l"]
            title = "Z"

            [[keybindings]]
            keys = "ctrl+alt+t"
            action = "newTab"

            [[schemes]]
            name = "muted"
            foreground = "#C0C0C0"
            cursor_color = "#E6E6E6"
            red = "rgb:ff/00/00"

            [font]
            size = 18
        "##;
        let settings = parse(toml).expect("parse");
        let text = to_toml(&settings);
        assert!(text.starts_with("version = 1"), "{text}");

        assert_eq!(parse(&text).expect("parse"), settings);
    }

    #[test]
    fn the_default_settings_bind_the_window_chords() {
        let settings = Settings::default();
        assert_eq!(settings.keybindings, DEFAULT_KEYBINDINGS);
        assert_eq!(settings.keybindings.len(), 17);
    }

    #[test]
    fn a_file_without_keybindings_gets_the_defaults() {
        let settings = parse("[[profiles]]\nname = \"a\"\ncommand = \"sh\"\n").expect("parse");
        assert_eq!(settings.keybindings, DEFAULT_KEYBINDINGS);

        let settings = parse("keybindings = []").expect("parse");
        assert_eq!(settings.keybindings, DEFAULT_KEYBINDINGS);
    }

    #[test]
    fn a_file_applies_its_chords_over_the_defaults() {
        let toml = r#"
            [[keybindings]]
            keys = "ctrl+shift+t"
            action = "paste"

            [[keybindings]]
            keys = "ctrl+alt+t"
            action = "newTab"

            [[keybindings]]
            keys = "ctrl+shift+c"
            action = "unbound"
        "#;
        let settings = parse(toml).expect("parse");

        let action = |chord: &str| {
            let chord = Chord::parse(chord).expect("chord");
            settings
                .keybindings
                .iter()
                .find(|binding| binding.keys == chord)
                .map(|binding| binding.action)
        };

        assert_eq!(action("ctrl+shift+t"), Some(Action::Paste));
        assert_eq!(action("ctrl+alt+t"), Some(Action::NewTab));
        assert_eq!(action("ctrl+shift+c"), None);
        // A default the file does not name is untouched.
        assert_eq!(action("ctrl+shift+d"), Some(Action::SplitHorizontal));
        // One default replaced, one chord added, one removed.
        assert_eq!(settings.keybindings.len(), DEFAULT_KEYBINDINGS.len());
    }

    #[test]
    fn a_broken_chord_or_action_rejects_the_file() {
        assert!(parse("[[keybindings]]\nkeys = \"ctrl+a+b\"\naction = \"newTab\"").is_err());
        assert!(parse("[[keybindings]]\nkeys = \"ctrl+t\"\naction = \"explode\"").is_err());
        assert!(parse("[[keybindings]]\nkeys = \"ctrl+t\"").is_err());
    }

    #[test]
    fn the_default_settings_paint_with_the_built_in_palette() {
        let settings = Settings::default();
        assert!(settings.schemes.is_empty());
        assert_eq!(settings.default_scheme, None);
        assert!(settings.scheme().is_none());
    }

    #[test]
    fn the_default_scheme_is_selected_by_name() {
        let toml = r##"
            default_scheme = "two"

            [[schemes]]
            name = "one"
            background = "#010203"

            [[schemes]]
            name = "two"
            background = "#040506"
        "##;
        let settings = parse(toml).expect("parse");
        let scheme = settings.scheme().expect("scheme");

        assert_eq!(scheme.name, "two");
        assert_eq!(
            scheme.background.map(Color::text_color),
            Some(types::TextColor::rgb(4, 5, 6))
        );
    }

    #[test]
    fn a_default_scheme_that_names_nothing_selects_none() {
        let toml = r##"
            default_scheme = "missing"

            [[schemes]]
            name = "one"
            background = "#010203"
        "##;
        let settings = parse(toml).expect("parse");

        assert_eq!(settings.default_scheme.as_deref(), Some("missing"));
        assert!(settings.scheme().is_none());
    }

    #[test]
    fn a_scheme_with_no_name_is_dropped() {
        let toml = r##"
            [[schemes]]
            background = "#010203"

            [[schemes]]
            name = "  "

            [[schemes]]
            name = "kept"
        "##;
        let settings = parse(toml).expect("parse");

        assert_eq!(settings.schemes.len(), 1);
        assert_eq!(settings.schemes[0].name, "kept");
    }

    #[test]
    fn a_broken_colour_rejects_the_file() {
        assert!(parse("[[schemes]]\nname = \"x\"\nred = \"#12345\"").is_err());
    }

    #[test]
    fn the_font_size_is_read_and_clamped() {
        assert_eq!(Settings::default().font.size, 16.0);
        assert_eq!(parse("").expect("parse").font.size, 16.0);

        // TOML has no float type in a plain `16`, so both forms are accepted.
        assert_eq!(parse("[font]\nsize = 20").expect("parse").font.size, 20.0);
        assert_eq!(parse("[font]\nsize = 20.5").expect("parse").font.size, 20.5);

        let settings = parse("[font]\nsize = 500").expect("parse");
        assert_eq!(settings.font.size, crate::fonts::MAX_SIZE);

        assert!(parse("[font]\nsize = \"large\"").is_err());
    }
}
