//! `SCREEN_INFORMATION` — one screen's buffer, viewport and console modes.
//!
//! Ported from `screenInfo.cpp` and the class in `screenInfo.hpp`. Upstream's
//! object also owns the VT state machine, font metrics, scroll-bar state, the
//! window's deferred resize and the server's object list; ADR 012 records which
//! of those belong to other layers here. What remains is the screen a host
//! module needs: the buffer, the viewport into it, the cursor, the attributes,
//! and the console's output modes.

use bitflags::bitflags;
use buffer::{OutputCell, TextBuffer};
use types::virtual_key::{
    VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN,
    VK_SHIFT, VK_SNAPSHOT,
};
use types::{CursorType, TextAttribute, Viewport};
use utils::{InclusiveRect, Point, Size};

bitflags! {
    /// The console's output mode bits, the `ENABLE_*` flags of `dwMode`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct OutputMode: u32 {
        /// Control characters are processed, not written.
        const PROCESSED_OUTPUT = 0x0001;
        /// Output wraps at the end of a row.
        const WRAP_AT_EOL_OUTPUT = 0x0004;
    }
}

/// One screen: a buffer, a viewport, a cursor and the console's modes.
#[derive(Debug)]
pub struct ScreenInformation {
    text_buffer: TextBuffer,
    viewport: Viewport,
    /// The bottom of the logical viewport, in buffer coordinates.
    virtual_bottom: i32,
    attributes: TextAttribute,
    popup_attributes: TextAttribute,
    output_mode: OutputMode,
    scroll_scale: u32,
    /// Wheel movement not yet worth a scroll, in `WHEEL_DELTA` units.
    wheel_delta: i16,
    /// Horizontal wheel movement not yet worth a scroll.
    hwheel_delta: i16,
    terminal_scrolling: bool,
    conpty_cursor_position_may_be_wrong: bool,
}

impl ScreenInformation {
    /// Creates a screen with a fresh buffer.
    ///
    /// `scrollback_height` rows are kept above the screen; the viewport starts
    /// over the visible rows and the virtual bottom sits on the last one, as
    /// upstream's `CreateInstance` leaves it.
    pub fn new(
        width: i32,
        height: i32,
        scrollback_height: i32,
        attributes: TextAttribute,
        popup_attributes: TextAttribute,
    ) -> Self {
        let mut screen = Self {
            text_buffer: TextBuffer::with_dimensions(width, height, scrollback_height),
            viewport: Viewport::new(Size::new(width, height)),
            virtual_bottom: 0,
            attributes,
            popup_attributes,
            output_mode: OutputMode::PROCESSED_OUTPUT | OutputMode::WRAP_AT_EOL_OUTPUT,
            scroll_scale: 1,
            wheel_delta: 0,
            hwheel_delta: 0,
            terminal_scrolling: false,
            conpty_cursor_position_may_be_wrong: false,
        };
        screen.update_bottom();
        screen
    }

    /// Returns the screen's text buffer.
    pub const fn text_buffer(&self) -> &TextBuffer {
        &self.text_buffer
    }

    /// Returns the screen's text buffer mutably.
    pub const fn text_buffer_mut(&mut self) -> &mut TextBuffer {
        &mut self.text_buffer
    }

    /// Returns the whole buffer, including scrollback.
    pub fn buffer_size(&self) -> Viewport {
        Viewport::new(Size::new(
            self.text_buffer.screen_width(),
            self.text_buffer.buffer_height(),
        ))
    }

    /// Returns the buffer as a terminal client sees it.
    ///
    /// In terminal-scrolling mode the height stops at the virtual bottom, so a
    /// client cannot address rows the user has not reached.
    pub fn terminal_buffer_size(&self) -> Viewport {
        let buffer = self.buffer_size();
        if self.terminal_scrolling && buffer.height() > self.virtual_bottom {
            Viewport::new(Size::new(buffer.width(), self.virtual_bottom + 1))
        } else {
            buffer
        }
    }

    /// Returns the viewport into the buffer.
    pub const fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Sets the viewport, clamping its size and origin into the buffer.
    ///
    /// `update_bottom` is true for API-driven moves and false when the user
    /// scrolls; upstream resets the virtual bottom only in the former case.
    pub fn set_viewport(&mut self, new_viewport: Viewport, update_bottom: bool) {
        if new_viewport == self.viewport {
            return;
        }

        let buffer = self.buffer_size();
        let width = new_viewport.width().clamp(1, buffer.width());
        let height = new_viewport.height().clamp(1, buffer.height());
        let x = new_viewport.origin().x.clamp(0, buffer.width() - width);
        let y = new_viewport.origin().y.clamp(0, buffer.height() - height);

        self.viewport = Viewport::from_origin_size(Point { x, y }, Size::new(width, height));

        if update_bottom {
            self.update_bottom();
        }
    }

    /// Moves the viewport's origin, keeping its size.
    ///
    /// With `absolute` the origin is the new position; otherwise it is a delta.
    /// Returns false when the move would leave the buffer.
    pub fn set_viewport_origin(&mut self, absolute: bool, origin: Point, update_bottom: bool) -> bool {
        let size = self.viewport.size();
        let new_origin = if absolute {
            if origin == self.viewport.origin() {
                return true;
            }
            origin
        } else {
            if origin.x == 0 && origin.y == 0 {
                return true;
            }
            Point {
                x: self.viewport.origin().x + origin.x,
                y: self.viewport.origin().y + origin.y,
            }
        };

        let left = new_origin.x;
        let mut top = new_origin.y;

        // In terminal-scrolling mode the viewport may not go below the logical
        // bottom; slide it back up instead.
        if self.terminal_scrolling && !update_bottom && top + size.height() - 1 > self.virtual_bottom {
            top += self.virtual_bottom - (top + size.height() - 1);
        }

        let right = left + size.width() - 1;
        let bottom = top + size.height() - 1;
        let buffer = self.buffer_size();

        if left < 0 || top < 0 || right < 0 || bottom < 0 || right >= buffer.width() || bottom >= buffer.height() {
            return false;
        }

        self.viewport = Viewport::from_origin_size(Point { x: left, y: top }, size);

        if update_bottom && self.virtual_bottom < self.viewport.bottom_right().y {
            self.update_bottom();
        }

        true
    }

    /// Moves the virtual bottom to the viewport's last row.
    pub fn update_bottom(&mut self) {
        self.virtual_bottom = self.viewport.bottom_right().y;
    }

    /// Returns the section a client would see without scrolling.
    pub fn virtual_viewport(&self) -> Viewport {
        let top = self.virtual_bottom - self.viewport.height() + 1;
        Viewport::from_origin_size(
            Point {
                x: self.viewport.origin().x,
                y: top,
            },
            self.viewport.size(),
        )
    }

    /// Returns the section VT sequences can address.
    pub fn vt_page_area(&self) -> Viewport {
        let height = self.viewport.height();
        let width = self.text_buffer.screen_width();
        let top = (self.virtual_bottom - height + 1).max(0);
        Viewport::from_origin_size(Point { x: 0, y: top }, Size::new(width, height))
    }

    /// Clamps an inclusive rectangle into the buffer.
    pub fn clip_to_screen_buffer(&self, rect: InclusiveRect) -> InclusiveRect {
        let buffer = self.buffer_size();
        InclusiveRect::new(
            rect.left().max(0),
            rect.top().max(0),
            rect.right().min(buffer.width() - 1),
            rect.bottom().min(buffer.height() - 1),
        )
    }

    /// Returns the cursor's position, as (column, row).
    pub fn cursor_position(&self) -> Point {
        let (row, column) = self.text_buffer.cursor().position();
        Point { x: column, y: row }
    }

    /// Moves the cursor and extends the virtual bottom when it moves past it.
    ///
    /// Returns false for a position outside the buffer, upstream's
    /// `STATUS_INVALID_PARAMETER`.
    pub fn set_cursor_position(&mut self, position: Point) -> bool {
        let buffer = self.buffer_size();
        if position.x < 0 || position.y < 0 || position.x >= buffer.width() || position.y >= buffer.height() {
            return false;
        }

        self.text_buffer.cursor_mut().set_position(position.y, position.x);

        if position.y > self.virtual_bottom {
            self.virtual_bottom = position.y;
        }

        true
    }

    /// Scrolls the viewport so `position` is visible.
    pub fn make_cursor_visible(&mut self, position: Point) {
        let viewport = self.viewport;
        let buffer = self.buffer_size();
        let x = position.x.clamp(0, buffer.width() - 1);
        let y = position.y.clamp(0, buffer.height() - 1);

        let mut origin = viewport.origin();
        origin.y = origin.y.min(y);
        origin.y = origin.y.max(y - (viewport.height() - 1));
        origin.x = origin.x.min(x);
        origin.x = origin.x.max(x - (viewport.width() - 1));

        if origin != viewport.origin() {
            self.set_viewport_origin(true, origin, false);
        }
    }

    /// Scrolls the viewport so the visible cursor is on screen.
    pub fn make_current_cursor_visible(&mut self) {
        if self.text_buffer.cursor().is_visible() {
            let position = self.cursor_position();
            self.make_cursor_visible(position);
        }
    }

    /// Handles a key press's snap-on-input: a text key reveals the cursor.
    ///
    /// Modifier-only keys and Print Screen do not, as upstream's `IsInputKey`.
    pub fn snap_on_input(&mut self, virtual_key: u16) {
        if is_input_key(virtual_key) {
            self.make_current_cursor_visible();
        }
    }

    /// Sets the cursor's size and visibility.
    ///
    /// Changing the size returns the cursor to the legacy shape, as upstream
    /// does so the size and shape cannot disagree.
    pub fn set_cursor_information(&mut self, size: u32, visible: bool) {
        let cursor = self.text_buffer.cursor_mut();
        let original_size = cursor.width();
        let size = size.min(i32::MAX as u32) as i32;
        cursor.set_width(size);
        cursor.set_visible(visible);
        if size != original_size {
            cursor.set_shape(CursorType::Legacy);
        }
    }

    /// Sets the cursor shape.
    ///
    /// `set_main` is accepted for the alt-buffer propagation upstream does; the
    /// portable screen has no alternate buffer of its own (ADR 012).
    pub fn set_cursor_type(&mut self, cursor_type: CursorType, _set_main: bool) {
        self.text_buffer.cursor_mut().set_shape(cursor_type);
    }

    /// Draws the cursor double-size for insert/overtype mode.
    pub fn set_cursor_double(&mut self, double_cursor: bool) {
        self.text_buffer.cursor_mut().set_double(double_cursor);
    }

    /// Returns the attribute new text is written with.
    ///
    /// Upstream keeps this in the `TextBuffer`; in this workspace the adapter
    /// owns the VT write attributes, so the console keeps the legacy attribute
    /// its own write path uses (ADR 012).
    pub const fn attributes(&self) -> TextAttribute {
        self.attributes
    }

    /// Sets the attribute new text is written with.
    pub const fn set_attributes(&mut self, attributes: TextAttribute) {
        self.attributes = attributes;
    }

    /// Returns the attribute a popup is drawn with.
    pub const fn popup_attributes(&self) -> TextAttribute {
        self.popup_attributes
    }

    /// Sets the attribute a popup is drawn with.
    pub const fn set_popup_attributes(&mut self, popup_attributes: TextAttribute) {
        self.popup_attributes = popup_attributes;
    }

    /// Replaces both the current and the popup attribute.
    ///
    /// The repaint upstream triggers belongs to the renderer, which is
    /// host-driven (ADR 006).
    pub fn set_default_attributes(&mut self, attributes: TextAttribute, popup_attributes: TextAttribute) {
        if self.attributes == attributes && self.popup_attributes == popup_attributes {
            return;
        }
        self.attributes = attributes;
        self.popup_attributes = popup_attributes;
    }

    /// Blanks the whole buffer with the current attribute.
    pub fn clear_text_data(&mut self) {
        let width = self.text_buffer.screen_width();
        let height = self.text_buffer.buffer_height();
        let mut blank = OutputCell::from_char(' ');
        blank.set_attributes(self.attributes);
        self.text_buffer.fill_rect(0, 0, height - 1, width - 1, &blank);
    }

    /// Returns the console's output modes.
    pub const fn output_mode(&self) -> OutputMode {
        self.output_mode
    }

    /// Replaces the console's output modes.
    pub const fn set_output_mode(&mut self, output_mode: OutputMode) {
        self.output_mode = output_mode;
    }

    /// Returns the scroll wheel's scale.
    pub const fn scroll_scale(&self) -> u32 {
        self.scroll_scale
    }

    /// Sets the scroll wheel's scale.
    pub const fn set_scroll_scale(&mut self, scroll_scale: u32) {
        self.scroll_scale = scroll_scale;
    }

    /// Returns the vertical wheel movement not yet worth a scroll.
    pub const fn wheel_delta(&self) -> i16 {
        self.wheel_delta
    }

    /// Sets the vertical wheel remainder.
    pub const fn set_wheel_delta(&mut self, wheel_delta: i16) {
        self.wheel_delta = wheel_delta;
    }

    /// Returns the horizontal wheel movement not yet worth a scroll.
    pub const fn hwheel_delta(&self) -> i16 {
        self.hwheel_delta
    }

    /// Sets the horizontal wheel remainder.
    pub const fn set_hwheel_delta(&mut self, hwheel_delta: i16) {
        self.hwheel_delta = hwheel_delta;
    }

    /// Returns whether the console addresses the buffer as a terminal does.
    pub const fn terminal_scrolling(&self) -> bool {
        self.terminal_scrolling
    }

    /// Turns terminal-scrolling mode on or off.
    pub const fn set_terminal_scrolling(&mut self, terminal_scrolling: bool) {
        self.terminal_scrolling = terminal_scrolling;
    }

    /// Returns whether ConPTY's cursor position may lag the buffer's.
    pub const fn conpty_cursor_position_may_be_wrong(&self) -> bool {
        self.conpty_cursor_position_may_be_wrong
    }

    /// Records that ConPTY's cursor position may be stale.
    pub const fn set_conpty_cursor_position_may_be_wrong(&mut self) {
        self.conpty_cursor_position_may_be_wrong = true;
    }

    /// Records that ConPTY's cursor position is synchronized again.
    pub const fn reset_conpty_cursor_position_may_be_wrong(&mut self) {
        self.conpty_cursor_position_may_be_wrong = false;
    }
}

/// Returns whether a key press reveals the cursor.
///
/// Upstream's `IsInputKey`: modifiers and Print Screen do not count.
fn is_input_key(virtual_key: u16) -> bool {
    !matches!(
        virtual_key,
        VK_CONTROL
            | VK_LCONTROL
            | VK_RCONTROL
            | VK_MENU
            | VK_LMENU
            | VK_RMENU
            | VK_SHIFT
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_LWIN
            | VK_RWIN
            | VK_SNAPSHOT
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::virtual_key::{VK_F1, VK_SHIFT};

    fn screen(width: i32, height: i32, scrollback: i32) -> ScreenInformation {
        ScreenInformation::new(width, height, scrollback, TextAttribute::new(), TextAttribute::new())
    }

    #[test]
    fn a_new_screen_shows_the_visible_rows() {
        let screen = screen(80, 24, 0);
        assert_eq!(screen.viewport().width(), 80);
        assert_eq!(screen.viewport().height(), 24);
        assert_eq!(screen.buffer_size().width(), 80);
        assert_eq!(screen.buffer_size().height(), 24);
        assert_eq!(screen.terminal_buffer_size().height(), 24);
    }

    #[test]
    fn scrollback_extends_the_buffer_but_not_the_viewport() {
        let screen = screen(80, 24, 100);
        assert_eq!(screen.buffer_size().height(), 124);
        assert_eq!(screen.viewport().height(), 24);
        assert_eq!(screen.terminal_buffer_size().height(), 124);
    }

    #[test]
    fn the_virtual_viewport_and_page_area_follow_the_viewport() {
        let mut screen = screen(80, 24, 100);
        screen.set_viewport(
            Viewport::from_origin_size(Point { x: 0, y: 50 }, Size::new(80, 24)),
            true,
        );

        // The virtual bottom moved with the viewport.
        assert_eq!(screen.virtual_viewport().origin().y, 50);
        let page = screen.vt_page_area();
        assert_eq!(page.origin().y, 50);
        assert_eq!(page.width(), 80);
        assert_eq!(page.height(), 24);
    }

    #[test]
    fn set_viewport_clamps_into_the_buffer() {
        let mut screen = screen(80, 24, 100);
        screen.set_viewport(
            Viewport::from_origin_size(Point { x: 10, y: 50 }, Size::new(40, 10)),
            false,
        );
        assert_eq!(screen.viewport().origin(), Point { x: 10, y: 50 });
        assert_eq!(screen.viewport().width(), 40);

        // A viewport larger than the buffer, placed past its end, lands at 0,0.
        screen.set_viewport(
            Viewport::from_origin_size(Point { x: 500, y: 500 }, Size::new(500, 500)),
            false,
        );
        assert_eq!(screen.viewport().origin(), Point { x: 0, y: 0 });
        assert_eq!(screen.viewport().width(), 80);
        assert_eq!(screen.viewport().height(), 124);
    }

    #[test]
    fn set_viewport_origin_rejects_a_move_outside_the_buffer() {
        let mut screen = screen(80, 24, 100);
        assert!(screen.set_viewport_origin(true, Point { x: 0, y: 50 }, false));
        assert_eq!(screen.viewport().origin().y, 50);

        // The buffer is 124 rows tall; a 24-row viewport cannot start at 120.
        assert!(!screen.set_viewport_origin(true, Point { x: 0, y: 120 }, false));
        assert_eq!(screen.viewport().origin().y, 50);
    }

    #[test]
    fn terminal_scrolling_stops_the_viewport_below_the_bottom() {
        let mut screen = screen(80, 24, 100);
        screen.set_terminal_scrolling(true);
        screen.set_viewport(
            Viewport::from_origin_size(Point { x: 0, y: 50 }, Size::new(80, 24)),
            true,
        );

        // Scrolling away from the bottom slides back to the logical viewport.
        assert!(screen.set_viewport_origin(true, Point { x: 0, y: 100 }, false));
        assert_eq!(screen.viewport().origin().y, 50);
        assert_eq!(screen.terminal_buffer_size().height(), 74);
    }

    #[test]
    fn making_the_cursor_visible_scrolls_the_viewport() {
        let mut screen = screen(80, 24, 100);
        assert!(screen.set_cursor_position(Point { x: 10, y: 50 }));
        screen.make_cursor_visible(Point { x: 10, y: 50 });
        assert_eq!(screen.viewport().origin().y, 27);
    }

    #[test]
    fn set_cursor_position_rejects_out_of_bounds_and_extends_the_bottom() {
        let mut screen = screen(80, 24, 10);

        assert!(!screen.set_cursor_position(Point { x: 80, y: 0 }));
        assert!(!screen.set_cursor_position(Point { x: 0, y: 34 }));
        assert!(!screen.set_cursor_position(Point { x: -1, y: 0 }));

        assert!(screen.set_cursor_position(Point { x: 4, y: 30 }));
        assert_eq!(screen.cursor_position(), Point { x: 4, y: 30 });
        // The virtual bottom follows a cursor that moved past it.
        assert_eq!(screen.vt_page_area().bottom_right().y, 30);
    }

    #[test]
    fn clipping_an_inclusive_rect_stays_inside_the_buffer() {
        let screen = screen(80, 24, 0);
        let clipped = screen.clip_to_screen_buffer(InclusiveRect::new(-5, -5, 100, 100));
        assert_eq!(clipped, InclusiveRect::new(0, 0, 79, 23));
    }

    #[test]
    fn cursor_information_changes_size_visibility_and_shape() {
        let mut screen = screen(80, 24, 0);

        screen.set_cursor_information(50, false);
        assert_eq!(screen.text_buffer().cursor().width(), 50);
        assert!(!screen.text_buffer().cursor().is_visible());

        screen.set_cursor_type(CursorType::FullBox, false);
        assert_eq!(screen.text_buffer().cursor().shape(), CursorType::FullBox);

        // Changing only the visibility keeps the shape.
        screen.set_cursor_information(50, true);
        assert_eq!(screen.text_buffer().cursor().shape(), CursorType::FullBox);

        // Changing the size returns to the legacy shape.
        screen.set_cursor_information(25, true);
        assert_eq!(screen.text_buffer().cursor().shape(), CursorType::Legacy);
    }

    #[test]
    fn the_default_attributes_replace_both_attributes() {
        let mut screen = screen(80, 24, 0);
        let attributes = TextAttribute::new();
        screen.set_default_attributes(attributes, attributes);
        assert_eq!(screen.attributes(), attributes);
        assert_eq!(screen.popup_attributes(), attributes);
    }

    #[test]
    fn clearing_the_screen_blanks_it_with_the_current_attribute() {
        let mut screen = screen(80, 24, 0);
        screen.text_buffer_mut().write_string("hello");
        screen.clear_text_data();

        let cell = screen.text_buffer().get_cell_data_at(0, 0).expect("a cell");
        assert_eq!(cell.chars(), " ");
        assert_eq!(*cell.get_attributes(), TextAttribute::new());
    }

    #[test]
    fn modifier_keys_do_not_snap_the_viewport() {
        let mut screen = screen(80, 24, 100);
        assert!(screen.set_cursor_position(Point { x: 0, y: 50 }));

        screen.snap_on_input(VK_SHIFT);
        assert_eq!(screen.viewport().origin().y, 0);

        screen.snap_on_input(VK_F1);
        assert_eq!(screen.viewport().origin().y, 27);
    }

    #[test]
    fn the_output_mode_starts_processed_and_wrapping() {
        let screen = screen(80, 24, 0);
        assert!(screen.output_mode().contains(OutputMode::PROCESSED_OUTPUT));
        assert!(screen.output_mode().contains(OutputMode::WRAP_AT_EOL_OUTPUT));
    }

    #[test]
    fn the_conpty_cursor_flag_is_set_and_reset() {
        let mut screen = screen(80, 24, 0);
        assert!(!screen.conpty_cursor_position_may_be_wrong());
        screen.set_conpty_cursor_position_may_be_wrong();
        assert!(screen.conpty_cursor_position_may_be_wrong());
        screen.reset_conpty_cursor_position_may_be_wrong();
        assert!(!screen.conpty_cursor_position_may_be_wrong());
    }
}
