//! `AdaptDispatch` — VT adapter state.
//!
//! Ported from `adaptDispatch.hpp/cpp` in the Windows Terminal C++ codebase.
//!
//! The struct holds the state the dispatch handlers share: modes, margins, the
//! page manager, the terminal output, the sixel parser, and the SGR stack. The
//! `ITermDispatch` handlers that mutate a buffer through this state are not
//! ported yet; see `plan.md` §"Pre-Phase-5 cleanup" item 2.

use crate::dispatch_types::*;
use crate::sixel_parser::SIXEL_DEFAULT_CONFORMANCE;
use crate::term_dispatch::ITermDispatch;
use crate::{FontBuffer, MacroBuffer, PageManager, SixelParser, TerminalOutput};

/// Cursor state for save/restore operations.
#[derive(Debug, Clone)]
pub struct CursorState {
    /// Row position.
    pub row: VTInt,
    /// Column position.
    pub column: VTInt,
    /// Page number.
    pub page: VTInt,
    /// Whether delayed EOL wrap is set.
    pub is_delayed_eol_wrap: bool,
    /// Whether origin mode is relative.
    pub is_origin_mode_relative: bool,
    /// Text attributes.
    pub attributes: types::TextAttribute,
    /// Terminal output state.
    pub term_output: TerminalOutput,
}

impl Default for CursorState {
    fn default() -> Self {
        Self {
            row: 0,
            column: 0,
            page: 1,
            is_delayed_eol_wrap: false,
            is_origin_mode_relative: false,
            attributes: types::TextAttribute::default(),
            term_output: TerminalOutput::default(),
        }
    }
}

/// Operations that can be performed on text.
#[derive(Debug, Clone)]
pub struct ChangeOps {
    /// Character attributes OR mask.
    pub and_attr_mask: types::CharacterAttributes,
    /// Character attributes XOR mask.
    pub xor_attr_mask: types::CharacterAttributes,
    /// Optional foreground color.
    pub foreground: Option<types::TextColor>,
    /// Optional background color.
    pub background: Option<types::TextColor>,
    /// Optional underline color.
    pub underline_color: Option<types::TextColor>,
}

/// A cursor offset, absolute (1-based VT coordinates) or relative.
///
/// Ported from `AdaptDispatch::Offset`; absolute positions subtract one because
/// VT's origin is `1,1` and the buffer's is `0,0`.
#[derive(Debug, Clone, Copy)]
struct Offset {
    value: VTInt,
    is_absolute: bool,
}

impl Offset {
    const fn absolute(value: VTInt) -> Self {
        Self {
            value: value - 1,
            is_absolute: true,
        }
    }

    const fn forward(value: VTInt) -> Self {
        Self {
            value,
            is_absolute: false,
        }
    }

    const fn backward(value: VTInt) -> Self {
        Self {
            value: -value,
            is_absolute: false,
        }
    }

    const fn unchanged() -> Self {
        Self::forward(0)
    }
}

/// Main VT adapter dispatcher.
///
/// Ported handlers live here; the families that are not ported yet fall through
/// to `ITermDispatch`'s defaults, which call [`AdaptDispatch::unhandled`] and are
/// recorded so a test can see them.
#[derive(Debug)]
pub struct AdaptDispatch {
    /// Page manager for buffer switching.
    page_manager: PageManager,
    /// Terminal output state.
    term_output: TerminalOutput,
    /// Sixel parser.
    sixel_parser: SixelParser,
    /// Font buffer.
    font_buffer: FontBuffer,
    /// Macro buffer.
    macro_buffer: MacroBuffer,
    /// Whether we're using the alternate buffer.
    using_alt_buffer: bool,
    /// Scrolling margins, 0-based and inclusive; `(0, 0)` means unset, which the
    /// getters read as the full page (upstream `_scrollMargins`).
    scroll_top_margin: VTInt,
    scroll_bottom_margin: VTInt,
    /// Horizontal margins, same convention.
    scroll_left_margin: VTInt,
    scroll_right_margin: VTInt,
    /// Terminal modes.
    modes: ModeSet,
    /// SGR save/restore stack.
    sgr_stack: Vec<types::CharacterAttributes>,
    /// Cursor styles.
    cursor_style: CursorStyle,
    /// How many callbacks reached a family that is not ported yet.
    unhandled_count: usize,
    /// The name of the last such callback.
    last_unhandled: Option<&'static str>,
}

impl AdaptDispatch {
    /// Creates a new AdaptDispatch.
    pub fn new() -> Self {
        // DECAWM is set at power-on: a terminal that does not wrap by default
        // overprints the last column instead of starting a new line.
        let mut modes = ModeSet::default();
        modes.insert(ModeFlags::DECAWM);

        Self {
            page_manager: PageManager::new(),
            term_output: TerminalOutput::new(false),
            sixel_parser: SixelParser::new(SIXEL_DEFAULT_CONFORMANCE),
            font_buffer: FontBuffer::new(),
            macro_buffer: MacroBuffer::new(),
            using_alt_buffer: false,
            scroll_top_margin: 0,
            scroll_bottom_margin: 0,
            scroll_left_margin: 0,
            scroll_right_margin: 0,
            modes,
            sgr_stack: Vec::new(),
            cursor_style: CursorStyle::UserDefault,
            unhandled_count: 0,
            last_unhandled: None,
        }
    }

    /// Returns whether insert/replace mode is enabled.
    pub fn is_insert_mode(&self) -> bool {
        self.modes.contains(ModeFlags::IRM)
    }

    /// Returns whether origin mode is enabled.
    pub fn is_origin_mode(&self) -> bool {
        self.modes.contains(ModeFlags::DECOM)
    }

    /// Returns whether auto wrap mode is enabled.
    pub fn is_auto_wrap(&self) -> bool {
        self.modes.contains(ModeFlags::DECAWM)
    }

    /// Returns whether line feed new line mode is enabled.
    pub fn is_lnm_mode(&self) -> bool {
        self.modes.contains(ModeFlags::LNM)
    }

    /// Returns whether bracketed paste mode is enabled.
    pub fn is_bracketed_paste(&self) -> bool {
        self.modes.contains(ModeFlags::BRACKETED_PASTE)
    }

    /// Returns whether the alternate screen buffer is active.
    pub fn is_alternate_buffer_active(&self) -> bool {
        self.using_alt_buffer
    }

    /// Returns the terminal output state.
    pub fn term_output(&self) -> &TerminalOutput {
        &self.term_output
    }

    /// Returns the terminal output state mutably.
    pub fn term_output_mut(&mut self) -> &mut TerminalOutput {
        &mut self.term_output
    }

    /// Returns the page manager.
    pub fn page_manager(&self) -> &PageManager {
        &self.page_manager
    }

    /// Returns the page manager mutably.
    pub fn page_manager_mut(&mut self) -> &mut PageManager {
        &mut self.page_manager
    }

    /// Returns the sixel parser.
    pub fn sixel_parser(&self) -> &SixelParser {
        &self.sixel_parser
    }

    /// Returns the sixel parser mutably.
    pub fn sixel_parser_mut(&mut self) -> &mut SixelParser {
        &mut self.sixel_parser
    }

    /// Returns the macro buffer.
    pub fn macro_buffer(&self) -> &MacroBuffer {
        &self.macro_buffer
    }

    /// Returns the macro buffer mutably.
    pub fn macro_buffer_mut(&mut self) -> &mut MacroBuffer {
        &mut self.macro_buffer
    }

    /// Returns the font buffer.
    pub fn font_buffer(&self) -> &FontBuffer {
        &self.font_buffer
    }

    /// Returns the font buffer mutably.
    pub fn font_buffer_mut(&mut self) -> &mut FontBuffer {
        &mut self.font_buffer
    }

    /// Gets the vertical margins (top, bottom).
    pub fn vertical_margins(&self) -> (VTInt, VTInt) {
        (self.scroll_top_margin, self.scroll_bottom_margin)
    }

    /// Gets the horizontal margins (left, right).
    pub fn horizontal_margins(&self) -> (VTInt, VTInt) {
        (self.scroll_left_margin, self.scroll_right_margin)
    }

    /// Sets the vertical margins.
    pub fn set_vertical_margins(&mut self, top: VTInt, bottom: VTInt) {
        self.scroll_top_margin = top;
        self.scroll_bottom_margin = bottom;
    }

    /// Sets the horizontal margins.
    pub fn set_horizontal_margins(&mut self, left: VTInt, right: VTInt) {
        self.scroll_left_margin = left;
        self.scroll_right_margin = right;
    }

    /// Gets the cursor style.
    pub fn cursor_style(&self) -> CursorStyle {
        self.cursor_style
    }

    /// Sets the cursor style.
    pub fn set_cursor_style(&mut self, style: CursorStyle) {
        self.cursor_style = style;
    }

    /// Soft resets the adapter.
    pub fn soft_reset(&mut self) {
        self.modes = ModeSet::default();
        self.sgr_stack.clear();
        self.scroll_top_margin = 0;
        self.scroll_bottom_margin = 0;
        self.scroll_left_margin = 0;
        self.scroll_right_margin = 0;
        self.term_output.soft_reset();
        self.sixel_parser.soft_reset();
    }

    /// Hard resets the adapter.
    pub fn hard_reset(&mut self, erase: bool) {
        self.soft_reset();
        self.using_alt_buffer = false;
        self.cursor_style = CursorStyle::UserDefault;
        self.modes = ModeSet::default();
        if erase {
            // Erasing the pages is part of the dispatch work; the page manager
            // owns the buffers, so the reset path has nothing to clear here yet.
        }
    }

    /// Sets a mode flag.
    pub fn set_mode(&mut self, flag: ModeFlags, enabled: bool) {
        if enabled {
            self.modes.insert(flag);
        } else {
            self.modes.remove(flag);
        }
    }

    /// Checks if a mode flag is set.
    pub fn mode_is_set(&self, flag: ModeFlags) -> bool {
        self.modes.contains(flag)
    }

    /// Returns the SGR stack.
    pub fn sgr_stack(&self) -> &[types::CharacterAttributes] {
        &self.sgr_stack
    }

    /// Pushes attributes to the SGR stack.
    pub fn push_sgr_stack(&mut self, attrs: types::CharacterAttributes) {
        self.sgr_stack.push(attrs);
    }

    /// Pops attributes from the SGR stack.
    pub fn pop_sgr_stack(&mut self) -> Option<types::CharacterAttributes> {
        self.sgr_stack.pop()
    }

    /// Returns the active page's cursor position as `(row, column)`, 0-based.
    pub fn active_cursor_position(&mut self) -> (VTInt, VTInt) {
        let buffer = self.page_manager.active_page().buffer();
        let buffer = buffer.read().unwrap_or_else(|e| e.into_inner());
        buffer.cursor().position()
    }

    /// Returns how many callbacks reached a family with no ported handler.
    pub fn unhandled_count(&self) -> usize {
        self.unhandled_count
    }

    /// Returns the name of the last callback with no ported handler.
    pub fn last_unhandled(&self) -> Option<&'static str> {
        self.last_unhandled
    }

    /// The size of the active page, from the buffer rather than the viewport so
    /// a resize cannot leave it stale.
    fn page_size(&mut self) -> (VTInt, VTInt) {
        let buffer = self.page_manager.active_page().buffer();
        let buffer = buffer.read().unwrap_or_else(|e| e.into_inner());
        (buffer.screen_width(), buffer.screen_height())
    }

    /// Effective vertical margins for a page of `page_height` rows.
    ///
    /// Ported from `_GetVerticalMargins`: margins that are unset or out of range
    /// mean the full page, so an adapter that has never seen DECSTBM still moves
    /// through every row.
    fn effective_vertical_margins(&mut self, page_height: VTInt) -> (VTInt, VTInt) {
        let bottommost = page_height - 1;
        if self.scroll_top_margin >= bottommost {
            self.scroll_top_margin = 0;
            self.scroll_bottom_margin = 0;
        }
        let set = self.scroll_top_margin < self.scroll_bottom_margin;
        let top = if set { self.scroll_top_margin } else { 0 };
        let bottom = if set { self.scroll_bottom_margin } else { bottommost };
        (top, bottom.min(bottommost))
    }

    /// Effective horizontal margins for a page `page_width` columns wide.
    fn effective_horizontal_margins(&mut self, page_width: VTInt) -> (VTInt, VTInt) {
        let rightmost = page_width - 1;
        if self.scroll_left_margin >= rightmost {
            self.scroll_left_margin = 0;
            self.scroll_right_margin = 0;
        }
        let set = self.scroll_left_margin < self.scroll_right_margin;
        let left = if set { self.scroll_left_margin } else { 0 };
        let right = if set { self.scroll_right_margin } else { rightmost };
        (left, right.min(rightmost))
    }

    /// Ported from `_CursorMovePosition`: relative or absolute movement, clamped
    /// to the page and, when asked, to the scrolling margins.
    fn move_cursor(&mut self, row_offset: Offset, col_offset: Offset, clamp_in_margins: bool) {
        let (page_width, page_height) = self.page_size();
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (left, right) = self.effective_horizontal_margins(page_width);

        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        let (current_row, current_col) = buffer.cursor().position();
        let origin_mode = self.modes.contains(ModeFlags::DECOM);

        let mut row = if row_offset.is_absolute {
            if origin_mode { top } else { 0 }
        } else {
            current_row
        };
        let mut col = if col_offset.is_absolute {
            if origin_mode { left } else { 0 }
        } else {
            current_col
        };

        row = (row + row_offset.value).clamp(0, (page_height - 1).max(0));
        col = (col + col_offset.value).clamp(0, (page_width - 1).max(0));

        if clamp_in_margins || origin_mode {
            if current_col >= left && current_col <= right {
                if current_row >= top {
                    row = row.max(top);
                }
                if current_row <= bottom {
                    row = row.min(bottom);
                }
            }
            if row >= top && row <= bottom {
                if current_col >= left {
                    col = col.max(left);
                }
                if current_col <= right {
                    col = col.min(right);
                }
            }
        }

        buffer.cursor_mut().set_position(row, col);
    }

    /// Ported from `_DoSetTopBottomScrollingMargins` (DECSTBM).
    fn set_vertical_scrolling_margins(&mut self, top_margin: VTInt, bottom_margin: VTInt, home_cursor: bool) {
        let (_, page_height) = self.page_size();
        let mut actual_top = if top_margin == 0 { 1 } else { top_margin };
        let mut actual_bottom = if bottom_margin == 0 { page_height } else { bottom_margin };

        if actual_top < actual_bottom && actual_bottom <= page_height {
            if actual_top == 1 && actual_bottom == page_height {
                actual_top = 0;
                actual_bottom = 0;
            } else {
                actual_top -= 1;
                actual_bottom -= 1;
            }
            self.scroll_top_margin = actual_top;
            self.scroll_bottom_margin = actual_bottom;

            if home_cursor {
                ITermDispatch::cursor_position(self, 1, 1);
            }
        }
    }

    /// Ported from `_DoSetLeftRightScrollingMargins` (DECSLRM).
    fn set_horizontal_scrolling_margins(&mut self, left_margin: VTInt, right_margin: VTInt, home_cursor: bool) {
        let (page_width, _) = self.page_size();
        let mut actual_left = if left_margin == 0 { 1 } else { left_margin };
        let mut actual_right = if right_margin == 0 { page_width } else { right_margin };

        if actual_left < actual_right && actual_right <= page_width {
            if actual_left == 1 && actual_right == page_width {
                actual_left = 0;
                actual_right = 0;
            } else {
                actual_left -= 1;
                actual_right -= 1;
            }
            self.scroll_left_margin = actual_left;
            self.scroll_right_margin = actual_right;

            if home_cursor {
                ITermDispatch::cursor_position(self, 1, 1);
            }
        }
    }

    /// Ported from `_WriteToBuffer`: writes text at the cursor, wrapping when
    /// the line is full, honouring insert/replace mode and the margins.
    ///
    /// The wrap is delayed by one character, as on a real VT: filling the last
    /// column leaves the cursor there so the cell can still be overwritten, and
    /// the line breaks when the next character arrives.
    fn write_to_buffer(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let wrap_at_eol = self.modes.contains(ModeFlags::DECAWM);
        let insert = self.modes.contains(ModeFlags::IRM);
        let attributes = self.page_manager.active_page().attributes();
        let mut chars = text.chars().peekable();

        while let Some(&ch) = chars.peek() {
            let (page_width, page_height) = self.page_size();
            let (top, bottom) = self.effective_vertical_margins(page_height);
            let (left, right) = self.effective_horizontal_margins(page_width);

            let buffer = self.page_manager.active_page().buffer();
            let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
            let (mut row, mut col) = buffer.cursor().position();

            if let Some(delayed) = buffer.cursor().delayed_eol_wrap() {
                buffer.cursor_mut().reset_delayed_eol_wrap();
                // Only act on the delayed wrap if the cursor has not moved since
                // it was marked; a CUP in between cancels it.
                if wrap_at_eol && delayed == (row, col) {
                    line_feed_in_region(&mut buffer, top, bottom, left, right);
                    (row, col) = buffer.cursor().position();
                }
            }

            let limit = if row >= top && row <= bottom && col <= right {
                right + 1
            } else {
                page_width
            };
            // A combining mark has no width of its own; without grapheme
            // clustering it is written as one cell so the loop can advance.
            let width = (types::codepoint_width(ch as u32) as VTInt).max(1);

            if col + width <= limit {
                let mut cell = buffer::OutputCell::from_char(ch);
                cell.set_attributes(attributes);
                if let Some(buffer_row) = buffer.get_row_at_screen_line_mut(row) {
                    buffer_row.write_cells(col, std::slice::from_ref(&cell), insert);
                }
                chars.next();
                col += width;
            } else if wrap_at_eol && col < limit {
                // A double-width glyph will not fit in the last column: wrap
                // before drawing it rather than splitting it.
                buffer.cursor_mut().delay_eol_wrap();
                continue;
            } else {
                // Autowrap is off and the glyph cannot fit; there is nowhere to
                // put it, so it is discarded (upstream drops it for the same
                // reason).
                chars.next();
            }

            if col >= limit {
                col = limit - 1;
                buffer.cursor_mut().set_position(row, col);
                if wrap_at_eol {
                    buffer.cursor_mut().delay_eol_wrap();
                }
            } else {
                buffer.cursor_mut().set_position(row, col);
            }
        }
    }
}

/// Ported from the non-scrolling and margin-scrolling arms of `_DoLineFeed`:
/// move down one row with an optional return to the left margin, or scroll the
/// region up when already on the bottom margin.
fn line_feed_in_region(buffer: &mut buffer::TextBuffer, top: VTInt, bottom: VTInt, left: VTInt, right: VTInt) {
    let (row, column) = buffer.cursor().position();
    let height = buffer.screen_height();

    let new_column = if row >= top && row <= bottom && column >= left {
        left
    } else {
        0
    };

    if row != bottom || new_column < left || new_column > right {
        let new_row = (row + 1).min(height - 1);
        buffer.cursor_mut().set_position(new_row, new_column);
    } else {
        buffer.scroll_region_up(top, bottom);
        buffer.cursor_mut().set_position(row, new_column);
    }
}

impl Default for AdaptDispatch {
    fn default() -> Self {
        Self::new()
    }
}

/// Cursor positioning and the scrolling margins, ported from
/// `adaptDispatch.cpp` (`_CursorMovePosition`, CUU–CUP, DECSTBM, DECSLRM).
///
/// Everything else still falls through to the trait's defaults, which are
/// recorded by [`AdaptDispatch::unhandled`]; the ratchet test below keeps the
/// ported set honest.
impl ITermDispatch for AdaptDispatch {
    fn unhandled(&mut self, sequence: &'static str) {
        self.unhandled_count += 1;
        self.last_unhandled = Some(sequence);
    }

    // An unknown sequence is ignored by design, not a missing handler.
    fn unknown_sequence(&mut self) {}

    fn print(&mut self, ch: char) {
        let translated = self.term_output.translate_key(ch);
        // A DEL that stays DEL is ignored, as it is when a 94-character set has
        // no mapping for it (upstream `AdaptDispatch::Print`).
        if translated as u32 != 0x7F {
            self.write_to_buffer(&translated.to_string());
        }
    }

    fn print_string(&mut self, string: &str) {
        if self.term_output.needs_translation() {
            let translated: String = string.chars().map(|ch| self.term_output.translate_key(ch)).collect();
            self.write_to_buffer(&translated);
        } else {
            self.write_to_buffer(string);
        }
    }

    fn cursor_up(&mut self, distance: VTInt) {
        self.move_cursor(Offset::backward(distance), Offset::unchanged(), true);
    }

    fn cursor_down(&mut self, distance: VTInt) {
        self.move_cursor(Offset::forward(distance), Offset::unchanged(), true);
    }

    fn cursor_forward(&mut self, distance: VTInt) {
        self.move_cursor(Offset::unchanged(), Offset::forward(distance), true);
    }

    fn cursor_backward(&mut self, distance: VTInt) {
        self.move_cursor(Offset::unchanged(), Offset::backward(distance), true);
    }

    fn cursor_next_line(&mut self, distance: VTInt) {
        self.move_cursor(Offset::forward(distance), Offset::absolute(1), true);
    }

    fn cursor_prev_line(&mut self, distance: VTInt) {
        self.move_cursor(Offset::backward(distance), Offset::absolute(1), true);
    }

    fn cursor_horizontal_position_absolute(&mut self, column: VTInt) {
        self.move_cursor(Offset::unchanged(), Offset::absolute(column), false);
    }

    fn vertical_line_position_absolute(&mut self, line: VTInt) {
        self.move_cursor(Offset::absolute(line), Offset::unchanged(), false);
    }

    fn horizontal_position_relative(&mut self, distance: VTInt) {
        self.move_cursor(Offset::unchanged(), Offset::forward(distance), false);
    }

    fn vertical_position_relative(&mut self, distance: VTInt) {
        self.move_cursor(Offset::forward(distance), Offset::unchanged(), false);
    }

    fn cursor_position(&mut self, line: VTInt, column: VTInt) {
        self.move_cursor(Offset::absolute(line), Offset::absolute(column), false);
    }

    fn set_top_bottom_scrolling_margins(&mut self, top_margin: VTInt, bottom_margin: VTInt) {
        self.set_vertical_scrolling_margins(top_margin, bottom_margin, true);
    }

    fn set_left_right_scrolling_margins(&mut self, left_margin: VTInt, right_margin: VTInt) {
        // DECSLRM is ignored unless DECLRMM (private mode 69) is set.
        if self.modes.contains(ModeFlags::DECLRMM) {
            self.set_horizontal_scrolling_margins(left_margin, right_margin, true);
        }
    }
}

/// Mode flags for terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModeSet {
    bits: u32,
}

impl ModeSet {
    /// Inserts a flag.
    pub fn insert(&mut self, flag: ModeFlags) {
        self.bits |= flag.bits();
    }

    /// Removes a flag.
    pub fn remove(&mut self, flag: ModeFlags) {
        self.bits &= !flag.bits();
    }

    /// Checks if a flag is set.
    pub fn contains(&self, flag: ModeFlags) -> bool {
        (self.bits & flag.bits()) != 0
    }
}

bitflags::bitflags! {
    /// Terminal mode flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct ModeFlags: u32 {
        /// Insert/Replace mode (IRM).
        const IRM = 1 << 0;
        /// Origin mode (DECOM).
        const DECOM = 1 << 1;
        /// Auto wrap mode (DECAWM).
        const DECAWM = 1 << 2;
        /// Line feed new line mode (LNM).
        const LNM = 1 << 3;
        /// Bracketed paste mode.
        const BRACKETED_PASTE = 1 << 4;
        /// Cursor keys mode (DECCKM).
        const DECCKM = 1 << 5;
        /// Column mode (DECCOLM).
        const DECCOLM = 1 << 6;
        /// Screen mode (DECSCNM).
        const DECSCNM = 1 << 7;
        /// Erase color mode (DECECM).
        const DECECM = 1 << 8;
        /// Left/right margin mode (DECLRMM), which gates DECSLRM.
        const DECLRMM = 1 << 9;
    }
}

#[cfg(test)]
mod tests {
    use crate::term_dispatch::ITermDispatch;

    use super::*;

    #[test]
    fn test_adapt_dispatch_new() {
        let dispatch = AdaptDispatch::new();
        assert!(!dispatch.is_insert_mode());
        assert!(!dispatch.is_origin_mode());
        assert!(dispatch.is_auto_wrap());
        assert!(!dispatch.is_alternate_buffer_active());
    }

    #[test]
    fn test_soft_reset() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_mode(ModeFlags::IRM, true);
        dispatch.soft_reset();
        assert!(!dispatch.is_insert_mode());
    }

    #[test]
    fn test_hard_reset() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_mode(ModeFlags::IRM, true);
        dispatch.hard_reset(true);
        assert!(!dispatch.is_insert_mode());
    }

    #[test]
    fn test_set_mode() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_mode(ModeFlags::IRM, true);
        assert!(dispatch.is_insert_mode());

        dispatch.set_mode(ModeFlags::IRM, false);
        assert!(!dispatch.is_insert_mode());
    }

    #[test]
    fn test_vertical_margins() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_vertical_margins(1, 20);
        assert_eq!(dispatch.vertical_margins(), (1, 20));
    }

    #[test]
    fn test_horizontal_margins() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_horizontal_margins(5, 75);
        assert_eq!(dispatch.horizontal_margins(), (5, 75));
    }

    #[test]
    fn test_cursor_style() {
        let mut dispatch = AdaptDispatch::new();
        assert_eq!(dispatch.cursor_style(), CursorStyle::UserDefault);
        dispatch.set_cursor_style(CursorStyle::BlinkingBlock);
        assert_eq!(dispatch.cursor_style(), CursorStyle::BlinkingBlock);
    }

    // Ported from `ut_adapter/adapterTest.cpp` CursorPositionTest and
    // CursorMovementTest: CUP is 1-based on the wire, 0-based in the buffer.
    #[test]
    fn cursor_position_is_one_based_on_the_wire() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(10, 20);
        assert_eq!(dispatch.active_cursor_position(), (9, 19));
    }

    #[test]
    fn cursor_movement_clamps_to_the_page() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_forward(1000);
        assert_eq!(dispatch.active_cursor_position(), (0, 79));

        dispatch.cursor_backward(1000);
        assert_eq!(dispatch.active_cursor_position(), (0, 0));

        dispatch.cursor_down(1000);
        assert_eq!(dispatch.active_cursor_position(), (23, 0));

        dispatch.cursor_up(1000);
        assert_eq!(dispatch.active_cursor_position(), (0, 0));
    }

    #[test]
    fn next_line_moves_down_and_returns_to_column_zero() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_forward(10);
        dispatch.cursor_next_line(2);
        assert_eq!(dispatch.active_cursor_position(), (2, 0));

        dispatch.cursor_prev_line(1);
        assert_eq!(dispatch.active_cursor_position(), (1, 0));
    }

    // DECSTBM stores 0-based margins and homes the cursor on success.
    #[test]
    fn scrolling_margins_home_the_cursor_and_clamp_movement() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_top_bottom_scrolling_margins(2, 20);
        assert_eq!(dispatch.vertical_margins(), (1, 19));
        assert_eq!(dispatch.active_cursor_position(), (0, 0));

        dispatch.cursor_down(1000);
        assert_eq!(dispatch.active_cursor_position(), (19, 0));
    }

    #[test]
    fn full_page_margins_clear_instead_of_storing() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_top_bottom_scrolling_margins(1, 24);
        assert_eq!(dispatch.vertical_margins(), (0, 0));
    }

    #[test]
    fn invalid_margins_are_ignored() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_top_bottom_scrolling_margins(20, 10);
        assert_eq!(dispatch.vertical_margins(), (0, 0));
        // An out-of-range bottom is also rejected rather than clamped.
        dispatch.set_top_bottom_scrolling_margins(1, 25);
        assert_eq!(dispatch.vertical_margins(), (0, 0));
    }

    // With origin mode set, an absolute position is relative to the margins.
    #[test]
    fn origin_mode_makes_absolute_positions_margin_relative() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_top_bottom_scrolling_margins(2, 20);
        dispatch.set_mode(ModeFlags::DECOM, true);

        dispatch.cursor_position(1, 1);
        assert_eq!(dispatch.active_cursor_position(), (1, 0));
    }

    #[test]
    fn decslrm_is_ignored_until_lr_margin_mode_is_set() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_left_right_scrolling_margins(5, 75);
        assert_eq!(dispatch.horizontal_margins(), (0, 0));

        dispatch.set_mode(ModeFlags::DECLRMM, true);
        dispatch.set_left_right_scrolling_margins(5, 75);
        assert_eq!(dispatch.horizontal_margins(), (4, 74));
    }

    // The ratchet: a ported family must not report, and an unported one must.
    // When a family is ported, add it to the first half of this test.
    #[test]
    fn ported_families_do_not_report_and_unported_ones_do() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_up(1);
        dispatch.cursor_down(1);
        dispatch.cursor_forward(1);
        dispatch.cursor_backward(1);
        dispatch.cursor_next_line(1);
        dispatch.cursor_prev_line(1);
        dispatch.cursor_horizontal_position_absolute(1);
        dispatch.vertical_line_position_absolute(1);
        dispatch.horizontal_position_relative(1);
        dispatch.vertical_position_relative(1);
        dispatch.cursor_position(1, 1);
        dispatch.set_top_bottom_scrolling_margins(1, 24);
        dispatch.set_left_right_scrolling_margins(1, 80);
        dispatch.unknown_sequence();
        dispatch.print('a');
        dispatch.print_string("bc");
        assert_eq!(dispatch.unhandled_count(), 0);
        assert_eq!(dispatch.last_unhandled(), None);

        dispatch.set_graphics_rendition(&VTParameters::default());
        assert_eq!(dispatch.unhandled_count(), 1);
        assert_eq!(dispatch.last_unhandled(), Some("set_graphics_rendition"));
    }

    fn cell_text(dispatch: &mut AdaptDispatch, row: i32, column: i32) -> String {
        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        buffer.get_text_data_at(row, column).unwrap_or_default().to_string()
    }

    #[test]
    fn print_writes_a_cell_and_advances() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print('A');
        assert_eq!(cell_text(&mut dispatch, 0, 0), "A");
        assert_eq!(dispatch.active_cursor_position(), (0, 1));
    }

    // Filling the last column does not wrap yet: the cell can still be
    // overwritten, and the line breaks on the next character.
    #[test]
    fn print_wraps_on_the_character_after_the_line_is_full() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print_string(&"x".repeat(80));
        assert_eq!(dispatch.active_cursor_position(), (0, 79));
        assert_eq!(cell_text(&mut dispatch, 0, 79), "x");

        dispatch.print('y');
        assert_eq!(dispatch.active_cursor_position(), (1, 1));
        assert_eq!(cell_text(&mut dispatch, 1, 0), "y");
    }

    #[test]
    fn moving_the_cursor_cancels_a_delayed_wrap() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print_string(&"x".repeat(80));
        dispatch.cursor_position(1, 1);
        dispatch.print('Z');

        assert_eq!(cell_text(&mut dispatch, 0, 0), "Z");
        assert_eq!(dispatch.active_cursor_position(), (0, 1));
    }

    #[test]
    fn print_wraps_at_the_right_margin() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_mode(ModeFlags::DECLRMM, true);
        dispatch.set_left_right_scrolling_margins(5, 75);
        dispatch.cursor_horizontal_position_absolute(5);

        // Columns 4..=74 are 71 cells; the next one wraps to column 4.
        dispatch.print_string(&"a".repeat(71));
        assert_eq!(dispatch.active_cursor_position(), (0, 74));

        dispatch.print('b');
        assert_eq!(dispatch.active_cursor_position(), (1, 5));
        assert_eq!(cell_text(&mut dispatch, 1, 4), "b");
    }

    #[test]
    fn delete_is_ignored() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print('\u{7f}');
        assert_eq!(dispatch.active_cursor_position(), (0, 0));
        assert_eq!(dispatch.unhandled_count(), 0);
    }

    #[test]
    fn insert_mode_shifts_the_rest_of_the_line() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print_string("AB");
        dispatch.cursor_position(1, 1);
        dispatch.set_mode(ModeFlags::IRM, true);
        dispatch.print('X');

        assert_eq!(cell_text(&mut dispatch, 0, 0), "X");
        assert_eq!(cell_text(&mut dispatch, 0, 1), "A");
        assert_eq!(cell_text(&mut dispatch, 0, 2), "B");
    }
}
