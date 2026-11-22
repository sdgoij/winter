//! The shell's input vocabulary, independent of a window toolkit.
//!
//! ADR 009 says input translation takes portable events and a mode snapshot.
//! This module is that portable event: `terminal_widget` translates egui's
//! events into it today, and a native shell translates its own into it
//! (ADR 040). The translation to `term` lives here so the rules — printable keys
//! arrive twice, modifier sides, the AltGr chord — are written once.

use term::input::{Key, KeyEvent, Modifiers as KeyModifiers, MouseButtons};

/// One event a window toolkit produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Text the toolkit composed, including IME output.
    Text(String),
    /// A paste the toolkit recognised.
    Paste(String),
    /// A key, with the toolkit's collapsed modifier state.
    Key {
        /// The key's identity.
        key: Key,
        /// Whether this is a press rather than a release.
        pressed: bool,
        /// Whether this is an auto-repeat rather than the first press.
        repeat: bool,
        /// The modifier state, without the side that was pressed.
        modifiers: Modifiers,
    },
}

/// The modifier state a toolkit reports.
///
/// Both egui and WinUI report that *a* Ctrl or Alt is held, not which one. The
/// side is tracked from the key events themselves, which is what `term` needs
/// for AltGr.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Shift is held.
    pub shift: bool,
    /// Ctrl is held.
    pub ctrl: bool,
    /// Alt is held.
    pub alt: bool,
    /// The platform's command key is held: Super on Windows and Linux, Command
    /// on macOS, where it suppresses the text event a printable key produces.
    pub command: bool,
}

/// The modifier sides a toolkit collapses but the encoder distinguishes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModifierSides {
    /// The right Alt key is down.
    pub right_alt: bool,
    /// The right Ctrl key is down.
    pub right_ctrl: bool,
}

/// What one event asks the pane to send.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The key events to send, in order.
    pub keys: Vec<KeyEvent>,
    /// Text to paste through the terminal's bracketed-paste filter.
    pub paste: Option<String>,
    /// Whether the event is typing or a press, which returns the view to the
    /// live screen.
    pub activity: bool,
}

/// Turns one portable event into the key events and paste it means.
pub fn translate(event: &Event, sides: &mut ModifierSides) -> Outcome {
    match event {
        Event::Text(text) => Outcome {
            keys: text.chars().map(text_key).collect(),
            paste: None,
            activity: true,
        },
        Event::Paste(text) => Outcome {
            keys: Vec::new(),
            paste: Some(text.clone()),
            activity: false,
        },
        Event::Key {
            key,
            pressed,
            repeat,
            modifiers,
        } => {
            track_modifier(sides, *key, *pressed);

            // A printable key arrives again as text, so sending the key event
            // as well would type it twice. Ctrl, Alt and Command suppress the
            // text event, so those chords go here.
            let printable = matches!(key, Key::Char(_) | Key::Space);
            let suppressed = printable && !modifiers.ctrl && !modifiers.alt && !modifiers.command;

            Outcome {
                keys: if suppressed {
                    Vec::new()
                } else {
                    vec![KeyEvent {
                        key: *key,
                        modifiers: key_modifiers(*modifiers, sides.right_alt, sides.right_ctrl),
                        pressed: *pressed,
                        repeat: *repeat,
                        ..KeyEvent::down(*key)
                    }]
                },
                paste: None,
                activity: *pressed,
            }
        }
    }
}

/// The key event a character produces: the parser's text path.
fn text_key(character: char) -> KeyEvent {
    KeyEvent {
        key: Key::Unidentified,
        codepoint: Some(character),
        ..KeyEvent::down(Key::Unidentified)
    }
}

/// Records which side of a modifier key an event touched.
pub fn track_modifier(sides: &mut ModifierSides, key: Key, pressed: bool) {
    match key {
        Key::RightAlt => sides.right_alt = pressed,
        Key::RightControl => sides.right_ctrl = pressed,
        _ => {}
    }
}

/// Composes the terminal's modifier state from the toolkit's collapsed state
/// and the sides the shell has tracked.
pub fn key_modifiers(modifiers: Modifiers, right_alt: bool, right_ctrl: bool) -> KeyModifiers {
    KeyModifiers {
        shift: modifiers.shift,
        left_ctrl: modifiers.ctrl && !right_ctrl,
        right_ctrl: modifiers.ctrl && right_ctrl,
        left_alt: modifiers.alt && !right_alt,
        right_alt: modifiers.alt && right_alt,
        caps_lock: false,
        num_lock: false,
        enhanced: false,
    }
}

/// Maps a screen row to a document row for the current scroll.
///
/// Document row zero is the oldest row still held; the screen's first row is
/// `history - scroll`.
pub const fn screen_to_document(history: i32, scroll: i32, screen_row: i32) -> i32 {
    history - scroll + screen_row
}

/// Returns whether the application, rather than the selection, gets the mouse.
///
/// Shift is the user's override: it selects even while an application tracks
/// the mouse, which is how a person copies text out of a full-screen program.
pub const fn forwards_to_application(tracking: bool, shift: bool) -> bool {
    tracking && !shift
}

/// Maps the pointer's buttons to the protocol's set of held buttons.
pub const fn mouse_buttons(primary: bool, middle: bool, secondary: bool) -> MouseButtons {
    MouseButtons {
        left: primary,
        middle,
        right: secondary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn text_becomes_one_key_event_per_character() {
        let mut sides = ModifierSides::default();
        let outcome = translate(&Event::Text("ab".to_string()), &mut sides);

        assert!(outcome.activity);
        assert_eq!(outcome.keys.len(), 2);
        assert_eq!(outcome.keys[0].codepoint, Some('a'));
        assert_eq!(outcome.keys[1].codepoint, Some('b'));
        assert_eq!(outcome.keys[0].key, Key::Unidentified);
    }

    #[test]
    fn a_printable_key_without_a_chord_is_left_to_the_text_event() {
        let mut sides = ModifierSides::default();
        let outcome = translate(&key(Key::Char('q'), Modifiers::default()), &mut sides);

        // Nothing is sent, but the press still returns the view to the live
        // screen.
        assert!(outcome.keys.is_empty());
        assert!(outcome.activity);
    }

    #[test]
    fn a_control_chord_carries_the_key() {
        let mut sides = ModifierSides::default();
        let outcome = translate(
            &key(
                Key::Char('q'),
                Modifiers {
                    ctrl: true,
                    ..Modifiers::default()
                },
            ),
            &mut sides,
        );

        assert_eq!(outcome.keys.len(), 1);
        assert_eq!(outcome.keys[0].key, Key::Char('q'));
        assert!(outcome.keys[0].modifiers.left_ctrl);
    }

    #[test]
    fn a_release_is_not_activity() {
        let mut sides = ModifierSides::default();
        let outcome = translate(
            &Event::Key {
                key: Key::Left,
                pressed: false,
                repeat: false,
                modifiers: Modifiers::default(),
            },
            &mut sides,
        );

        assert_eq!(outcome.keys.len(), 1);
        assert!(!outcome.keys[0].pressed);
        assert!(!outcome.activity);
    }

    #[test]
    fn the_right_alt_makes_an_alt_gr_chord() {
        let mut sides = ModifierSides::default();
        translate(&key(Key::RightAlt, Modifiers::default()), &mut sides);

        let chord = translate(
            &key(
                Key::Char('e'),
                Modifiers {
                    alt: true,
                    ctrl: true,
                    ..Modifiers::default()
                },
            ),
            &mut sides,
        );

        assert!(chord.keys[0].modifiers.is_alt_gr());
    }

    #[test]
    fn a_paste_is_reported_as_paste_not_keys() {
        let mut sides = ModifierSides::default();
        let outcome = translate(&Event::Paste("pasted".to_string()), &mut sides);

        assert_eq!(outcome.paste.as_deref(), Some("pasted"));
        assert!(outcome.keys.is_empty());
        assert!(!outcome.activity);
    }

    #[test]
    fn the_modifier_sides_come_from_the_tracked_keys() {
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::default()
        };

        let left = key_modifiers(ctrl, false, false);
        assert!(left.left_ctrl);
        assert!(!left.right_ctrl);

        let right = key_modifiers(ctrl, false, true);
        assert!(!right.left_ctrl);
        assert!(right.right_ctrl);
    }

    #[test]
    fn a_right_alt_chord_is_distinguishable_from_left_alt() {
        let chord = Modifiers {
            alt: true,
            ctrl: true,
            ..Modifiers::default()
        };

        assert!(key_modifiers(chord, true, false).is_alt_gr());
        assert!(!key_modifiers(chord, false, false).is_alt_gr());
    }

    #[test]
    fn a_screen_row_maps_into_the_document() {
        assert_eq!(screen_to_document(5, 2, 0), 3);
        assert_eq!(screen_to_document(5, 0, 4), 9);
        assert_eq!(screen_to_document(0, 0, 0), 0);
    }

    #[test]
    fn shift_takes_the_mouse_back_from_a_tracking_application() {
        assert!(forwards_to_application(true, false));
        assert!(!forwards_to_application(true, true));
        assert!(!forwards_to_application(false, false));
    }

    #[test]
    fn the_pointer_buttons_map_to_the_protocol_buttons() {
        assert_eq!(
            mouse_buttons(true, false, false),
            MouseButtons {
                left: true,
                middle: false,
                right: false,
            }
        );
        assert_eq!(
            mouse_buttons(false, true, false),
            MouseButtons {
                left: false,
                middle: true,
                right: false,
            }
        );
        assert_eq!(
            mouse_buttons(false, false, true),
            MouseButtons {
                left: false,
                middle: false,
                right: true,
            }
        );
    }
}
