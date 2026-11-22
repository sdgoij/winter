//! Selection input — the keys that move, extend and end a selection.
//!
//! Ported from `src/host/selectionInput.cpp`: `HandleKeySelectionEvent`, the
//! line-selection keys (`HandleKeyboardLineSelectionEvent`), mark-mode
//! navigation (`_HandleMarkModeSelectionNav`), `WordByWordSelection` and the
//! boundary helpers beside them. The state they move is
//! [`SelectionState`](crate::selection_state); the region operations are in
//! [`selection`](crate::selection).
//!
//! Not ported: `_HandleColorSelection`, which needs `ColorSelection` and the
//! buffer search and is only reachable when `Settings::enable_color_selection`
//! is on. With the setting off — its default — upstream takes the same path as
//! this module, so a numeric key falls through.
//!
//! ## Coordinates
//!
//! Upstream selects in buffer coordinates and reads cells by them. This port's
//! `TextBuffer` addresses screen lines and keeps its scrollback in a separate
//! queue, so the caret and the selection here use the same screen-line space.
//! A selection can therefore reach the scrollback extent the screen reports,
//! but text lookups (`is_word_delim_at`, the wide-character guards) see only
//! the screen lines the buffer can address.

use crate::cmdline::is_word_delim_text;
use crate::console_information::ConsoleInformation;
use crate::input_key_info::{InputKeyInfo, is_system_key};
use crate::screen_info::ScreenInformation;
use crate::scrolling::Scrolling;
use crate::selection_state::{SelectionFlags, SelectionState};
use types::Viewport;
use types::virtual_key::{
    VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_INSERT, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_UP,
};
use utils::{InclusiveRect, Point};

/// What the caller should do with a key the selection saw.
///
/// Ported from `Selection::KeySelectionEventResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySelectionEventResult {
    /// The key means nothing to the selection.
    EventNotHandled,
    /// The selection handled the key; the console must not see it.
    EventHandled,
    /// The caller should copy the selection to the clipboard.
    CopyToClipboard,
}

impl SelectionState {
    /// Handles a key while the console is selecting.
    ///
    /// Escape cancels, Enter and Ctrl+C/Ctrl+Insert ask for a copy, mark mode
    /// navigates with the arrows, and a mouse selection is cancelled by any key
    /// that is not a system key.
    pub fn handle_key_selection_event(
        &mut self,
        console: &mut ConsoleInformation,
        screen: &mut ScreenInformation,
        scrolling: &mut Scrolling,
        key: &InputKeyInfo,
    ) -> KeySelectionEventResult {
        let virtual_key = key.virtual_key();

        // While the button is down the mouse owns the selection, so only the
        // mouse-up may end it.
        if !self.is_mouse_button_down() {
            if virtual_key == VK_ESCAPE {
                self.clear_selection(console, screen, false);
                return KeySelectionEventResult::EventHandled;
            }

            // C-c, C-Ins; Ctrl+Shift+C is the same key record.
            if virtual_key == VK_RETURN
                || (key.is_ctrl_pressed() && (virtual_key == u16::from(b'C') || virtual_key == VK_INSERT))
            {
                return KeySelectionEventResult::CopyToClipboard;
            }
        }

        if !self.is_mouse_initiated_selection() {
            if self.handle_mark_mode_selection_nav(console, screen, key) {
                return KeySelectionEventResult::EventHandled;
            }
        } else if !self.is_mouse_button_down() {
            if self.is_line_selection() && self.handle_keyboard_line_selection_event(console, screen, scrolling, key) {
                return KeySelectionEventResult::EventHandled;
            }

            // Pressing an ordinary key ends a mouse selection; a modifier or a
            // lock toggle alone does not.
            if !is_system_key(virtual_key) {
                self.clear_selection(console, screen, false);
            }
        }

        KeySelectionEventResult::EventNotHandled
    }

    /// Handles a key that extends a line-mode selection.
    ///
    /// A valid key with the console not selecting starts a one-cell line
    /// selection at the cursor. Returns whether the key was handled.
    ///
    /// The keyboard cases must stay in step with
    /// [`is_valid_keyboard_line_selection`].
    pub fn handle_keyboard_line_selection_event(
        &mut self,
        console: &mut ConsoleInformation,
        screen: &mut ScreenInformation,
        scrolling: &mut Scrolling,
        key: &InputKeyInfo,
    ) -> bool {
        let virtual_key = key.virtual_key();
        if !is_valid_keyboard_line_selection(key) {
            return false;
        }

        if !self.is_in_selecting_state(console) {
            let position = screen.cursor_position();
            self.initialize_mouse_selection(console, scrolling, position, key.is_alt_pressed());
            self.align_alternate_selection(true);
            self.set_visible(true);

            // Shift+Left/Right started the selection; they do not also extend it.
            if key.is_shift_only() && matches!(virtual_key, VK_LEFT | VK_RIGHT) {
                return true;
            }
        }

        let anchor = self.selection_anchor();
        let rectangle = self.selection_rectangle();

        // The selection point is the corner opposite the anchor.
        let mut selection_point = crate::utils::get_opposite_corner(rectangle, anchor);

        let buffer_size = screen.buffer_size();
        let window_height = screen.viewport().height();
        debug_assert!(buffer_size.contains(selection_point));

        let input_line = input_line_boundaries(console);

        if key.is_shift_only() {
            match virtual_key {
                // One cell, wrapping at the edges of the buffer.
                VK_LEFT => {
                    buffer_size.decrement_in_bounds(&mut selection_point);
                }
                VK_RIGHT => {
                    buffer_size.increment_in_bounds(&mut selection_point);
                    // Do not split a wide character in half.
                    if is_trailing_half(screen, selection_point) {
                        buffer_size.increment_in_bounds(&mut selection_point);
                    }
                }
                // One row, stopping at the top or bottom of the buffer.
                VK_UP => {
                    if selection_point.y > buffer_size.top() {
                        selection_point.y -= 1;
                    }
                }
                VK_DOWN => {
                    if selection_point.y < buffer_size.bottom_inclusive() {
                        selection_point.y += 1;
                    }
                }
                // One screen at a time.
                VK_NEXT => {
                    selection_point.y = selection_point
                        .y
                        .saturating_add(window_height)
                        .min(buffer_size.bottom_inclusive());
                }
                VK_PRIOR => {
                    selection_point.y = selection_point.y.saturating_sub(window_height).max(buffer_size.top());
                }
                VK_HOME => {
                    let mut handled = false;
                    if let Some((input_start, _)) = input_line {
                        // Inside the input line, stop on its first character
                        // first; a second press then takes the prompt too.
                        if buffer_size.compare_in_bounds(selection_point, input_start) > 0
                            && input_start.y == selection_point.y
                        {
                            selection_point.x = input_start.x;
                            handled = true;
                        }
                    }
                    if !handled {
                        selection_point.x = buffer_size.left();
                    }
                }
                VK_END => {
                    let mut handled = false;
                    if let Some((input_start, input_end)) = input_line {
                        if buffer_size.compare_in_bounds(selection_point, input_start) >= 0 {
                            // Inside the input area: stop on its last character,
                            // not on the cursor cell past it.
                            if input_end.y == selection_point.y && selection_point.x < input_end.x {
                                selection_point.x = input_end.x;
                                handled = true;
                            }
                        } else if input_start.y == selection_point.y {
                            // Outside the input but on its first line: stop just
                            // before the input, then jump to its end next time.
                            let end_of_output = input_start.x - 1;
                            if selection_point.x < end_of_output {
                                selection_point.x = end_of_output;
                                handled = true;
                            } else if selection_point.x == end_of_output && selection_point.y == input_end.y {
                                selection_point.x = input_end.x;
                                handled = true;
                            }
                        }
                    }
                    if !handled {
                        selection_point.x = buffer_size.right_inclusive();
                    }
                }
                _ => {}
            }
        } else if key.is_shift_and_ctrl_only() {
            match virtual_key {
                VK_LEFT => {
                    selection_point =
                        Self::word_by_word_selection(true, buffer_size, anchor, selection_point, console, screen);
                }
                VK_RIGHT => {
                    selection_point =
                        Self::word_by_word_selection(false, buffer_size, anchor, selection_point, console, screen);
                }
                // Ctrl does not change the vertical steps.
                VK_UP => {
                    if selection_point.y > buffer_size.top() {
                        selection_point.y -= 1;
                    }
                }
                VK_DOWN => {
                    if selection_point.y < buffer_size.bottom_inclusive() {
                        selection_point.y += 1;
                    }
                }
                // Ctrl+Home/End reach the whole valid area.
                VK_HOME => {
                    let (start, _) = self.valid_area_boundaries(console, screen);
                    selection_point = start;
                }
                VK_END => {
                    let (_, end) = self.valid_area_boundaries(console, screen);
                    selection_point = end;
                }
                _ => {}
            }
        }

        // Never plant the selection point on the trailing half of a wide
        // character: prefer its lead, then the next cell.
        if is_trailing_half(screen, selection_point) && !buffer_size.decrement_in_bounds(&mut selection_point) {
            buffer_size.increment_in_bounds(&mut selection_point);
        }

        self.extend_selection(selection_point, screen);
        true
    }

    /// Returns the point one word away from `selection_point`.
    ///
    /// Moves in reading order, or in reverse when `reverse` is set, stopping on
    /// the first character after a run of delimiters. A move that would leave
    /// the buffer stops at the edge.
    pub fn word_by_word_selection(
        reverse: bool,
        buffer_size: Viewport,
        anchor: Point,
        selection_point: Point,
        console: &ConsoleInformation,
        screen: &ScreenInformation,
    ) -> Point {
        let mut out = selection_point;

        // Step onto the first character to inspect.
        if reverse {
            buffer_size.decrement_in_bounds(&mut out);
        } else {
            buffer_size.increment_in_bounds(&mut out);
        }

        let mut current_is_delim = is_word_delim_at(screen, out);

        // Inside the edit line movement is sticky: it stops at the line's ends
        // so a repeated press is needed to leave it. Without a pending read the
        // whole buffer is the bound.
        let (max_left, max_right) = match input_line_boundaries(console) {
            Some((start, end)) => (start, end),
            None => (
                Point::new(buffer_size.left(), buffer_size.top()),
                Point::new(buffer_size.right_inclusive(), buffer_size.bottom_inclusive()),
            ),
        };

        // Moving away from the anchor adds to the selection; moving back over
        // already-selected text takes away from it, and the stopping cell must
        // stay selected.
        let unhighlighting = if reverse {
            buffer_size.compare_in_bounds(out, anchor) > 0
        } else {
            buffer_size.compare_in_bounds(out, anchor) < 0
        };

        let move_succeeded = 'walk: loop {
            let previous_is_delim = current_is_delim;

            if buffer_size.compare_in_bounds(out, max_left) == 0 {
                // At the left edge there is nowhere left to move.
                break 'walk false;
            }
            // The right bound is exclusive of the cell past it.
            if buffer_size.compare_in_bounds(out, max_right) >= 0 {
                break 'walk false;
            }

            let moved = if reverse {
                buffer_size.decrement_in_bounds(&mut out)
            } else {
                buffer_size.increment_in_bounds(&mut out)
            };
            if !moved {
                break 'walk false;
            }

            current_is_delim = is_word_delim_at(screen, out);

            // Forward: stop on a delimiter-to-character change. Reverse: stop
            // on a character-to-delimiter change.
            let stop = if reverse {
                !previous_is_delim && current_is_delim
            } else {
                previous_is_delim && !current_is_delim
            };
            if stop {
                break 'walk true;
            }
        };

        // The loop had to step one cell too far to see the change. Step back so
        // the stop cell is included, unless it hit the buffer edge or is being
        // unhighlighted.
        if move_succeeded && !unhighlighting {
            if reverse {
                buffer_size.increment_in_bounds(&mut out);
            } else {
                buffer_size.decrement_in_bounds(&mut out);
            }
        }

        out
    }

    /// Returns the first and last valid cells of the text on the screen.
    ///
    /// The last cell is the pending input line's end when there is one, else
    /// the saved cursor of a mark selection, else the cursor.
    pub fn valid_area_boundaries(&self, console: &ConsoleInformation, screen: &ScreenInformation) -> (Point, Point) {
        let end = match input_line_boundaries(console) {
            Some((_, end)) => end,
            None => {
                if self.is_in_selecting_state(console) && self.is_keyboard_mark_selection() {
                    self.saved_cursor_position().unwrap_or(Point::new(0, 0))
                } else {
                    screen.cursor_position()
                }
            }
        };

        (Point::new(0, 0), end)
    }

    /// Moves the mark-mode cursor with the arrow keys.
    ///
    /// With Shift held the move extends the selection; otherwise it resets the
    /// anchor to the cursor and starts again.
    fn handle_mark_mode_selection_nav(
        &mut self,
        console: &mut ConsoleInformation,
        screen: &mut ScreenInformation,
        key: &InputKeyInfo,
    ) -> bool {
        let virtual_key = key.virtual_key();
        if !matches!(
            virtual_key,
            VK_RIGHT | VK_LEFT | VK_UP | VK_DOWN | VK_NEXT | VK_PRIOR | VK_END | VK_HOME
        ) {
            return false;
        }

        let (cursor_row, cursor_column) = screen.text_buffer().cursor().position();
        let mut cursor = Point::new(cursor_column, cursor_row);

        // How far one press moves sideways. A wide character is stepped over
        // whole, and a cell that is the trailing half of one is skipped back
        // past both halves.
        let mut next_right = 1;
        let mut next_left = 0;
        if is_wide_lead(screen, cursor) {
            next_right = 2;
        }
        if cursor.x > 0 {
            let left = Point::new(cursor.x - 1, cursor.y);
            if is_trailing_half(screen, left) {
                next_left = 2;
            } else if is_wide_lead(screen, left) {
                if cursor.x - 1 > 0 {
                    next_left = if is_trailing_half(screen, Point::new(cursor.x - 2, cursor.y)) {
                        3
                    } else {
                        2
                    };
                } else {
                    next_left = 1;
                }
            } else {
                next_left = 1;
            }
        }

        let buffer_size = screen.buffer_size();
        let terminal_size = screen.terminal_buffer_size();
        let viewport_height = screen.viewport().height().max(1);

        match virtual_key {
            VK_RIGHT => {
                if cursor.x + next_right < buffer_size.width() {
                    cursor.x += next_right;
                }
            }
            VK_LEFT => {
                if cursor.x > 0 {
                    cursor.x = (cursor.x - next_left).max(0);
                }
            }
            VK_UP => {
                if cursor.y > 0 {
                    cursor.y -= 1;
                }
            }
            VK_DOWN => {
                if cursor.y + 1 < terminal_size.height() {
                    cursor.y += 1;
                }
            }
            VK_NEXT => {
                cursor.y = cursor
                    .y
                    .saturating_add(viewport_height - 1)
                    .min(terminal_size.height() - 1);
            }
            VK_PRIOR => {
                cursor.y = cursor.y.saturating_sub(viewport_height - 1).max(0);
            }
            VK_END => {
                // End is the end of the line; Ctrl+End is the end of the buffer.
                cursor.x = buffer_size.right_inclusive();
                if key.is_ctrl_pressed() {
                    cursor.y = self.valid_area_boundaries(console, screen).1.y;
                }
            }
            VK_HOME => {
                // Home is the start of the line; Ctrl+Home is the start of the
                // buffer.
                cursor.x = buffer_size.left();
                if key.is_ctrl_pressed() {
                    cursor.y = buffer_size.top();
                }
            }
            _ => return false,
        }

        screen.text_buffer_mut().cursor_mut().set_position(cursor.y, cursor.x);

        if key.is_shift_pressed() {
            // The first extension chooses line or block form from the Alt key,
            // as a mouse selection would.
            if !self.is_area_selected() {
                self.check_and_set_alternate_selection(key.is_alt_pressed());
            }
            self.extend_selection(cursor, screen);
        } else {
            if self.is_area_selected() {
                self.set_visible(false);
                self.remove_flags(SelectionFlags::NOT_EMPTY);
                self.set_use_alternate_selection(false);
            }

            self.set_anchor(cursor);
            screen.make_cursor_visible(cursor);
            self.set_rectangle(InclusiveRect::new(cursor.x, cursor.y, cursor.x, cursor.y));
        }

        true
    }
}

/// Returns whether `key` is one of the combinations a line selection accepts.
///
/// Kept in step with [`SelectionState::handle_keyboard_line_selection_event`].
pub fn is_valid_keyboard_line_selection(key: &InputKeyInfo) -> bool {
    let virtual_key = key.virtual_key();
    if key.is_shift_only() {
        matches!(
            virtual_key,
            VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_NEXT | VK_PRIOR | VK_HOME | VK_END
        )
    } else if key.is_shift_and_ctrl_only() {
        matches!(virtual_key, VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END)
    } else {
        false
    }
}

/// Returns whether `position` lies between `start` and `end`, edges included.
pub fn is_within_boundaries(view: Viewport, position: Point, start: Point, end: Point) -> bool {
    view.compare_in_bounds(start, position) <= 0 && view.compare_in_bounds(position, end) <= 0
}

/// Returns the pending cooked read's first and last cells.
///
/// Upstream asks the read's pager where it placed the edit line. The pager
/// belongs to the server layer and is not ported, so there is no pair to
/// report and every caller takes upstream's "no input line" path.
pub(crate) fn input_line_boundaries(_console: &ConsoleInformation) -> Option<(Point, Point)> {
    None
}

/// Returns whether the cell at `position` is the leading half of a wide
/// character.
fn is_wide_lead(screen: &ScreenInformation, position: Point) -> bool {
    screen
        .text_buffer()
        .get_cell_data_at(position.y, position.x)
        .is_some_and(|cell| cell.is_wide_lead())
}

/// Returns whether the cell at `position` is the trailing half of a wide
/// character.
fn is_trailing_half(screen: &ScreenInformation, position: Point) -> bool {
    screen
        .text_buffer()
        .get_row_at_screen_line(position.y)
        .is_some_and(|row| row.is_trailing_half(position.x))
}

/// Returns whether the cell at `position` is a word delimiter.
///
/// A blank cell is a space in the console model, and upstream's buffer stores a
/// space in it; this buffer stores empty text, so an empty cell is a delimiter
/// here too.
fn is_word_delim_at(screen: &ScreenInformation, position: Point) -> bool {
    match screen.text_buffer().get_cell_data_at(position.y, position.x) {
        Some(cell) if cell.is_empty() => true,
        Some(cell) => is_word_delim_text(cell.chars()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console_information::ConsoleFlags;
    use types::{ControlKeyState, TextAttribute};

    const NONE: ControlKeyState = ControlKeyState {
        left_alt: false,
        right_alt: false,
        left_ctrl: false,
        right_ctrl: false,
        shift: false,
        num_lock: false,
        scroll_lock: false,
        caps_lock: false,
        enhanced: false,
    };

    fn key(virtual_key: u16, shift: bool, ctrl: bool, alt: bool) -> InputKeyInfo {
        InputKeyInfo::new(
            virtual_key,
            ControlKeyState {
                shift,
                left_ctrl: ctrl,
                left_alt: alt,
                ..NONE
            },
        )
    }

    fn screen(width: i32, height: i32) -> ScreenInformation {
        ScreenInformation::new(width, height, 0, TextAttribute::new(), TextAttribute::new())
    }

    fn selecting_screen() -> ScreenInformation {
        let mut screen = screen(20, 10);
        assert!(screen.set_cursor_position(Point::new(3, 2)));
        screen
    }

    fn mark_selection(
        console: &mut ConsoleInformation,
        screen: &mut ScreenInformation,
        scrolling: &mut Scrolling,
    ) -> SelectionState {
        let mut selection = SelectionState::new();
        selection.initialize_mark_selection(console, screen, scrolling);
        selection
    }

    #[test]
    fn escape_cancels_the_selection() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = mark_selection(&mut console, &mut screen, &mut scrolling);

        let result = selection.handle_key_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(VK_ESCAPE, false, false, false),
        );

        assert_eq!(result, KeySelectionEventResult::EventHandled);
        assert!(!console.flags().contains(ConsoleFlags::SELECTING));
        assert!(!selection.is_visible());
    }

    #[test]
    fn return_and_ctrl_c_ask_for_a_copy() {
        let cases = [
            key(VK_RETURN, false, false, false),
            key(u16::from(b'C'), false, true, false),
            key(u16::from(b'C'), true, true, false),
            key(VK_INSERT, false, true, false),
        ];

        for case in cases {
            let mut console = ConsoleInformation::new();
            let mut screen = selecting_screen();
            let mut scrolling = Scrolling::new();
            let mut selection = mark_selection(&mut console, &mut screen, &mut scrolling);

            assert_eq!(
                selection.handle_key_selection_event(&mut console, &mut screen, &mut scrolling, &case),
                KeySelectionEventResult::CopyToClipboard,
                "expected {case:?} to ask for a copy"
            );
            // A copy request leaves the selection in place.
            assert!(console.flags().contains(ConsoleFlags::SELECTING));
        }
    }

    #[test]
    fn an_unmodified_c_is_not_a_copy() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = mark_selection(&mut console, &mut screen, &mut scrolling);

        let result = selection.handle_key_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(u16::from(b'C'), false, false, false),
        );

        assert_eq!(result, KeySelectionEventResult::EventNotHandled);
        assert!(console.flags().contains(ConsoleFlags::SELECTING));
    }

    #[test]
    fn an_ordinary_key_cancels_a_mouse_selection() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();
        selection.initialize_mouse_selection(&mut console, &mut scrolling, Point::new(1, 1), false);

        let result = selection.handle_key_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(u16::from(b'a'), false, false, false),
        );

        assert_eq!(result, KeySelectionEventResult::EventNotHandled);
        assert!(!console.flags().contains(ConsoleFlags::SELECTING));
    }

    #[test]
    fn a_system_key_keeps_a_mouse_selection_alive() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();
        selection.initialize_mouse_selection(&mut console, &mut scrolling, Point::new(1, 1), false);

        let result = selection.handle_key_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(types::virtual_key::VK_SHIFT, true, false, false),
        );

        assert_eq!(result, KeySelectionEventResult::EventNotHandled);
        assert!(console.flags().contains(ConsoleFlags::SELECTING));
    }

    #[test]
    fn mark_mode_moves_the_cursor_with_the_arrows() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = mark_selection(&mut console, &mut screen, &mut scrolling);

        assert!(selection.handle_mark_mode_selection_nav(
            &mut console,
            &mut screen,
            &key(VK_RIGHT, false, false, false)
        ));
        assert_eq!(screen.cursor_position(), Point::new(4, 2));

        assert!(selection.handle_mark_mode_selection_nav(
            &mut console,
            &mut screen,
            &key(VK_DOWN, false, false, false)
        ));
        assert_eq!(screen.cursor_position(), Point::new(4, 3));

        assert!(selection.handle_mark_mode_selection_nav(
            &mut console,
            &mut screen,
            &key(VK_LEFT, false, false, false)
        ));
        assert_eq!(screen.cursor_position(), Point::new(3, 3));

        assert!(selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_UP, false, false, false)));
        assert_eq!(screen.cursor_position(), Point::new(3, 2));
    }

    #[test]
    fn mark_mode_stops_at_the_buffer_edges() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = mark_selection(&mut console, &mut screen, &mut scrolling);

        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_UP, false, false, false));
        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_UP, false, false, false));
        assert_eq!(screen.cursor_position(), Point::new(3, 0));

        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_HOME, false, false, false));
        assert_eq!(screen.cursor_position(), Point::new(0, 0));

        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_END, false, false, false));
        assert_eq!(screen.cursor_position(), Point::new(19, 0));

        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_DOWN, false, false, false));
        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_NEXT, false, false, false));
        assert_eq!(screen.cursor_position(), Point::new(19, 9));
    }

    #[test]
    fn ctrl_home_and_ctrl_end_reach_the_valid_area() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = mark_selection(&mut console, &mut screen, &mut scrolling);

        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_END, false, true, false));
        // A mark selection's valid end is the cursor it saved, (3, 2).
        assert_eq!(screen.cursor_position(), Point::new(19, 2));

        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_HOME, false, true, false));
        assert_eq!(screen.cursor_position(), Point::new(0, 0));
    }

    #[test]
    fn shift_extends_the_mark_selection() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = mark_selection(&mut console, &mut screen, &mut scrolling);

        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_RIGHT, true, false, false));
        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_RIGHT, true, false, false));

        assert_eq!(screen.cursor_position(), Point::new(5, 2));
        assert!(selection.is_area_selected());
        assert!(selection.is_visible());
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(3, 2, 5, 2));
    }

    #[test]
    fn releasing_shift_resets_the_anchor() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = mark_selection(&mut console, &mut screen, &mut scrolling);

        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_RIGHT, true, false, false));
        assert!(selection.is_area_selected());

        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_RIGHT, false, false, false));

        assert!(!selection.is_area_selected());
        assert!(!selection.is_visible());
        assert_eq!(selection.selection_anchor(), Point::new(5, 2));
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(5, 2, 5, 2));
    }

    #[test]
    fn shift_arrow_starts_a_line_selection_without_extending_it() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        assert!(selection.handle_keyboard_line_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(VK_RIGHT, true, false, false),
        ));

        assert!(console.flags().contains(ConsoleFlags::SELECTING));
        assert!(selection.is_line_selection());
        assert_eq!(selection.selection_anchor(), Point::new(3, 2));
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(3, 2, 3, 2));
        // The second press extends the one-cell selection.
        assert!(selection.handle_keyboard_line_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(VK_RIGHT, true, false, false),
        ));
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(3, 2, 4, 2));
    }

    #[test]
    fn shift_home_and_end_reach_the_line_edges() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        selection.handle_keyboard_line_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(VK_END, true, false, false),
        );
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(3, 2, 19, 2));

        // The anchor stays where the selection started, so moving home pulls
        // the other corner back to the line start.
        selection.handle_keyboard_line_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(VK_HOME, true, false, false),
        );
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(0, 2, 3, 2));
    }

    #[test]
    fn ctrl_shift_end_reaches_the_last_valid_row() {
        let mut console = ConsoleInformation::new();
        let mut screen = selecting_screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        selection.handle_keyboard_line_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(VK_END, true, true, false),
        );
        // The start press leaves the point at the cursor; the next one moves.
        selection.handle_keyboard_line_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(VK_END, true, true, false),
        );

        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(3, 2, 3, 2));
    }

    #[test]
    fn invalid_combinations_are_not_line_selection_keys() {
        assert!(!is_valid_keyboard_line_selection(&key(VK_LEFT, false, true, false)));
        assert!(!is_valid_keyboard_line_selection(&key(VK_LEFT, false, false, true)));
        assert!(!is_valid_keyboard_line_selection(&key(VK_LEFT, false, false, false)));
        assert!(is_valid_keyboard_line_selection(&key(VK_LEFT, true, false, false)));
        assert!(is_valid_keyboard_line_selection(&key(VK_LEFT, true, true, false)));
        // Page keys are line-selection keys only without Ctrl.
        assert!(!is_valid_keyboard_line_selection(&key(VK_PRIOR, true, true, false)));
    }

    fn word_screen(text: &str) -> ScreenInformation {
        let mut screen = screen(80, 10);
        screen.text_buffer_mut().write_string(text);
        screen.text_buffer_mut().cursor_mut().set_position(0, 0);
        screen
    }

    #[test]
    fn word_by_word_previous_walks_back_over_words() {
        let text = "this is some test text.";
        let screen = word_screen(text);
        let console = ConsoleInformation::new();
        let buffer_size = screen.buffer_size();
        let characters: Vec<char> = text.chars().collect();

        let anchor = Point::new(characters.len() as i32, 0);
        let mut point = anchor;

        loop {
            let mut expected = point;
            // The walk steps off the current cell first, so at least one cell
            // is consumed even when the point sits just after a word.
            expected.x -= 1;
            while expected.x > 0 && characters[(expected.x - 1) as usize] != ' ' {
                expected.x -= 1;
            }

            point = SelectionState::word_by_word_selection(true, buffer_size, anchor, point, &console, &screen);
            assert_eq!(point, expected);

            if point.x == 0 {
                break;
            }
        }
    }

    #[test]
    fn word_by_word_next_walks_forward_over_words() {
        let text = "this is some test text.";
        let screen = word_screen(text);
        let console = ConsoleInformation::new();
        let buffer_size = screen.buffer_size();
        let characters: Vec<char> = text.chars().collect();

        let anchor = Point::new(0, 0);
        let mut point = anchor;
        let right = characters.len() as i32;

        loop {
            let mut expected = point;
            while expected.x + 1 < right && characters[(expected.x + 1) as usize] != ' ' {
                expected.x += 1;
            }
            expected.x += 1;

            // Past the text the walk continues over blank cells to the end.
            if expected.x >= right {
                expected = Point::new(buffer_size.right_inclusive(), buffer_size.bottom_inclusive());
            }

            point = SelectionState::word_by_word_selection(false, buffer_size, anchor, point, &console, &screen);
            assert_eq!(point, expected);

            if point.y >= buffer_size.bottom_inclusive() {
                break;
            }
        }
    }

    #[test]
    fn shift_right_skips_the_trailing_half_of_a_wide_character() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen(20, 10);
        screen.text_buffer_mut().write_string("日a");
        assert!(screen.set_cursor_position(Point::new(0, 0)));
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        // Start the selection, then extend past the wide character.
        selection.handle_keyboard_line_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(VK_RIGHT, true, false, false),
        );
        selection.handle_keyboard_line_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(VK_RIGHT, true, false, false),
        );
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(0, 0, 2, 0));

        // A point landing on the trailing half is moved back to the lead.
        selection.handle_keyboard_line_selection_event(
            &mut console,
            &mut screen,
            &mut scrolling,
            &key(VK_LEFT, true, false, false),
        );
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(0, 0, 0, 0));
    }

    #[test]
    fn mark_mode_steps_over_a_wide_character() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen(20, 10);
        screen.text_buffer_mut().write_string("日a");
        assert!(screen.set_cursor_position(Point::new(0, 0)));
        let mut scrolling = Scrolling::new();
        let mut selection = mark_selection(&mut console, &mut screen, &mut scrolling);

        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_RIGHT, false, false, false));
        assert_eq!(screen.cursor_position(), Point::new(2, 0));

        selection.handle_mark_mode_selection_nav(&mut console, &mut screen, &key(VK_LEFT, false, false, false));
        assert_eq!(screen.cursor_position(), Point::new(0, 0));
    }

    #[test]
    fn within_boundaries_includes_the_edges() {
        let view = Viewport::new(utils::Size::new(10, 5));
        assert!(is_within_boundaries(
            view,
            Point::new(0, 0),
            Point::new(0, 0),
            Point::new(9, 4)
        ));
        assert!(is_within_boundaries(
            view,
            Point::new(9, 4),
            Point::new(0, 0),
            Point::new(9, 4)
        ));
        assert!(!is_within_boundaries(
            view,
            Point::new(10, 4),
            Point::new(0, 0),
            Point::new(9, 4)
        ));
        assert!(!is_within_boundaries(
            view,
            Point::new(0, 0),
            Point::new(1, 0),
            Point::new(9, 4)
        ));
    }
}
