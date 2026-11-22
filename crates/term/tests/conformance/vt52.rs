//! VT52 mode (DEC STD 070 chapter 3, xterm's control sequences).

use crate::support::Screen;

/// A screen in VT52 mode: DECANM reset selects it, and the parser switches
/// after the sequence that carried it ends.
fn screen() -> Screen {
    let mut screen = Screen::new(10, 4);
    screen.feed_str("\x1b[?2l");
    screen
}

#[test]
fn esc_a_b_c_d_move_the_cursor_one_cell() {
    let mut screen = screen();
    screen.feed_str("\x1bC");
    assert_eq!(screen.cursor(), (0, 1));

    screen.feed_str("\x1bB");
    assert_eq!(screen.cursor(), (1, 1));

    screen.feed_str("\x1bA");
    assert_eq!(screen.cursor(), (0, 1));

    screen.feed_str("\x1bD");
    assert_eq!(screen.cursor(), (0, 0));
}

#[test]
fn esc_y_addresses_the_cursor_with_ascii_coordinates() {
    let mut screen = screen();
    // The lowest coordinate is a space and means one; `"` and `#` are two and
    // three.
    screen.feed_str("\x1bY\"#");
    assert_eq!(screen.cursor(), (2, 3));

    screen.feed_str("\x1bY  ");
    assert_eq!(screen.cursor(), (0, 0));
}

#[test]
fn esc_h_homes_the_cursor() {
    let mut screen = screen();
    screen.feed_str("\x1bY\"#\x1bH");
    assert_eq!(screen.cursor(), (0, 0));
}

#[test]
fn esc_k_and_esc_j_erase_to_the_end() {
    let mut screen = Screen::new(10, 3);
    screen.feed_str("\x1b[?2l");
    screen.feed_str("aaaaa\r\nbbbbb\r\nccccc");
    // Row 2, column 3, one-based.
    screen.feed_str("\x1bY!\"");
    assert_eq!(screen.cursor(), (1, 2));

    screen.feed_str("\x1bK");
    assert_eq!(screen.line(1), "bb");

    screen.feed_str("\x1bJ");
    assert_eq!(screen.text(), "aaaaa\nbb\n");
}

#[test]
fn esc_i_reverse_indexes_and_scrolls_at_the_top() {
    let mut screen = screen();
    screen.feed_str("a\r\nb\r\nc");
    screen.feed_str("\x1bH\x1bI");
    // The region is the whole screen, so `c` moves down to the last row.
    assert_eq!(screen.text(), "\na\nb\nc");
}

#[test]
fn esc_z_identifies_a_vt52() {
    let mut screen = screen();
    screen.feed_str("\x1bZ");
    assert_eq!(screen.responses(), vec!["\x1b/Z".to_string()]);
}

#[test]
fn esc_f_and_esc_g_switch_the_graphics_set() {
    let mut screen = screen();
    screen.feed_str("\x1bFq");
    assert_eq!(screen.cell(0, 0), "\u{2500}");

    screen.feed_str("\x1bGq");
    assert_eq!(screen.cell(0, 1), "q");
}

#[test]
fn esc_less_than_returns_to_ansi_mode() {
    let mut screen = screen();
    screen.feed_str("\x1b<");
    screen.feed_str("\x1b[2;3H");
    assert_eq!(screen.cursor(), (1, 2));
}
