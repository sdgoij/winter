//! Mode 9001 — win32-input-mode records.

use crate::support::Screen;
use term::input::{Key, KeyEvent, Modifiers};

fn key_event(pressed: bool) -> KeyEvent {
    KeyEvent {
        key: Key::Char('a'),
        codepoint: Some('a'),
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
        pressed,
        virtual_key: 0x41,
        scan_code: 0x1E,
        ..KeyEvent::down(Key::Unidentified)
    }
}

#[test]
fn the_mode_reports_the_key_record() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[?9001h");

    assert_eq!(
        screen.terminal().handle_key(&key_event(true)).as_deref(),
        Some("\x1b[65;30;97;1;16;1_")
    );
    assert_eq!(
        screen.terminal().handle_key(&key_event(false)).as_deref(),
        Some("\x1b[65;30;97;0;16;1_")
    );
}

#[test]
fn the_kitty_protocol_takes_precedence() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[?9001h\x1b[=1u");

    let event = KeyEvent {
        key: Key::Char('a'),
        codepoint: Some('a'),
        modifiers: Modifiers {
            left_ctrl: true,
            ..Modifiers::default()
        },
        ..KeyEvent::down(Key::Unidentified)
    };
    assert_eq!(screen.terminal().handle_key(&event).as_deref(), Some("\x1b[97;5u"));
}

#[test]
fn the_mode_off_uses_the_vt_encoding() {
    let mut screen = Screen::new(10, 3);

    // The platform supplies the shifted character, so this is the one the
    // event carries.
    assert_eq!(screen.terminal().handle_key(&key_event(true)).as_deref(), Some("a"));
}
