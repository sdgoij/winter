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

use crate::console_information::{ClipboardData, ConsoleInformation};
use crate::screen_info::ScreenInformation;
use crate::scrolling::Scrolling;
use crate::selection_input::{input_line_boundaries, is_within_boundaries};
use crate::selection_state::{SelectionFlags, SelectionState};
use crate::utils::{compare_coords, get_opposite_corner};
use buffer::{AttributeColorsFn, CopyRequest, TextBuffer};
use utils::{Color, InclusiveRect, Point, PointSpan, Size};

/// What the window layer supplies for a formatted copy.
///
/// Upstream reads these from the screen's current font and the render settings;
/// the portable host has neither, so the caller passes them.
pub struct ClipboardFormatting<'a> {
    /// The unscaled font height in points.
    pub font_height_points: f32,
    /// The font face name.
    pub font_face: &'a str,
    /// The default background colour, also used for padding.
    pub background: Color,
    /// Whether intensity is drawn as bold.
    pub is_intense_bold: bool,
    /// Resolves an attribute to its foreground, background and underline.
    pub attribute_colors: &'a AttributeColorsFn<'a>,
}

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

    /// Returns the visible selection as text spans.
    ///
    /// Ported from `Selection::GetSelectionSpans`. The upstream generation
    /// cache is an optimisation only, so the spans are recomputed here.
    pub fn selection_spans(&self, buffer: &TextBuffer, buffer_size: Size) -> Vec<PointSpan> {
        if !self.is_visible() {
            return Vec::new();
        }

        let (start, end) = self.selection_anchors(buffer_size);
        buffer.get_text_spans(start, end, !self.is_line_selection(), false)
    }

    /// Returns the selection as plain text, ready for the clipboard.
    ///
    /// The request is the one `Clipboard::StoreSelectionToClipboard` builds;
    /// `single_line` is upstream's Shift-held collapse to one line.
    pub fn plain_text(&self, buffer: &TextBuffer, single_line: bool) -> String {
        match self.clipboard_request(buffer, single_line) {
            Some(request) => buffer.get_plain_text(&request),
            None => String::new(),
        }
    }

    /// Returns the selection as a CF_HTML clipboard fragment.
    #[allow(clippy::too_many_arguments)]
    pub fn html(
        &self,
        buffer: &TextBuffer,
        single_line: bool,
        font_height_points: f32,
        font_face: &str,
        background: Color,
        is_intense_bold: bool,
        attribute_colors: &AttributeColorsFn<'_>,
    ) -> String {
        match self.clipboard_request(buffer, single_line) {
            Some(request) => buffer.gen_html(
                &request,
                font_height_points,
                font_face,
                background,
                is_intense_bold,
                attribute_colors,
            ),
            None => String::new(),
        }
    }

    /// Returns the selection as an RTF document.
    #[allow(clippy::too_many_arguments)]
    pub fn rtf(
        &self,
        buffer: &TextBuffer,
        single_line: bool,
        font_height_points: f32,
        font_face: &str,
        background: Color,
        is_intense_bold: bool,
        attribute_colors: &AttributeColorsFn<'_>,
    ) -> String {
        match self.clipboard_request(buffer, single_line) {
            Some(request) => buffer.gen_rtf(
                &request,
                font_height_points,
                font_face,
                background,
                is_intense_bold,
                attribute_colors,
            ),
            None => String::new(),
        }
    }

    /// Builds the copy request for the selection, or `None` when nothing is
    /// selected.
    fn clipboard_request(&self, buffer: &TextBuffer, single_line: bool) -> Option<CopyRequest> {
        if !self.is_area_selected() {
            return None;
        }

        let buffer_size = Size::new(buffer.screen_width(), buffer.buffer_height());
        let (start, end) = self.selection_anchors(buffer_size);
        Some(CopyRequest::from_config(
            buffer,
            start,
            end,
            single_line,
            !self.is_line_selection(),
            false,
            false,
        ))
    }

    /// Returns the clipboard data for the selection, or `None` when nothing is
    /// selected.
    ///
    /// `formatting` adds the HTML and RTF forms; without it the copy is plain
    /// text only, which is what upstream does unless formatting was asked for.
    pub fn clipboard_data(
        &self,
        buffer: &TextBuffer,
        single_line: bool,
        formatting: Option<&ClipboardFormatting<'_>>,
    ) -> Option<ClipboardData> {
        let request = self.clipboard_request(buffer, single_line)?;
        let text = buffer.get_plain_text(&request);

        let (html, rtf) = match formatting {
            Some(format) => (
                Some(buffer.gen_html(
                    &request,
                    format.font_height_points,
                    format.font_face,
                    format.background,
                    format.is_intense_bold,
                    format.attribute_colors,
                )),
                Some(buffer.gen_rtf(
                    &request,
                    format.font_height_points,
                    format.font_face,
                    format.background,
                    format.is_intense_bold,
                    format.attribute_colors,
                )),
            ),
            None => (None, None),
        };

        Some(ClipboardData { text, html, rtf })
    }

    /// Stores the selection on the console's pending clipboard and ends it.
    ///
    /// Ported from `Clipboard::Copy` and `StoreSelectionToClipboard`. The
    /// selection is cleared even when nothing was stored, as upstream does.
    pub fn copy_selection_to_clipboard(
        &mut self,
        console: &mut ConsoleInformation,
        screen: &mut ScreenInformation,
        single_line: bool,
        formatting: Option<&ClipboardFormatting<'_>>,
    ) -> bool {
        let data = self.clipboard_data(screen.text_buffer(), single_line, formatting);
        let stored = data.is_some();
        if let Some(data) = data {
            console.copy_to_clipboard(data);
        }

        self.clear_selection(console, screen, false);
        stored
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

    fn write_row(screen: &mut ScreenInformation, row_index: i32, text: &str) {
        let row = screen
            .text_buffer_mut()
            .get_row_at_screen_line_mut(row_index)
            .expect("row is within the page");
        for (column, character) in text.chars().enumerate() {
            row.write_char(column as i32, character, false);
        }
    }

    #[test]
    fn a_line_selection_reads_as_plain_text() {
        let mut console = ConsoleInformation::new();
        let mut screen = ScreenInformation::new(10, 5, 0, TextAttribute::new(), TextAttribute::new());
        let mut scrolling = Scrolling::new();
        write_row(&mut screen, 0, "hi");
        write_row(&mut screen, 1, "world");

        let mut selection = SelectionState::new();
        assert!(screen.set_cursor_position(Point { x: 0, y: 0 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);
        selection.extend_selection(Point { x: 4, y: 1 }, &mut screen);

        assert_eq!(selection.plain_text(screen.text_buffer(), false), "hi\r\nworld");
        // Shift collapses the rows into one line but, as upstream does, leaves
        // the unwritten columns in place.
        assert_eq!(selection.plain_text(screen.text_buffer(), true), "hi        world");
    }

    #[test]
    fn a_block_selection_keeps_its_columns() {
        let mut console = ConsoleInformation::new();
        let mut screen = ScreenInformation::new(10, 5, 0, TextAttribute::new(), TextAttribute::new());
        let mut scrolling = Scrolling::new();
        write_row(&mut screen, 0, "hi");
        write_row(&mut screen, 1, "world");

        let mut selection = SelectionState::new();
        selection.set_line_selection(false);
        assert!(screen.set_cursor_position(Point { x: 0, y: 0 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);
        selection.extend_selection(Point { x: 4, y: 1 }, &mut screen);

        assert_eq!(selection.plain_text(screen.text_buffer(), false), "hi   \r\nworld");
        // A block selection keeps its line structure even when collapsed.
        assert_eq!(selection.plain_text(screen.text_buffer(), true), "hi   \r\nworld");
    }

    #[test]
    fn an_invisible_selection_has_no_spans() {
        let screen = ScreenInformation::new(10, 5, 0, TextAttribute::new(), TextAttribute::new());
        let selection = SelectionState::new();

        assert!(
            selection
                .selection_spans(screen.text_buffer(), utils::Size::new(10, 5))
                .is_empty()
        );
    }

    #[test]
    fn the_spans_follow_the_selection_form() {
        let mut console = ConsoleInformation::new();
        let mut screen = ScreenInformation::new(10, 5, 0, TextAttribute::new(), TextAttribute::new());
        let mut scrolling = Scrolling::new();
        write_row(&mut screen, 0, "hi");
        write_row(&mut screen, 1, "world");

        let mut selection = SelectionState::new();
        assert!(screen.set_cursor_position(Point { x: 0, y: 0 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);
        selection.extend_selection(Point { x: 4, y: 1 }, &mut screen);

        let size = utils::Size::new(10, 5);
        assert_eq!(
            selection.selection_spans(screen.text_buffer(), size),
            vec![PointSpan::new(Point { x: 0, y: 0 }, Point { x: 5, y: 1 })]
        );

        selection.set_line_selection(false);
        assert_eq!(
            selection.selection_spans(screen.text_buffer(), size),
            vec![
                PointSpan::new(Point { x: 0, y: 0 }, Point { x: 5, y: 0 }),
                PointSpan::new(Point { x: 0, y: 1 }, Point { x: 5, y: 1 }),
            ]
        );
    }

    fn fixed_colors(_: &TextAttribute) -> (Color, Color, Color) {
        (
            Color::rgb(0x11, 0x22, 0x33),
            Color::rgb(0x44, 0x55, 0x66),
            Color::rgb(0x77, 0x88, 0x99),
        )
    }

    #[test]
    fn formatted_copy_is_empty_without_a_selection() {
        let screen = ScreenInformation::new(10, 5, 0, TextAttribute::new(), TextAttribute::new());
        let selection = SelectionState::new();
        let colors: &AttributeColorsFn<'_> = &fixed_colors;

        assert_eq!(
            selection.html(
                screen.text_buffer(),
                false,
                12.0,
                "Cascadia Mono",
                Color::default(),
                false,
                colors
            ),
            ""
        );
        assert_eq!(
            selection.rtf(
                screen.text_buffer(),
                false,
                12.0,
                "Cascadia Mono",
                Color::default(),
                false,
                colors
            ),
            ""
        );
    }

    #[test]
    fn a_selection_has_formatted_copy() {
        let mut console = ConsoleInformation::new();
        let mut screen = ScreenInformation::new(10, 5, 0, TextAttribute::new(), TextAttribute::new());
        let mut scrolling = Scrolling::new();
        write_row(&mut screen, 0, "hi");

        let mut selection = SelectionState::new();
        assert!(screen.set_cursor_position(Point { x: 0, y: 0 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);
        selection.extend_selection(Point { x: 1, y: 0 }, &mut screen);

        let colors: &AttributeColorsFn<'_> = &fixed_colors;
        let html = selection.html(
            screen.text_buffer(),
            false,
            12.0,
            "Cascadia Mono",
            Color::rgb(0xAA, 0xBB, 0xCC),
            false,
            colors,
        );
        assert!(html.contains("<SPAN STYLE=\"color:#112233;"));
        assert!(html.contains(">hi</SPAN>"));

        let rtf = selection.rtf(
            screen.text_buffer(),
            false,
            12.0,
            "Cascadia Mono",
            Color::rgb(0xAA, 0xBB, 0xCC),
            false,
            colors,
        );
        assert!(rtf.starts_with("{\\rtf1"));
        assert!(rtf.contains(" hi}"));
    }

    #[test]
    fn copying_stores_the_text_and_clears_the_selection() {
        let mut console = ConsoleInformation::new();
        let mut screen = ScreenInformation::new(10, 5, 0, TextAttribute::new(), TextAttribute::new());
        let mut scrolling = Scrolling::new();
        write_row(&mut screen, 0, "hi");

        let mut selection = SelectionState::new();
        assert!(screen.set_cursor_position(Point { x: 0, y: 0 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);
        selection.extend_selection(Point { x: 1, y: 0 }, &mut screen);

        assert!(selection.copy_selection_to_clipboard(&mut console, &mut screen, false, None));

        let data = console.use_pending_clipboard().expect("clipboard data");
        assert_eq!(data.text, "hi");
        assert_eq!(data.html, None);
        assert_eq!(data.rtf, None);
        assert!(!console.flags().contains(ConsoleFlags::SELECTING));
    }

    #[test]
    fn copying_with_formatting_adds_html_and_rtf() {
        let mut console = ConsoleInformation::new();
        let mut screen = ScreenInformation::new(10, 5, 0, TextAttribute::new(), TextAttribute::new());
        let mut scrolling = Scrolling::new();
        write_row(&mut screen, 0, "hi");

        let mut selection = SelectionState::new();
        assert!(screen.set_cursor_position(Point { x: 0, y: 0 }));
        selection.initialize_mark_selection(&mut console, &mut screen, &mut scrolling);
        selection.extend_selection(Point { x: 1, y: 0 }, &mut screen);

        let formatting = ClipboardFormatting {
            font_height_points: 12.0,
            font_face: "Cascadia Mono",
            background: Color::rgb(0xAA, 0xBB, 0xCC),
            is_intense_bold: false,
            attribute_colors: &fixed_colors,
        };

        assert!(selection.copy_selection_to_clipboard(&mut console, &mut screen, false, Some(&formatting)));

        let data = console.use_pending_clipboard().expect("clipboard data");
        assert_eq!(data.text, "hi");
        assert!(data.html.expect("html").contains("<SPAN STYLE=\"color:#112233;"));
        assert!(data.rtf.expect("rtf").starts_with("{\\rtf1"));
    }

    #[test]
    fn copying_without_a_selection_stores_nothing() {
        let mut console = ConsoleInformation::new();
        let mut screen = ScreenInformation::new(10, 5, 0, TextAttribute::new(), TextAttribute::new());
        let mut selection = SelectionState::new();

        assert!(!selection.copy_selection_to_clipboard(&mut console, &mut screen, false, None));
        assert_eq!(console.use_pending_clipboard(), None);
    }
}
