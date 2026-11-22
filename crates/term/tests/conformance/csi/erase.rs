//! Erase in display and in line (ECMA-48 §8.3.36–8.3.41, §8.3.35).

use crate::support::Screen;

/// Three rows spelling `aaaaa`, `bbbbb`, `ccccc`.
fn filled() -> Screen {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("aaaaa\r\nbbbbb\r\nccccc");
    screen
}

#[test]
fn el_0_erases_from_the_cursor_to_the_end_of_the_line() {
    let mut screen = filled();
    screen.feed_str("\x1b[2;3H\x1b[K");

    assert_eq!(screen.line(1), "bb");
    assert_eq!(screen.line(0), "aaaaa");
    assert_eq!(screen.line(2), "ccccc");
    assert_eq!(screen.cursor(), (1, 2), "EL must not move the cursor");
}

#[test]
fn el_1_erases_from_the_start_of_the_line_through_the_cursor() {
    let mut screen = filled();
    screen.feed_str("\x1b[2;3H\x1b[1K");

    assert_eq!(screen.line(1), "   bb");
}

#[test]
fn el_2_erases_the_whole_line_wherever_the_cursor_is() {
    let mut screen = filled();
    screen.feed_str("\x1b[2;3H\x1b[2K");

    assert_eq!(screen.line(1), "");
    assert_eq!(screen.cursor(), (1, 2));
}

#[test]
fn ed_0_erases_from_the_cursor_to_the_end_of_the_screen() {
    let mut screen = filled();
    screen.feed_str("\x1b[2;3H\x1b[J");

    assert_eq!(screen.text(), "aaaaa\nbb\n");
}

#[test]
fn ed_1_erases_from_the_start_of_the_screen_through_the_cursor() {
    let mut screen = filled();
    screen.feed_str("\x1b[2;3H\x1b[1J");

    assert_eq!(screen.text(), "\n   bb\nccccc");
}

#[test]
fn ed_2_erases_the_screen_and_leaves_the_cursor_alone() {
    let mut screen = filled();
    screen.feed_str("\x1b[2;3H\x1b[2J");

    assert_eq!(screen.text(), "\n\n");
    assert_eq!(screen.cursor(), (1, 2));
}

#[test]
fn ech_blanks_cells_without_moving_what_follows() {
    let mut screen = filled();
    screen.feed_str("\x1b[2;2H\x1b[2X");

    assert_eq!(screen.line(1), "b  bb");
}

#[test]
fn ech_past_the_right_margin_stops_at_the_margin() {
    let mut screen = filled();
    screen.feed_str("\x1b[1;8H\x1b[99X");

    assert_eq!(screen.line(0), "aaaaa");
}

#[test]
fn an_erase_parameter_of_zero_is_the_default_form() {
    let mut screen = filled();
    screen.feed_str("\x1b[2;3H\x1b[0J");

    assert_eq!(screen.text(), "aaaaa\nbb\n");
}
