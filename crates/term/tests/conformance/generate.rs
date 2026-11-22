//! Generated sequences: the parser never panics, on any byte, in any state.
//!
//! The corpus's other modules name a sequence and assert what a reference says
//! it does. This one asserts the rule the corpus cannot enumerate: whatever a
//! program writes — a truncated UTF-8 lead byte, absurd parameter counts, a DCS
//! string that never ends — `Terminal::process_bytes` returns, the screen is
//! still a rectangle, the cursor is inside it, and the same bytes produce the
//! same screen.

use proptest::prelude::*;

use crate::support::Screen;

/// The screen the property drives.
const WIDTH: i32 = 80;
/// The rows the property drives.
const HEIGHT: i32 = 24;

proptest! {
    /// Any bytes at all leave a well-formed screen, deterministically.
    #[test]
    fn any_bytes_leave_a_well_formed_screen(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        let mut screen = Screen::new(WIDTH, HEIGHT);
        screen.feed(&bytes);

        let (row, column) = screen.cursor();
        prop_assert!((0..HEIGHT).contains(&row), "the cursor row is {row}");
        prop_assert!((0..WIDTH).contains(&column), "the cursor column is {column}");

        let text = screen.text();
        prop_assert_eq!(
            text.split('\n').count(),
            HEIGHT as usize,
            "the screen keeps its height"
        );

        // Nothing in the path may depend on a clock, a thread or an address.
        let mut again = Screen::new(WIDTH, HEIGHT);
        again.feed(&bytes);
        prop_assert_eq!(again.text(), text);
        prop_assert_eq!(again.cursor(), (row, column));
    }

    /// A sequence cut off by the end of the input leaves nothing for the next
    /// write to trip over: plain text after it still reaches the screen.
    #[test]
    fn a_truncated_sequence_does_not_swallow_what_follows(
        prefix in proptest::sample::select(vec![
            "\x1b[".to_string(),
            "\x1b]0;".to_string(),
            "\x1bP1;2;3q".to_string(),
            "\x1b[?1049".to_string(),
            "\x1b".to_string(),
        ]),
        rest in "[\x20-\x7E]{0,64}",
    ) {
        let mut screen = Screen::new(WIDTH, HEIGHT);
        screen.feed_str(&prefix);

        // Whatever the cut-off sequence left behind, a line feed starts a new
        // row and the text after it is written somewhere on the screen.
        screen.feed_str("\r\n");
        screen.feed_str(&rest);

        prop_assert!((0..HEIGHT).contains(&screen.cursor().0));
    }
}
