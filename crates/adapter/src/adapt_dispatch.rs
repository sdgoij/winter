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
use crate::term_dispatch::{ITermDispatch, OptionalFeature};
use crate::{FontBuffer, MacroBuffer, PageManager, SixelParser, TerminalOutput};
use types::TextAttribute;

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
    /// SGR save/restore stack (XTPUSHSGR / XTPOPSGR).
    sgr_stack: types::SgrStack,
    /// Cursor styles.
    cursor_style: CursorStyle,
    /// How many callbacks reached a family that is not ported yet.
    unhandled_count: usize,
    /// The name of the last such callback.
    last_unhandled: Option<&'static str>,
    /// Bytes the adapter owes the host, in order (DECRQM replies and later DSR).
    ///
    /// Upstream writes these through `ITerminalApi::ReturnResponse`; until that
    /// seam exists the adapter queues them and the host drains the queue.
    pending_responses: Vec<String>,
    /// Terminal features announced in DA1.
    optional_features: std::collections::HashSet<OptionalFeature>,
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
            sgr_stack: types::SgrStack::new(),
            cursor_style: CursorStyle::UserDefault,
            unhandled_count: 0,
            last_unhandled: None,
            pending_responses: Vec::new(),
            optional_features: std::collections::HashSet::new(),
        }
    }

    /// Appends a CSI response to the queue the host drains.
    fn push_csi(&mut self, body: &str) {
        self.pending_responses.push(format!("\x1b[{}", body));
    }

    /// Ported from `_CursorPositionReport`: 1-based, page-relative, and
    /// margin-relative when origin mode is set.
    fn cursor_position_report(&mut self, extended: bool) -> String {
        let (mut row, mut column) = self.active_cursor_position();
        column += 1;
        row += 1;

        if self.modes.contains(ModeFlags::DECOM) {
            let (page_width, page_height) = self.page_size();
            let (top, _) = self.effective_vertical_margins(page_height);
            let (left, _) = self.effective_horizontal_margins(page_width);
            column -= left;
            row -= top;
        }

        if extended {
            let page = self.page_manager.active_page().number();
            format!("?{};{};{}R", row, column, page)
        } else {
            format!("{};{}R", row, column)
        }
    }

    /// Drains the responses the adapter owes the host.
    pub fn take_responses(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_responses)
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
        self.sgr_stack = types::SgrStack::new();
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
    pub fn set_mode_flag(&mut self, flag: ModeFlags, enabled: bool) {
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

    /// Ported from `_ModeParamsHelper`: applies a mode parameter.
    ///
    /// Input-layer modes (keypad, mouse, focus) are recorded here and consumed by
    /// the input layer when it exists; recording them is what lets DECRQM answer
    /// truthfully in the meantime.
    pub(crate) fn apply_mode_param(&mut self, param: ModeParams, enable: bool) {
        use ModeParams::*;

        match param {
            Irm => self.set_mode_flag(ModeFlags::IRM, enable),
            Lnm => self.set_mode_flag(ModeFlags::LNM, enable),
            DecCkm => self.set_mode_flag(ModeFlags::DECCKM, enable),
            DecArm => self.set_mode_flag(ModeFlags::DECARM, enable),
            DecNkm => self.set_mode_flag(ModeFlags::DECNKM, enable),
            DecBkm => self.set_mode_flag(ModeFlags::DECBKM, enable),
            Vt200MouseMode => self.set_mode_flag(ModeFlags::MOUSE_VT200, enable),
            ButtonEventMouseMode => self.set_mode_flag(ModeFlags::MOUSE_BUTTON, enable),
            AnyEventMouseMode => self.set_mode_flag(ModeFlags::MOUSE_ANY, enable),
            Utf8ExtendedMode => self.set_mode_flag(ModeFlags::MOUSE_UTF8, enable),
            SgrExtendedMode => self.set_mode_flag(ModeFlags::MOUSE_SGR, enable),
            FocusEventMode => self.set_mode_flag(ModeFlags::FOCUS_EVENT, enable),
            AlternateScroll => self.set_mode_flag(ModeFlags::ALTERNATE_SCROLL, enable),
            Win32InputMode => self.set_mode_flag(ModeFlags::WIN32_INPUT, enable),
            EnableDecColmSupport => self.set_mode_flag(ModeFlags::ALLOW_DECCOLM, enable),
            DecScnm => self.set_mode_flag(ModeFlags::SCREEN_REVERSED, enable),
            SynchronizedOutput => self.set_mode_flag(ModeFlags::SYNCHRONIZED_OUTPUT, enable),
            DecSdm => self.set_mode_flag(ModeFlags::SIXEL_DISPLAY, enable),
            DecEcm => self.set_mode_flag(ModeFlags::DECECM, enable),
            BracketedPasteMode => self.set_mode_flag(ModeFlags::BRACKETED_PASTE, enable),
            DecColm => self.set_mode_flag(ModeFlags::DECCOLM, enable),
            DecOm => {
                self.set_mode_flag(ModeFlags::DECOM, enable);
                // Setting or resetting origin mode homes the cursor.
                ITermDispatch::cursor_position(self, 1, 1);
            }
            DecAwm => {
                self.set_mode_flag(ModeFlags::DECAWM, enable);
                if !enable {
                    // Turning autowrap off also discards a pending wrap.
                    let buffer = self.page_manager.active_page().buffer();
                    let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
                    buffer.cursor_mut().reset_delayed_eol_wrap();
                }
            }
            Att610StartCursorBlink => {
                let buffer = self.page_manager.active_page().buffer();
                let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
                buffer.cursor_mut().set_blinking(enable);
            }
            DecTcem => {
                let buffer = self.page_manager.active_page().buffer();
                let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
                buffer.cursor_mut().set_visible(enable);
            }
            DecLrmm => {
                self.set_mode_flag(ModeFlags::DECLRMM, enable);
                // Enabling or disabling left/right margins resets them.
                self.set_horizontal_scrolling_margins(0, 0, false);
            }
            DecPccm => {
                self.set_mode_flag(ModeFlags::PAGE_CURSOR_COUPLING, enable);
                if enable {
                    self.page_manager.make_active_page_visible();
                }
            }
            AlternateScreenBuffer => {
                self.using_alt_buffer = enable;
                if enable {
                    self.page_manager.activate_alternate();
                } else {
                    self.page_manager.deactivate_alternate();
                }
            }
            // DECANM switches the parser between ANSI and VT52, which this parser
            // does not model; GCM is deliberately inert upstream too.
            DecAnm | GraphemeClusterMode => {}
        }
    }

    /// The DECRQM state for a mode: 0 unsupported, 1 enabled, 2 disabled.
    fn mode_state(&mut self, param: ModeParams) -> u8 {
        use ModeParams::*;

        let flag = match param {
            Irm => ModeFlags::IRM,
            Lnm => ModeFlags::LNM,
            DecCkm => ModeFlags::DECCKM,
            DecArm => ModeFlags::DECARM,
            DecNkm => ModeFlags::DECNKM,
            DecBkm => ModeFlags::DECBKM,
            Vt200MouseMode => ModeFlags::MOUSE_VT200,
            ButtonEventMouseMode => ModeFlags::MOUSE_BUTTON,
            AnyEventMouseMode => ModeFlags::MOUSE_ANY,
            Utf8ExtendedMode => ModeFlags::MOUSE_UTF8,
            SgrExtendedMode => ModeFlags::MOUSE_SGR,
            FocusEventMode => ModeFlags::FOCUS_EVENT,
            AlternateScroll => ModeFlags::ALTERNATE_SCROLL,
            Win32InputMode => ModeFlags::WIN32_INPUT,
            EnableDecColmSupport => ModeFlags::ALLOW_DECCOLM,
            DecScnm => ModeFlags::SCREEN_REVERSED,
            SynchronizedOutput => ModeFlags::SYNCHRONIZED_OUTPUT,
            DecSdm => ModeFlags::SIXEL_DISPLAY,
            DecEcm => ModeFlags::DECECM,
            BracketedPasteMode => ModeFlags::BRACKETED_PASTE,
            DecColm => ModeFlags::DECCOLM,
            DecOm => ModeFlags::DECOM,
            DecAwm => ModeFlags::DECAWM,
            DecLrmm => ModeFlags::DECLRMM,
            DecPccm => ModeFlags::PAGE_CURSOR_COUPLING,
            Att610StartCursorBlink => {
                let buffer = self.page_manager.active_page().buffer();
                let buffer = buffer.read().unwrap_or_else(|e| e.into_inner());
                return u8::from(!buffer.cursor().is_blinking()) + 1;
            }
            DecTcem => {
                let buffer = self.page_manager.active_page().buffer();
                let buffer = buffer.read().unwrap_or_else(|e| e.into_inner());
                return u8::from(!buffer.cursor().is_visible()) + 1;
            }
            AlternateScreenBuffer => return u8::from(!self.using_alt_buffer) + 1,
            DecAnm | GraphemeClusterMode => return 0,
        };

        u8::from(!self.modes.contains(flag)) + 1
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

    /// The attributes erasing writes, ported from `_GetEraseAttributes`.
    ///
    /// DECECM (erase colour mode) erases to the default background; otherwise the
    /// page's current attributes are used, stripped of everything but colour.
    fn erase_attributes(&mut self) -> TextAttribute {
        if self.modes.contains(ModeFlags::DECECM) {
            TextAttribute::new()
        } else {
            let mut attributes = self.page_manager.active_page().attributes();
            attributes.set_standard_erase();
            attributes
        }
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
                    do_line_feed(&mut buffer, top, bottom, left, right, true, true);
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
/// Ported from `_DoLineFeed`: move down one row with an optional return to the
/// left margin, or scroll the region up when already on the bottom margin.
///
/// `wrap_forced` records on the row that it was broken by a wrap rather than by
/// an explicit line feed; reflow will read it.
fn do_line_feed(
    buffer: &mut buffer::TextBuffer,
    top: VTInt,
    bottom: VTInt,
    left: VTInt,
    right: VTInt,
    with_return: bool,
    wrap_forced: bool,
) {
    let (row, column) = buffer.cursor().position();
    let height = buffer.screen_height();

    if let Some(buffer_row) = buffer.get_row_at_screen_line_mut(row) {
        buffer_row.set_wrap(wrap_forced);
    }

    let new_column = if with_return {
        let clamp_to_margin = row >= top && row <= bottom && column >= left;
        if clamp_to_margin { left } else { 0 }
    } else {
        column
    };

    if row != bottom || new_column < left || new_column > right {
        let new_row = (row + 1).min(height - 1);
        buffer.cursor_mut().set_position(new_row, new_column);
    } else {
        buffer.scroll_region_up(top, bottom);
        buffer.cursor_mut().set_position(row, new_column);
    }
}

/// Fills a half-open rectangle (`bottom`/`right` exclusive) with erased cells.
fn fill_region(
    buffer: &mut buffer::TextBuffer,
    top: VTInt,
    left: VTInt,
    bottom: VTInt,
    right: VTInt,
    attributes: TextAttribute,
) {
    if bottom <= top || right <= left {
        return;
    }
    let mut cell = buffer::OutputCell::from_char(' ');
    cell.set_attributes(attributes);
    buffer.fill_rect(top, left, bottom - 1, right - 1, &cell);
}

/// Ported from `_SelectiveEraseRect`: clears the text of unprotected cells and
/// leaves their attributes alone, which is what makes DECSED/DECSEL selective.
fn selective_erase_region(buffer: &mut buffer::TextBuffer, top: VTInt, left: VTInt, bottom: VTInt, right: VTInt) {
    for row in top..bottom {
        for column in left..right {
            let attributes = buffer
                .get_row_at_screen_line(row)
                .map(|buffer_row| *buffer_row.get_cell(column).get_attributes());
            let Some(attributes) = attributes else {
                continue;
            };
            if attributes.is_protected() {
                continue;
            }
            if let Some(buffer_row) = buffer.get_row_at_screen_line_mut(row) {
                let mut cell = buffer::OutputCell::from_char(' ');
                cell.set_attributes(attributes);
                buffer_row.write_cell(column, cell);
            }
        }
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

    fn set_graphics_rendition(&mut self, options: &VTParameters) {
        let mut attributes = self.page_manager.active_page().attributes();
        Self::apply_graphics_options(options, &mut attributes);
        self.page_manager.active_page().set_attributes(attributes);
    }

    fn set_character_protection_attribute(&mut self, options: &VTParameters) {
        let mut attributes = self.page_manager.active_page().attributes();
        Self::apply_logical_attribute(options, &mut attributes);
        self.page_manager.active_page().set_attributes(attributes);
    }

    fn push_graphics_rendition(&mut self, options: &VTParameters) {
        let attributes = self.page_manager.active_page().attributes();
        let values: Vec<VTInt> = (0..options.len()).map(|index| options.at(index).value_or(0)).collect();
        self.sgr_stack.push(attributes, &values);
    }

    fn pop_graphics_rendition(&mut self) {
        let current = self.page_manager.active_page().attributes();
        let restored = self.sgr_stack.pop(current);
        self.page_manager.active_page().set_attributes(restored);
    }

    fn carriage_return(&mut self) {
        self.move_cursor(Offset::unchanged(), Offset::absolute(1), true);
    }

    fn line_feed(&mut self, line_feed_type: LineFeedType) {
        let with_return = match line_feed_type {
            LineFeedType::DependsOnMode => self.modes.contains(ModeFlags::LNM),
            LineFeedType::WithReturn => true,
            LineFeedType::WithoutReturn => false,
        };

        let (page_width, page_height) = self.page_size();
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (left, right) = self.effective_horizontal_margins(page_width);

        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        do_line_feed(&mut buffer, top, bottom, left, right, with_return, false);
    }

    fn reverse_line_feed(&mut self) {
        let (page_width, page_height) = self.page_size();
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (left, right) = self.effective_horizontal_margins(page_width);

        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        let (row, column) = buffer.cursor().position();

        if row == top && column >= left && column <= right {
            // At the top of the region there is nowhere to go, so the content is
            // pushed down instead: an inserted line at the margin.
            buffer.scroll_region_down(top, bottom);
        } else if row > 0 {
            buffer.cursor_mut().set_position(row - 1, column);
        }
    }

    fn back_index(&mut self) {
        let (page_width, page_height) = self.page_size();
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (left, right) = self.effective_horizontal_margins(page_width);

        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        let (row, column) = buffer.cursor().position();

        if column == left && row >= top && row <= bottom {
            buffer.scroll_region_right(top, bottom, left, right);
        } else if column > 0 {
            buffer.cursor_mut().set_position(row, column - 1);
        }
    }

    fn forward_index(&mut self) {
        let (page_width, page_height) = self.page_size();
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (left, right) = self.effective_horizontal_margins(page_width);

        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        let (row, column) = buffer.cursor().position();

        if column == right && row >= top && row <= bottom {
            buffer.scroll_region_left(top, bottom, left, right);
        } else if column < page_width - 1 {
            buffer.cursor_mut().set_position(row, column + 1);
        }
    }

    fn erase_in_display(&mut self, erase_type: EraseType) {
        if matches!(erase_type, EraseType::Scrollback) {
            // A buffer with no scrollback has nothing above the screen to clear.
            return;
        }

        let (page_width, page_height) = self.page_size();
        let attributes = self.erase_attributes();
        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        let (row, column) = buffer.cursor().position();
        buffer.cursor_mut().reset_delayed_eol_wrap();

        match erase_type {
            EraseType::FromBeginning => {
                fill_region(&mut buffer, 0, 0, row, page_width, attributes);
                fill_region(&mut buffer, row, 0, row + 1, column + 1, attributes);
            }
            EraseType::ToEnd => {
                fill_region(&mut buffer, row, column, row + 1, page_width, attributes);
                fill_region(&mut buffer, row + 1, 0, page_height, page_width, attributes);
            }
            EraseType::All => {
                fill_region(&mut buffer, 0, 0, page_height, page_width, attributes);
            }
            EraseType::Scrollback => {}
        }
    }

    fn erase_in_line(&mut self, erase_type: EraseType) {
        let (page_width, _) = self.page_size();
        let attributes = self.erase_attributes();
        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        let (row, column) = buffer.cursor().position();
        buffer.cursor_mut().reset_delayed_eol_wrap();

        match erase_type {
            EraseType::FromBeginning => fill_region(&mut buffer, row, 0, row + 1, column + 1, attributes),
            EraseType::ToEnd => fill_region(&mut buffer, row, column, row + 1, page_width, attributes),
            EraseType::All => fill_region(&mut buffer, row, 0, row + 1, page_width, attributes),
            EraseType::Scrollback => {}
        }
    }

    fn erase_characters(&mut self, count: VTInt) {
        if count <= 0 {
            return;
        }
        let (page_width, _) = self.page_size();
        let attributes = self.erase_attributes();
        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        let (row, column) = buffer.cursor().position();
        buffer.cursor_mut().reset_delayed_eol_wrap();

        // ECH never wraps: it stops at the end of the line.
        let end = (column + count).min(page_width);
        fill_region(&mut buffer, row, column, row + 1, end, attributes);
    }

    fn selective_erase_in_display(&mut self, erase_type: EraseType) {
        let (page_width, page_height) = self.page_size();
        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        let (row, column) = buffer.cursor().position();
        buffer.cursor_mut().reset_delayed_eol_wrap();

        match erase_type {
            EraseType::FromBeginning => {
                selective_erase_region(&mut buffer, 0, 0, row, page_width);
                selective_erase_region(&mut buffer, row, 0, row + 1, column + 1);
            }
            EraseType::ToEnd => {
                selective_erase_region(&mut buffer, row, column, row + 1, page_width);
                selective_erase_region(&mut buffer, row + 1, 0, page_height, page_width);
            }
            EraseType::All => selective_erase_region(&mut buffer, 0, 0, page_height, page_width),
            EraseType::Scrollback => {}
        }
    }

    fn selective_erase_in_line(&mut self, erase_type: EraseType) {
        let (page_width, _) = self.page_size();
        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        let (row, column) = buffer.cursor().position();
        buffer.cursor_mut().reset_delayed_eol_wrap();

        match erase_type {
            EraseType::FromBeginning => selective_erase_region(&mut buffer, row, 0, row + 1, column + 1),
            EraseType::ToEnd => selective_erase_region(&mut buffer, row, column, row + 1, page_width),
            EraseType::All => selective_erase_region(&mut buffer, row, 0, row + 1, page_width),
            EraseType::Scrollback => {}
        }
    }

    fn set_mode(&mut self, param: &ModeParams) {
        self.apply_mode_param(*param, true);
    }

    fn reset_mode(&mut self, param: &ModeParams) {
        self.apply_mode_param(*param, false);
    }

    fn request_mode(&mut self, param: &ModeParams) {
        let param = *param;
        let state = self.mode_state(param);
        let private = if param.is_private() { "?" } else { "" };
        self.pending_responses
            .push(format!("\x1b[{}{};{}$y", private, param.param_number(), state));
    }

    fn device_status_report(&mut self, status_type: StatusType, _id: VTParameter) {
        let body = match status_type {
            StatusType::OperatingStatus => "0n".to_string(),
            StatusType::CursorPositionReport => self.cursor_position_report(false),
            StatusType::ExtendedCursorPositionReport => self.cursor_position_report(true),
            StatusType::PrinterStatus => "?13n".to_string(),
            StatusType::UserDefinedKeys => "?23n".to_string(),
            StatusType::KeyboardStatus => "?27;0;0;5n".to_string(),
            StatusType::LocatorStatus => "?53n".to_string(),
            StatusType::LocatorIdentity => "?57;0n".to_string(),
            StatusType::DataIntegrity => "?70n".to_string(),
            StatusType::MultipleSessionStatus => "?83n".to_string(),
            // The macro space and checksum reports need the macro buffer to
            // answer; they are the two DSR forms that stay open.
            StatusType::MacroSpaceReport | StatusType::MemoryChecksum => return,
        };
        self.push_csi(&body);
    }

    fn device_attributes(&mut self) {
        // DA1: VT100 with advanced video, sixel, selective erase, soft fonts,
        // 8-bit interface, horizontal scrolling, colour, Greek/Turkish sets,
        // rectangular operations, macros, Latin-2, and clipboard if enabled.
        if self.optional_features.contains(&OptionalFeature::ClipboardWrite) {
            self.push_csi("?61;4;6;7;14;21;22;23;24;28;32;42;52c");
        } else {
            self.push_csi("?61;4;6;7;14;21;22;23;24;28;32;42c");
        }
    }

    fn secondary_device_attributes(&mut self) {
        self.push_csi(">0;10;1c");
    }

    fn tertiary_device_attributes(&mut self) {
        self.pending_responses.push("\x1bP!|00000000\x1b\\".to_string());
    }

    fn vt52_device_attributes(&mut self) {
        // A terminal emulating a VT52 answers with ESC / Z, not ESC / K.
        self.pending_responses.push("\x1b/Z".to_string());
    }

    fn request_terminal_parameters(&mut self, permission: ReportingPermission) {
        match permission {
            ReportingPermission::Unsolicited => self.push_csi("2;1;1;128;128;1;0x"),
            ReportingPermission::Solicited => self.push_csi("3;1;1;128;128;1;0x"),
        }
    }

    fn set_optional_features(&mut self, features: std::collections::HashSet<OptionalFeature>) {
        self.optional_features = features;
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
        /// Auto-repeat (DECARM); input-layer state, tracked for DECRQM.
        const DECARM = 1 << 10;
        /// Numeric keypad (DECNKM).
        const DECNKM = 1 << 11;
        /// Backarrow key (DECBKM).
        const DECBKM = 1 << 12;
        /// VT200 mouse tracking.
        const MOUSE_VT200 = 1 << 13;
        /// Button-event mouse tracking.
        const MOUSE_BUTTON = 1 << 14;
        /// Any-event mouse tracking.
        const MOUSE_ANY = 1 << 15;
        /// UTF-8 mouse coordinates.
        const MOUSE_UTF8 = 1 << 16;
        /// SGR mouse coordinates.
        const MOUSE_SGR = 1 << 17;
        /// Focus event reporting.
        const FOCUS_EVENT = 1 << 18;
        /// Alternate scroll mode.
        const ALTERNATE_SCROLL = 1 << 19;
        /// Win32 input records.
        const WIN32_INPUT = 1 << 20;
        /// DECCOLM is permitted (XTERM mode 40).
        const ALLOW_DECCOLM = 1 << 21;
        /// Reverse the screen (DECSCNM); the renderer reads this.
        const SCREEN_REVERSED = 1 << 22;
        /// Hold the frame until the program ends the block (mode 2026).
        const SYNCHRONIZED_OUTPUT = 1 << 23;
        /// Sixel image display mode (DECSDM).
        const SIXEL_DISPLAY = 1 << 24;
        /// Keep the visible page following the active one (DECPCCM).
        const PAGE_CURSOR_COUPLING = 1 << 25;
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
        dispatch.set_mode_flag(ModeFlags::IRM, true);
        dispatch.soft_reset();
        assert!(!dispatch.is_insert_mode());
    }

    #[test]
    fn test_hard_reset() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_mode_flag(ModeFlags::IRM, true);
        dispatch.hard_reset(true);
        assert!(!dispatch.is_insert_mode());
    }

    #[test]
    fn test_set_mode() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_mode_flag(ModeFlags::IRM, true);
        assert!(dispatch.is_insert_mode());

        dispatch.set_mode_flag(ModeFlags::IRM, false);
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
        dispatch.set_mode_flag(ModeFlags::DECOM, true);

        dispatch.cursor_position(1, 1);
        assert_eq!(dispatch.active_cursor_position(), (1, 0));
    }

    #[test]
    fn decslrm_is_ignored_until_lr_margin_mode_is_set() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_left_right_scrolling_margins(5, 75);
        assert_eq!(dispatch.horizontal_margins(), (0, 0));

        dispatch.set_mode_flag(ModeFlags::DECLRMM, true);
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
        dispatch.set_graphics_rendition(&VTParameters::default());
        dispatch.set_character_protection_attribute(&VTParameters::default());
        dispatch.push_graphics_rendition(&VTParameters::default());
        dispatch.pop_graphics_rendition();
        dispatch.carriage_return();
        dispatch.line_feed(LineFeedType::WithoutReturn);
        dispatch.reverse_line_feed();
        dispatch.back_index();
        dispatch.forward_index();
        dispatch.erase_in_display(EraseType::All);
        dispatch.erase_in_line(EraseType::All);
        dispatch.erase_characters(1);
        dispatch.selective_erase_in_display(EraseType::All);
        dispatch.selective_erase_in_line(EraseType::All);
        dispatch.set_mode(&ModeParams::DecAwm);
        dispatch.reset_mode(&ModeParams::DecAwm);
        dispatch.request_mode(&ModeParams::DecOm);
        dispatch.device_status_report(StatusType::OperatingStatus, VTParameter::default());
        dispatch.device_attributes();
        dispatch.secondary_device_attributes();
        dispatch.tertiary_device_attributes();
        dispatch.vt52_device_attributes();
        dispatch.request_terminal_parameters(ReportingPermission::Solicited);
        dispatch.set_optional_features(std::collections::HashSet::new());
        assert_eq!(dispatch.unhandled_count(), 0);
        assert_eq!(dispatch.last_unhandled(), None);

        dispatch.set_window_title("x");
        assert_eq!(dispatch.unhandled_count(), 1);
        assert_eq!(dispatch.last_unhandled(), Some("set_window_title"));
    }

    fn page_attributes(dispatch: &mut AdaptDispatch) -> types::TextAttribute {
        dispatch.page_manager_mut().active_page().attributes()
    }

    #[test]
    fn sgr_writes_the_page_attributes() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_graphics_rendition(&VTParameters::from_values(vec![1, 31]));

        let attributes = page_attributes(&mut dispatch);
        assert!(attributes.is_intense());
        assert_eq!(attributes.get_foreground(), types::TextColor::index16(4));
    }

    #[test]
    fn printed_text_carries_the_page_attributes() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_graphics_rendition(&VTParameters::from_values(vec![31]));
        dispatch.print('A');

        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        let cell = buffer.get_cell_data_at(0, 0).unwrap();
        assert_eq!(cell.get_attributes().get_foreground(), types::TextColor::index16(4));
    }

    #[test]
    fn push_and_pop_restore_attributes() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_graphics_rendition(&VTParameters::from_values(vec![1]));
        dispatch.push_graphics_rendition(&VTParameters::default());

        dispatch.set_graphics_rendition(&VTParameters::from_values(vec![22]));
        assert!(!page_attributes(&mut dispatch).is_intense());

        dispatch.pop_graphics_rendition();
        assert!(page_attributes(&mut dispatch).is_intense());
    }

    #[test]
    fn line_feed_moves_down_without_returning() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(1, 6);
        dispatch.line_feed(LineFeedType::WithoutReturn);
        assert_eq!(dispatch.active_cursor_position(), (1, 5));
    }

    #[test]
    fn line_feed_with_lnm_returns_to_the_margin() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(1, 6);
        dispatch.set_mode_flag(ModeFlags::LNM, true);
        dispatch.line_feed(LineFeedType::DependsOnMode);
        assert_eq!(dispatch.active_cursor_position(), (1, 0));
    }

    #[test]
    fn line_feed_at_the_bottom_scrolls_the_region() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(2, 1);
        dispatch.print('B');
        dispatch.cursor_position(24, 1);
        dispatch.line_feed(LineFeedType::WithoutReturn);

        assert_eq!(dispatch.active_cursor_position(), (23, 0));
        assert_eq!(cell_text(&mut dispatch, 0, 0), "B");
        assert_eq!(cell_text(&mut dispatch, 23, 0), " ");
    }

    // RI at the top margin has nowhere to go, so it inserts a line instead.
    #[test]
    fn reverse_line_feed_at_the_top_inserts_a_line() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print('A');
        dispatch.cursor_position(1, 1);
        dispatch.reverse_line_feed();

        assert_eq!(dispatch.active_cursor_position(), (0, 0));
        assert_eq!(cell_text(&mut dispatch, 1, 0), "A");
    }

    #[test]
    fn reverse_line_feed_moves_up_otherwise() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(5, 3);
        dispatch.reverse_line_feed();
        assert_eq!(dispatch.active_cursor_position(), (3, 2));
    }

    #[test]
    fn carriage_return_respects_the_left_margin() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_mode_flag(ModeFlags::DECLRMM, true);
        dispatch.set_left_right_scrolling_margins(5, 75);
        dispatch.cursor_position(1, 10);
        dispatch.carriage_return();

        assert_eq!(dispatch.active_cursor_position(), (0, 4));
    }

    #[test]
    fn back_index_scrolls_right_at_the_left_margin() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print('A');
        dispatch.print('B');
        dispatch.cursor_position(1, 1);
        dispatch.back_index();

        assert_eq!(dispatch.active_cursor_position(), (0, 0));
        assert_eq!(cell_text(&mut dispatch, 0, 1), "A");
        assert_eq!(cell_text(&mut dispatch, 0, 2), "B");
    }

    #[test]
    fn forward_index_scrolls_left_at_the_right_margin() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(1, 2);
        dispatch.print('A');
        dispatch.cursor_position(1, 80);
        dispatch.forward_index();

        assert_eq!(dispatch.active_cursor_position(), (0, 79));
        assert_eq!(cell_text(&mut dispatch, 0, 0), "A");
    }

    #[test]
    fn erase_in_line_to_end_blanks_the_tail() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print_string("hello");
        dispatch.cursor_position(1, 3);
        dispatch.erase_in_line(EraseType::ToEnd);

        assert_eq!(cell_text(&mut dispatch, 0, 1), "e");
        assert_eq!(cell_text(&mut dispatch, 0, 2), " ");
        assert_eq!(cell_text(&mut dispatch, 0, 4), " ");
    }

    #[test]
    fn erase_in_line_from_beginning_blanks_the_head() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print_string("hello");
        dispatch.cursor_position(1, 3);
        dispatch.erase_in_line(EraseType::FromBeginning);

        assert_eq!(cell_text(&mut dispatch, 0, 0), " ");
        assert_eq!(cell_text(&mut dispatch, 0, 2), " ");
        assert_eq!(cell_text(&mut dispatch, 0, 3), "l");
    }

    #[test]
    fn erase_characters_stops_at_the_end_of_the_line() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(2, 1);
        dispatch.print('Z');
        dispatch.cursor_position(1, 1);
        dispatch.erase_characters(1000);

        assert_eq!(cell_text(&mut dispatch, 0, 79), " ");
        // The erase did not wrap into the next row.
        assert_eq!(cell_text(&mut dispatch, 1, 0), "Z");
        assert_eq!(dispatch.active_cursor_position(), (0, 0));
    }

    #[test]
    fn erase_in_display_all_blanks_every_row() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print_string("top");
        dispatch.cursor_position(2, 1);
        dispatch.print_string("second");
        dispatch.cursor_position(2, 3);
        dispatch.erase_in_display(EraseType::All);

        assert_eq!(cell_text(&mut dispatch, 0, 0), " ");
        assert_eq!(cell_text(&mut dispatch, 1, 0), " ");
        assert_eq!(cell_text(&mut dispatch, 23, 79), " ");
        assert_eq!(dispatch.active_cursor_position(), (1, 2));
    }

    #[test]
    fn erase_in_display_to_end_keeps_the_rows_above() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print_string("top");
        dispatch.cursor_position(2, 3);
        dispatch.print_string("second");
        dispatch.erase_in_display(EraseType::ToEnd);

        assert_eq!(cell_text(&mut dispatch, 0, 0), "t");
        assert_eq!(cell_text(&mut dispatch, 1, 7), "d");
        assert_eq!(cell_text(&mut dispatch, 1, 8), " ");
        assert_eq!(cell_text(&mut dispatch, 2, 0), " ");
    }

    #[test]
    fn erase_uses_the_page_attributes_unless_erase_colour_mode() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_graphics_rendition(&VTParameters::from_values(vec![31]));
        dispatch.erase_in_line(EraseType::All);

        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        let cell = buffer.get_cell_data_at(0, 0).unwrap();
        assert_eq!(cell.get_attributes().get_foreground(), types::TextColor::index16(4));
        drop(buffer);

        // DECECM erases to the default background instead of the current colour.
        dispatch.set_mode_flag(ModeFlags::DECECM, true);
        dispatch.erase_in_line(EraseType::All);
        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        let cell = buffer.get_cell_data_at(0, 0).unwrap();
        assert!(cell.get_attributes().get_foreground().is_default());
    }

    #[test]
    fn selective_erase_skips_protected_cells() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print_string("AB");
        dispatch.set_character_protection_attribute(&VTParameters::from_values(vec![1]));
        dispatch.print('C');

        dispatch.selective_erase_in_line(EraseType::All);

        assert_eq!(cell_text(&mut dispatch, 0, 0), " ");
        assert_eq!(cell_text(&mut dispatch, 0, 1), " ");
        assert_eq!(cell_text(&mut dispatch, 0, 2), "C");
    }

    #[test]
    fn decset_changes_state_and_decrqm_reports_it() {
        let mut dispatch = AdaptDispatch::new();
        assert!(!dispatch.is_insert_mode());

        dispatch.set_mode(&ModeParams::Irm);
        assert!(dispatch.is_insert_mode());

        dispatch.request_mode(&ModeParams::Irm);
        assert_eq!(dispatch.take_responses(), vec!["\x1b[4;1$y".to_string()]);

        dispatch.reset_mode(&ModeParams::Irm);
        dispatch.request_mode(&ModeParams::Irm);
        assert_eq!(dispatch.take_responses(), vec!["\x1b[4;2$y".to_string()]);
    }

    #[test]
    fn origin_mode_homes_the_cursor() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(5, 5);
        dispatch.set_mode(&ModeParams::DecOm);

        assert!(dispatch.is_origin_mode());
        assert_eq!(dispatch.active_cursor_position(), (0, 0));
    }

    #[test]
    fn turning_off_autowrap_discards_a_pending_wrap() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print_string(&"x".repeat(80));
        assert_eq!(dispatch.active_cursor_position(), (0, 79));

        dispatch.reset_mode(&ModeParams::DecAwm);
        dispatch.print('y');

        // Autowrap is off, so the character overwrites the last column.
        assert_eq!(dispatch.active_cursor_position(), (0, 79));
        assert_eq!(cell_text(&mut dispatch, 0, 79), "y");
    }

    #[test]
    fn alternate_screen_mode_switches_pages() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_mode(&ModeParams::AlternateScreenBuffer);

        assert!(dispatch.is_alternate_buffer_active());
        assert_eq!(dispatch.page_manager_mut().active_page().number(), 2);

        dispatch.reset_mode(&ModeParams::AlternateScreenBuffer);
        assert!(!dispatch.is_alternate_buffer_active());
        assert_eq!(dispatch.page_manager_mut().active_page().number(), 1);
    }

    #[test]
    fn decrqm_reports_unsupported_for_unmodelled_modes() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.request_mode(&ModeParams::DecAnm);
        assert_eq!(dispatch.take_responses(), vec!["\x1b[?2;0$y".to_string()]);
    }

    #[test]
    fn da1_announces_the_terminal_capabilities() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.device_attributes();
        assert_eq!(
            dispatch.take_responses(),
            vec!["\x1b[?61;4;6;7;14;21;22;23;24;28;32;42c".to_string()]
        );
    }

    #[test]
    fn da1_includes_clipboard_only_when_enabled() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_optional_features(std::collections::HashSet::from([OptionalFeature::ClipboardWrite]));
        dispatch.device_attributes();
        assert_eq!(
            dispatch.take_responses(),
            vec!["\x1b[?61;4;6;7;14;21;22;23;24;28;32;42;52c".to_string()]
        );
    }

    #[test]
    fn cpr_reports_a_one_based_position() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(10, 5);
        dispatch.device_status_report(StatusType::CursorPositionReport, VTParameter::default());
        assert_eq!(dispatch.take_responses(), vec!["\x1b[10;5R".to_string()]);
    }

    #[test]
    fn extended_cpr_includes_the_page_number() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(10, 5);
        dispatch.device_status_report(StatusType::ExtendedCursorPositionReport, VTParameter::default());
        assert_eq!(dispatch.take_responses(), vec!["\x1b[?10;5;1R".to_string()]);
    }

    #[test]
    fn vt52_identifies_with_slash_z() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.vt52_device_attributes();
        assert_eq!(dispatch.take_responses(), vec!["\x1b/Z".to_string()]);
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
        dispatch.set_mode_flag(ModeFlags::DECLRMM, true);
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
        dispatch.set_mode_flag(ModeFlags::IRM, true);
        dispatch.print('X');

        assert_eq!(cell_text(&mut dispatch, 0, 0), "X");
        assert_eq!(cell_text(&mut dispatch, 0, 1), "A");
        assert_eq!(cell_text(&mut dispatch, 0, 2), "B");
    }
}
