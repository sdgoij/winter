//! Key chords and the bindings that use them.
//!
//! The syntax is upstream's `KeyChordSerialization.cpp`: a chord is
//! `[ctrl+][alt+][shift+]<key>`, case-insensitive, in any modifier order, and a
//! second key part is a mistake. Two parts of upstream's grammar are absent
//! because the shell cannot see them: `vk()`/`sc()` escapes, and the numpad,
//! browser and `win` keys. The vocabulary is what an egui key event reports
//! (ADR 023).

use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::actions::Action;
use crate::validation::Warning;

/// The modifiers a chord carries.
///
/// Windows' `win` modifier is not here: the window cannot observe it, so a
/// chord that named it would never match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    /// The Control key.
    pub ctrl: bool,
    /// The Shift key.
    pub shift: bool,
    /// The Alt key.
    pub alt: bool,
}

/// A key a chord can name.
///
/// A printable key keeps the character an event reports, with letters in
/// lowercase so `Ctrl+T` and `ctrl+t` are one chord. The rest are the keys
/// whose names are not a single character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable key: a letter, digit or punctuation mark.
    Char(char),
    /// Tab.
    Tab,
    /// Return.
    Enter,
    /// Escape.
    Escape,
    /// Space.
    Space,
    /// Backspace.
    Backspace,
    /// Insert.
    Insert,
    /// Delete.
    Delete,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// A function key, 1 through 24.
    Function(u8),
}

/// One key chord: a key and the modifiers that must be held with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    /// The modifiers the chord requires. A matching event must have exactly
    /// these.
    pub modifiers: Modifiers,
    /// The key the chord names.
    pub key: Key,
}

/// A chord string that names no chord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChordError(String);

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ChordError {}

impl Chord {
    /// Parses `[ctrl+][alt+][shift+]<key>`.
    ///
    /// Parts may be spaced and in any order; a repeated modifier is the same
    /// flag set twice, as upstream's flag OR is.
    pub fn parse(text: &str) -> Result<Self, ChordError> {
        let mut modifiers = Modifiers::default();
        let mut key = None;

        for part in text.split('+') {
            let part = part.trim();
            if part.is_empty() {
                return Err(ChordError(format!("`{text}` has an empty part")));
            }

            if part.eq_ignore_ascii_case("ctrl") {
                modifiers.ctrl = true;
            } else if part.eq_ignore_ascii_case("shift") {
                modifiers.shift = true;
            } else if part.eq_ignore_ascii_case("alt") {
                modifiers.alt = true;
            } else if key.is_some() {
                return Err(ChordError(format!("`{text}` names more than one key")));
            } else if let Some(named) = parse_key(part) {
                key = Some(named);
            } else {
                return Err(ChordError(format!("`{text}` names no key `{part}`")));
            }
        }

        key.map(|key| Self { modifiers, key })
            .ok_or_else(|| ChordError(format!("`{text}` names no key")))
    }

    /// Whether an event with `key` and `modifiers` is this chord.
    ///
    /// The modifiers must match exactly: `Ctrl+Shift+T` is not the chord
    /// `ctrl+t`.
    pub fn matches(&self, key: Key, modifiers: Modifiers) -> bool {
        self.key == key && self.modifiers == modifiers
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.ctrl {
            f.write_str("ctrl+")?;
        }
        if self.modifiers.alt {
            f.write_str("alt+")?;
        }
        if self.modifiers.shift {
            f.write_str("shift+")?;
        }
        write_key(f, self.key)
    }
}

impl Serialize for Chord {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(D::Error::custom)
    }
}

/// A chord and the action it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keybinding {
    /// The chord that triggers the action.
    pub keys: Chord,
    /// What the chord asks the window to do.
    pub action: Action,
}

/// The chords a fresh installation binds.
pub const DEFAULT_KEYBINDINGS: &[Keybinding] = &[
    Keybinding {
        keys: chord(Key::Char('t'), true, true, false),
        action: Action::NewTab,
    },
    Keybinding {
        keys: chord(Key::Char('w'), true, true, false),
        action: Action::CloseTab,
    },
    Keybinding {
        keys: chord(Key::Char('d'), true, true, false),
        action: Action::SplitHorizontal,
    },
    Keybinding {
        keys: chord(Key::Char('e'), true, true, false),
        action: Action::SplitVertical,
    },
    Keybinding {
        keys: chord(Key::Tab, true, false, false),
        action: Action::NextTab,
    },
    Keybinding {
        keys: chord(Key::Tab, true, true, false),
        action: Action::PrevTab,
    },
    Keybinding {
        keys: chord(Key::Char('c'), true, true, false),
        action: Action::Copy,
    },
    Keybinding {
        keys: chord(Key::Char('v'), true, true, false),
        action: Action::Paste,
    },
    Keybinding {
        keys: chord(Key::Up, true, true, false),
        action: Action::ScrollUp,
    },
    Keybinding {
        keys: chord(Key::Down, true, true, false),
        action: Action::ScrollDown,
    },
    // Shift+PgUp/PgDn is what the console host has always scrolled with.
    Keybinding {
        keys: chord(Key::PageUp, false, true, false),
        action: Action::ScrollUpPage,
    },
    Keybinding {
        keys: chord(Key::PageDown, false, true, false),
        action: Action::ScrollDownPage,
    },
    Keybinding {
        keys: chord(Key::PageUp, true, true, false),
        action: Action::ScrollUpPage,
    },
    Keybinding {
        keys: chord(Key::PageDown, true, true, false),
        action: Action::ScrollDownPage,
    },
    Keybinding {
        keys: chord(Key::Home, true, true, false),
        action: Action::ScrollToTop,
    },
    Keybinding {
        keys: chord(Key::End, true, true, false),
        action: Action::ScrollToBottom,
    },
    Keybinding {
        keys: chord(Key::Char(','), true, false, false),
        action: Action::OpenSettings,
    },
    Keybinding {
        keys: chord(Key::Char('p'), true, true, false),
        action: Action::CommandPalette,
    },
    Keybinding {
        keys: chord(Key::Char('f'), true, true, false),
        action: Action::Find,
    },
    Keybinding {
        keys: chord(Key::Left, false, false, true),
        action: Action::FocusLeft,
    },
    Keybinding {
        keys: chord(Key::Right, false, false, true),
        action: Action::FocusRight,
    },
    Keybinding {
        keys: chord(Key::Up, false, false, true),
        action: Action::FocusUp,
    },
    Keybinding {
        keys: chord(Key::Down, false, false, true),
        action: Action::FocusDown,
    },
    Keybinding {
        keys: chord(Key::Left, false, true, true),
        action: Action::ResizeLeft,
    },
    Keybinding {
        keys: chord(Key::Right, false, true, true),
        action: Action::ResizeRight,
    },
    Keybinding {
        keys: chord(Key::Up, false, true, true),
        action: Action::ResizeUp,
    },
    Keybinding {
        keys: chord(Key::Down, false, true, true),
        action: Action::ResizeDown,
    },
];

const fn chord(key: Key, ctrl: bool, shift: bool, alt: bool) -> Chord {
    Chord {
        modifiers: Modifiers { ctrl, shift, alt },
        key,
    }
}

/// Applies the user's bindings over the defaults.
///
/// A user chord replaces a default with the same chord and a new chord is
/// appended; a repeated chord in the user's list keeps the last, because the
/// earlier one is removed first. `unbound` removes a chord and is not itself
/// kept.
pub fn resolve(user: &[Keybinding]) -> Vec<Keybinding> {
    resolve_with_warnings(user).0
}

/// Applies the user's bindings and reports what was odd about them.
///
/// A duplicate chord and an `unbound` that names nothing bound are not errors:
/// the merge has a defined answer for both, and a file that is merely confused
/// still starts the shell (ADR 022). The warnings are what the window shows.
pub fn resolve_with_warnings(user: &[Keybinding]) -> (Vec<Keybinding>, Vec<Warning>) {
    let mut resolved = DEFAULT_KEYBINDINGS.to_vec();
    let mut warnings = Vec::new();

    for (index, binding) in user.iter().enumerate() {
        if user[..index].iter().any(|earlier| earlier.keys == binding.keys) {
            warnings.push(Warning::DuplicateChord { chord: binding.keys });
        }

        let was_bound = resolved.iter().any(|existing| existing.keys == binding.keys);
        resolved.retain(|existing| existing.keys != binding.keys);

        if binding.action != Action::Unbound {
            resolved.push(binding.clone());
        } else if !was_bound {
            warnings.push(Warning::UnboundMissingChord { chord: binding.keys });
        }
    }

    (resolved, warnings)
}

fn parse_key(part: &str) -> Option<Key> {
    let lower = part.to_ascii_lowercase();

    let key = match lower.as_str() {
        "tab" => Key::Tab,
        "enter" => Key::Enter,
        "escape" | "esc" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "insert" => Key::Insert,
        "delete" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "plus" => Key::Char('+'),
        "comma" => Key::Char(','),
        "minus" => Key::Char('-'),
        "period" => Key::Char('.'),
        _ => {
            if let Some(number) = lower.strip_prefix('f').and_then(|digits| digits.parse::<u8>().ok())
                && (1..=24).contains(&number)
            {
                return Some(Key::Function(number));
            }

            // A single printable character names itself; letters compare in
            // lowercase, as upstream's virtual keys do.
            let mut characters = part.chars();
            return match (characters.next(), characters.next()) {
                (Some(character), None) if character.is_ascii_graphic() => {
                    Some(Key::Char(character.to_ascii_lowercase()))
                }
                _ => None,
            };
        }
    };

    Some(key)
}

fn write_key(f: &mut fmt::Formatter<'_>, key: Key) -> fmt::Result {
    match key {
        Key::Char(character) => write!(f, "{character}"),
        Key::Tab => f.write_str("tab"),
        Key::Enter => f.write_str("enter"),
        Key::Escape => f.write_str("esc"),
        Key::Space => f.write_str("space"),
        Key::Backspace => f.write_str("backspace"),
        Key::Insert => f.write_str("insert"),
        Key::Delete => f.write_str("delete"),
        Key::Home => f.write_str("home"),
        Key::End => f.write_str("end"),
        Key::PageUp => f.write_str("pgup"),
        Key::PageDown => f.write_str("pgdn"),
        Key::Left => f.write_str("left"),
        Key::Right => f.write_str("right"),
        Key::Up => f.write_str("up"),
        Key::Down => f.write_str("down"),
        Key::Function(number) => write!(f, "f{number}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_is_case_and_order_insensitive() {
        let expected = Chord::parse("ctrl+shift+t").expect("chord");

        assert_eq!(Chord::parse("Ctrl+Shift+T").expect("chord"), expected);
        assert_eq!(Chord::parse("shift+ctrl+t").expect("chord"), expected);
        assert_eq!(Chord::parse("  ctrl + shift + t  ").expect("chord"), expected);
        assert_eq!(
            Chord::parse("ctrl+ctrl+t").expect("chord"),
            Chord::parse("ctrl+t").expect("chord")
        );
        assert_eq!(Chord::parse("ESC").expect("chord"), Chord::parse("esc").expect("chord"));
        assert_eq!(Chord::parse("F12").expect("chord").key, Key::Function(12));
    }

    #[test]
    fn a_chord_rejects_two_keys_and_unknown_parts() {
        assert!(Chord::parse("ctrl+a+b").is_err());
        assert!(Chord::parse("tab+enter").is_err());
        assert!(Chord::parse("ctrl+hyper+t").is_err());
        assert!(Chord::parse("ctrl++t").is_err());
        assert!(Chord::parse("ctrl+").is_err());
        assert!(Chord::parse("").is_err());
        assert!(Chord::parse("ctrl").is_err());
    }

    #[test]
    fn the_modifiers_must_match_exactly() {
        let chord = Chord::parse("alt+left").expect("chord");

        assert!(chord.matches(
            Key::Left,
            Modifiers {
                ctrl: false,
                shift: false,
                alt: true
            }
        ));
        assert!(!chord.matches(
            Key::Left,
            Modifiers {
                ctrl: true,
                shift: false,
                alt: true
            }
        ));
        assert!(!chord.matches(
            Key::Left,
            Modifiers {
                ctrl: false,
                shift: true,
                alt: true
            }
        ));
        assert!(!chord.matches(Key::Left, Modifiers::default()));
        assert!(!chord.matches(
            Key::Right,
            Modifiers {
                ctrl: false,
                shift: false,
                alt: true
            }
        ));
    }

    #[test]
    fn every_default_binding_round_trips_through_its_string() {
        for binding in DEFAULT_KEYBINDINGS {
            let text = binding.keys.to_string();
            assert_eq!(Chord::parse(&text).expect(&text), binding.keys, "{text}");
        }
    }

    #[test]
    fn a_chord_serialises_as_its_string_and_parses_back() {
        let text = "keys = \"ctrl+shift+t\"\naction = \"newTab\"\n";
        let binding: Keybinding = toml::from_str(text).expect("binding");

        assert_eq!(binding.keys, Chord::parse("ctrl+shift+t").expect("chord"));
        assert_eq!(binding.keys.to_string(), "ctrl+shift+t");
        assert_eq!(binding.action, Action::NewTab);

        let written = toml::to_string(&binding).expect("toml");
        assert_eq!(toml::from_str::<Keybinding>(&written).expect("binding"), binding);
    }

    #[test]
    fn merging_reports_duplicate_and_missing_chords() {
        let repeated = Chord::parse("ctrl+alt+t").expect("chord");
        let never_bound = Chord::parse("ctrl+alt+q").expect("chord");
        let user = [
            Keybinding {
                keys: repeated,
                action: Action::NewTab,
            },
            Keybinding {
                keys: repeated,
                action: Action::Copy,
            },
            Keybinding {
                keys: never_bound,
                action: Action::Unbound,
            },
        ];

        let (resolved, warnings) = resolve_with_warnings(&user);

        assert!(warnings.contains(&Warning::DuplicateChord { chord: repeated }));
        assert!(warnings.contains(&Warning::UnboundMissingChord { chord: never_bound }));
        // The duplicate collapses into one binding, and the unbound one adds
        // nothing.
        assert_eq!(resolved.len(), DEFAULT_KEYBINDINGS.len() + 1);
        assert_eq!(
            resolved.last().expect("binding").action,
            Action::Copy,
            "the last duplicate wins"
        );

        // Unbinding a chord that is bound is not a warning.
        let (unbound, quiet) = resolve_with_warnings(&[Keybinding {
            keys: Chord::parse("ctrl+shift+c").expect("chord"),
            action: Action::Unbound,
        }]);
        assert!(quiet.is_empty(), "{quiet:?}");
        assert_eq!(unbound.len(), DEFAULT_KEYBINDINGS.len() - 1);
    }

    #[test]
    fn a_user_chord_replaces_the_default_with_the_same_chord() {
        let user = [Keybinding {
            keys: Chord::parse("ctrl+shift+t").expect("chord"),
            action: Action::Paste,
        }];
        let resolved = resolve(&user);

        assert_eq!(resolved.len(), DEFAULT_KEYBINDINGS.len());
        assert_eq!(
            resolved.iter().filter(|binding| binding.keys == user[0].keys).count(),
            1
        );
        assert_eq!(resolved.last().expect("binding").action, Action::Paste);
    }

    #[test]
    fn a_new_chord_is_appended() {
        let resolved = resolve(&[Keybinding {
            keys: Chord::parse("ctrl+alt+t").expect("chord"),
            action: Action::NewTab,
        }]);

        assert_eq!(resolved.len(), DEFAULT_KEYBINDINGS.len() + 1);
        assert_eq!(resolved.last().expect("binding").action, Action::NewTab);
    }

    #[test]
    fn unbound_removes_a_chord_and_is_not_kept() {
        let resolved = resolve(&[Keybinding {
            keys: Chord::parse("ctrl+shift+c").expect("chord"),
            action: Action::Unbound,
        }]);

        assert_eq!(resolved.len(), DEFAULT_KEYBINDINGS.len() - 1);
        assert!(
            !resolved
                .iter()
                .any(|binding| binding.keys == Chord::parse("ctrl+shift+c").expect("chord"))
        );
        assert!(!resolved.iter().any(|binding| binding.action == Action::Unbound));
    }

    #[test]
    fn a_repeated_user_chord_keeps_the_last_action() {
        let resolved = resolve(&[
            Keybinding {
                keys: Chord::parse("ctrl+alt+t").expect("chord"),
                action: Action::Copy,
            },
            Keybinding {
                keys: Chord::parse("ctrl+alt+t").expect("chord"),
                action: Action::Paste,
            },
        ]);

        let matching: Vec<_> = resolved
            .iter()
            .filter(|binding| binding.keys == Chord::parse("ctrl+alt+t").expect("chord"))
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].action, Action::Paste);
    }
}
