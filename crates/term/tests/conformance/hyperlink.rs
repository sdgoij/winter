//! OSC 8 — hyperlinks carried by the buffer (xterm's control sequences).

use crate::support::Screen;

#[test]
fn osc_8_marks_the_cells_and_keeps_the_uri() {
    let mut screen = Screen::new(20, 3);
    screen.feed_str("\x1b]8;;https://example.com\x07link\x1b]8;;\x07");

    assert_eq!(screen.line(0), "link");
    assert_eq!(screen.hyperlink(0, 0).as_deref(), Some("https://example.com"));
    assert_eq!(screen.hyperlink(0, 3).as_deref(), Some("https://example.com"));
}

#[test]
fn closing_osc_8_ends_the_link() {
    let mut screen = Screen::new(20, 3);
    screen.feed_str("\x1b]8;;https://example.com\x07link\x1b]8;;\x07 plain");

    assert_eq!(screen.hyperlink(0, 4), None);
    assert_eq!(screen.hyperlink(0, 5), None);
}

#[test]
fn two_links_keep_their_own_uris() {
    let mut screen = Screen::new(40, 3);
    screen.feed_str("\x1b]8;;https://one.example\x07a\x1b]8;;\x07");
    screen.feed_str("\x1b]8;;https://two.example\x07b\x1b]8;;\x07");

    assert_eq!(screen.hyperlink(0, 0).as_deref(), Some("https://one.example"));
    assert_eq!(screen.hyperlink(0, 1).as_deref(), Some("https://two.example"));
}
