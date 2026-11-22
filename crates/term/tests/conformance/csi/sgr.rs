//! SGR — select graphic rendition (ECMA-48 §8.3.117).

use crate::support::Screen;
use types::{TextAttribute, TextColor, UnderlineStyle};

/// Writes `sequence` followed by `A` and returns the cell's attributes.
///
/// Every case starts from a fresh screen, so the result is the state the
/// sequence establishes and nothing carried over from another test.
fn attribute_after(sequence: &str) -> TextAttribute {
    let mut screen = Screen::new(10, 4);
    screen.feed_str(sequence);
    screen.feed_str("A");
    screen.attributes(0, 0)
}

#[test]
fn the_ansi_foreground_colours_are_16_colour_indices() {
    for (parameter, index) in [
        (30u8, 0u8),
        (31, 1),
        (32, 2),
        (33, 3),
        (34, 4),
        (35, 5),
        (36, 6),
        (37, 7),
    ] {
        let attributes = attribute_after(&format!("\x1b[{parameter}m"));
        assert_eq!(
            attributes.get_foreground(),
            TextColor::index16(index),
            "SGR {parameter}"
        );
    }
}

#[test]
fn the_ansi_background_colours_are_16_colour_indices() {
    for (parameter, index) in [(40u8, 0u8), (41, 1), (44, 4), (47, 7)] {
        let attributes = attribute_after(&format!("\x1b[{parameter}m"));
        assert_eq!(
            attributes.get_background(),
            TextColor::index16(index),
            "SGR {parameter}"
        );
    }
}

#[test]
fn sgr_39_and_49_restore_only_their_colour() {
    let attributes = attribute_after("\x1b[31;42m\x1b[39m");
    assert!(attributes.get_foreground().is_default());
    assert_eq!(attributes.get_background(), TextColor::index16(2));

    let attributes = attribute_after("\x1b[31;42m\x1b[49m");
    assert_eq!(attributes.get_foreground(), TextColor::index16(1));
    assert!(attributes.get_background().is_default());
}

#[test]
fn an_8_bit_colour_is_stored_as_a_256_colour_index() {
    let attributes = attribute_after("\x1b[38;5;196m");
    assert_eq!(attributes.get_foreground(), TextColor::index256(196));

    let attributes = attribute_after("\x1b[48;5;9m");
    assert_eq!(attributes.get_background(), TextColor::index256(9));
}

#[test]
fn a_24_bit_colour_is_stored_as_red_green_blue() {
    let attributes = attribute_after("\x1b[38;2;12;34;56m");
    assert_eq!(attributes.get_foreground(), TextColor::rgb(12, 34, 56));

    let attributes = attribute_after("\x1b[48;2;200;100;0m");
    assert_eq!(attributes.get_background(), TextColor::rgb(200, 100, 0));
}

#[test]
fn intense_and_faint_are_set_by_1_and_2_and_cleared_by_22() {
    assert!(attribute_after("\x1b[1m").is_intense());
    assert!(attribute_after("\x1b[2m").is_faint());

    let attributes = attribute_after("\x1b[1;2m\x1b[22m");
    assert!(!attributes.is_intense());
    assert!(!attributes.is_faint());
}

#[test]
fn italic_is_set_by_3_and_cleared_by_23() {
    assert!(attribute_after("\x1b[3m").is_italic());
    assert!(!attribute_after("\x1b[3m\x1b[23m").is_italic());
}

#[test]
fn underline_styles_are_set_by_the_colon_form() {
    assert_eq!(
        attribute_after("\x1b[4m").get_underline_style(),
        UnderlineStyle::SinglyUnderlined
    );
    assert_eq!(
        attribute_after("\x1b[21m").get_underline_style(),
        UnderlineStyle::DoublyUnderlined
    );
    assert_eq!(
        attribute_after("\x1b[4:3m").get_underline_style(),
        UnderlineStyle::CurlyUnderlined
    );
    assert_eq!(
        attribute_after("\x1b[4:4m").get_underline_style(),
        UnderlineStyle::DottedUnderlined
    );
    assert_eq!(
        attribute_after("\x1b[4:5m").get_underline_style(),
        UnderlineStyle::DashedUnderlined
    );
    assert_eq!(
        attribute_after("\x1b[4m\x1b[24m").get_underline_style(),
        UnderlineStyle::NoUnderline
    );
}

#[test]
fn reverse_invisible_and_crossed_out_are_toggled_by_their_pairs() {
    assert!(attribute_after("\x1b[7m").is_reverse_video());
    assert!(!attribute_after("\x1b[7m\x1b[27m").is_reverse_video());

    assert!(attribute_after("\x1b[8m").is_invisible());
    assert!(!attribute_after("\x1b[8m\x1b[28m").is_invisible());

    assert!(attribute_after("\x1b[9m").is_crossed_out());
    assert!(!attribute_after("\x1b[9m\x1b[29m").is_crossed_out());
}

#[test]
fn several_parameters_in_one_sequence_all_apply() {
    let attributes = attribute_after("\x1b[1;3;4;7;9m");
    assert!(attributes.is_intense());
    assert!(attributes.is_italic());
    assert_eq!(attributes.get_underline_style(), UnderlineStyle::SinglyUnderlined);
    assert!(attributes.is_reverse_video());
    assert!(attributes.is_crossed_out());
}

#[test]
fn an_empty_parameter_means_zero_and_resets_everything() {
    let mut screen = Screen::new(10, 4);
    screen.feed_str("\x1b[1;31;42;4mX");
    screen.feed_str("\x1b[m");
    screen.feed_str("Y");

    let reset = screen.attributes(0, 1);
    assert!(reset.get_foreground().is_default());
    assert!(reset.get_background().is_default());
    assert!(!reset.is_intense());
    assert_eq!(reset.get_underline_style(), UnderlineStyle::NoUnderline);

    // The cell written before the reset keeps what it was given.
    let kept = screen.attributes(0, 0);
    assert_eq!(kept.get_foreground(), TextColor::index16(1));
    assert_eq!(kept.get_background(), TextColor::index16(2));
}

#[test]
fn rendition_applies_from_the_next_character() {
    let mut screen = Screen::new(10, 4);
    screen.feed_str("A\x1b[31mB");

    assert!(screen.attributes(0, 0).get_foreground().is_default());
    assert_eq!(screen.attributes(0, 1).get_foreground(), TextColor::index16(1));
}
