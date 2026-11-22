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
mod validation;
mod watch;

pub use actions::Action;
pub use fonts::Font;
pub use keybindings::{Chord, ChordError, DEFAULT_KEYBINDINGS, Key, Keybinding, Modifiers, resolve};
pub use palette::{Color, ColorError, Scheme};
pub use parser::{
    LoadError, Loaded, SaveError, SettingsError, load, load_with_report, parse, parse_with_warnings, save, save_to,
    settings_path, to_toml,
};
pub use profiles::{Profile, default_profiles, default_shell};
pub use validation::Warning;
pub use watch::{Stamp, stamp};

use serde::{Deserialize, Serialize};

/// The schema version this build writes and understands.
///
/// A document that names a higher version is refused: the fields whose meaning
/// changed are exactly the ones an older build would misread.
pub const CURRENT_VERSION: u32 = 1;

/// The settings file's root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default = "empty")]
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

/// What an absent field holds before normalization.
///
/// This is deliberately *not* `Settings::default()`: a missing key must be
/// distinguishable from a key the file wrote, or normalization would report a
/// guess the file never asked for.
fn empty() -> Settings {
    Settings {
        version: CURRENT_VERSION,
        default_profile: None,
        default_scheme: None,
        profiles: Vec::new(),
        keybindings: Vec::new(),
        schemes: Vec::new(),
        font: Font::default(),
    }
}

impl Default for Settings {
    fn default() -> Self {
        let profiles = default_profiles();
        Self {
            version: CURRENT_VERSION,
            default_profile: profiles.first().map(|profile| profile.name.clone()),
            default_scheme: None,
            profiles,
            keybindings: DEFAULT_KEYBINDINGS.to_vec(),
            schemes: Vec::new(),
            font: Font::default(),
        }
    }
}

impl Settings {
    /// Returns the profile a new tab starts by default.
    pub fn profile(&self) -> Option<&Profile> {
        self.default_profile
            .as_deref()
            .and_then(|name| self.profile_named(name))
            .or_else(|| self.profiles.first())
    }

    /// Returns the profile `name` selects, if the settings hold one.
    ///
    /// This is how the window honors a name that did not come from
    /// `default_profile` — a menu entry, in particular. An unknown name finds
    /// nothing rather than another profile.
    pub fn profile_named(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.name == name)
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
    /// gets the default profiles, a blank command gets the platform shell, a
    /// default that names nothing falls back to the first profile, the user's
    /// chords are applied over the defaults, a scheme with no name is dropped
    /// because nothing can select it, and a font size outside the usable range
    /// is clamped.
    ///
    /// Returns what it changed so the window can show it; a warning never stops
    /// a file from loading.
    pub(crate) fn normalize(mut self) -> (Self, Vec<Warning>) {
        let mut warnings = Vec::new();

        self.version = self.version.max(CURRENT_VERSION);

        if self.profiles.is_empty() {
            self.profiles = default_profiles();
        }

        for (index, profile) in self.profiles.iter_mut().enumerate() {
            if profile.name.trim().is_empty() {
                warnings.push(Warning::NamelessProfile { index });
                profile.name = "default".to_string();
            }
            if profile.command.trim().is_empty() {
                let shell = profiles::default_shell();
                warnings.push(Warning::MissingCommand {
                    profile: profile.name.clone(),
                    shell: shell.clone(),
                });
                profile.command = shell;
            }
        }

        for (index, profile) in self.profiles.iter().enumerate() {
            if self.profiles[..index]
                .iter()
                .any(|earlier| earlier.name == profile.name)
            {
                warnings.push(Warning::DuplicateProfileName {
                    name: profile.name.clone(),
                });
            }
        }

        let known = self
            .default_profile
            .as_deref()
            .is_some_and(|name| self.profiles.iter().any(|profile| profile.name == name));
        if !known {
            let used = self.profiles[0].name.clone();
            if let Some(name) = self.default_profile.take() {
                warnings.push(Warning::UnknownDefaultProfile {
                    name,
                    used: used.clone(),
                });
            }
            self.default_profile = Some(used);
        }

        let (keybindings, chord_warnings) = keybindings::resolve_with_warnings(&self.keybindings);
        self.keybindings = keybindings;
        warnings.extend(chord_warnings);

        let mut schemes = Vec::with_capacity(self.schemes.len());
        for (index, scheme) in std::mem::take(&mut self.schemes).into_iter().enumerate() {
            if scheme.name.trim().is_empty() {
                warnings.push(Warning::NamelessScheme { index });
            } else {
                schemes.push(scheme);
            }
        }
        for (index, scheme) in schemes.iter().enumerate() {
            if schemes[..index].iter().any(|earlier| earlier.name == scheme.name) {
                warnings.push(Warning::DuplicateSchemeName {
                    name: scheme.name.clone(),
                });
            }
        }
        self.schemes = schemes;

        if let Some(name) = &self.default_scheme
            && !self.schemes.iter().any(|scheme| scheme.name == *name)
        {
            warnings.push(Warning::UnknownDefaultScheme { name: name.clone() });
        }

        let written = self.font.size;
        self.font = self.font.normalize();
        if self.font.size != written {
            warnings.push(Warning::ClampedFontSize {
                written,
                used: self.font.size,
            });
        }

        (self, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_settings_run_the_platform_shell() {
        let settings = Settings::default();
        let first = settings.profiles.first().expect("a default profile");

        assert_eq!(settings.version, CURRENT_VERSION);
        assert_eq!(settings.default_profile.as_deref(), Some(first.name.as_str()));
        assert_eq!(
            settings.profile().map(|profile| profile.name.as_str()),
            Some(first.name.as_str())
        );
        assert!(!settings.profile().expect("profile").command.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn a_fresh_installation_offers_powershell_cmd_and_bash() {
        let settings = Settings::default();
        let names: Vec<&str> = settings.profiles.iter().map(|profile| profile.name.as_str()).collect();

        assert_eq!(names, ["PowerShell", "Command Prompt", "Bash"]);
        assert_eq!(settings.default_profile.as_deref(), Some("PowerShell"));
        assert_eq!(settings.profiles[0].command, default_shell());
        assert!(
            settings.profiles[1].command.to_ascii_lowercase().ends_with("cmd.exe"),
            "{}",
            settings.profiles[1].command
        );
        assert_eq!(settings.profiles[2].command, "bash.exe");
    }

    #[cfg(windows)]
    #[test]
    fn a_file_can_select_a_default_profile_by_name() {
        let settings = parse("default_profile = \"Command Prompt\"").expect("parse");
        assert_eq!(
            settings.profile().map(|profile| profile.name.as_str()),
            Some("Command Prompt")
        );
    }

    #[test]
    fn an_empty_document_gets_the_defaults() {
        let settings = parse("").expect("parse");
        let defaults = Settings::default();

        assert_eq!(settings.profiles, defaults.profiles);
        assert_eq!(settings.default_profile, defaults.default_profile);
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
    fn a_profile_is_found_by_name() {
        let settings = parse(
            "[[profiles]]\nname = \"one\"\ncommand = \"sh\"\n\n[[profiles]]\nname = \"two\"\ncommand = \"zsh\"\n",
        )
        .expect("parse");

        assert_eq!(
            settings.profile_named("two").map(|profile| profile.command.as_str()),
            Some("zsh")
        );
        assert!(settings.profile_named("three").is_none());
    }

    #[test]
    fn an_empty_list_or_command_is_filled_in() {
        let settings = parse("").expect("parse");
        assert_eq!(settings.profiles.len(), Settings::default().profiles.len());
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
    fn a_clean_document_has_no_warnings() {
        let (_, warnings) = parse_with_warnings("[[profiles]]\nname = \"a\"\ncommand = \"sh\"\n").expect("parse");
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn what_normalization_changed_is_reported() {
        let toml = r##"
            default_profile = "missing"
            default_scheme = "missing"

            [[profiles]]
            command = "sh"

            [[profiles]]
            name = "one"
            command = "   "

            [[profiles]]
            name = "one"
            command = "sh"

            [[keybindings]]
            keys = "ctrl+alt+t"
            action = "newTab"

            [[keybindings]]
            keys = "ctrl+alt+t"
            action = "copy"

            [[keybindings]]
            keys = "ctrl+alt+q"
            action = "unbound"

            [[schemes]]
            background = "#010203"

            [[schemes]]
            name = "s"

            [[schemes]]
            name = "s"

            [font]
            size = 500
        "##;
        let (settings, warnings) = parse_with_warnings(toml).expect("parse");

        assert!(warnings.contains(&Warning::NamelessProfile { index: 0 }));
        assert!(warnings.contains(&Warning::MissingCommand {
            profile: "one".to_string(),
            shell: default_shell(),
        }));
        assert!(warnings.contains(&Warning::DuplicateProfileName {
            name: "one".to_string(),
        }));
        assert!(warnings.contains(&Warning::UnknownDefaultProfile {
            name: "missing".to_string(),
            used: "default".to_string(),
        }));
        assert!(warnings.contains(&Warning::DuplicateChord {
            chord: Chord::parse("ctrl+alt+t").expect("chord"),
        }));
        assert!(warnings.contains(&Warning::UnboundMissingChord {
            chord: Chord::parse("ctrl+alt+q").expect("chord"),
        }));
        assert!(warnings.contains(&Warning::NamelessScheme { index: 0 }));
        assert!(warnings.contains(&Warning::DuplicateSchemeName { name: "s".to_string() }));
        assert!(warnings.contains(&Warning::UnknownDefaultScheme {
            name: "missing".to_string(),
        }));
        assert!(warnings.contains(&Warning::ClampedFontSize {
            written: 500.0,
            used: crate::fonts::MAX_SIZE,
        }));

        // The file still works.
        assert_eq!(settings.profile().map(|profile| profile.name.as_str()), Some("default"));
        assert_eq!(settings.font.size, crate::fonts::MAX_SIZE);
        assert_eq!(settings.keybindings.len(), DEFAULT_KEYBINDINGS.len() + 1);
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
