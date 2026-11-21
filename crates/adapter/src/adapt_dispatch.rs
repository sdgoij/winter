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

/// Main VT adapter dispatcher.
///
/// State only, for now: the handlers arrive with the `ITermDispatch`
/// implementation, and each one is expected to name the sequence it serves.
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
    /// Scrolling margins (top, bottom).
    scroll_top_margin: VTInt,
    scroll_bottom_margin: VTInt,
    /// Horizontal margins (left, right).
    scroll_left_margin: VTInt,
    scroll_right_margin: VTInt,
    /// Terminal modes.
    modes: ModeSet,
    /// SGR save/restore stack.
    sgr_stack: Vec<types::CharacterAttributes>,
    /// Cursor styles.
    cursor_style: CursorStyle,
}

impl AdaptDispatch {
    /// Creates a new AdaptDispatch.
    pub fn new() -> Self {
        Self {
            page_manager: PageManager::new(),
            term_output: TerminalOutput::new(false),
            sixel_parser: SixelParser::new(SIXEL_DEFAULT_CONFORMANCE),
            font_buffer: FontBuffer::new(),
            macro_buffer: MacroBuffer::new(),
            using_alt_buffer: false,
            scroll_top_margin: 0,
            scroll_bottom_margin: 23,
            scroll_left_margin: 0,
            scroll_right_margin: 79,
            modes: ModeSet::default(),
            sgr_stack: Vec::new(),
            cursor_style: CursorStyle::UserDefault,
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
        self.scroll_bottom_margin = 23;
        self.scroll_left_margin = 0;
        self.scroll_right_margin = 79;
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
}

impl Default for AdaptDispatch {
    fn default() -> Self {
        Self::new()
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapt_dispatch_new() {
        let dispatch = AdaptDispatch::new();
        assert!(!dispatch.is_insert_mode());
        assert!(!dispatch.is_origin_mode());
        assert!(!dispatch.is_auto_wrap());
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
}
