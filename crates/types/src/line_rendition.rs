//! `LineRendition` — screen/buffer line conversion types.
//!
//! Ported from LineRendition in the Windows Terminal C++ buffer module.

/// Specifies the direction of a line coordinate transformation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineRendition {
    /// Screen coordinate -> Buffer coordinate.
    ScreenToBuffer,
    /// Buffer coordinate -> Screen coordinate.
    BufferToScreen,
}

/// Applies a line rendition transformation.
///
/// For ScreenToBuffer: adds buffer_height to the screen line.
/// For BufferToScreen: subtracts buffer_height from the buffer line.
pub fn apply_line_rendition(line: i32, rendition: LineRendition, buffer_height: i32) -> i32 {
    match rendition {
        LineRendition::ScreenToBuffer => line + buffer_height,
        LineRendition::BufferToScreen => line - buffer_height,
    }
}

/// Converts a screen line to a buffer line.
pub fn screen_to_buffer_line(line: i32, buffer_height: i32) -> i32 {
    apply_line_rendition(line, LineRendition::ScreenToBuffer, buffer_height)
}

/// Converts a buffer line to a screen line.
pub fn buffer_to_screen_line(line: i32, buffer_height: i32) -> i32 {
    apply_line_rendition(line, LineRendition::BufferToScreen, buffer_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screen_to_buffer() {
        assert_eq!(screen_to_buffer_line(0, 50), 50);
        assert_eq!(screen_to_buffer_line(23, 50), 73);
    }

    #[test]
    fn test_buffer_to_screen() {
        assert_eq!(buffer_to_screen_line(50, 50), 0);
        assert_eq!(buffer_to_screen_line(73, 50), 23);
    }

    #[test]
    fn test_roundtrip() {
        let buffer_height = 50;
        for screen_line in 0..24 {
            let buffer = screen_to_buffer_line(screen_line, buffer_height);
            let back = buffer_to_screen_line(buffer, buffer_height);
            assert_eq!(back, screen_line);
        }
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn test_screen_buffer_roundtrip(screen_line in any::<i32>(), buffer_height in 1i32..1000i32) {
            let buffer = screen_to_buffer_line(screen_line, buffer_height);
            let back = buffer_to_screen_line(buffer, buffer_height);
            prop_assert_eq!(back, screen_line);
        }

        #[test]
        fn test_buffer_screen_roundtrip(buffer_line in any::<i32>(), buffer_height in 1i32..1000i32) {
            let screen = buffer_to_screen_line(buffer_line, buffer_height);
            let back = screen_to_buffer_line(screen, buffer_height);
            prop_assert_eq!(back, buffer_line);
        }

        #[test]
        fn test_double_apply_returns_original(line in any::<i32>(), buffer_height in 1i32..1000i32) {
            // Applying ScreenToBuffer then BufferToScreen should return the original
            let after_screen_to_buffer = apply_line_rendition(line, LineRendition::ScreenToBuffer, buffer_height);
            let after_both = apply_line_rendition(after_screen_to_buffer, LineRendition::BufferToScreen, buffer_height);
            prop_assert_eq!(after_both, line);

            // Applying BufferToScreen then ScreenToBuffer should return the original
            let after_buffer_to_screen = apply_line_rendition(line, LineRendition::BufferToScreen, buffer_height);
            let after_both2 = apply_line_rendition(after_buffer_to_screen, LineRendition::ScreenToBuffer, buffer_height);
            prop_assert_eq!(after_both2, line);
        }
    }
}
