//! Cursor movement (ECMA-48 §8.3.18–8.3.23, §8.3.68, §8.3.77).

use crate::support::Screen;

/// A 20 by 10 screen: wide and tall enough that clamping is a deliberate case.
fn screen() -> Screen {
    Screen::new(20, 10)
}

#[test]
fn cup_positions_the_cursor_with_one_based_coordinates() {
    let mut screen = screen();
    screen.feed_str("\x1b[3;4H");
    assert_eq!(screen.cursor(), (2, 3));
}

#[test]
fn cup_and_cup_with_no_parameters_home_the_cursor() {
    let mut screen = screen();
    screen.feed_str("\x1b[5;6H\x1b[H");
    assert_eq!(screen.cursor(), (0, 0));

    screen.feed_str("\x1b[5;6f");
    assert_eq!(screen.cursor(), (4, 5));

    screen.feed_str("\x1b[;H");
    assert_eq!(screen.cursor(), (0, 0));
}

#[test]
fn cuu_and_cud_move_within_the_screen() {
    let mut screen = screen();
    screen.feed_str("\x1b[6;6H\x1b[2A");
    assert_eq!(screen.cursor(), (3, 5));

    screen.feed_str("\x1b[99A");
    assert_eq!(screen.cursor(), (0, 5));

    screen.feed_str("\x1b[2B");
    assert_eq!(screen.cursor(), (2, 5));

    screen.feed_str("\x1b[99B");
    assert_eq!(screen.cursor(), (9, 5));
}

#[test]
fn cuf_and_cub_stop_at_the_margins() {
    let mut screen = screen();
    screen.feed_str("\x1b[5C");
    assert_eq!(screen.cursor(), (0, 5));

    screen.feed_str("\x1b[99C");
    assert_eq!(screen.cursor(), (0, 19));

    screen.feed_str("\x1b[5D");
    assert_eq!(screen.cursor(), (0, 14));

    screen.feed_str("\x1b[99D");
    assert_eq!(screen.cursor(), (0, 0));
}

#[test]
fn a_movement_without_a_parameter_moves_one_cell() {
    let mut screen = screen();
    screen.feed_str("\x1b[3;3H\x1b[A\x1b[D");
    assert_eq!(screen.cursor(), (1, 1));
}

#[test]
fn cha_and_vpa_move_one_axis_only() {
    let mut screen = screen();
    screen.feed_str("\x1b[3;4H\x1b[10G");
    assert_eq!(screen.cursor(), (2, 9));

    screen.feed_str("\x1b[5d");
    assert_eq!(screen.cursor(), (4, 9));
}

#[test]
fn cnl_and_cpl_move_to_the_first_column() {
    let mut screen = screen();
    screen.feed_str("\x1b[1;4H\x1b[2E");
    assert_eq!(screen.cursor(), (2, 0));

    screen.feed_str("\x1b[5F");
    assert_eq!(screen.cursor(), (0, 0));
}

#[test]
fn hpr_and_vpr_move_one_axis_relative_to_the_cursor() {
    let mut screen = screen();
    screen.feed_str("\x1b[2;3H\x1b[4a");
    assert_eq!(screen.cursor(), (1, 6));

    screen.feed_str("\x1b[4e");
    assert_eq!(screen.cursor(), (5, 6));
}

#[test]
fn decsc_and_decrc_restore_the_saved_position() {
    let mut screen = screen();
    screen.feed_str("\x1b[3;4H\x1b7\x1b[1;1H");
    assert_eq!(screen.cursor(), (0, 0));

    screen.feed_str("\x1b8");
    assert_eq!(screen.cursor(), (2, 3));
}

#[test]
fn csi_s_and_csi_u_save_and_restore_the_cursor() {
    let mut screen = screen();
    screen.feed_str("\x1b[7;8H\x1b[s\x1b[1;1H\x1b[u");
    assert_eq!(screen.cursor(), (6, 7));
}

#[test]
fn eight_bit_c1_csi_is_accepted_after_esc_space_7() {
    let mut screen = screen();
    screen.feed(b"\x1b 7");
    screen.feed(b"\x9b2;3H");
    assert_eq!(screen.cursor(), (1, 2));
}
