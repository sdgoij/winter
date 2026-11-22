//! DECSET/DECRST private modes (DEC STD 070 §4.4, xterm's control sequences).

use crate::support::Screen;

#[test]
fn dectcem_shows_and_hides_the_cursor() {
    let mut screen = Screen::new(10, 3);
    assert!(screen.cursor_visible(), "the cursor starts visible");

    screen.feed_str("\x1b[?25l");
    assert!(!screen.cursor_visible());

    screen.feed_str("\x1b[?25h");
    assert!(screen.cursor_visible());
}

#[test]
fn decawm_off_keeps_the_cursor_on_the_last_column() {
    let mut wrapping = Screen::new(5, 3);
    wrapping.feed_str("abcdefgh");
    assert_eq!(wrapping.text(), "abcde\nfgh\n");

    let mut unwrapping = Screen::new(5, 3);
    unwrapping.feed_str("\x1b[?7l");
    unwrapping.feed_str("abcdefgh");
    assert!(unwrapping.line(1).is_empty(), "nothing may wrap once DECAWM is reset");
    assert_eq!(unwrapping.line(0).chars().count(), 5);
    assert!(!unwrapping.cell(0, 4).is_empty(), "the last column stays in use");
}

#[test]
fn the_alternate_screen_buffer_leaves_the_main_buffer_intact() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("main");

    screen.feed_str("\x1b[?1049h");
    assert_eq!(screen.line(0), "", "the alternate buffer starts blank");
    screen.feed_str("alt");
    assert_eq!(screen.line(0), "alt");

    screen.feed_str("\x1b[?1049l");
    assert_eq!(screen.line(0), "main");
}

#[test]
fn decrqm_reports_whether_a_mode_is_set() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[?25$p");
    assert_eq!(screen.responses(), vec!["\x1b[?25;1$y".to_string()]);

    screen.feed_str("\x1b[?25l\x1b[?25$p");
    assert_eq!(screen.responses(), vec!["\x1b[?25;2$y".to_string()]);
}

#[test]
fn decrqm_reports_an_unknown_mode_as_zero() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[?9999$p");
    assert_eq!(screen.responses(), vec!["\x1b[?9999;0$y".to_string()]);
}
