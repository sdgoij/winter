//! OSC — operating system commands (ECMA-48 §8.3.89, xterm's control sequences).

use crate::support::Screen;
use types::TextColor;

fn screen() -> Screen {
    Screen::new(20, 5)
}

#[test]
fn osc_0_and_2_set_the_window_title() {
    let mut screen = screen();
    screen.feed_str("\x1b]2;Cascadia\x07");
    assert_eq!(screen.terminal().window_title(), Some("Cascadia"));

    // OSC 0 is the same command with an icon name; the status line is where a
    // terminal shows it.
    screen.feed_str("\x1b]0;Terminal\x1b\\");
    assert_eq!(screen.terminal().window_title(), Some("Terminal"));
}

#[test]
fn osc_7_sets_the_working_directory() {
    let mut screen = screen();
    screen.feed_str("\x1b]7;file://host/tmp\x07");
    assert_eq!(screen.terminal().current_working_directory(), Some("file://host/tmp"));
    // A second report replaces the first.
    screen.feed_str("\x1b]7;file://host/home\x1b\\");
    assert_eq!(screen.terminal().current_working_directory(), Some("file://host/home"));
}

#[test]
fn osc_52_stores_the_clipboard_and_drains_once() {
    let mut screen = screen();
    screen.feed_str("\x1b]52;c;aGVsbG8=\x07");
    assert_eq!(screen.terminal().take_clipboard_content().as_deref(), Some("hello"));
    assert_eq!(screen.terminal().take_clipboard_content(), None);
}

#[test]
fn osc_4_sets_a_palette_entry() {
    let mut screen = screen();
    screen.feed_str("\x1b]4;1;rgb:12/34/56\x07");
    assert_eq!(screen.color(1), Some(TextColor::rgb(0x12, 0x34, 0x56)));
}

#[test]
fn osc_104_restores_the_default_palette_entry() {
    let mut fresh = screen();
    let default = fresh.color(1);
    assert!(default.is_some());

    let mut screen = screen();
    screen.feed_str("\x1b]4;1;rgb:12/34/56\x07");
    assert_ne!(screen.color(1), default);

    screen.feed_str("\x1b]104;1\x07");
    assert_eq!(screen.color(1), default);
}
