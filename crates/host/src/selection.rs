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
use crate::selection_state::{SelectionFlags, SelectionState};
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
}
