//! That a program's bytes reach the right cells as the right characters.
//!
//! The console host sends UTF-8, so a character is one, two, three or four
//! bytes, and a read can split it anywhere. These cases hold the decoder to
//! that: the cell text is the code point the bytes name, never its bytes.

use crate::support::Screen;

#[test]
fn a_three_byte_character_fills_one_cell() {
    let mut screen = Screen::new(4, 1);
    screen.feed_str("\u{2014}");

    assert_eq!(screen.line(0), "\u{2014}");
}

#[test]
fn an_accented_character_keeps_its_accent() {
    let mut screen = Screen::new(8, 1);
    screen.feed_str("fa\u{e7}ade");

    assert_eq!(screen.line(0), "fa\u{e7}ade");
}

#[test]
fn a_wide_character_takes_its_cells_and_the_next_one_follows() {
    let mut screen = Screen::new(4, 1);
    screen.feed_str("\u{30cd}\u{30b3}");

    assert_eq!(screen.line(0), "\u{30cd}\u{30b3}");
    assert_eq!(screen.cell(0, 0), "\u{30cd}");
    assert_eq!(screen.cell(0, 2), "\u{30b3}");
}

#[test]
fn an_astral_character_is_one_code_point() {
    let mut screen = Screen::new(4, 1);
    screen.feed_str("\u{1f600}");

    assert_eq!(screen.line(0), "\u{1f600}");
}

#[test]
fn a_character_split_across_reads_is_not_replaced() {
    let mut screen = Screen::new(4, 1);
    // U+2014 is E2 80 94; the console host may deliver it in any pieces.
    screen.feed(&[0xE2, 0x80]);
    screen.feed(&[0x94]);

    assert_eq!(screen.line(0), "\u{2014}");
}

#[test]
fn an_invalid_byte_becomes_the_replacement_character() {
    let mut screen = Screen::new(4, 1);
    screen.feed(&[0x80]);

    assert_eq!(screen.line(0), "\u{fffd}");
}
