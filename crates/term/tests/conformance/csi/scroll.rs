//! Scrolling and the scrolling margins (ECMA-48 §8.3.144, §8.3.145,
//! DEC STD 070 §4.6, DECSTBM).

use crate::support::Screen;

/// Four rows spelling `r0` to `r3`, with the cursor on the last row.
fn rows() -> Screen {
    let mut screen = Screen::new(10, 4);
    screen.feed_str("r0\r\nr1\r\nr2\r\nr3");
    screen
}

#[test]
fn su_scrolls_the_screen_up() {
    let mut screen = rows();
    screen.feed_str("\x1b[2S");

    assert_eq!(screen.text(), "r2\nr3\n\n");
}

#[test]
fn sd_scrolls_the_screen_down() {
    let mut screen = rows();
    screen.feed_str("\x1b[T");

    assert_eq!(screen.text(), "\nr0\nr1\nr2");
}

#[test]
fn a_line_feed_on_the_last_row_scrolls_without_changing_the_column() {
    let mut screen = rows();
    screen.feed_str("\n");

    assert_eq!(screen.text(), "r1\nr2\nr3\n");
    assert_eq!(screen.cursor(), (3, 2), "LF does not change the column");
}

#[test]
fn a_return_before_the_line_feed_also_homes_the_cursor() {
    let mut screen = rows();
    screen.feed_str("\r\n");

    assert_eq!(screen.text(), "r1\nr2\nr3\n");
    assert_eq!(screen.cursor(), (3, 0));
}

#[test]
fn decstbm_scrolls_only_the_region_it_names() {
    let mut screen = rows();
    // Rows 2 through 3, one-based, leaving rows 1 and 4 outside the region.
    screen.feed_str("\x1b[2;3r");
    assert_eq!(screen.cursor(), (0, 0), "DECSTBM homes the cursor");

    screen.feed_str("\x1b[3;1H\r\n");

    assert_eq!(screen.text(), "r0\nr2\n\nr3");
}

#[test]
fn decstbm_with_no_parameters_restores_the_whole_screen() {
    let mut screen = rows();
    screen.feed_str("\x1b[2;3r\x1b[r\x1b[4;1H\r\n");

    assert_eq!(screen.text(), "r1\nr2\nr3\n");
}

#[test]
fn ri_at_the_top_margin_scrolls_the_region_down() {
    let mut screen = rows();
    screen.feed_str("\x1b[1;1H\x1bM");

    assert_eq!(screen.text(), "\nr0\nr1\nr2");
}

#[test]
fn su_inside_the_region_leaves_the_outside_rows_alone() {
    let mut screen = rows();
    screen.feed_str("\x1b[2;3r\x1b[1;1H\x1b[S");

    assert_eq!(screen.text(), "r0\nr2\n\nr3");
}
