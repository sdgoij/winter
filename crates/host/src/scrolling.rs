//! `Scrolling` — the console's wheel and scroll-key handling.
//!
//! Ported from `scrolling.cpp` and `scrolling.hpp`. Upstream's scroll bar lives
//! in the window, so its `SB_*` commands reach `SCREEN_INFORMATION` through the
//! window object; here the commands are applied to the viewport directly, which
//! is what the window does with them (ADR 012). The wheel metrics are the system
//! configuration upstream reads at startup: they start at 0, which leaves wheel
//! scrolling off until the window layer supplies them.
//!
//! `s_ScrollIfNecessary`, which drags a selection along as the pointer leaves
//! the window, needs the window's client rectangle and the selection state, so
//! it stays with those layers.

use crate::input_key_info::InputKeyInfo;
use crate::screen_info::ScreenInformation;
use types::virtual_key::{
    VK_CONTROL, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT,
    VK_SHIFT, VK_UP,
};

/// One wheel notch, `WHEEL_DELTA`.
const WHEEL_DELTA: i32 = 120;

/// A scroll-bar command, as a window's scroll bar reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollCommand {
    /// One line towards the start.
    LineUp,
    /// One line towards the end.
    LineDown,
    /// One viewport towards the start.
    PageUp,
    /// One viewport towards the end.
    PageDown,
    /// The start of the buffer.
    Top,
}

/// The axis a scroll command acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Vertical,
    Horizontal,
}

/// What a key event did while the console is in scroll mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyScrollEvent {
    /// The console consumed the key.
    Handled,
    /// The console did not consume the key; the edit line may want it.
    NotHandled,
    /// The console consumed the key and the window should beep.
    Beep,
}

/// The console's scroll-wheel and scroll-key state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scrolling {
    /// How many lines one wheel notch scrolls; 0 turns wheel scrolling off.
    wheel_scroll_lines: i32,
    /// How many columns one horizontal notch scrolls; 0 turns it off.
    wheel_scroll_chars: i32,
    in_scroll_mode: bool,
}

impl Scrolling {
    /// Creates the state with wheel scrolling off, as upstream's metrics start.
    pub const fn new() -> Self {
        Self {
            wheel_scroll_lines: 0,
            wheel_scroll_chars: 0,
            in_scroll_mode: false,
        }
    }

    /// Sets how many lines one wheel notch scrolls.
    ///
    /// Upstream reads this from the system configuration; a portable host
    /// supplies it.
    pub const fn set_wheel_scroll_lines(&mut self, lines: i32) {
        self.wheel_scroll_lines = lines;
    }

    /// Returns how many lines one wheel notch scrolls.
    pub const fn wheel_scroll_lines(&self) -> i32 {
        self.wheel_scroll_lines
    }

    /// Sets how many columns one horizontal notch scrolls.
    pub const fn set_wheel_scroll_chars(&mut self, chars: i32) {
        self.wheel_scroll_chars = chars;
    }

    /// Returns how many columns one horizontal notch scrolls.
    pub const fn wheel_scroll_chars(&self) -> i32 {
        self.wheel_scroll_chars
    }

    /// Returns whether the console is in scroll mode.
    pub const fn is_in_scroll_mode(&self) -> bool {
        self.in_scroll_mode
    }

    /// Enters scroll mode.
    ///
    /// Upstream also clears the selection and updates the window title; both
    /// belong to the layers that own them.
    pub fn do_scroll(&mut self) {
        self.in_scroll_mode = true;
    }

    /// Leaves scroll mode.
    pub fn clear_scroll(&mut self) {
        self.in_scroll_mode = false;
    }

    /// Handles a wheel event, moving the viewport.
    ///
    /// `wheel_delta` is the raw `WM_MOUSEWHEEL` delta: positive scrolls towards
    /// the start of the buffer. Remainders are kept on the screen so a slow
    /// wheel that sends fractions of a notch still scrolls.
    pub fn handle_mouse_wheel(
        &mut self,
        is_mouse_wheel: bool,
        is_mouse_hwheel: bool,
        wheel_delta: i16,
        has_shift: bool,
        screen: &mut ScreenInformation,
    ) {
        if is_mouse_wheel && self.wheel_scroll_lines > 0 {
            let actual_delta = (WHEEL_DELTA / self.wheel_scroll_lines).max(1);
            let accumulated = accumulate(screen.wheel_delta(), wheel_delta);
            screen.set_wheel_delta(accumulated);

            if i32::from(accumulated).abs() >= actual_delta {
                let mut step = 1;

                if has_shift {
                    // Shift scrolls half a screen, scaled by the registry value.
                    step = ((screen.viewport().height() * screen.scroll_scale() as i32) / 2).max(1);
                    step *= if accumulated < 0 { -1 } else { 1 };
                    screen.set_wheel_delta((i32::from(accumulated) % step) as i16);
                } else {
                    step *= i32::from(accumulated) / actual_delta;
                    screen.set_wheel_delta((i32::from(accumulated) % actual_delta) as i16);
                }

                let mut origin = screen.viewport().origin();
                origin.y -= step;
                let buffer = screen.buffer_size();
                if origin.y < 0 {
                    origin.y = 0;
                } else if origin.y + screen.viewport().height() > buffer.height() {
                    origin.y = buffer.height() - screen.viewport().height();
                }
                screen.set_viewport_origin(true, origin, false);
            }
        } else if is_mouse_hwheel && self.wheel_scroll_chars > 0 {
            let actual_delta = (WHEEL_DELTA / self.wheel_scroll_chars).max(1);
            let accumulated = accumulate(screen.hwheel_delta(), wheel_delta);
            screen.set_hwheel_delta(accumulated);

            if i32::from(accumulated).abs() >= actual_delta {
                let mut step = 1;

                if has_shift {
                    // Upstream uses the viewport's last column as the page here.
                    step = (screen.viewport().origin().x + screen.viewport().width() - 1).max(1);
                }

                step *= i32::from(accumulated) / actual_delta;
                screen.set_hwheel_delta((i32::from(accumulated) % actual_delta) as i16);

                let mut origin = screen.viewport().origin();
                origin.x += step;
                let buffer = screen.buffer_size();
                if origin.x < 0 {
                    origin.x = 0;
                } else if origin.x + screen.viewport().width() > buffer.width() {
                    origin.x = buffer.width() - screen.viewport().width();
                }
                screen.set_viewport_origin(true, origin, false);
            }
        }
    }

    /// Handles a key press while the console is in scroll mode.
    ///
    /// `edit_line_empty` is whether a pending cooked read has an empty line:
    /// Ctrl+Home and Ctrl+End belong to the editor when it does not.
    pub fn handle_key_scrolling_event(
        &mut self,
        key: &InputKeyInfo,
        edit_line_empty: bool,
        screen: &mut ScreenInformation,
    ) -> KeyScrollEvent {
        let virtual_key = key.virtual_key();
        let ctrl = key.is_ctrl_pressed();

        // Escape, Enter and Ctrl+C leave scroll mode.
        if virtual_key == VK_ESCAPE || virtual_key == VK_RETURN || (virtual_key == u16::from(b'C') && ctrl) {
            self.clear_scroll();
            return KeyScrollEvent::Handled;
        }

        let (command, axis) = match virtual_key {
            VK_UP => (ScrollCommand::LineUp, Axis::Vertical),
            VK_DOWN => (ScrollCommand::LineDown, Axis::Vertical),
            VK_LEFT => (ScrollCommand::LineUp, Axis::Horizontal),
            VK_RIGHT => (ScrollCommand::LineDown, Axis::Horizontal),
            VK_NEXT => (ScrollCommand::PageDown, Axis::Vertical),
            VK_PRIOR => (ScrollCommand::PageUp, Axis::Vertical),
            VK_END => {
                if ctrl {
                    if edit_line_empty {
                        // Ctrl+End with nothing typed jumps to the cursor.
                        screen.make_current_cursor_visible();
                        return KeyScrollEvent::Handled;
                    }
                    // With text, the editor erases to the end of the line.
                    return KeyScrollEvent::NotHandled;
                }
                (ScrollCommand::PageDown, Axis::Horizontal)
            }
            VK_HOME => {
                if ctrl {
                    if edit_line_empty {
                        (ScrollCommand::Top, Axis::Vertical)
                    } else {
                        return KeyScrollEvent::NotHandled;
                    }
                } else {
                    (ScrollCommand::PageUp, Axis::Horizontal)
                }
            }
            VK_SHIFT | VK_CONTROL | VK_MENU => return KeyScrollEvent::Handled,
            _ => return KeyScrollEvent::Beep,
        };

        scroll(screen, command, axis);
        KeyScrollEvent::Handled
    }
}

impl Default for Scrolling {
    fn default() -> Self {
        Self::new()
    }
}

/// Adds a wheel delta to the remainder, resetting it when the direction flips.
fn accumulate(remainder: i16, wheel_delta: i16) -> i16 {
    let total = if (remainder > 0) == (wheel_delta > 0) {
        i32::from(remainder) + i32::from(wheel_delta)
    } else {
        i32::from(wheel_delta)
    };
    total.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// Moves the viewport by `command` along `axis`, clamped into the buffer.
fn scroll(screen: &mut ScreenInformation, command: ScrollCommand, axis: Axis) {
    let viewport = screen.viewport();
    let buffer = screen.buffer_size();
    let mut origin = viewport.origin();
    let width = viewport.width();
    let height = viewport.height();

    match (axis, command) {
        (Axis::Vertical, ScrollCommand::LineUp) => origin.y -= 1,
        (Axis::Vertical, ScrollCommand::LineDown) => origin.y += 1,
        (Axis::Vertical, ScrollCommand::PageUp) => origin.y -= height,
        (Axis::Vertical, ScrollCommand::PageDown) => origin.y += height,
        (Axis::Vertical, ScrollCommand::Top) => origin.y = 0,
        (Axis::Horizontal, ScrollCommand::LineUp) => origin.x -= 1,
        (Axis::Horizontal, ScrollCommand::LineDown) => origin.x += 1,
        (Axis::Horizontal, ScrollCommand::PageUp) => origin.x -= width,
        (Axis::Horizontal, ScrollCommand::PageDown) => origin.x += width,
        (Axis::Horizontal, ScrollCommand::Top) => origin.x = 0,
    }

    origin.y = origin.y.clamp(0, (buffer.height() - height).max(0));
    origin.x = origin.x.clamp(0, (buffer.width() - width).max(0));
    screen.set_viewport_origin(true, origin, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{ControlKeyState, TextAttribute};
    use utils::Point;

    fn screen() -> ScreenInformation {
        ScreenInformation::new(20, 10, 40, TextAttribute::new(), TextAttribute::new())
    }

    fn positioned(mut screen: ScreenInformation, x: i32, y: i32) -> ScreenInformation {
        screen.set_viewport(
            types::Viewport::from_origin_size(Point { x, y }, utils::Size::new(20, 10)),
            false,
        );
        screen
    }

    fn narrow(mut screen: ScreenInformation, x: i32) -> ScreenInformation {
        screen.set_viewport(
            types::Viewport::from_origin_size(Point { x, y: 0 }, utils::Size::new(10, 10)),
            false,
        );
        screen
    }

    fn scrolling(lines: i32, chars: i32) -> Scrolling {
        let mut scrolling = Scrolling::new();
        scrolling.set_wheel_scroll_lines(lines);
        scrolling.set_wheel_scroll_chars(chars);
        scrolling
    }

    fn key(virtual_key: u16, ctrl: bool) -> InputKeyInfo {
        InputKeyInfo::new(
            virtual_key,
            ControlKeyState {
                left_ctrl: ctrl,
                ..ControlKeyState::default()
            },
        )
    }

    #[test]
    fn wheel_scrolling_is_off_until_the_metrics_are_set() {
        let mut screen = positioned(screen(), 0, 10);
        let mut scrolling = scrolling(0, 0);
        scrolling.handle_mouse_wheel(true, false, WHEEL_DELTA as i16, false, &mut screen);
        assert_eq!(screen.viewport().origin().y, 10);
    }

    #[test]
    fn one_notch_scrolls_the_configured_number_of_lines() {
        let mut screen = positioned(screen(), 0, 10);
        let mut scrolling = scrolling(3, 0);
        scrolling.handle_mouse_wheel(true, false, WHEEL_DELTA as i16, false, &mut screen);
        assert_eq!(screen.viewport().origin().y, 7);
    }

    #[test]
    fn a_partial_notch_accumulates_until_it_is_worth_a_line() {
        let mut screen = positioned(screen(), 0, 10);
        let mut scrolling = scrolling(3, 0);

        scrolling.handle_mouse_wheel(true, false, 20, false, &mut screen);
        assert_eq!(screen.viewport().origin().y, 10);
        assert_eq!(screen.wheel_delta(), 20);

        scrolling.handle_mouse_wheel(true, false, 20, false, &mut screen);
        assert_eq!(screen.viewport().origin().y, 9);
        assert_eq!(screen.wheel_delta(), 0);
    }

    #[test]
    fn changing_direction_discards_the_remainder() {
        let mut screen = positioned(screen(), 0, 10);
        let mut scrolling = scrolling(3, 0);

        scrolling.handle_mouse_wheel(true, false, 20, false, &mut screen);
        assert_eq!(screen.wheel_delta(), 20);

        scrolling.handle_mouse_wheel(true, false, -20, false, &mut screen);
        assert_eq!(screen.wheel_delta(), -20);
        assert_eq!(screen.viewport().origin().y, 10);
    }

    #[test]
    fn shift_scrolls_half_a_screen() {
        let mut screen = positioned(screen(), 0, 10);
        let mut scrolling = scrolling(3, 0);
        scrolling.handle_mouse_wheel(true, false, WHEEL_DELTA as i16, true, &mut screen);
        assert_eq!(screen.viewport().origin().y, 5);
    }

    #[test]
    fn the_viewport_stops_at_the_buffer_ends() {
        let mut screen = positioned(screen(), 0, 0);
        let mut scrolling = scrolling(3, 0);

        // A wheel towards the start at the top does not move.
        scrolling.handle_mouse_wheel(true, false, WHEEL_DELTA as i16, false, &mut screen);
        assert_eq!(screen.viewport().origin().y, 0);

        // A wheel towards the end at the bottom does not move either.
        screen.set_viewport(
            types::Viewport::from_origin_size(Point { x: 0, y: 40 }, utils::Size::new(20, 10)),
            false,
        );
        for _ in 0..3 {
            scrolling.handle_mouse_wheel(true, false, -(WHEEL_DELTA as i16), false, &mut screen);
        }
        assert_eq!(screen.viewport().origin().y, 40);
    }

    #[test]
    fn the_horizontal_wheel_scrolls_columns() {
        let mut screen = narrow(screen(), 5);
        let mut scrolling = scrolling(0, 3);

        scrolling.handle_mouse_wheel(false, true, WHEEL_DELTA as i16, false, &mut screen);
        assert_eq!(screen.viewport().origin().x, 8);
        scrolling.handle_mouse_wheel(false, true, -(WHEEL_DELTA as i16), false, &mut screen);
        assert_eq!(screen.viewport().origin().x, 5);
    }

    #[test]
    fn escape_enter_and_ctrl_c_leave_scroll_mode() {
        let mut screen = positioned(screen(), 0, 10);
        let mut scrolling = Scrolling::new();

        for virtual_key in [VK_ESCAPE, VK_RETURN] {
            scrolling.do_scroll();
            assert_eq!(
                scrolling.handle_key_scrolling_event(&key(virtual_key, false), true, &mut screen),
                KeyScrollEvent::Handled
            );
            assert!(!scrolling.is_in_scroll_mode());
        }

        scrolling.do_scroll();
        assert_eq!(
            scrolling.handle_key_scrolling_event(&key(u16::from(b'C'), true), true, &mut screen),
            KeyScrollEvent::Handled
        );
        assert!(!scrolling.is_in_scroll_mode());
    }

    #[test]
    fn the_arrow_keys_scroll_one_line() {
        let mut screen = positioned(screen(), 0, 10);
        let mut scrolling = Scrolling::new();

        scrolling.handle_key_scrolling_event(&key(VK_UP, false), true, &mut screen);
        assert_eq!(screen.viewport().origin().y, 9);
        scrolling.handle_key_scrolling_event(&key(VK_DOWN, false), true, &mut screen);
        assert_eq!(screen.viewport().origin().y, 10);
    }

    #[test]
    fn the_page_keys_scroll_a_screen() {
        let mut screen = positioned(screen(), 0, 10);
        let mut scrolling = Scrolling::new();

        scrolling.handle_key_scrolling_event(&key(VK_NEXT, false), true, &mut screen);
        assert_eq!(screen.viewport().origin().y, 20);
        scrolling.handle_key_scrolling_event(&key(VK_PRIOR, false), true, &mut screen);
        assert_eq!(screen.viewport().origin().y, 10);
    }

    #[test]
    fn home_and_end_scroll_horizontally_without_ctrl() {
        let mut screen = narrow(screen(), 5);
        let mut scrolling = Scrolling::new();

        scrolling.handle_key_scrolling_event(&key(VK_END, false), true, &mut screen);
        assert_eq!(screen.viewport().origin().x, 10);
        scrolling.handle_key_scrolling_event(&key(VK_HOME, false), true, &mut screen);
        assert_eq!(screen.viewport().origin().x, 0);
    }

    #[test]
    fn ctrl_home_scrolls_to_the_top_when_the_line_is_empty() {
        let mut screen = positioned(screen(), 0, 20);
        let mut scrolling = Scrolling::new();

        assert_eq!(
            scrolling.handle_key_scrolling_event(&key(VK_HOME, true), true, &mut screen),
            KeyScrollEvent::Handled
        );
        assert_eq!(screen.viewport().origin().y, 0);
    }

    #[test]
    fn ctrl_home_is_left_to_the_editor_when_the_line_has_text() {
        let mut screen = positioned(screen(), 0, 20);
        let mut scrolling = Scrolling::new();

        assert_eq!(
            scrolling.handle_key_scrolling_event(&key(VK_HOME, true), false, &mut screen),
            KeyScrollEvent::NotHandled
        );
        assert_eq!(screen.viewport().origin().y, 20);
    }

    #[test]
    fn ctrl_end_shows_the_cursor_when_the_line_is_empty() {
        let mut screen = positioned(screen(), 0, 20);
        let mut scrolling = Scrolling::new();

        assert_eq!(
            scrolling.handle_key_scrolling_event(&key(VK_END, true), true, &mut screen),
            KeyScrollEvent::Handled
        );
        assert_eq!(screen.viewport().origin().y, 0);
    }

    #[test]
    fn modifier_keys_are_consumed_without_moving() {
        let mut screen = positioned(screen(), 0, 10);
        let mut scrolling = Scrolling::new();

        for virtual_key in [VK_SHIFT, VK_CONTROL, VK_MENU] {
            assert_eq!(
                scrolling.handle_key_scrolling_event(&key(virtual_key, false), true, &mut screen),
                KeyScrollEvent::Handled
            );
        }
        assert_eq!(screen.viewport().origin().y, 10);
    }

    #[test]
    fn an_unhandled_key_asks_for_a_beep() {
        let mut screen = positioned(screen(), 0, 10);
        let mut scrolling = Scrolling::new();

        assert_eq!(
            scrolling.handle_key_scrolling_event(&key(u16::from(b'X'), false), true, &mut screen),
            KeyScrollEvent::Beep
        );
    }
}
