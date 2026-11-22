//! Sixel images (DEC STD 070 §5.3, xterm's sixel graphics).

use crate::support::Screen;

/// A one-colour image painted with the sixel byte `~`, which sets the middle
/// four of the six pixel rows.
fn image(colour: &str) -> String {
    format!("\x1bP0;1;0q{colour}~\x1b\\")
}

#[test]
fn an_image_paints_the_colour_it_defines() {
    let mut screen = Screen::new(20, 5);
    // `#1;2;r;g;b` defines colour 1 as red, green and blue percentages.
    screen.feed_str(&image("#1;2;0;100;0"));

    assert_eq!(screen.image_pixel(0), Some(0xFF00_FF00));
    // One pixel column is padded out to a whole character cell.
    assert_eq!(screen.image_pixel_width(0), Some(10));
}

#[test]
fn the_defined_colour_is_honoured() {
    let mut screen = Screen::new(20, 5);
    screen.feed_str(&image("#1;2;100;0;0"));

    assert_eq!(screen.image_pixel(0), Some(0xFFFF_0000));
}

#[test]
fn an_image_starts_on_the_row_the_cursor_is_on() {
    let mut screen = Screen::new(20, 5);
    screen.feed_str("\r\n");
    screen.feed_str(&image("#1;2;0;100;0"));

    assert_eq!(screen.image_pixel(0), None);
    assert_eq!(screen.image_pixel(1), Some(0xFF00_FF00));
}

#[test]
fn text_written_over_the_image_erases_the_pixels_under_it() {
    let mut screen = Screen::new(20, 5);
    screen.feed_str(&image("#1;2;0;100;0"));
    assert!(screen.image_pixel_width(0).is_some());

    screen.feed_str("\x1b[1;1HA");
    assert_eq!(screen.line(0), "A");
    // The image covered only that cell, so nothing is left of it (ADR 005).
    assert_eq!(screen.image_pixel(0), None);
}

#[test]
fn text_erases_only_the_cells_it_covers() {
    let mut screen = Screen::new(20, 5);
    // Twenty pixel columns span two cells.
    screen.feed_str("\x1bP0;1;0q#1;2;0;100;0!20~\x1b\\");
    assert_eq!(screen.image_pixel_width(0), Some(20));
    assert_eq!(screen.image_columns(0), Some((0, 2)));
    assert_eq!(screen.image_pixel(0), Some(0xFF00_FF00));

    screen.feed_str("\x1b[1;1HA");
    // The written cell's pixels are zeroed; the rest of the band remains.
    assert_eq!(screen.image_pixel(0), Some(0));
    assert_eq!(screen.image_columns(0), Some((0, 2)));
}

#[test]
fn an_unterminated_image_paints_nothing() {
    let mut screen = Screen::new(20, 5);
    // No string terminator: the DCS never completes.
    screen.feed_str("\x1bP0;1;0q#1;2;0;100;0~");

    assert_eq!(screen.image_pixel(0), None);
}

#[test]
fn a_second_image_replaces_the_first() {
    let mut screen = Screen::new(20, 5);
    screen.feed_str(&image("#1;2;0;100;0"));
    assert_eq!(screen.image_pixel(0), Some(0xFF00_FF00));

    screen.feed_str(&image("#1;2;100;0;0"));
    assert_eq!(screen.image_pixel(0), Some(0xFFFF_0000));
}
