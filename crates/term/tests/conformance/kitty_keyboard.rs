//! Kitty keyboard protocol flags and their effect on key encoding.

use crate::support::Screen;
use term::input::{Key, KeyEvent, Modifiers};

fn ctrl() -> Modifiers {
    Modifiers {
        left_ctrl: true,
        ..Modifiers::default()
    }
}

fn character(character: char, modifiers: Modifiers) -> KeyEvent {
    KeyEvent {
        key: Key::Char(character),
        codepoint: Some(character),
        modifiers,
        ..KeyEvent::down(Key::Unidentified)
    }
}

#[test]
fn the_query_reports_the_flags_in_effect() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[?u");
    assert_eq!(screen.responses(), vec!["\x1b[?0u".to_string()]);
}

#[test]
fn setting_the_disambiguate_flag_changes_the_encoding() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[=1u");

    assert_eq!(
        screen.terminal().handle_key(&character('a', ctrl())).as_deref(),
        Some("\x1b[97;5u")
    );
    // A plain text key is still text.
    assert_eq!(
        screen
            .terminal()
            .handle_key(&character('a', Modifiers::default()))
            .as_deref(),
        Some("a")
    );
}

#[test]
fn a_pushed_flag_set_is_popped_again() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[>1u");
    assert_eq!(
        screen.terminal().handle_key(&character('a', ctrl())).as_deref(),
        Some("\x1b[97;5u")
    );

    screen.feed_str("\x1b[<u");
    assert_eq!(
        screen.terminal().handle_key(&character('a', ctrl())).as_deref(),
        Some("\x01")
    );
}

#[test]
fn the_set_mode_is_an_or_and_the_reset_mode_clears() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[=1u");
    screen.feed_str("\x1b[=2;2u");
    assert!(screen.responses().is_empty());

    screen.feed_str("\x1b[?u");
    assert_eq!(screen.responses(), vec!["\x1b[?3u".to_string()]);

    // Mode 3 clears the bits it names.
    screen.feed_str("\x1b[=2;3u\x1b[?u");
    assert_eq!(screen.responses(), vec!["\x1b[?1u".to_string()]);
}

#[test]
fn every_key_reported_as_an_escape_code_sends_modifiers_too() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[=8u");

    assert_eq!(
        screen.terminal().handle_key(&KeyEvent::down(Key::LeftShift)).as_deref(),
        Some("\x1b[57441u")
    );
    assert_eq!(
        screen
            .terminal()
            .handle_key(&character('a', Modifiers::default()))
            .as_deref(),
        Some("\x1b[97u")
    );
}
