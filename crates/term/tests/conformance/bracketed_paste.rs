//! Bracketed paste (mode 2004, xterm's control sequences).

use crate::support::Screen;

#[test]
fn the_mode_decides_whether_a_paste_is_wrapped() {
    let mut screen = Screen::new(10, 3);
    assert_eq!(screen.terminal().paste("hi"), "hi");

    screen.feed_str("\x1b[?2004h");
    assert_eq!(screen.terminal().paste("hi"), "\x1b[200~hi\x1b[201~");

    screen.feed_str("\x1b[?2004l");
    assert_eq!(screen.terminal().paste("hi"), "hi");
}

#[test]
fn control_codes_are_removed_from_a_paste() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[?2004h");
    assert_eq!(screen.terminal().paste("a\x01b"), "\x1b[200~ab\x1b[201~");
}

#[test]
fn a_pasted_end_marker_cannot_close_the_block() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[?2004h");
    assert_eq!(screen.terminal().paste("echo\x1b[201~"), "\x1b[200~echo[201~\x1b[201~");
}

#[test]
fn line_endings_become_carriage_returns() {
    let mut screen = Screen::new(10, 3);
    assert_eq!(screen.terminal().paste("a\r\nb\nc"), "a\rb\rc");
}

#[test]
fn pasting_does_not_touch_the_screen() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[?2004h");
    let _ = screen.terminal().paste("hello");
    assert_eq!(screen.text(), "\n\n");
}

#[test]
fn the_mode_is_reported_back() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[?2004h\x1b[?2004$p");
    assert_eq!(screen.responses(), vec!["\x1b[?2004;1$y".to_string()]);
}
