//! Selection operations — starting, extending and clearing a selection.
//!
//! Ported from `selection.cpp`. These are the methods upstream's `Selection`
//! exposes; the state itself is in
//! [`selection_state`](crate::selection_state) (ADR 014). Painting, the
//! accessibility notifications, the window title and the write block belong to
//! the layers that own them, so each operation is the state transition alone.
//!
//! `SelectAll` and `s_GetInputLineBoundaries` need the pending cooked read's
//! line, and the color-selection and mark-mode navigation halves are
//! `selectionInput.cpp`; they land with their consumers.

use crate::console_information::ConsoleInformation;
use crate::screen_info::ScreenInformation;
use crate::scrolling::Scrolling;
use crate::selection_input::{input_line_boundaries, is_within_boundaries};
use crate::selection_state::{SelectionFlags, SelectionState};
use crate::utils::{compare_coords, get_opposite_corner};
use utils::{InclusiveRect, Point};

impl SelectionState {
    /// Starts a mark-mode selection at the cursor.
    ///
    /// The cursor is saved and made a full box so it can be moved by the
    /// keyboard while it is the selection cursor.
    pub fn initialize_mark_selection(
        &mut self,
        console: &mut ConsoleInformation,
        screen: &mut ScreenInformation,
        scrolling: &mut Scrolling,
    ) {
        self.clear_selection(console, screen, true);
        scrolling.clear_scroll();

        self.set_selecting(console, true);
        self.set_flags(SelectionFlags::empty());

        let cursor = *screen.text_buffer().cursor();
        self.save_cursor(&cursor);

        let position = screen.cursor_position();
        screen.set_cursor_information(100, true);
        screen.set_cursor_position(position);

        self.set_anchor(position);
    }

    /// Starts a mouse selection at `position`, one cell wide.
    ///
    /// `alternate_down` is whether Alt was held: it flips line and block mode.
    pub fn initialize_mouse_selection(
        &mut self,
        console: &mut ConsoleInformation,
        scrolling: &mut Scrolling,
        position: Point,
        alternate_down: bool,
    ) {
        scrolling.clear_scroll();
        self.set_selecting(console, true);

        self.set_flags(SelectionFlags::MOUSE_SELECTION | SelectionFlags::NOT_EMPTY);
        self.set_anchor(position);
        self.set_rectangle(InclusiveRect::new(position.x, position.y, position.x, position.y));

        self.check_and_set_alternate_selection(alternate_down);
    }

    /// Flips to the other selection form when Alt is held.
    pub const fn check_and_set_alternate_selection(&mut self, alternate_down: bool) {
        self.set_use_alternate_selection(alternate_down);
    }

    /// Replaces both ends of the selection.
    ///
    /// A selection adjusted this way stops following later mouse movement, so a
    /// double-click's word selection is not overwritten by the drag that
    /// follows it.
    pub fn adjust_selection(&mut self, anchor: Point, end: Point, screen: &mut ScreenInformation) {
        self.set_anchor(anchor);
        self.extend_selection(end, screen);
        self.set_allow_mouse_drag_selection(false);
    }

    /// Extends the selection from its anchor to `position`.
    pub fn extend_selection(&mut self, position: Point, screen: &mut ScreenInformation) {
        self.set_allow_mouse_drag_selection(true);

        let buffer_size = screen.terminal_buffer_size().size();
        let position = Point {
            x: position.x.clamp(0, buffer_size.width() - 1),
            y: position.y.clamp(0, buffer_size.height() - 1),
        };

        if !self.is_area_selected() {
            // Only mark mode may start an area by extending; a mouse selection
            // without one is waiting for the button to move.
            if self.is_mouse_initiated_selection() {
                return;
            }

            screen.make_cursor_visible(position);
            self.insert_flags(SelectionFlags::NOT_EMPTY);
            let anchor = self.selection_anchor();
            self.set_rectangle(InclusiveRect::new(anchor.x, anchor.y, anchor.x, anchor.y));
            self.set_visible(true);
        } else {
            screen.make_cursor_visible(position);
        }

        let anchor = self.selection_anchor();
        let rectangle = InclusiveRect::new(
            position.x.min(anchor.x),
            position.y.min(anchor.y),
            position.x.max(anchor.x),
            position.y.max(anchor.y),
        );

        if rectangle != self.selection_rectangle() {
            self.set_rectangle(rectangle);
        }
    }

    /// Ends the selection and restores what it borrowed.
    ///
    /// `starting_new_selection` is upstream's `fStartingNewSelection`: the write
    /// block stays in place for a selection that begins immediately after.
    pub fn clear_selection(
        &mut self,
        console: &mut ConsoleInformation,
        screen: &mut ScreenInformation,
        starting_new_selection: bool,
    ) {
        if !self.is_in_selecting_state(console) {
            return;
        }

        if self.is_mouse_initiated_selection() {
            // A mouse selection has no saved cursor to restore.
            self.set_visible(false);
            self.set_selecting(console, false);
        } else {
            if self.is_area_selected() {
                self.set_visible(false);
            }
            self.set_selecting(console, false);
            self.restore_cursor(screen.text_buffer_mut().cursor_mut());
        }

        self.set_flags(SelectionFlags::empty());
        self.set_use_alternate_selection(false);

        // Upstream releases the console write block unless another selection is
        // about to start; that block belongs to the console lock here.
        let _ = starting_new_selection;
    }

    /// Clears any selection and selects the region from `start` to `end`.
    ///
    /// Ported from `SelectNewRegion`.
    pub fn select_new_region(
        &mut self,
        console: &mut ConsoleInformation,
        screen: &mut ScreenInformation,
        scrolling: &mut Scrolling,
        start: Point,
        end: Point,
    ) {
        self.clear_selection(console, screen, false);
        self.initialize_mouse_selection(console, scrolling, start, false);
        self.set_visible(true);
        self.extend_selection(end, screen);
    }

    /// Selects all valid text.
    ///
    /// Ported from `SelectAll`. Without a pending cooked read, "all" runs from
    /// the buffer origin to the saved cursor of a mark selection, or to the
    /// cursor otherwise. The viewport is put back where it was after the new
    /// selection has been shown.
    pub fn select_all(
        &mut self,
        console: &mut ConsoleInformation,
        screen: &mut ScreenInformation,
        scrolling: &mut Scrolling,
    ) {
        let window_origin = screen.viewport().origin();
        let old_selection_existed = self.is_area_selected();
        let old_rectangle = self.selection_rectangle();
        let old_anchor = self.selection_anchor();

        let (new_start, new_end) = match input_line_boundaries(console) {
            None => self.valid_area_boundaries(console, screen),
            Some((input_start, input_end)) => {
                let buffer_size = screen.buffer_size();
                let mut one_after_end = input_end;
                buffer_size.increment_in_bounds(&mut one_after_end);
                let cursor = screen.cursor_position();
                let within = |position| is_within_boundaries(buffer_size, position, input_start, input_end);

                if !old_selection_existed {
                    // The input line alone is selected when the cursor is on
                    // it or one cell past its end.
                    if within(cursor) || is_within_boundaries(buffer_size, cursor, one_after_end, one_after_end) {
                        (input_start, input_end)
                    } else {
                        self.valid_area_boundaries(console, screen)
                    }
                } else {
                    let old_opposite = get_opposite_corner(old_rectangle, old_anchor);
                    let old_within_input = within(old_anchor) && within(old_opposite);
                    let all_input_selected = (compare_coords(buffer_size.size(), input_start, old_anchor) == 0
                        && compare_coords(buffer_size.size(), input_end, old_opposite) == 0)
                        || (compare_coords(buffer_size.size(), input_start, old_opposite) == 0
                            && compare_coords(buffer_size.size(), input_end, old_anchor) == 0);

                    if old_within_input && !all_input_selected {
                        (input_start, input_end)
                    } else {
                        self.valid_area_boundaries(console, screen)
                    }
                }
            }
        };

        let (mut start, mut end) = (new_start, new_end);

        // A block selection spans the full width so it covers all the text.
        if !self.is_line_selection() {
            start.x = 0;
            end.x = screen.buffer_size().right_inclusive();
        }

        self.select_new_region(console, screen, scrolling, start, end);

        // Showing the selection may have scrolled the view; put it back.
        let _ = screen.set_viewport_origin(true, window_origin, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console_information::ConsoleFlags;
    use types::TextAttribute;

    fn screen() -> ScreenInformation {
        ScreenInformation::new(20, 10, 5, TextAttribute::new(), TextAttribute::new())
    }

    #[test]
    fn mark_selection_saves_the_cursor_and_anchors_at_it() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut scrolling = Scrolling::new();
        scrolling.do_scroll();
        let mut selection = SelectionState::new();

        assert!(screen.set_cursor_position(Point { x: 3, y: 2 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);

        assert!(console.flags().contains(ConsoleFlags::SELECTING));
        assert_eq!(selection.selection_anchor(), Point { x: 3, y: 2 });
        assert_eq!(screen.text_buffer().cursor().width(), 100);
        assert!(!scrolling.is_in_scroll_mode());
        assert!(selection.is_keyboard_mark_selection());
    }

    #[test]
    fn mouse_selection_starts_one_cell_wide() {
        let mut console = ConsoleInformation::new();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        selection.initialize_mouse_selection(&mut console, &mut scrolling, Point { x: 4, y: 3 }, false);

        assert!(console.flags().contains(ConsoleFlags::SELECTING));
        assert!(selection.is_mouse_initiated_selection());
        assert!(selection.is_area_selected());
        assert_eq!(selection.selection_anchor(), Point { x: 4, y: 3 });
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(4, 3, 4, 3));
        assert!(selection.is_line_selection());
    }

    #[test]
    fn alt_flips_the_selection_form() {
        let mut console = ConsoleInformation::new();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        selection.initialize_mouse_selection(&mut console, &mut scrolling, Point { x: 1, y: 1 }, true);
        assert!(!selection.is_line_selection());
    }

    #[test]
    fn extending_a_mark_selection_creates_the_area() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        assert!(screen.set_cursor_position(Point { x: 3, y: 2 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);
        selection.extend_selection(Point { x: 6, y: 5 }, &mut screen);

        assert!(selection.is_area_selected());
        assert!(selection.is_visible());
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(3, 2, 6, 5));
    }

    #[test]
    fn extending_clamps_into_the_terminal_buffer() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        assert!(screen.set_cursor_position(Point { x: 3, y: 2 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);
        selection.extend_selection(Point { x: 30, y: 40 }, &mut screen);

        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(3, 2, 19, 14));
    }

    #[test]
    fn adjusting_sets_both_ends_and_stops_the_drag() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        selection.initialize_mouse_selection(&mut console, &mut scrolling, Point { x: 0, y: 0 }, false);
        selection.adjust_selection(Point { x: 1, y: 1 }, Point { x: 5, y: 4 }, &mut screen);

        assert_eq!(selection.selection_anchor(), Point { x: 1, y: 1 });
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(1, 1, 5, 4));
        assert!(!selection.should_allow_mouse_drag_selection(Point { x: 3, y: 3 }));
        assert!(selection.should_allow_mouse_drag_selection(Point { x: 15, y: 3 }));
    }

    #[test]
    fn clearing_a_mark_selection_restores_the_cursor() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        assert!(screen.set_cursor_position(Point { x: 3, y: 2 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);
        selection.extend_selection(Point { x: 6, y: 5 }, &mut screen);

        // Move the cursor while it is the selection cursor.
        assert!(screen.set_cursor_position(Point { x: 10, y: 8 }));
        selection.clear_selection(&mut console, &mut screen, false);

        assert!(!console.flags().contains(ConsoleFlags::SELECTING));
        assert!(!selection.is_visible());
        assert!(!selection.is_area_selected());
        assert_eq!(screen.cursor_position(), Point { x: 3, y: 2 });
        assert_eq!(screen.text_buffer().cursor().width(), 0);
    }

    #[test]
    fn clearing_a_mouse_selection_leaves_the_cursor() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        assert!(screen.set_cursor_position(Point { x: 3, y: 2 }));
        selection.initialize_mouse_selection(&mut console, &mut scrolling, Point { x: 1, y: 1 }, false);
        selection.clear_selection(&mut console, &mut screen, false);

        assert!(!console.flags().contains(ConsoleFlags::SELECTING));
        assert_eq!(screen.cursor_position(), Point { x: 3, y: 2 });
    }

    #[test]
    fn clearing_without_a_selection_changes_nothing() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut selection = SelectionState::new();

        selection.clear_selection(&mut console, &mut screen, false);
        assert!(!console.flags().contains(ConsoleFlags::SELECTING));
        assert!(!selection.is_visible());
    }

    #[test]
    fn selecting_a_new_region_replaces_the_old_selection() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        assert!(screen.set_cursor_position(Point { x: 3, y: 2 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);
        selection.extend_selection(Point { x: 6, y: 5 }, &mut screen);

        selection.select_new_region(
            &mut console,
            &mut screen,
            &mut scrolling,
            Point { x: 1, y: 1 },
            Point { x: 4, y: 3 },
        );

        assert!(console.flags().contains(ConsoleFlags::SELECTING));
        assert!(selection.is_mouse_initiated_selection());
        assert!(selection.is_visible());
        assert_eq!(selection.selection_anchor(), Point { x: 1, y: 1 });
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(1, 1, 4, 3));
    }

    #[test]
    fn select_all_reaches_the_cursor_from_the_buffer_origin() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        assert!(screen.set_cursor_position(Point { x: 3, y: 2 }));
        selection.select_all(&mut console, &mut screen, &mut scrolling);

        assert!(console.flags().contains(ConsoleFlags::SELECTING));
        assert!(selection.is_visible());
        assert!(selection.is_line_selection());
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(0, 0, 3, 2));
    }

    #[test]
    fn select_all_in_block_mode_spans_the_full_width() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();
        selection.set_line_selection(false);

        assert!(screen.set_cursor_position(Point { x: 3, y: 2 }));
        selection.select_all(&mut console, &mut screen, &mut scrolling);

        assert!(!selection.is_line_selection());
        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(0, 0, 19, 2));
    }

    #[test]
    fn select_all_ends_at_the_saved_cursor_of_a_mark_selection() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        assert!(screen.set_cursor_position(Point { x: 3, y: 2 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);
        // Move the selection cursor, as the mark-mode keys would.
        assert!(screen.set_cursor_position(Point { x: 10, y: 8 }));

        selection.select_all(&mut console, &mut screen, &mut scrolling);

        assert_eq!(selection.selection_rectangle(), InclusiveRect::new(0, 0, 3, 2));
    }

    #[test]
    fn select_all_restores_the_viewport_origin() {
        let mut console = ConsoleInformation::new();
        let mut screen = screen();
        let mut scrolling = Scrolling::new();
        let mut selection = SelectionState::new();

        assert!(screen.set_viewport_origin(true, Point { x: 0, y: 3 }, false));
        assert!(screen.set_cursor_position(Point { x: 3, y: 2 }));

        selection.select_all(&mut console, &mut screen, &mut scrolling);

        assert_eq!(screen.viewport().origin(), Point { x: 0, y: 3 });
    }
}
