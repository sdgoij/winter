//! Insert and delete characters and lines (ECMA-48 §8.3.64, §8.3.47, §8.3.62,
//! §8.3.32).

use crate::support::Screen;

/// Four rows, so an inserted or deleted line has something to push against.
fn rows() -> Screen {
    let mut screen = Screen::new(10, 4);
    screen.feed_str("aaa\r\nbbb\r\nccc\r\nddd");
    screen
}

#[test]
fn ich_shifts_the_rest_of_the_line_right() {
    let mut screen = rows();
    screen.feed_str("\x1b[2;2H\x1b[2@");

    assert_eq!(screen.line(1), "b  bb");
    assert_eq!(screen.cursor(), (1, 1), "ICH must not move the cursor");
}

#[test]
fn dch_shifts_the_rest_of_the_line_left_and_blanks_the_end() {
    let mut screen = rows();
    screen.feed_str("\x1b[2;2H\x1b[2P");

    assert_eq!(screen.line(1), "b");
}

#[test]
fn il_inserts_a_blank_line_at_the_cursor() {
    let mut screen = rows();
    screen.feed_str("\x1b[2;1H\x1b[L");

    assert_eq!(screen.text(), "aaa\n\nbbb\nccc");
}

#[test]
fn dl_removes_the_line_at_the_cursor() {
    let mut screen = rows();
    screen.feed_str("\x1b[2;1H\x1b[M");

    assert_eq!(screen.text(), "aaa\nccc\nddd\n");
}

#[test]
fn il_and_dl_move_the_cursor_to_the_left_margin() {
    let mut screen = rows();
    screen.feed_str("\x1b[3;3H\x1b[L");

    assert_eq!(screen.text(), "aaa\nbbb\n\nccc");
    // Upstream: "The IL and DL controls are also expected to move the cursor
    // to the left margin."
    assert_eq!(screen.cursor(), (2, 0));
}
