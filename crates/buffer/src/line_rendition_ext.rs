//! `LineRenditionExt` — dimension conversion helpers for screen/buffer lines.
//!
//! Ported from `LineRenditionExt` in the Windows Terminal C++ buffer module.

use types::LineRendition;

/// Converts a screen line to a buffer line.
pub fn screen_to_buffer_line(screen_line: i32, buffer_height: i32) -> i32 {
    screen_line + buffer_height
}

/// Converts a buffer line to a screen line.
pub fn buffer_to_screen_line(buffer_line: i32, buffer_height: i32) -> i32 {
    buffer_line - buffer_height
}

/// Applies a line rendition transformation.
pub fn apply_line_rendition(line: i32, rendition: LineRendition, buffer_height: i32) -> i32 {
    match rendition {
        LineRendition::ScreenToBuffer => screen_to_buffer_line(line, buffer_height),
        LineRendition::BufferToScreen => buffer_to_screen_line(line, buffer_height),
    }
}

/// Checks if a screen line is in the scrollback region.
pub fn is_scrollback_line(screen_line: i32, scrollback_height: i32) -> bool {
    screen_line >= scrollback_height
}

/// Checks if a buffer line is in the visible area.
pub fn is_visible_line(buffer_line: i32, scrollback_height: i32) -> bool {
    buffer_line < scrollback_height
}

/// Gets the screen line for a visible buffer line.
pub fn buffer_line_to_screen(buffer_line: i32, scrollback_height: i32) -> i32 {
    if buffer_line < scrollback_height {
        buffer_line
    } else {
        buffer_line - scrollback_height
    }
}

/// Gets the buffer line for a visible screen line.
pub fn screen_line_to_buffer(screen_line: i32, scrollback_height: i32) -> i32 {
    screen_line + scrollback_height
}

/// Converts a scrollback offset (0 = most recent scrollback row) to a buffer line.
pub fn scrollback_offset_to_buffer(offset: i32, scrollback_height: i32, screen_height: i32) -> i32 {
    (screen_height + scrollback_height - 1) - offset
}

/// Converts a buffer line to a scrollback offset.
pub fn buffer_to_scrollback_offset(
    buffer_line: i32,
    scrollback_height: i32,
    screen_height: i32,
) -> i32 {
    (screen_height + scrollback_height - 1) - buffer_line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screen_to_buffer_line() {
        assert_eq!(screen_to_buffer_line(0, 300), 300);
        assert_eq!(screen_to_buffer_line(23, 300), 323);
    }

    #[test]
    fn test_buffer_to_screen_line() {
        assert_eq!(buffer_to_screen_line(300, 300), 0);
        assert_eq!(buffer_to_screen_line(323, 300), 23);
    }

    #[test]
    fn test_roundtrip() {
        let scrollback_height = 300;
        let screen_height = 24;
        for screen_line in 0..screen_height {
            let buffer = screen_to_buffer_line(screen_line, scrollback_height);
            let back = buffer_to_screen_line(buffer, scrollback_height);
            assert_eq!(back, screen_line);
        }
    }

    #[test]
    fn test_is_scrollback_line() {
        assert!(!is_scrollback_line(23, 300)); // visible
        assert!(is_scrollback_line(300, 300)); // first scrollback
        assert!(is_scrollback_line(350, 300)); // deep scrollback
    }

    #[test]
    fn test_is_visible_line() {
        assert!(is_visible_line(0, 300)); // top of scrollback
        assert!(is_visible_line(299, 300)); // last scrollback row
        assert!(!is_visible_line(300, 300)); // first visible row
        assert!(!is_visible_line(500, 300)); // deep in visible area
    }

    #[test]
    fn test_line_rendition_screen_to_buffer() {
        let result = apply_line_rendition(10, LineRendition::ScreenToBuffer, 300);
        assert_eq!(result, 310);
    }

    #[test]
    fn test_line_rendition_buffer_to_screen() {
        let result = apply_line_rendition(310, LineRendition::BufferToScreen, 300);
        assert_eq!(result, 10);
    }

    #[test]
    fn test_scrollback_offset_conversion() {
        let scrollback_height = 300;
        let screen_height = 24;

        // Most recent scrollback row (offset 0) should be the row just above the visible area
        let buffer = scrollback_offset_to_buffer(0, scrollback_height, screen_height);
        assert_eq!(buffer, screen_height + scrollback_height - 1);

        // First scrollback row (offset = scrollback_height) should be row 0
        let buffer =
            scrollback_offset_to_buffer(scrollback_height, scrollback_height, screen_height);
        assert_eq!(buffer, screen_height - 1);
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn test_screen_to_buffer_roundtrip(screen_line in 0i32..100i32, scrollback in 10i32..1000i32) {
            let buffer = screen_to_buffer_line(screen_line, scrollback);
            let back = buffer_to_screen_line(buffer, scrollback);
            prop_assert_eq!(back, screen_line);
        }

        #[test]
        fn test_buffer_to_screen_roundtrip(scrollback in 10i32..1000i32, buffer_line in 10i32..1000i32) {
            if buffer_line >= scrollback {
                let screen = buffer_to_screen_line(buffer_line, scrollback);
                let back = screen_to_buffer_line(screen, scrollback);
                prop_assert_eq!(back, buffer_line);
            }
        }

        #[test]
        fn test_double_line_rendition_returns_original(line in 0i32..100i32, scrollback in 10i32..1000i32) {
            let after_screen_to_buffer = apply_line_rendition(line, LineRendition::ScreenToBuffer, scrollback);
            let after_both = apply_line_rendition(after_screen_to_buffer, LineRendition::BufferToScreen, scrollback);
            prop_assert_eq!(after_both, line);
        }

        #[test]
        fn test_scrollback_offset_correct(scrollback in 10i32..500i32, screen_height in 10i32..200i32) {
            let buffer = scrollback_offset_to_buffer(scrollback - 1, scrollback, screen_height);
            // Should be the first scrollback row (screen_height)
            prop_assert_eq!(buffer, screen_height);
        }
    }
}
