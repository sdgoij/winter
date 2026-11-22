//! What normalization changed, and what the file asked for that does not exist.
//!
//! The parser never fails for a file that is merely odd: ADR 022's rule is that
//! a partial file is usable. The guesses that rule requires are collected here
//! instead of being silent, so the settings window can say them out loud
//! (ADR 027). A warning is never a reason to reject a file or stop the shell.

use std::fmt;

use crate::Chord;

/// One thing `normalize` had to decide for the file.
#[derive(Debug, Clone, PartialEq)]
pub enum Warning {
    /// A profile had no name, so it is called `default`.
    NamelessProfile {
        /// The profile's position in the list, from zero.
        index: usize,
    },
    /// A profile had no command, so the platform shell runs instead.
    MissingCommand {
        /// The profile's name after normalization.
        profile: String,
        /// The shell that is used instead.
        shell: String,
    },
    /// `default_profile` names no profile, so the first one is used.
    UnknownDefaultProfile {
        /// The name the file wrote.
        name: String,
        /// The profile that is used instead.
        used: String,
    },
    /// A scheme had no name, so nothing can select it and it is dropped.
    NamelessScheme {
        /// The scheme's position in the list, from zero.
        index: usize,
    },
    /// `default_scheme` names no scheme, so the built-in palette paints.
    UnknownDefaultScheme {
        /// The name the file wrote.
        name: String,
    },
    /// Two profiles share a name; the first is the one selected.
    DuplicateProfileName {
        /// The repeated name.
        name: String,
    },
    /// Two schemes share a name; the first is the one selected.
    DuplicateSchemeName {
        /// The repeated name.
        name: String,
    },
    /// A theme had no name, so nothing can select it and it is dropped.
    NamelessTheme {
        /// The theme's position in the list, from zero.
        index: usize,
    },
    /// `theme` names no theme, so the shell's own colours paint.
    UnknownTheme {
        /// The name the file wrote.
        name: String,
    },
    /// Two themes share a name; the first is the one selected.
    DuplicateThemeName {
        /// The repeated name.
        name: String,
    },
    /// The font size was outside the range the shell honours.
    ClampedFontSize {
        /// The size the file wrote.
        written: f32,
        /// The size that is used.
        used: f32,
    },
    /// Two bindings name the same chord; the later one wins.
    DuplicateChord {
        /// The repeated chord.
        chord: Chord,
    },
    /// `unbound` names a chord that was not bound.
    UnboundMissingChord {
        /// The chord that was not bound.
        chord: Chord,
    },
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NamelessProfile { index } => {
                write!(f, "profile {index} has no name; it is called \"default\"")
            }
            Self::MissingCommand { profile, shell } => {
                write!(f, "profile \"{profile}\" has no command; {shell} runs instead")
            }
            Self::UnknownDefaultProfile { name, used } => write!(
                f,
                "default_profile names \"{name}\", which is not a profile; \"{used}\" is used"
            ),
            Self::NamelessScheme { index } => {
                write!(f, "scheme {index} has no name and is ignored")
            }
            Self::UnknownDefaultScheme { name } => write!(
                f,
                "default_scheme names \"{name}\", which is not a scheme; the built-in palette paints"
            ),
            Self::DuplicateProfileName { name } => {
                write!(f, "more than one profile is named \"{name}\"; the first is used")
            }
            Self::DuplicateSchemeName { name } => {
                write!(f, "more than one scheme is named \"{name}\"; the first is used")
            }
            Self::NamelessTheme { index } => {
                write!(f, "theme {index} has no name and is ignored")
            }
            Self::UnknownTheme { name } => write!(
                f,
                "theme names \"{name}\", which is not a theme; the shell's own colours paint"
            ),
            Self::DuplicateThemeName { name } => {
                write!(f, "more than one theme is named \"{name}\"; the first is used")
            }
            Self::ClampedFontSize { written, used } => {
                write!(
                    f,
                    "font size {written} is outside the range the shell honours; {used} is used"
                )
            }
            Self::DuplicateChord { chord } => {
                write!(f, "the chord {chord} is bound more than once; the last binding wins")
            }
            Self::UnboundMissingChord { chord } => {
                write!(f, "the chord {chord} is unbound but was not bound")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_warning_names_what_the_file_wrote_and_what_is_used() {
        assert_eq!(
            Warning::UnknownDefaultProfile {
                name: "missing".to_string(),
                used: "one".to_string(),
            }
            .to_string(),
            "default_profile names \"missing\", which is not a profile; \"one\" is used"
        );
        assert_eq!(
            Warning::ClampedFontSize {
                written: 500.0,
                used: 100.0
            }
            .to_string(),
            "font size 500 is outside the range the shell honours; 100 is used"
        );
        assert_eq!(
            Warning::UnboundMissingChord {
                chord: Chord::parse("ctrl+t").expect("chord")
            }
            .to_string(),
            "the chord ctrl+t is unbound but was not bound"
        );
    }
}
