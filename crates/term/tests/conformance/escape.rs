//! ESC sequences (ECMA-48 §5.4, DEC STD 070).

use crate::support::Screen;
use types::LineRendition;

#[test]
fn esc_7_and_esc_8_save_and_restore_the_cursor() {
    let mut screen = Screen::new(20, 5);
    screen.feed_str("\x1b[3;4H\x1b7\x1b[1;1H");
    assert_eq!(screen.cursor(), (0, 0));

    screen.feed_str("\x1b8");
    assert_eq!(screen.cursor(), (2, 3));
}

#[test]
fn esc_d_indexes_without_returning() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[2;3H\x1bD");
    assert_eq!(screen.cursor(), (2, 2));
}

#[test]
fn esc_e_returns_and_indexes() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[2;3H\x1bE");
    assert_eq!(screen.cursor(), (2, 0));
}

#[test]
fn esc_m_reverse_index_scrolls_at_the_top() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("a\r\nb\r\nc");
    screen.feed_str("\x1b[1;1H\x1bM");
    assert_eq!(screen.text(), "\na\nb");
}

#[test]
fn shift_out_and_shift_in_select_the_alternate_set() {
    let mut screen = Screen::new(10, 3);
    // `A` is the British set, which differs from ASCII at `#`.
    screen.feed_str("\x1b)A\x0e#");
    assert_eq!(screen.cell(0, 0), "\u{a3}");

    screen.feed(b"\x0f#");
    assert_eq!(screen.cell(0, 1), "#");
}

#[test]
fn esc_hash_selects_the_line_rendition() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[1;1H\x1b#3");
    assert_eq!(screen.line_rendition(0), LineRendition::DoubleHeightTop);

    screen.feed_str("\x1b#6");
    assert_eq!(screen.line_rendition(0), LineRendition::DoubleWidth);

    screen.feed_str("\x1b#5");
    assert_eq!(screen.line_rendition(0), LineRendition::SingleWidth);
}

#[test]
fn esc_h_sets_a_tab_stop() {
    let mut screen = Screen::new(20, 3);
    screen.feed_str("\x1b[1;6H\x1bH");
    screen.feed_str("\x1b[1;1H\t");
    assert_eq!(screen.cursor(), (0, 5));
}

#[test]
fn esc_open_paren_selects_a_character_set() {
    // `A` is the British set, which differs from ASCII at `#`.
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b(A#");
    assert_eq!(screen.cell(0, 0), "\u{a3}");

    // Designating ASCII again restores the Latin character.
    screen.feed_str("\x1b(B#");
    assert_eq!(screen.cell(0, 1), "#");
}
