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
        /// The platform's virtual-key code, for win32-input-mode.
        ///
        /// A toolkit that has no Win32 key identity reports zero, which falls
        /// back to the legacy encoding.
        virtual_key: u16,
        /// The platform's scan code, for win32-input-mode.
        scan_code: u16,
        /// The platform's repeat count, for win32-input-mode.
        repeat_count: u16,
        /// The character the key produced, when the toolkit reported it.
        ///
        /// A toolkit delivers a printable key twice: once as a key and once as
        /// text. The shell pairs them so a Win32 input record can carry the
        /// translated character (ADR 056).
        codepoint: Option<char>,
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

/// Returns whether the key's text arrives as a character event.
///
/// A printable key reaches a terminal toolkit twice: as a key and as the
/// character it composed. Ctrl, Alt and the command key suppress the text
/// event, so those chords are keys. When the pty speaks win32-input-mode the
/// character is folded back into the key's record instead (ADR 056).
pub fn produces_text(key: Key, modifiers: Modifiers) -> bool {
    matches!(key, Key::Char(_) | Key::Space) && !modifiers.ctrl && !modifiers.alt && !modifiers.command
}

/// Turns one portable event into the key events and paste it means.
///
/// `win32` says whether the active terminal asked for Win32 input records. A
/// record carries the key-down and key-up separately, so a printable key is
/// sent as a key rather than left to its character event; otherwise conhost
/// synthesises a press and a release together and a held key is never held.
pub fn translate(event: &Event, sides: &mut ModifierSides, win32: bool) -> Outcome {
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
            virtual_key,
            scan_code,
            repeat_count,
            codepoint,
        } => {
            track_modifier(sides, *key, *pressed);

            // A printable key is typed by the character event. This event can
            // carry that character when the shell paired the two; win32 mode
            // needs it in the record, and the legacy encoding sends it as text.
            let text = produces_text(*key, *modifiers);
            let record = win32 && *virtual_key != 0;

            let keys = if text && !record {
                match (codepoint, pressed) {
                    (Some(character), true) => vec![KeyEvent {
                        key: Key::Unidentified,
                        codepoint: Some(*character),
                        ..KeyEvent::down(Key::Unidentified)
                    }],
                    _ => Vec::new(),
                }
            } else {
                vec![KeyEvent {
                    key: *key,
                    codepoint: *codepoint,
                    modifiers: key_modifiers(*modifiers, sides.right_alt, sides.right_ctrl),
                    pressed: *pressed,
                    repeat: *repeat,
                    virtual_key: *virtual_key,
                    scan_code: *scan_code,
                    repeat_count: (*repeat_count).max(1),
                    ..KeyEvent::down(*key)
                }]
            };

            Outcome {
                keys,
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

/// The settings key a terminal key names, for matching a chord.
///
/// Only the keys a window chord can name are mapped; a key with no name here
/// cannot be bound and always reaches the terminal.
pub fn settings_key(key: Key) -> Option<settings::Key> {
    Some(match key {
        Key::Char(character) => settings::Key::Char(character.to_ascii_lowercase()),
        Key::Tab => settings::Key::Tab,
        Key::Enter => settings::Key::Enter,
        Key::Escape => settings::Key::Escape,
        Key::Space => settings::Key::Space,
        Key::Backspace => settings::Key::Backspace,
        Key::Insert => settings::Key::Insert,
        Key::Delete => settings::Key::Delete,
        Key::Home => settings::Key::Home,
        Key::End => settings::Key::End,
        Key::PageUp => settings::Key::PageUp,
        Key::PageDown => settings::Key::PageDown,
        Key::Left => settings::Key::Left,
        Key::Right => settings::Key::Right,
        Key::Up => settings::Key::Up,
        Key::Down => settings::Key::Down,
        Key::Function(number) => settings::Key::Function(number),
        _ => return None,
    })
}

/// The window action `key` and `modifiers` name, if the settings file binds it.
///
/// A chord must match exactly, so `Ctrl+Shift+T` does not answer to `ctrl+t`.
/// A shell that cannot perform the action declines it and lets the key reach
/// the terminal, which is how a copy with nothing selected still interrupts.
pub fn chord_action(bindings: &[settings::Keybinding], key: Key, modifiers: Modifiers) -> Option<settings::Action> {
    let key = settings_key(key)?;
    let chord = settings::Modifiers {
        ctrl: modifiers.ctrl,
        shift: modifiers.shift,
        alt: modifiers.alt,
    };

    bindings
        .iter()
        .find(|binding| binding.keys.matches(key, chord))
        .map(|binding| binding.action)
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
            virtual_key: 0,
            scan_code: 0,
            repeat_count: 1,
            codepoint: None,
        }
    }

    fn record(key: Key, virtual_key: u16, codepoint: Option<char>, pressed: bool) -> Event {
        Event::Key {
            key,
            pressed,
            repeat: false,
            modifiers: Modifiers::default(),
            virtual_key,
            scan_code: 0,
            repeat_count: 1,
            codepoint,
        }
    }

    #[test]
    fn text_becomes_one_key_event_per_character() {
        let mut sides = ModifierSides::default();
        let outcome = translate(&Event::Text("ab".to_string()), &mut sides, false);

        assert!(outcome.activity);
        assert_eq!(outcome.keys.len(), 2);
        assert_eq!(outcome.keys[0].codepoint, Some('a'));
        assert_eq!(outcome.keys[1].codepoint, Some('b'));
        assert_eq!(outcome.keys[0].key, Key::Unidentified);
    }

    #[test]
    fn a_printable_key_without_a_chord_is_left_to_the_text_event() {
        let mut sides = ModifierSides::default();
        let outcome = translate(&key(Key::Char('q'), Modifiers::default()), &mut sides, false);

        // Nothing is sent, but the press still returns the view to the live
        // screen.
        assert!(outcome.keys.is_empty());
        assert!(outcome.activity);
    }

    #[test]
    fn a_paired_character_is_text_without_win32_mode() {
        let mut sides = ModifierSides::default();
        let outcome = translate(&record(Key::Char('q'), 0x51, Some('q'), true), &mut sides, false);

        // The character event's payload, not an empty key event.
        assert_eq!(outcome.keys.len(), 1);
        assert_eq!(outcome.keys[0].key, Key::Unidentified);
        assert_eq!(outcome.keys[0].codepoint, Some('q'));
    }

    #[test]
    fn a_win32_record_carries_the_virtual_key_and_character() {
        let mut sides = ModifierSides::default();
        let outcome = translate(&record(Key::Char('s'), 0x53, Some('s'), true), &mut sides, true);

        assert_eq!(outcome.keys.len(), 1);
        let event = &outcome.keys[0];
        assert_eq!(event.key, Key::Char('s'));
        assert_eq!(event.codepoint, Some('s'));
        assert_eq!(event.virtual_key, 0x53);
        assert!(event.pressed);
    }

    #[test]
    fn a_win32_release_is_sent() {
        let mut sides = ModifierSides::default();
        let outcome = translate(&record(Key::Char('s'), 0x53, None, false), &mut sides, true);

        assert_eq!(outcome.keys.len(), 1);
        assert!(!outcome.keys[0].pressed);
        assert_eq!(outcome.keys[0].virtual_key, 0x53);
        assert!(!outcome.activity);
    }

    #[test]
    fn a_win32_key_without_a_virtual_key_stays_text() {
        let mut sides = ModifierSides::default();
        let outcome = translate(&record(Key::Char('q'), 0, Some('q'), true), &mut sides, true);

        // A portable toolkit has no Win32 key identity; an empty record would
        // type nothing, so the character is sent as text instead.
        assert_eq!(outcome.keys[0].key, Key::Unidentified);
        assert_eq!(outcome.keys[0].codepoint, Some('q'));
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
            false,
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
                virtual_key: 0,
                scan_code: 0,
                repeat_count: 1,
                codepoint: None,
            },
            &mut sides,
            false,
        );

        assert_eq!(outcome.keys.len(), 1);
        assert!(!outcome.keys[0].pressed);
        assert!(!outcome.activity);
    }

    #[test]
    fn the_right_alt_makes_an_alt_gr_chord() {
        let mut sides = ModifierSides::default();
        translate(&key(Key::RightAlt, Modifiers::default()), &mut sides, false);

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
            false,
        );

        assert!(chord.keys[0].modifiers.is_alt_gr());
    }

    #[test]
    fn a_paste_is_reported_as_paste_not_keys() {
        let mut sides = ModifierSides::default();
        let outcome = translate(&Event::Paste("pasted".to_string()), &mut sides, false);

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

    fn chord_modifiers(ctrl: bool, shift: bool) -> Modifiers {
        Modifiers {
            shift,
            ctrl,
            alt: false,
            command: false,
        }
    }

    #[test]
    fn a_bound_chord_names_its_action() {
        let bindings = settings::DEFAULT_KEYBINDINGS;

        assert_eq!(
            chord_action(bindings, Key::Char('t'), chord_modifiers(true, true)),
            Some(settings::Action::NewTab)
        );
        assert_eq!(
            chord_action(bindings, Key::Tab, chord_modifiers(true, false)),
            Some(settings::Action::NextTab)
        );
    }

    #[test]
    fn a_chord_must_match_exactly() {
        let bindings = settings::DEFAULT_KEYBINDINGS;

        // `ctrl+t` alone is the terminal's.
        assert_eq!(
            chord_action(bindings, Key::Char('t'), chord_modifiers(true, false)),
            None
        );
        assert_eq!(
            chord_action(bindings, Key::Char('T'), chord_modifiers(true, true)),
            Some(settings::Action::NewTab)
        );
    }

    #[test]
    fn a_key_the_file_cannot_name_is_never_a_chord() {
        let bindings = settings::DEFAULT_KEYBINDINGS;

        assert_eq!(settings_key(Key::Numpad(3)), None);
        assert_eq!(
            chord_action(bindings, Key::Numpad(3), chord_modifiers(true, true)),
            None
        );
    }
}
