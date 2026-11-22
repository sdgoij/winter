//! `Selection` state — what the console is selecting.
//!
//! Ported from `selectionState.cpp` and `SelectionData` in `selection.hpp`.
//! The anchor, rectangle, flags and the saved cursor live here (ADR 014); the
//! region operations are `selection.cpp` and the key/mouse handling is
//! `selectionInput.cpp`, which land separately. The buffer contributes only
//! text extraction, once the copy path needs it.
//!
//! The "selecting" and "quick edit" bits a client observes are console flags,
//! so the two predicates that read or write them take the console.

use bitflags::bitflags;
use buffer::Cursor;
use types::CursorType;
use utils::{InclusiveRect, Point, Size};

use crate::console_information::{ConsoleFlags, ConsoleInformation};

bitflags! {
    /// The selection bits of `dwSelectionFlags`.
    ///
    /// The first four are the values `GetConsoleSelectionInfo` reports; the
    /// inverted bit is internal.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct SelectionFlags: u32 {
        /// A selection is being made.
        const IN_PROGRESS = 0x0001;
        /// The selection covers at least one cell.
        const NOT_EMPTY = 0x0002;
        /// A non-mark-mode selection started this, usually the mouse.
        const MOUSE_SELECTION = 0x0004;
        /// The mouse button is held down.
        const MOUSE_DOWN = 0x0008;
        /// The selection is turned off, not drawn.
        const INVERTED = 0x0010;
    }
}

/// The bits `GetConsoleSelectionInfo` exposes, upstream's
/// `CONSOLE_SELECTION_VALID`.
const PUBLIC_FLAGS: SelectionFlags = SelectionFlags::from_bits_truncate(0x000f);

/// The cursor state selection saves while it borrows the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SavedCursor {
    position: Point,
    size: i32,
    visible: bool,
    cursor_type: CursorType,
}

/// The console's selection interaction state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionState {
    visible: bool,
    line_selection: bool,
    use_alternate_selection: bool,
    allow_mouse_drag_selection: bool,
    flags: SelectionFlags,
    anchor: Point,
    rectangle: InclusiveRect,
    saved_cursor: Option<SavedCursor>,
}

impl SelectionState {
    /// Creates an empty, keyboard-driven, line-aligned selection.
    pub const fn new() -> Self {
        Self {
            visible: false,
            line_selection: true,
            use_alternate_selection: false,
            allow_mouse_drag_selection: true,
            flags: SelectionFlags::empty(),
            anchor: Point { x: 0, y: 0 },
            rectangle: InclusiveRect::new(0, 0, 0, 0),
            saved_cursor: None,
        }
    }

    /// Returns whether the console is selecting.
    pub fn is_in_selecting_state(&self, console: &ConsoleInformation) -> bool {
        console.flags().contains(ConsoleFlags::SELECTING)
    }

    /// Sets the console's selecting flag.
    pub fn set_selecting(&self, console: &mut ConsoleInformation, selecting: bool) {
        if selecting {
            console.flags_mut().insert(ConsoleFlags::SELECTING);
        } else {
            console.flags_mut().remove(ConsoleFlags::SELECTING);
        }
    }

    /// Returns whether quick-edit (mouse) selection is enabled.
    pub fn is_in_quick_edit_mode(&self, console: &ConsoleInformation) -> bool {
        console.flags().contains(ConsoleFlags::QUICK_EDIT_MODE)
    }

    /// Returns whether the selection aligns to whole lines.
    ///
    /// Line selection and the alternate flag are opposites: line and block are
    /// each expressed by a different combination.
    pub const fn is_line_selection(&self) -> bool {
        self.line_selection != self.use_alternate_selection
    }

    /// Flips the alternate flag so line or block selection is produced.
    pub const fn align_alternate_selection(&mut self, align_to_line: bool) {
        self.use_alternate_selection = if align_to_line {
            !self.line_selection
        } else {
            self.line_selection
        };
    }

    /// Returns whether the selection covers at least one cell.
    pub const fn is_area_selected(&self) -> bool {
        self.flags.contains(SelectionFlags::NOT_EMPTY)
    }

    /// Returns whether mark mode started the selection.
    pub const fn is_keyboard_mark_selection(&self) -> bool {
        !self.flags.contains(SelectionFlags::MOUSE_SELECTION)
    }

    /// Returns whether anything but mark mode started the selection.
    pub const fn is_mouse_initiated_selection(&self) -> bool {
        self.flags.contains(SelectionFlags::MOUSE_SELECTION)
    }

    /// Returns whether the mouse button is held down.
    pub const fn is_mouse_button_down(&self) -> bool {
        self.flags.contains(SelectionFlags::MOUSE_DOWN)
    }

    /// Records that the mouse button is down.
    ///
    /// Upstream also captures the mouse so a release outside the window is
    /// seen; that belongs to the window layer.
    pub fn mouse_down(&mut self) {
        self.flags.insert(SelectionFlags::MOUSE_DOWN);
    }

    /// Records that the mouse button came up.
    pub fn mouse_up(&mut self) {
        self.flags.remove(SelectionFlags::MOUSE_DOWN);
    }

    /// Saves the cursor so it can be put back when the selection ends.
    pub fn save_cursor(&mut self, cursor: &Cursor) {
        let (row, column) = cursor.position();
        self.saved_cursor = Some(SavedCursor {
            position: Point { x: column, y: row },
            size: cursor.width(),
            visible: cursor.is_visible(),
            cursor_type: cursor.shape(),
        });
    }

    /// Restores the cursor saved by [`save_cursor`](Self::save_cursor).
    pub fn restore_cursor(&self, cursor: &mut Cursor) {
        let Some(saved) = self.saved_cursor else {
            return;
        };
        cursor.set_width(saved.size);
        cursor.set_visible(saved.visible);
        cursor.set_shape(saved.cursor_type);
        cursor.set_position(saved.position.y, saved.position.x);
    }

    /// Returns the corner the selection started from.
    pub const fn selection_anchor(&self) -> Point {
        self.anchor
    }

    /// Returns the inclusive rectangle the selection covers.
    pub const fn selection_rectangle(&self) -> InclusiveRect {
        self.rectangle
    }

    /// Returns the flags a client reads.
    pub const fn public_flags(&self) -> SelectionFlags {
        self.flags.intersection(PUBLIC_FLAGS)
    }

    /// Returns the selection's start and end anchors.
    ///
    /// The end is exclusive and lies one cell past the last selected one. An
    /// invisible selection reports an empty pair, as upstream does.
    pub fn selection_anchors(&self, buffer_size: Size) -> (Point, Point) {
        if !self.visible {
            return (Point { x: 0, y: 0 }, Point { x: -1, y: -1 });
        }

        let mut start = self.anchor;
        let mut end = Point {
            x: if self.anchor.x == self.rectangle.left() {
                self.rectangle.right()
            } else {
                self.rectangle.left()
            },
            y: if self.anchor.y == self.rectangle.top() {
                self.rectangle.bottom()
            } else {
                self.rectangle.top()
            },
        };

        // The bottom-right corner becomes the exclusive end. Line selection
        // compares whole points; block selection compares only columns.
        let bottom_right_is_end = if self.is_line_selection() {
            start <= end
        } else {
            start.x <= end.x
        };
        if bottom_right_is_end {
            increment_in_exclusive_bounds(&mut end, buffer_size);
        } else {
            increment_in_exclusive_bounds(&mut start, buffer_size);
        }

        if start > end { (end, start) } else { (start, end) }
    }

    /// Returns whether a mouse drag may change the selection.
    ///
    /// A double-click selection sets this false so a small drag right after it
    /// does not overwrite it.
    pub fn should_allow_mouse_drag_selection(&self, mouse_position: Point) -> bool {
        self.allow_mouse_drag_selection || !self.rectangle.contains(mouse_position)
    }

    /// Returns whether the selection is drawn.
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    /// Sets the selection's visibility.
    pub(crate) const fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Sets the anchor, the corner the selection started from.
    pub(crate) const fn set_anchor(&mut self, anchor: Point) {
        self.anchor = anchor;
    }

    /// Sets the inclusive rectangle the selection covers.
    pub(crate) const fn set_rectangle(&mut self, rectangle: InclusiveRect) {
        self.rectangle = rectangle;
    }

    /// Adds flag bits.
    pub(crate) fn insert_flags(&mut self, flags: SelectionFlags) {
        self.flags.insert(flags);
    }

    /// Sets whether the alternate selection form is used.
    ///
    /// Together with the line flag this chooses line or block selection; the
    /// Alt key flipped it when a mouse selection started.
    pub(crate) const fn set_use_alternate_selection(&mut self, alternate: bool) {
        self.use_alternate_selection = alternate;
    }

    /// Replaces the flag bits.
    pub(crate) const fn set_flags(&mut self, flags: SelectionFlags) {
        self.flags = flags;
    }

    /// Sets whether a mouse drag may change the selection.
    pub(crate) const fn set_allow_mouse_drag_selection(&mut self, allow: bool) {
        self.allow_mouse_drag_selection = allow;
    }

    /// Sets line or block mode.
    ///
    /// Upstream clears the selection first so the painted state matches; the
    /// caller does that here.
    pub const fn set_line_selection(&mut self, line: bool) {
        self.line_selection = line;
    }
}

impl Default for SelectionState {
    fn default() -> Self {
        Self::new()
    }
}

/// Advances `position` by one cell inside a viewport that has one extra column
/// past its inclusive right, upstream's `Viewport::IncrementInExclusiveBounds`.
fn increment_in_exclusive_bounds(position: &mut Point, size: Size) {
    let width = i64::from(size.width()) + 1;
    let height = i64::from(size.height());
    let maximum = width * height;
    let offset = width * i64::from(position.y) + i64::from(position.x) + 1;
    let clamped = offset.clamp(0, maximum);

    position.x = (clamped % width) as i32;
    position.y = (clamped / width) as i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size() -> Size {
        Size::new(80, 25)
    }

    fn visible_selection(anchor: Point, rectangle: InclusiveRect) -> SelectionState {
        let mut selection = SelectionState::new();
        selection.set_visible(true);
        selection.set_anchor(anchor);
        selection.set_rectangle(rectangle);
        selection
    }

    #[test]
    fn the_initial_state_is_empty_and_keyboard_driven() {
        let selection = SelectionState::new();
        assert!(!selection.is_visible());
        assert!(selection.is_line_selection());
        assert!(!selection.is_area_selected());
        assert!(selection.is_keyboard_mark_selection());
        assert!(!selection.is_mouse_initiated_selection());
        assert!(!selection.is_mouse_button_down());
        assert_eq!(selection.public_flags(), SelectionFlags::empty());
        assert_eq!(
            selection.selection_anchors(size()),
            (Point { x: 0, y: 0 }, Point { x: -1, y: -1 })
        );
    }

    #[test]
    fn line_selection_is_the_opposite_of_the_alternate_flag() {
        let mut selection = SelectionState::new();
        assert!(selection.is_line_selection());

        selection.align_alternate_selection(false);
        assert!(!selection.is_line_selection());

        selection.align_alternate_selection(true);
        assert!(selection.is_line_selection());

        // The flag composes with the alternate bit, so setting it while the
        // alternate is clear reads as a block selection.
        selection.set_line_selection(false);
        assert!(!selection.is_line_selection());

        selection.set_line_selection(true);
        assert!(selection.is_line_selection());
    }

    #[test]
    fn the_selecting_state_is_the_console_flag() {
        let selection = SelectionState::new();
        let mut console = ConsoleInformation::new();
        assert!(!selection.is_in_selecting_state(&console));

        selection.set_selecting(&mut console, true);
        assert!(console.flags().contains(ConsoleFlags::SELECTING));
        assert!(selection.is_in_selecting_state(&console));

        selection.set_selecting(&mut console, false);
        assert!(!console.flags().contains(ConsoleFlags::SELECTING));
    }

    #[test]
    fn quick_edit_mode_is_read_from_the_console() {
        let selection = SelectionState::new();
        let mut console = ConsoleInformation::new();
        assert!(!selection.is_in_quick_edit_mode(&console));

        console.flags_mut().insert(ConsoleFlags::QUICK_EDIT_MODE);
        assert!(selection.is_in_quick_edit_mode(&console));
    }

    #[test]
    fn the_mouse_button_sets_and_clears_its_flag() {
        let mut selection = SelectionState::new();
        selection.mouse_down();
        assert!(selection.is_mouse_button_down());
        assert!(selection.public_flags().contains(SelectionFlags::MOUSE_DOWN));

        selection.mouse_up();
        assert!(!selection.is_mouse_button_down());
    }

    #[test]
    fn saving_and_restoring_the_cursor_round_trips() {
        let mut selection = SelectionState::new();
        let mut cursor = Cursor::new();
        cursor.set_position(4, 7);
        cursor.set_width(30);
        cursor.set_visible(false);
        cursor.set_shape(CursorType::FullBox);

        selection.save_cursor(&cursor);

        cursor.set_position(0, 0);
        cursor.set_width(10);
        cursor.set_visible(true);
        cursor.set_shape(CursorType::Legacy);

        selection.restore_cursor(&mut cursor);
        assert_eq!(cursor.position(), (4, 7));
        assert_eq!(cursor.width(), 30);
        assert!(!cursor.is_visible());
        assert_eq!(cursor.shape(), CursorType::FullBox);
    }

    #[test]
    fn restoring_without_a_saved_cursor_changes_nothing() {
        let selection = SelectionState::new();
        let mut cursor = Cursor::new();
        cursor.set_position(1, 2);

        selection.restore_cursor(&mut cursor);
        assert_eq!(cursor.position(), (1, 2));
    }

    #[test]
    fn the_public_flags_hide_the_inverted_bit() {
        let mut selection = SelectionState::new();
        selection.insert_flags(SelectionFlags::IN_PROGRESS | SelectionFlags::NOT_EMPTY | SelectionFlags::INVERTED);

        let public = selection.public_flags();
        assert!(public.contains(SelectionFlags::IN_PROGRESS));
        assert!(public.contains(SelectionFlags::NOT_EMPTY));
        assert!(!public.contains(SelectionFlags::INVERTED));
    }

    #[test]
    fn a_line_selection_reports_an_exclusive_end() {
        let selection = visible_selection(Point { x: 2, y: 1 }, InclusiveRect::new(2, 1, 5, 3));
        assert_eq!(
            selection.selection_anchors(size()),
            (Point { x: 2, y: 1 }, Point { x: 6, y: 3 })
        );
    }

    #[test]
    fn a_block_selection_compares_only_columns() {
        // The anchor is at the bottom-right; the exclusive end moves with it.
        let selection = visible_selection(Point { x: 5, y: 3 }, InclusiveRect::new(2, 1, 5, 3));
        assert_eq!(
            selection.selection_anchors(size()),
            (Point { x: 2, y: 1 }, Point { x: 6, y: 3 })
        );
    }

    #[test]
    fn the_exclusive_end_sits_one_past_the_last_column() {
        // The viewport has one extra column past its inclusive right, so the
        // end of a selection on the last column is reported at x == width.
        let selection = visible_selection(Point { x: 3, y: 2 }, InclusiveRect::new(3, 2, 3, 2));
        assert_eq!(
            selection.selection_anchors(Size::new(4, 3)),
            (Point { x: 3, y: 2 }, Point { x: 4, y: 2 })
        );
    }

    #[test]
    fn a_drag_is_allowed_outside_the_selection_unless_forbidden() {
        let mut selection = visible_selection(Point { x: 0, y: 0 }, InclusiveRect::new(1, 1, 5, 5));
        let inside = Point { x: 2, y: 2 };
        let outside = Point { x: 8, y: 8 };

        assert!(selection.should_allow_mouse_drag_selection(inside));
        assert!(selection.should_allow_mouse_drag_selection(outside));

        selection.set_allow_mouse_drag_selection(false);
        assert!(!selection.should_allow_mouse_drag_selection(inside));
        assert!(selection.should_allow_mouse_drag_selection(outside));
    }
}
