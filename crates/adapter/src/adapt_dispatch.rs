//! `AdaptDispatch` — VT adapter state.
//!
//! Ported from `adaptDispatch.hpp/cpp` in the Windows Terminal C++ codebase.
//!
//! The struct holds the state the dispatch handlers share: modes, margins, the
//! page manager, the terminal output, the sixel parser, and the SGR stack. The
//! ported `ITermDispatch` families are listed in `plan.md` §"Pre-Phase-5
//! cleanup"; everything else falls through to the trait default, which
//! `AdaptDispatch::unhandled` records.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::dispatch_types::*;
use crate::sixel_parser::{DecodedImage, SIXEL_DEFAULT_CONFORMANCE};
use crate::term_dispatch::{ITermDispatch, OptionalFeature};
use crate::{FontBuffer, MacroBuffer, PageManager, SixelParser, TerminalOutput};
use types::color_table::{
    CURSOR_COLOR, ColorAlias, ColorTable, DEFAULT_BACKGROUND, DEFAULT_FOREGROUND, SELECTION_BACKGROUND, TABLE_SIZE,
    color_from_hls, color_from_rgb100, color_to_hls, color_to_rgb100,
};
use types::{CharacterAttributes, TextAttribute, TextColor, UnderlineStyle};

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
    /// Character attributes AND mask.
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

impl Default for ChangeOps {
    /// Matches upstream's member initialisers: an identity AND mask, no XOR
    /// changes and no colours.
    fn default() -> Self {
        Self {
            and_attr_mask: CharacterAttributes::All,
            xor_attr_mask: CharacterAttributes::Normal,
            foreground: None,
            background: None,
            underline_color: None,
        }
    }
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
    term_output: Arc<Mutex<TerminalOutput>>,
    /// Sixel parser.
    sixel_parser: std::sync::Arc<std::sync::Mutex<SixelParser>>,
    /// Font buffer.
    font_buffer: FontBuffer,
    /// Macro buffer.
    macro_buffer: std::sync::Arc<std::sync::Mutex<MacroBuffer>>,
    /// The VT colour table shared with the DECRSTS restore handler.
    color_table: std::sync::Arc<std::sync::Mutex<ColorTable>>,
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
    /// Macro sequences waiting to be injected into the parser by the host.
    pending_macro_input: Vec<Vec<u8>>,
    /// Terminal features announced in DA1.
    optional_features: std::collections::HashSet<OptionalFeature>,
    /// Tab stops by column; empty until first use, then every eighth column.
    tab_stops: Vec<bool>,
    /// Saved cursor states, indexed by buffer (0 main, 1 alternate).
    saved_cursor_state: [CursorState; 2],
    /// The window title set by OSC 0/2, if any.
    window_title: Option<String>,
    /// The working-directory URI set by OSC 7, as given.
    current_working_directory: Option<String>,
    /// Clipboard content set by OSC 52, waiting for the host to take it.
    clipboard_content: Option<String>,
    /// Window visibility requested by CSI 1/2 t; true means show.
    pending_window_visibility: Option<bool>,
    /// Window size requested by CSI 8 t as `(width, height)`; 0 means unchanged.
    pending_window_resize: Option<(VTInt, VTInt)>,
    /// Whether CSI 7 t asked for a full redraw.
    redraw_requested: bool,
    /// MIDI notes requested by DECPS, waiting for the host to play them.
    pending_sounds: Vec<MidiNote>,
    /// Bells requested by BEL.
    pending_bells: usize,
    /// Whether ANSWERBACK was requested.
    answerback_requested: bool,
    /// The ANSI/VT52 parser mode last requested by DECANM.
    ansi_mode: bool,
    /// A pending DECANM change for the parser to apply.
    pending_ansi_mode: Option<bool>,
    /// A pending C1-controls change for the parser to apply.
    pending_c1_controls: Option<bool>,
    /// Kitty keyboard protocol flags and their push stack (input state).
    kitty_flags: u8,
    kitty_stack: Vec<u8>,
}

/// A MIDI note requested by DECPS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiNote {
    /// MIDI note number; 72 is C5.
    pub note: u8,
    /// MIDI velocity; 0 is silent.
    pub velocity: u8,
    /// Duration in microseconds.
    pub duration_micros: u32,
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
            term_output: Arc::new(Mutex::new(TerminalOutput::new(false))),
            sixel_parser: std::sync::Arc::new(std::sync::Mutex::new(SixelParser::new(SIXEL_DEFAULT_CONFORMANCE))),
            font_buffer: FontBuffer::new(),
            macro_buffer: std::sync::Arc::new(std::sync::Mutex::new(MacroBuffer::new())),
            color_table: std::sync::Arc::new(std::sync::Mutex::new(ColorTable::new())),
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
            pending_macro_input: Vec::new(),
            optional_features: std::collections::HashSet::new(),
            tab_stops: Vec::new(),
            saved_cursor_state: [CursorState::default(), CursorState::default()],
            window_title: None,
            current_working_directory: None,
            clipboard_content: None,
            pending_window_visibility: None,
            pending_window_resize: None,
            redraw_requested: false,
            pending_sounds: Vec::new(),
            pending_bells: 0,
            answerback_requested: false,
            ansi_mode: true,
            pending_ansi_mode: None,
            pending_c1_controls: None,
            kitty_flags: 0,
            kitty_stack: Vec::new(),
        }
    }

    /// Initializes the default tab stops for a buffer `width` wide.
    fn ensure_tab_stops(&mut self, width: VTInt) {
        if self.tab_stops.len() == width as usize {
            return;
        }
        let mut stops = vec![false; width.max(0) as usize];
        for column in (0..width).step_by(8) {
            if let Some(stop) = stops.get_mut(column as usize) {
                *stop = true;
            }
        }
        self.tab_stops = stops;
    }

    /// The index of the saved-cursor slot for the buffer in use.
    fn cursor_state_index(&self) -> usize {
        usize::from(self.using_alt_buffer)
    }

    /// Appends a CSI response to the queue the host drains.
    fn push_csi(&mut self, body: &str) {
        self.pending_responses.push(format!("\x1b[{}", body));
    }

    /// Appends a DCS response to the queue the host drains.
    ///
    /// Upstream wraps in C1 when the SendC1 input mode is on; this adapter does
    /// not model that mode, so the 7-bit form is always used.
    fn push_dcs(&mut self, body: &str) {
        self.pending_responses.push(format!("\x1bP{}\x1b\\", body));
    }

    /// Appends an OSC response to the queue the host drains.
    fn push_osc(&mut self, body: &str) {
        self.pending_responses.push(format!("\x1b]{}\x1b\\", body));
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

    /// Returns the window title set by OSC 0/2.
    pub fn window_title(&self) -> Option<&str> {
        self.window_title.as_deref()
    }

    /// Returns the working-directory URI set by OSC 7.
    ///
    /// Upstream converts the file URI to a path with a Win32 API before handing
    /// it to the host. That conversion belongs to the host layer here, so the
    /// URI is recorded as given.
    pub fn current_working_directory(&self) -> Option<&str> {
        self.current_working_directory.as_deref()
    }

    /// Takes the clipboard content set by OSC 52.
    pub fn take_clipboard_content(&mut self) -> Option<String> {
        self.clipboard_content.take()
    }

    /// Takes the window visibility requested by CSI 1/2 t; true means show.
    pub fn take_window_visibility(&mut self) -> Option<bool> {
        self.pending_window_visibility.take()
    }

    /// Takes the window size requested by CSI 8 t as `(width, height)`.
    pub fn take_window_resize(&mut self) -> Option<(VTInt, VTInt)> {
        self.pending_window_resize.take()
    }

    /// Takes the full-redraw request made by CSI 7 t.
    pub fn take_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.redraw_requested)
    }

    /// Takes the notes requested by DECPS.
    pub fn take_sound_requests(&mut self) -> Vec<MidiNote> {
        std::mem::take(&mut self.pending_sounds)
    }

    /// Takes the number of bells requested since the last call.
    pub fn take_bell_requests(&mut self) -> usize {
        std::mem::take(&mut self.pending_bells)
    }

    /// Takes a pending ANSWERBACK request.
    pub fn take_answerback_request(&mut self) -> bool {
        std::mem::take(&mut self.answerback_requested)
    }

    /// Takes a pending DECANM change for the parser.
    pub fn take_ansi_mode_change(&mut self) -> Option<bool> {
        self.pending_ansi_mode.take()
    }

    /// Takes a pending C1-controls change for the parser.
    pub fn take_c1_controls_change(&mut self) -> Option<bool> {
        self.pending_c1_controls.take()
    }

    /// Returns the kitty keyboard flags in effect.
    pub const fn kitty_keyboard_flags(&self) -> u8 {
        self.kitty_flags
    }

    /// Returns the ANSI/VT52 parser mode last requested by DECANM.
    pub const fn is_ansi_mode(&self) -> bool {
        self.ansi_mode
    }

    /// The size of the visible page, which is what the window reports describe.
    fn visible_page_size(&mut self) -> (VTInt, VTInt) {
        let buffer = self.page_manager.visible_page().buffer();
        let buffer = buffer.read().unwrap_or_else(|e| e.into_inner());
        (buffer.screen_width(), buffer.screen_height())
    }

    /// Ported from `_ReportColorTable`: the DECCTR body DECRQTSR answers with.
    fn report_color_table(&mut self, model_value: VTInt) {
        use std::fmt::Write as _;

        let Some(model) = ColorModel::from_int(model_value) else {
            return;
        };

        let mut body = String::from("2$s");
        {
            let table = self.color_table.lock().unwrap_or_else(|e| e.into_inner());
            for index in 0..TABLE_SIZE {
                let Some(color) = table.get(index) else {
                    continue;
                };
                if index > 0 {
                    body.push('/');
                }
                let (x, y, z) = match model {
                    ColorModel::Hls => color_to_hls(color),
                    ColorModel::Rgb => color_to_rgb100(color),
                };
                let _ = write!(body, "{};{};{};{};{}", index, model.to_int(), x, y, z);
            }
        }
        self.push_dcs(&body);
    }

    /// Ported from `_RestoreColorTable`: parses the DECCTR report DECRSTS
    /// delivers.
    fn restore_color_table_handler(&self) -> Box<dyn FnMut(char) -> bool + Send> {
        let table = std::sync::Arc::clone(&self.color_table);
        let mut parameter: VTInt = 0;
        let mut parameters: Vec<VTInt> = Vec::new();

        Box::new(move |ch| {
            match ch {
                '0'..='9' => {
                    parameter = parameter
                        .saturating_mul(10)
                        .saturating_add(ch as VTInt - '0' as VTInt)
                        .min(65535);
                }
                ';' => {
                    if parameters.len() < 5 {
                        parameters.push(parameter);
                    }
                    parameter = 0;
                }
                '/' | '\x1b' => {
                    parameters.push(parameter);

                    let color_number = parameters.first().copied().unwrap_or(0);
                    if (0..TABLE_SIZE as VTInt).contains(&color_number)
                        && let Some(model) = ColorModel::from_int(parameters.get(1).copied().unwrap_or(0))
                    {
                        let x = parameters.get(2).copied().unwrap_or(0);
                        let y = parameters.get(3).copied().unwrap_or(0);
                        let z = parameters.get(4).copied().unwrap_or(0);
                        let color = match model {
                            ColorModel::Hls => color_from_hls(x, y, z),
                            ColorModel::Rgb => color_from_rgb100(x, y, z),
                        };
                        table
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .set(color_number as usize, color);
                    }

                    parameters.clear();
                    parameter = 0;
                    return ch != '\x1b';
                }
                _ => {}
            }
            true
        })
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
    /// Returns the terminal output state.
    pub fn term_output(&self) -> MutexGuard<'_, TerminalOutput> {
        self.term_output.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Returns the terminal output state mutably.
    ///
    /// The shared lock is what lets the DECAUPSS handler own the assignment
    /// after the DCS introducer has already been dispatched.
    pub fn term_output_mut(&self) -> MutexGuard<'_, TerminalOutput> {
        self.term_output.lock().unwrap_or_else(|e| e.into_inner())
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
    pub fn sixel_parser(&self) -> std::sync::Arc<std::sync::Mutex<SixelParser>> {
        std::sync::Arc::clone(&self.sixel_parser)
    }

    /// Returns the macro buffer.
    pub fn macro_buffer(&self) -> std::sync::Arc<std::sync::Mutex<MacroBuffer>> {
        std::sync::Arc::clone(&self.macro_buffer)
    }

    /// Returns the colour table.
    pub fn color_table(&self) -> std::sync::Arc<std::sync::Mutex<ColorTable>> {
        std::sync::Arc::clone(&self.color_table)
    }

    /// Takes the macro sequences the application should feed to its parser.
    ///
    /// Upstream injects an invoked macro into its own state machine; this crate
    /// does not own one, so invocation is queued instead (ADR 001).
    pub fn take_macro_input(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.pending_macro_input)
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
        self.term_output.lock().unwrap_or_else(|e| e.into_inner()).soft_reset();
        self.sixel_parser.lock().unwrap_or_else(|e| e.into_inner()).soft_reset();
    }

    /// Hard resets the adapter.
    pub fn hard_reset(&mut self, erase: bool) {
        self.soft_reset();
        // RIS returns the parser to ANSI mode.
        self.ansi_mode = true;
        self.pending_ansi_mode = Some(true);
        self.using_alt_buffer = false;
        self.cursor_style = CursorStyle::UserDefault;
        self.modes = ModeSet::default();
        // RIS discards the macro definitions and restores the colour table, as
        // upstream's `ClearMacrosIfInUse` and `RestoreDefaultSettings` do.
        self.macro_buffer.lock().unwrap_or_else(|e| e.into_inner()).reset();
        self.color_table.lock().unwrap_or_else(|e| e.into_inner()).reset();
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

    /// Ported from `_SetColumnMode` (DECCOLM).
    ///
    /// The mode is inert until XTERM mode 40 (`ALLOW_DECCOLM`) allows it. A
    /// permitted change resizes the visible page to 132 or 80 columns, clears
    /// the screen, homes the cursor, and resets origin mode and both margin
    /// pairs.
    fn set_column_mode(&mut self, enable: bool) {
        if !self.modes.contains(ModeFlags::ALLOW_DECCOLM) {
            return;
        }

        const DECCOLM_SET_COLUMNS: VTInt = 132;
        const DECCOLM_RESET_COLUMNS: VTInt = 80;

        let width = if enable {
            DECCOLM_SET_COLUMNS
        } else {
            DECCOLM_RESET_COLUMNS
        };

        let height = self.page_manager.size().1;
        // DECCOLM sets the column count for every page, so the alternate screen
        // does not keep the old width when it is activated next.
        self.page_manager.resize_all(width, height);

        self.set_mode_flag(ModeFlags::DECCOLM, enable);
        self.set_mode_flag(ModeFlags::DECOM, false);
        self.set_mode_flag(ModeFlags::DECLRMM, false);
        self.scroll_left_margin = 0;
        self.scroll_right_margin = 0;

        ITermDispatch::cursor_position(self, 1, 1);
        ITermDispatch::erase_in_display(self, EraseType::All);
        self.set_vertical_scrolling_margins(0, 0, false);
        self.set_horizontal_scrolling_margins(0, 0, false);
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
            DecColm => self.set_column_mode(enable),
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
            // DECANM switches the parser between ANSI and VT52.
            DecAnm => self.set_ansi_mode(enable),
            // GCM is deliberately inert upstream too.
            GraphemeClusterMode => {}
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
            DecAnm => return u8::from(!self.ansi_mode) + 1,
            GraphemeClusterMode => return 0,
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

    fn blank_cell(&mut self) -> buffer::OutputCell {
        let mut cell = buffer::OutputCell::from_char(' ');
        cell.set_attributes(self.erase_attributes());
        cell
    }

    /// Scrolls a page rectangle vertically by `delta` rows (positive down).
    fn scroll_rect_vertical(&mut self, top: VTInt, bottom: VTInt, left: VTInt, right: VTInt, delta: VTInt) {
        let blank = self.blank_cell();
        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        buffer.scroll_rect_vertically(top, bottom, left, right, delta, &blank);
    }

    /// Scrolls a page rectangle horizontally by `delta` columns (positive right).
    fn scroll_rect_horizontal(&mut self, top: VTInt, bottom: VTInt, left: VTInt, right: VTInt, delta: VTInt) {
        let blank = self.blank_cell();
        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        buffer.scroll_rect_horizontally(top, bottom, left, right, delta, &blank);
    }

    /// Ported from `_InsertDeleteCharacterHelper`.
    fn insert_delete_character(&mut self, delta: VTInt) {
        if delta == 0 {
            return;
        }
        let (page_width, page_height) = self.page_size();
        let (row, column) = self.active_cursor_position();
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (left, right) = if row >= top && row <= bottom {
            self.effective_horizontal_margins(page_width)
        } else {
            (0, page_width - 1)
        };

        if column < left || column > right {
            return;
        }

        self.scroll_rect_horizontal(row, row, column, right, delta);
        // ICH and DCH reset the delayed-wrap flag, as upstream does.
        let buffer = self.page_manager.active_page().buffer();
        buffer
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .cursor_mut()
            .reset_delayed_eol_wrap();
    }

    /// Ported from `_InsertDeleteLineHelper`.
    fn insert_delete_line(&mut self, delta: VTInt) {
        if delta == 0 {
            return;
        }
        let (page_width, page_height) = self.page_size();
        let (row, column) = self.active_cursor_position();
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (left, right) = self.effective_horizontal_margins(page_width);

        if row < top || row > bottom || column < left || column > right {
            return;
        }

        self.scroll_rect_vertical(row, bottom, left, right, delta);
        // IL and DL move the cursor to the left margin.
        let buffer = self.page_manager.active_page().buffer();
        buffer
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .cursor_mut()
            .set_position(row, left);
    }

    /// Ported from `_InsertDeleteColumnHelper`.
    fn insert_delete_column(&mut self, delta: VTInt) {
        if delta == 0 {
            return;
        }
        let (page_width, page_height) = self.page_size();
        let (row, column) = self.active_cursor_position();
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (left, right) = self.effective_horizontal_margins(page_width);

        if row < top || row > bottom || column < left || column > right {
            return;
        }

        self.scroll_rect_horizontal(top, bottom, column, right, delta);
    }

    /// Ported from `_CalculateRectArea`: converts VT's 1-based, optionally
    /// margin-relative coordinates into a half-open page rectangle.
    ///
    /// `bottom`/`right` of zero mean "to the page (or margin) extent". The
    /// result is clamped to the page, so a malformed sequence cannot index a
    /// row; upstream's `ROW` accessors clamp in the same way.
    fn calculate_rect_area(
        &mut self,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
    ) -> (VTInt, VTInt, VTInt, VTInt) {
        let (page_width, page_height) = self.page_size();
        let (top_margin, bottom_margin) = self.effective_vertical_margins(page_height);
        let (left_margin, right_margin) = self.effective_horizontal_margins(page_width);

        let origin_mode = self.modes.contains(ModeFlags::DECOM);
        let y_offset = if origin_mode { top_margin } else { 0 };
        let y_maximum = if origin_mode { bottom_margin + 1 } else { page_height };
        let x_offset = if origin_mode { left_margin } else { 0 };
        let x_maximum = if origin_mode { right_margin + 1 } else { page_width };

        let left = (left + x_offset).min(x_maximum) - 1;
        let top = (top + y_offset).min(y_maximum) - 1;
        let right = (if right != 0 { right + x_offset } else { x_maximum }).min(x_maximum) - 1;
        let bottom = (if bottom != 0 { bottom + y_offset } else { y_maximum }).min(y_maximum) - 1;

        (
            top.clamp(0, page_height),
            left.clamp(0, page_width),
            (bottom + 1).clamp(0, page_height),
            (right + 1).clamp(0, page_width),
        )
    }

    /// Ported from `_ChangeRectOrStreamAttributes`: applies attribute changes to
    /// a rectangle, or to the same endpoints in reading order when DECSACE has
    /// selected the stream extent.
    fn change_rect_or_stream_attributes(
        &mut self,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
        ops: &ChangeOps,
    ) {
        let (top, left, bottom, right) = self.calculate_rect_area(top, left, bottom, right);
        let (page_width, _) = self.page_size();
        let line_count = bottom - top;

        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());

        if self.modes.contains(ModeFlags::RECTANGULAR_CHANGE_EXTENT) || line_count == 1 {
            change_rect_attributes(&mut buffer, top, left, bottom, right, ops);
        } else if line_count > 1 && right > left {
            // Stream extent: the top line runs to the end, the middle lines are
            // changed in full, and the bottom line runs from the start.
            change_rect_attributes(&mut buffer, top, left, top + 1, page_width, ops);
            change_rect_attributes(&mut buffer, top + 1, 0, bottom - 1, page_width, ops);
            change_rect_attributes(&mut buffer, bottom - 1, 0, bottom, right, ops);
        }
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

/// Maps an xterm colour resource number (10–19) to its table slot and alias.
///
/// Ported from `XtermResourceColorTableMappings`; `None` means the resource is
/// not supported.
fn xterm_color_mapping(resource: usize) -> Option<(usize, Option<ColorAlias>)> {
    const MAPPINGS: [Option<(usize, Option<ColorAlias>)>; 10] = [
        Some((DEFAULT_FOREGROUND, Some(ColorAlias::DefaultForeground))),
        Some((DEFAULT_BACKGROUND, Some(ColorAlias::DefaultBackground))),
        Some((CURSOR_COLOR, None)),
        None,
        None,
        None,
        None,
        Some((SELECTION_BACKGROUND, None)),
        None,
        None,
    ];
    resource
        .checked_sub(10)
        .and_then(|index| MAPPINGS.get(index).copied().flatten())
}

/// Decodes upstream's `COLORREF` packing (`0x00BBGGRR`).
fn colorref_to_text_color(color: u32) -> TextColor {
    TextColor::rgb(color as u8, (color >> 8) as u8, (color >> 16) as u8)
}

/// Paints a decoded sixel image into the buffer's row image bands (ADR 005).
///
/// Display mode set clamps the image at the bottom of the page, as the VT240
/// does; with it reset the page scrolls to make room. Returns the text cursor
/// row the image ends on.
fn paint_sixel_image(
    buffer: &mut buffer::TextBuffer,
    start: (i32, i32),
    image: &DecodedImage,
    display_mode: bool,
) -> i32 {
    let (cell_width, cell_height) = image.cell_size;
    let page_width = buffer.screen_width();
    let page_height = buffer.screen_height();
    if cell_width <= 0 || cell_height <= 0 || start.1 >= page_width {
        return start.0;
    }

    let available_columns = page_width - start.1;
    let cells_wide = ((image.width + cell_width - 1) / cell_width).min(available_columns);
    if cells_wide <= 0 {
        return start.0;
    }
    let band_width = cells_wide * cell_width;
    if band_width > image.stride {
        return start.0;
    }

    let rows = (image.height + cell_height - 1) / cell_height;
    let mut scrolled = 0;
    let mut last_row = start.0;

    for cell_row in 0..rows {
        let mut dest = start.0 + cell_row - scrolled;
        while dest >= page_height {
            if display_mode {
                return last_row;
            }
            buffer.scroll_region_up(0, page_height - 1);
            scrolled += 1;
            dest -= 1;
        }
        if dest < 0 {
            continue;
        }

        let top = cell_row * cell_height;
        let mut band_pixels = vec![0u32; (band_width * cell_height) as usize];
        let mut any_visible = false;
        for py in 0..cell_height {
            let source_y = top + py;
            if source_y >= image.height {
                break;
            }
            for px in 0..band_width {
                let value = image.pixels[(source_y * image.stride + px) as usize];
                any_visible |= value != 0;
                band_pixels[(py * band_width + px) as usize] = value;
            }
        }
        if !any_visible {
            continue;
        }

        let Some(row) = buffer.get_row_at_screen_line_mut(dest) else {
            continue;
        };
        if row.image_slice().is_none() {
            row.set_image_slice(Some(buffer::ImageSlice::new((cell_width, cell_height))));
        }
        let Some(slice) = row.image_slice_mut() else {
            continue;
        };

        let band_begin = start.1;
        let band_end = start.1 + cells_wide;
        slice.mutable_pixels(band_begin, band_end);
        let offset = (band_begin - slice.column_begin()) * cell_width;
        let pixel_width = slice.pixel_width();
        let pixels = slice.mutable_pixels(band_begin, band_end);
        for py in 0..cell_height {
            let source_start = (py * band_width) as usize;
            let dest_start = (py * pixel_width + offset) as usize;
            if dest_start + band_width as usize <= pixels.len() {
                pixels[dest_start..dest_start + band_width as usize]
                    .copy_from_slice(&band_pixels[source_start..source_start + band_width as usize]);
            }
        }
        last_row = dest;
    }

    if display_mode {
        start.0
    } else {
        let final_row = start.0 + image.final_row_top / cell_height - scrolled;
        final_row.clamp(0, page_height - 1)
    }
}

/// Fills a half-open rectangle (`bottom`/`right` exclusive) with `ch` and
/// `attributes`.
fn fill_rect_with(
    buffer: &mut buffer::TextBuffer,
    top: VTInt,
    left: VTInt,
    bottom: VTInt,
    right: VTInt,
    ch: char,
    attributes: TextAttribute,
) {
    if bottom <= top || right <= left {
        return;
    }
    let mut cell = buffer::OutputCell::from_char(ch);
    cell.set_attributes(attributes);
    buffer.fill_rect(top, left, bottom - 1, right - 1, &cell);
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
    fill_rect_with(buffer, top, left, bottom, right, ' ', attributes);
}

/// Ported from `_ChangeRectAttributes`: applies the attribute masks and the
/// optional colours to every cell in a half-open rectangle, leaving the text
/// untouched.
fn change_rect_attributes(
    buffer: &mut buffer::TextBuffer,
    top: VTInt,
    left: VTInt,
    bottom: VTInt,
    right: VTInt,
    ops: &ChangeOps,
) {
    if bottom <= top || right <= left {
        return;
    }
    for row in top..bottom {
        let Some(buffer_row) = buffer.get_row_at_screen_line_mut(row) else {
            continue;
        };
        for column in left..right {
            let mut attributes = *buffer_row.get_cell(column).get_attributes();
            let mut character_attributes = attributes.get_attrs();
            character_attributes &= ops.and_attr_mask;
            character_attributes ^= ops.xor_attr_mask;
            attributes.set_attrs(character_attributes);
            if let Some(foreground) = ops.foreground {
                attributes.set_foreground(foreground);
            }
            if let Some(background) = ops.background {
                attributes.set_background(background);
            }
            if let Some(underline_color) = ops.underline_color {
                attributes.set_underline_color(underline_color);
            }
            buffer_row.get_cell_mut(column).set_attributes(attributes);
        }
    }
}

/// The colour index the DEC checksum uses: only the first 16 palette entries
/// count; every other colour falls back to the default alias index.
fn legacy_color_index(color: types::TextColor, default_index: u8) -> u8 {
    let index = color.get_index();
    let legacy = (color.is_index16() || color.is_index256()) && index < 16;
    if legacy { index } else { default_index }
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

    fn soft_reset(&mut self) {
        AdaptDispatch::soft_reset(self);
    }

    fn hard_reset(&mut self, erase: bool) {
        AdaptDispatch::hard_reset(self, erase);
    }

    fn set_cursor_style(&mut self, cursor_style: CursorStyle) {
        AdaptDispatch::set_cursor_style(self, cursor_style);
    }

    fn warning_bell(&mut self) {
        self.pending_bells += 1;
    }

    fn enquire_answerback(&mut self) {
        self.answerback_requested = true;
    }

    fn insert_character(&mut self, count: VTInt) {
        self.insert_delete_character(count);
    }

    fn delete_character(&mut self, count: VTInt) {
        self.insert_delete_character(-count);
    }

    fn insert_line(&mut self, distance: VTInt) {
        self.insert_delete_line(distance);
    }

    fn delete_line(&mut self, distance: VTInt) {
        self.insert_delete_line(-distance);
    }

    fn insert_column(&mut self, distance: VTInt) {
        self.insert_delete_column(distance);
    }

    fn delete_column(&mut self, distance: VTInt) {
        self.insert_delete_column(-distance);
    }

    fn scroll_up(&mut self, distance: VTInt) {
        let (page_width, page_height) = self.page_size();
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (left, right) = self.effective_horizontal_margins(page_width);
        self.scroll_rect_vertical(top, bottom, left, right, -distance);
    }

    fn scroll_down(&mut self, distance: VTInt) {
        let (page_width, page_height) = self.page_size();
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (left, right) = self.effective_horizontal_margins(page_width);
        self.scroll_rect_vertical(top, bottom, left, right, distance);
    }

    fn page_position_absolute(&mut self, page: VTInt) {
        let coupled = self.modes.contains(ModeFlags::PAGE_CURSOR_COUPLING);
        self.page_manager.move_to(page, coupled);
    }

    fn page_position_relative(&mut self, page_count: VTInt) {
        let coupled = self.modes.contains(ModeFlags::PAGE_CURSOR_COUPLING);
        self.page_manager.move_relative(page_count, coupled);
    }

    fn page_position_back(&mut self, page_count: VTInt) {
        let coupled = self.modes.contains(ModeFlags::PAGE_CURSOR_COUPLING);
        self.page_manager.move_relative(-page_count, coupled);
    }

    fn next_page(&mut self, page_count: VTInt) {
        self.page_position_relative(page_count);
        self.cursor_position(1, 1);
    }

    fn preceding_page(&mut self, page_count: VTInt) {
        self.page_position_back(page_count);
        self.cursor_position(1, 1);
    }

    fn request_displayed_extent(&mut self) {
        let (width, height, number) = {
            let page = self.page_manager.visible_page();
            (page.width(), page.height(), page.number())
        };
        // Pan offsets are always zero until the viewport can pan.
        self.push_csi(&format!("{};{};1;1;{}\"w", height, width, number));
    }

    fn set_ansi_mode(&mut self, ansi_mode: bool) {
        // DECANM resets the designated character sets even if the mode does not
        // change, as upstream's `SetAnsiMode` does.
        self.term_output.lock().unwrap_or_else(|e| e.into_inner()).soft_reset();
        self.ansi_mode = ansi_mode;
        self.pending_ansi_mode = Some(ansi_mode);
    }

    fn accept_c1_controls(&mut self, enabled: bool) {
        self.pending_c1_controls = Some(enabled);
    }

    fn set_kitty_keyboard_protocol(&mut self, flags: VTParameter, mode: VTParameter) {
        let flags = flags.value_or(0).clamp(0, 255) as u8;
        match mode.value_or(1) {
            1 => self.kitty_flags = flags,
            2 => self.kitty_flags |= flags,
            3 => self.kitty_flags &= !flags,
            _ => {}
        }
    }

    fn query_kitty_keyboard_protocol(&mut self) {
        self.push_csi(&format!("?{}u", self.kitty_flags));
    }

    fn push_kitty_keyboard_protocol(&mut self, flags: VTParameter) {
        self.kitty_stack.push(self.kitty_flags);
        self.kitty_flags = flags.value_or(0).clamp(0, 255) as u8;
    }

    fn pop_kitty_keyboard_protocol(&mut self, count: VTParameter) {
        let count = count.value_or(1).max(1) as usize;
        for _ in 0..count {
            let Some(flags) = self.kitty_stack.pop() else {
                break;
            };
            self.kitty_flags = flags;
        }
    }

    fn screen_alignment_pattern(&mut self) {
        let (page_width, page_height) = self.page_size();
        {
            let page = self.page_manager.active_page();
            let mut attributes = page.attributes();
            attributes.set_standard_erase();
            page.set_attributes(attributes);
        }

        {
            let buffer = self.page_manager.active_page().buffer();
            let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
            let cell = buffer::OutputCell::from_char('E');
            buffer.fill_rect(0, 0, page_height - 1, page_width - 1, &cell);
            buffer.cursor_mut().set_position(0, 0);
        }

        self.modes.remove(ModeFlags::DECOM);
        self.modes.remove(ModeFlags::DECLRMM);
        self.scroll_top_margin = 0;
        self.scroll_bottom_margin = 0;
        self.scroll_left_margin = 0;
        self.scroll_right_margin = 0;
    }

    fn request_user_preference_charset(&mut self) {
        let (size, id) = {
            let output = self.term_output.lock().unwrap_or_else(|e| e.into_inner());
            (
                output.get_user_preference_charset_size(),
                output.get_user_preference_charset_id(),
            )
        };
        self.push_dcs(&format!("{}!u{}", if size == 96 { 1 } else { 0 }, id));
    }

    fn assign_user_preference_charset(
        &mut self,
        charset_size: CharsetSize,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>> {
        let output = Arc::clone(&self.term_output);
        let size_96 = matches!(charset_size, CharsetSize::Size96);
        let mut builder = VTID::new();

        Some(Box::new(move |ch| {
            let value = ch as u32;
            if (0x20..=0x2F).contains(&value) {
                builder = builder.with_intermediate(value as u8);
            } else if (0x30..=0x7E).contains(&value) {
                let id = builder.finalize_with(value as u8);
                output
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .assign_user_preference_charset(id, size_96);
            }
            true
        }))
    }

    fn play_sounds(&mut self, parameters: &VTParameters) {
        // Volume 0..7 maps to MIDI velocity 0..127; duration is in 1/32 s.
        let volume = parameters.at(0).value_or(0).clamp(0, 7) as u32;
        let velocity = (volume * 127 / 7) as u8;
        let duration_micros = parameters.at(1).value_or(0).clamp(0, 255) as u32 * 1_000_000 / 32;

        for parameter in parameters.iter().skip(2) {
            let value = parameter.value_or(0).clamp(0, 25);
            let note = (value + 71) as u8;
            self.pending_sounds.push(MidiNote {
                note,
                velocity: if note == 71 { 0 } else { velocity },
                duration_micros,
            });
        }
    }

    fn print(&mut self, ch: char) {
        let translated = self
            .term_output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .translate_key(ch);
        // A DEL that stays DEL is ignored, as it is when a 94-character set has
        // no mapping for it (upstream `AdaptDispatch::Print`).
        if translated as u32 != 0x7F {
            self.write_to_buffer(&translated.to_string());
        }
    }

    fn print_string(&mut self, string: &str) {
        let translated = {
            let output = self.term_output.lock().unwrap_or_else(|e| e.into_inner());
            if output.needs_translation() {
                Some(string.chars().map(|ch| output.translate_key(ch)).collect::<String>())
            } else {
                None
            }
        };

        match translated {
            Some(translated) => self.write_to_buffer(&translated),
            None => self.write_to_buffer(string),
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
            let buffer = self.page_manager.active_page().buffer();
            let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
            buffer.clear_scrollback();
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

    fn change_attributes_rectangular_area(
        &mut self,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
        attrs: &VTParameters,
    ) {
        // Applying the options to an empty attribute and to one with every bit
        // set yields the OR and AND masks; combining them into AND/XOR keeps
        // the per-cell work to two operations (DECRARA shares the same path).
        let mut all_off = TextAttribute::new();
        let mut all_on = TextAttribute::from_rgb(0, 0, Some(0));
        all_on.set_attrs(CharacterAttributes::All);
        Self::apply_graphics_options(attrs, &mut all_off);
        Self::apply_graphics_options(attrs, &mut all_on);

        let or_mask = all_off.get_attrs();
        let and_mask = all_on.get_attrs();

        // `all_on` starts black, so a colour that it still holds was not changed
        // by the options; one that it lost was reset to the default.
        let colour_changed =
            |applied: types::TextColor, patched: types::TextColor| !applied.is_default() || patched.is_default();
        let foreground =
            colour_changed(all_off.get_foreground(), all_on.get_foreground()).then_some(all_off.get_foreground());
        let background =
            colour_changed(all_off.get_background(), all_on.get_background()).then_some(all_off.get_background());
        let underline_color = colour_changed(all_off.get_underline_color(), all_on.get_underline_color())
            .then_some(all_off.get_underline_color());

        let ops = ChangeOps {
            and_attr_mask: and_mask & !or_mask,
            xor_attr_mask: or_mask,
            foreground,
            background,
            underline_color,
        };
        self.change_rect_or_stream_attributes(top, left, bottom, right, &ops);
    }

    fn reverse_attributes_rectangular_area(
        &mut self,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
        attrs: &VTParameters,
    ) {
        let mut reverse_mask = CharacterAttributes::Normal;

        if !attrs.is_empty() {
            let mut index = 0;
            while index < attrs.len() {
                if attrs.at(index).value_or(0) == 0 {
                    // Option 0 reverses every rendition bit except the underline
                    // style, which is replaced with a single underline.
                    let singly_underlined = CharacterAttributes::from_bits_retain(
                        (UnderlineStyle::SinglyUnderlined as u16) << CharacterAttributes::UNDERLINE_STYLE_SHIFT,
                    );
                    reverse_mask ^=
                        (CharacterAttributes::Rendition & !CharacterAttributes::UnderlineStyleMask) | singly_underlined;
                    index += 1;
                } else {
                    let mut all_off = TextAttribute::new();
                    index += Self::apply_graphics_option(attrs, index, &mut all_off);
                    reverse_mask ^= all_off.get_attrs();
                }
            }
        }

        if reverse_mask != CharacterAttributes::Normal {
            let ops = ChangeOps {
                xor_attr_mask: reverse_mask,
                ..ChangeOps::default()
            };
            self.change_rect_or_stream_attributes(top, left, bottom, right, &ops);
        }
    }

    fn copy_rectangular_area(
        &mut self,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
        page: VTInt,
        dst_top: VTInt,
        dst_left: VTInt,
        dst_page: VTInt,
    ) {
        if !PageManager::is_valid_page(page) || !PageManager::is_valid_page(dst_page) {
            return;
        }

        // Both rectangles are measured against the active page's extent. Every
        // page this manager creates is the same size, so the source page's own
        // extent does not need to be consulted.
        let (src_top, src_left, src_bottom, src_right) = self.calculate_rect_area(top, left, bottom, right);
        let src_width = src_right - src_left;
        let src_height = src_bottom - src_top;
        if src_width <= 0 || src_height <= 0 {
            return;
        }

        let dest_bottom = dst_top + src_height - 1;
        let dest_right = dst_left + src_width - 1;
        let (dst_top, dst_left, dst_bottom, dst_right) =
            self.calculate_rect_area(dst_top, dst_left, dest_bottom, dest_right);
        if dst_bottom <= dst_top || dst_right <= dst_left {
            return;
        }
        if dst_top == src_top && dst_left == src_left && page == dst_page {
            return;
        }

        // Source cells beyond the destination's space are clipped, so only the
        // destination size matters.
        let width = (dst_right - dst_left).min(src_width);
        let height = (dst_bottom - dst_top).min(src_height);

        let src_buffer = self.page_manager.get(page).buffer();
        let dst_buffer = self.page_manager.get(dst_page).buffer();

        // Snapshot first: the source and destination may be the same buffer and
        // the rectangles may overlap, so every source cell must be read before
        // any destination cell is overwritten.
        let mut cells = Vec::with_capacity((width * height) as usize);
        let mut images = Vec::with_capacity(height as usize);
        {
            let src = src_buffer.read().unwrap_or_else(|e| e.into_inner());
            for row in 0..height {
                images.push(
                    src.get_row_at_screen_line(src_top + row)
                        .and_then(|src_row| src_row.image_slice().cloned()),
                );
                for column in 0..width {
                    cells.push(src.get_cell_data_at(src_top + row, src_left + column).cloned());
                }
            }
        }

        let mut dst = dst_buffer.write().unwrap_or_else(|e| e.into_inner());
        for row in 0..height {
            let Some(buffer_row) = dst.get_row_at_screen_line_mut(dst_top + row) else {
                continue;
            };
            for column in 0..width {
                let index = (row * width + column) as usize;
                let Some(Some(cell)) = cells.get(index) else {
                    continue;
                };
                buffer_row.write_cell(dst_left + column, cell.clone());
            }
            buffer_row.copy_image_cells_from_slice(
                images.get(row as usize).and_then(Option::as_ref),
                src_left,
                dst_left,
                dst_right,
            );
        }
    }

    fn fill_rectangular_area(&mut self, ch: VTParameter, top: VTInt, left: VTInt, bottom: VTInt, right: VTInt) {
        let (top, left, bottom, right) = self.calculate_rect_area(top, left, bottom, right);

        // The standard allows the GL and GR ranges; the output code page here is
        // always UTF-8, so the BMP is accepted as well. Default and 0 mean space.
        let char_value = if ch.value_or(0) == 0 { 32 } else { ch.value() };
        let printable = (32..=126).contains(&char_value)
            || (160..=255).contains(&char_value)
            || ((256..=65535).contains(&char_value) && char::from_u32(char_value as u32).is_some());
        if !printable {
            return;
        }
        let Some(fill_ch) = char::from_u32(char_value as u32) else {
            return;
        };

        let fill_char = self
            .term_output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .translate_key(fill_ch);
        let attributes = self.page_manager.active_page().attributes();
        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        fill_rect_with(&mut buffer, top, left, bottom, right, fill_char, attributes);
    }

    fn erase_rectangular_area(&mut self, top: VTInt, left: VTInt, bottom: VTInt, right: VTInt) {
        let (top, left, bottom, right) = self.calculate_rect_area(top, left, bottom, right);
        let attributes = self.erase_attributes();
        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        fill_region(&mut buffer, top, left, bottom, right, attributes);
    }

    fn selective_erase_rectangular_area(&mut self, top: VTInt, left: VTInt, bottom: VTInt, right: VTInt) {
        let (top, left, bottom, right) = self.calculate_rect_area(top, left, bottom, right);
        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        selective_erase_region(&mut buffer, top, left, bottom, right);
    }

    fn select_attribute_change_extent(&mut self, change_extent: ChangeExtent) {
        match change_extent {
            ChangeExtent::Rectangle => self.set_mode_flag(ModeFlags::RECTANGULAR_CHANGE_EXTENT, true),
            ChangeExtent::Default | ChangeExtent::Stream => {
                self.set_mode_flag(ModeFlags::RECTANGULAR_CHANGE_EXTENT, false);
            }
        }
    }

    fn request_checksum_rectangular_area(
        &mut self,
        id: VTInt,
        page: VTInt,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
    ) {
        let mut checksum: u16 = 0;

        if self.optional_features.contains(&OptionalFeature::ChecksumReport)
            && page != 0
            && PageManager::is_valid_page(page)
        {
            let (top, left, bottom, right) = self.calculate_rect_area(top, left, bottom, right);
            let buffer = self.page_manager.get(page).buffer();
            let buffer = buffer.read().unwrap_or_else(|e| e.into_inner());

            for row in top..bottom {
                for column in left..right {
                    let Some(cell) = buffer.get_cell_data_at(row, column) else {
                        continue;
                    };
                    for unit in cell.chars().encode_utf16() {
                        // U+2426 is widely used for the DEC replacement glyph, so
                        // it keeps the DEC checksum value.
                        checksum = checksum.wrapping_sub(if unit == 0x2426 { 0x1B } else { unit });
                    }

                    let attributes = cell.get_attributes();
                    let rendition = attributes.is_protected() as u16 * 0x04
                        + attributes.is_invisible() as u16 * 0x08
                        + (attributes.get_underline_style() != UnderlineStyle::NoUnderline) as u16 * 0x10
                        + attributes.is_reverse_video() as u16 * 0x20
                        + attributes.is_blinking() as u16 * 0x40
                        + attributes.is_intense() as u16 * 0x80;
                    checksum = checksum.wrapping_sub(rendition);

                    let foreground = legacy_color_index(attributes.get_foreground(), 7);
                    let background = legacy_color_index(attributes.get_background(), 0);
                    checksum = checksum.wrapping_sub(u16::from(foreground) << 4);
                    checksum = checksum.wrapping_sub(u16::from(background));
                }
            }
        }

        self.push_dcs(&format!("{}!~{:04X}", id, checksum));
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

    fn device_status_report(&mut self, status_type: StatusType, id: VTParameter) {
        // DECCKSR answers with a DCS response rather than a CSI one.
        if matches!(status_type, StatusType::MemoryChecksum) {
            let checksum = self.macro_buffer.lock().unwrap_or_else(|e| e.into_inner()).checksum();
            self.push_dcs(&format!("{}!~{:04X}", id.value_or(0), checksum));
            return;
        }

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
            // DECMSR reports free macro space in 16-byte blocks.
            StatusType::MacroSpaceReport => {
                let space = self
                    .macro_buffer
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .space_available();
                format!("{}*{{", space / 16)
            }
            StatusType::MemoryChecksum => unreachable!(),
        };
        self.push_csi(&body);
    }

    fn define_sixel_image(
        &mut self,
        macro_parameter: VTInt,
        background_select: SixelBackground,
        background_color: VTParameter,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>> {
        let parser = std::sync::Arc::clone(&self.sixel_parser);
        let (page_width, page_height) = self.page_size();
        let display_mode = self.modes.contains(ModeFlags::SIXEL_DISPLAY);

        let ready = {
            let mut parser = parser.lock().unwrap_or_else(|e| e.into_inner());
            parser.set_display_mode(display_mode);
            let cell_size = SixelParser::cell_size_for_level(parser.conformance_level());
            parser.define_image(
                macro_parameter,
                background_select,
                background_color,
                page_width * cell_size.0,
                page_height * cell_size.1,
            )
        };
        if !ready {
            return None;
        }

        let buffer = self.page_manager.active_page().buffer();
        let (start, was_visible) = {
            let buffer = buffer.read().unwrap_or_else(|e| e.into_inner());
            (buffer.cursor().position(), buffer.cursor().is_visible())
        };
        if was_visible {
            let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
            buffer.cursor_mut().set_visible(false);
        }

        Some(Box::new(move |ch| {
            let finished = {
                let mut parser = parser.lock().unwrap_or_else(|e| e.into_inner());
                !parser.parse_char(ch)
            };
            if !finished {
                return true;
            }

            let image = {
                let mut parser = parser.lock().unwrap_or_else(|e| e.into_inner());
                parser.take_image()
            };

            let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
            if let Some(image) = image {
                let final_row = paint_sixel_image(&mut buffer, start, &image, display_mode);
                if !display_mode {
                    buffer.cursor_mut().set_position(final_row, start.1);
                }
            }
            if was_visible {
                buffer.cursor_mut().set_visible(true);
            }
            false
        }))
    }

    fn define_macro(
        &mut self,
        macro_id: VTInt,
        delete_control: MacroDeleteControl,
        encoding: MacroEncoding,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>> {
        let buffer = std::sync::Arc::clone(&self.macro_buffer);
        let ready = buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .init_parser(macro_id, delete_control, encoding);
        if !ready {
            return None;
        }

        Some(Box::new(move |ch| {
            buffer.lock().unwrap_or_else(|e| e.into_inner()).parse_definition(ch)
        }))
    }

    fn invoke_macro(&mut self, macro_id: VTInt) {
        let Ok(id) = usize::try_from(macro_id) else {
            return;
        };
        let bytes = {
            let buffer = self.macro_buffer.lock().unwrap_or_else(|e| e.into_inner());
            buffer.macro_bytes(id).map(<[u8]>::to_vec)
        };
        if let Some(bytes) = bytes
            && !bytes.is_empty()
        {
            self.pending_macro_input.push(bytes);
        }
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

    fn set_window_title(&mut self, title: &str) {
        // Xterm caps titles set by an OSC at 1000 UTF-16 code units.
        if title.encode_utf16().count() > 1000 {
            return;
        }
        self.window_title = Some(title.to_string());
    }

    fn set_current_working_directory(&mut self, uri: &str) {
        self.current_working_directory = Some(uri.to_string());
    }

    fn set_clipboard(&mut self, content: &str) {
        self.clipboard_content = Some(content.to_string());
    }

    fn add_hyperlink(&mut self, uri: &str, params: &str) {
        let page = self.page_manager.active_page();
        let buffer = page.buffer();
        let id = {
            let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
            buffer.get_hyperlink_id(uri, params)
        };

        let mut attributes = page.attributes();
        attributes.set_hyperlink_id(id);
        page.set_attributes(attributes);
    }

    fn end_hyperlink(&mut self) {
        let page = self.page_manager.active_page();
        let mut attributes = page.attributes();
        attributes.set_hyperlink_id(0);
        page.set_attributes(attributes);
    }

    fn window_manipulation(
        &mut self,
        function: WindowManipulationType,
        parameter1: VTParameter,
        parameter2: VTParameter,
    ) {
        use WindowManipulationType::*;

        match function {
            DeIconifyWindow => self.pending_window_visibility = Some(true),
            IconifyWindow => self.pending_window_visibility = Some(false),
            RefreshWindow => self.redraw_requested = true,
            // The host owns the window, so the request is recorded rather than
            // applied; `parameter2` is the width and `parameter1` the height.
            ResizeWindowInCharacters => {
                self.pending_window_resize = Some((parameter2.value_or(0), parameter1.value_or(0)));
            }
            ReportTextSizeInCharacters => {
                let (width, height) = self.visible_page_size();
                self.push_csi(&format!("8;{};{}t", height, width));
            }
            ReportTextSizeInPixels => {
                let (width, height) = self.visible_page_size();
                let (cell_width, cell_height) = SixelParser::cell_size_for_level(SIXEL_DEFAULT_CONFORMANCE);
                self.push_csi(&format!("4;{};{}t", height * cell_height, width * cell_width));
            }
            ReportCharacterCellSize => {
                let (cell_width, cell_height) = SixelParser::cell_size_for_level(SIXEL_DEFAULT_CONFORMANCE);
                self.push_csi(&format!("6;{};{}t", cell_height, cell_width));
            }
            Invalid => {}
        }
    }

    fn set_color_table_entry(&mut self, table_index: usize, color: u32) {
        self.color_table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set(table_index, colorref_to_text_color(color));
    }

    fn assign_color(&mut self, item: ColorItem, fg_index: VTInt, bg_index: VTInt) {
        // Negative indices cannot name a table slot; passing TABLE_SIZE makes
        // the table ignore them, as it does for any other out-of-range index.
        let fg = usize::try_from(fg_index).unwrap_or(TABLE_SIZE);
        let bg = usize::try_from(bg_index).unwrap_or(TABLE_SIZE);

        {
            let mut table = self.color_table.lock().unwrap_or_else(|e| e.into_inner());
            match item {
                ColorItem::NormalText => {
                    table.set_alias_index(ColorAlias::DefaultForeground, fg);
                    table.set_alias_index(ColorAlias::DefaultBackground, bg);
                }
                ColorItem::WindowFrame => {
                    table.set_alias_index(ColorAlias::FrameForeground, fg);
                    table.set_alias_index(ColorAlias::FrameBackground, bg);
                }
            }
        }

        // A colour alias change affects every cell, so the host must repaint.
        self.redraw_requested = true;
    }

    fn request_color_table_entry(&mut self, table_index: usize) {
        let color = self
            .color_table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(table_index);
        if let Some(color) = color {
            self.push_osc(&format!(
                "4;{};rgb:{:04x}/{:04x}/{:04x}",
                table_index,
                u16::from(color.get_r()) * 0x0101,
                u16::from(color.get_g()) * 0x0101,
                u16::from(color.get_b()) * 0x0101,
            ));
        }
    }

    fn reset_color_table(&mut self) {
        self.color_table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reset_indexed();
    }

    fn reset_color_table_entry(&mut self, table_index: usize) {
        self.color_table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reset_entry(table_index);
    }

    fn set_xterm_color_resource(&mut self, resource: usize, color: u32) {
        let Some((table_index, alias)) = xterm_color_mapping(resource) else {
            return;
        };
        let mut table = self.color_table.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(alias) = alias {
            table.set_alias_index(alias, table_index);
        }
        table.set(table_index, colorref_to_text_color(color));
    }

    fn request_xterm_color_resource(&mut self, resource: usize) {
        let Some((table_index, alias)) = xterm_color_mapping(resource) else {
            return;
        };
        let color = {
            let table = self.color_table.lock().unwrap_or_else(|e| e.into_inner());
            let index = alias.map_or(table_index, |alias| table.alias_index(alias));
            table.get(index)
        };
        if let Some(color) = color {
            self.push_osc(&format!(
                "{};rgb:{:04x}/{:04x}/{:04x}",
                resource,
                u16::from(color.get_r()) * 0x0101,
                u16::from(color.get_g()) * 0x0101,
                u16::from(color.get_b()) * 0x0101,
            ));
        }
    }

    fn reset_xterm_color_resource(&mut self, resource: usize) {
        let Some((table_index, alias)) = xterm_color_mapping(resource) else {
            return;
        };
        let mut table = self.color_table.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(alias) = alias {
            table.restore_default_alias_index(alias);
        }
        table.reset_entry(table_index);
    }

    fn request_terminal_state_report(&mut self, format: ReportFormat, format_option: VTParameter) {
        match format {
            ReportFormat::ColorTableReport => self.report_color_table(format_option.value_or(0)),
            ReportFormat::TerminalStateReport => {}
        }
    }

    fn restore_terminal_state(&mut self, format: ReportFormat) -> Option<Box<dyn FnMut(char) -> bool + Send>> {
        match format {
            ReportFormat::ColorTableReport => Some(self.restore_color_table_handler()),
            ReportFormat::TerminalStateReport => None,
        }
    }

    fn cursor_save_state(&mut self) {
        let (row, column) = self.active_cursor_position();
        let index = self.cursor_state_index();
        let page = self.page_manager.active_page();
        let attributes = page.attributes();
        let page_number = page.number();
        self.saved_cursor_state[index] = CursorState {
            row,
            column,
            page: page_number,
            is_delayed_eol_wrap: false,
            is_origin_mode_relative: self.modes.contains(ModeFlags::DECOM),
            attributes,
            term_output: self.term_output.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        };
    }

    fn cursor_restore_state(&mut self) {
        let index = self.cursor_state_index();
        let saved = self.saved_cursor_state[index].clone();
        let (page_width, _) = self.page_size();

        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        let row = saved.row.max(0);
        let column = saved.column.clamp(0, (page_width - 1).max(0));
        buffer.cursor_mut().set_position(row, column);
        drop(buffer);

        self.page_manager.active_page().set_attributes(saved.attributes);
        self.term_output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .restore_from(&saved.term_output);
        self.set_mode_flag(ModeFlags::DECOM, saved.is_origin_mode_relative);
    }

    fn horizontal_tab_set(&mut self) {
        let (page_width, _) = self.page_size();
        self.ensure_tab_stops(page_width);
        let (_, column) = self.active_cursor_position();
        if let Some(stop) = self.tab_stops.get_mut(column as usize) {
            *stop = true;
        }
    }

    fn forward_tab(&mut self, count: VTInt) {
        let (page_width, page_height) = self.page_size();
        self.ensure_tab_stops(page_width);
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (_left, right) = self.effective_horizontal_margins(page_width);
        let (row, mut column) = self.active_cursor_position();

        let clamp_to_margin = row >= top && row <= bottom && column <= right;
        let max_column = if clamp_to_margin { right } else { page_width - 1 };

        let mut performed = 0;
        while column < max_column && performed < count {
            column += 1;
            if self.tab_stops.get(column as usize).copied().unwrap_or(false) {
                performed += 1;
            }
        }

        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        buffer.cursor_mut().set_position(row, column);
    }

    fn backwards_tab(&mut self, count: VTInt) {
        let (page_width, page_height) = self.page_size();
        self.ensure_tab_stops(page_width);
        let (top, bottom) = self.effective_vertical_margins(page_height);
        let (left, _right) = self.effective_horizontal_margins(page_width);
        let (row, mut column) = self.active_cursor_position();

        let clamp_to_margin = row >= top && row <= bottom && column >= left;
        let min_column = if clamp_to_margin { left } else { 0 };

        let mut performed = 0;
        while column > min_column && performed < count {
            column -= 1;
            if self.tab_stops.get(column as usize).copied().unwrap_or(false) {
                performed += 1;
            }
        }

        let buffer = self.page_manager.active_page().buffer();
        let mut buffer = buffer.write().unwrap_or_else(|e| e.into_inner());
        buffer.cursor_mut().set_position(row, column);
    }

    fn tab_clear(&mut self, clear_type: TabClearType) {
        let (page_width, _) = self.page_size();
        self.ensure_tab_stops(page_width);
        match clear_type {
            TabClearType::ClearCurrentColumn => {
                let (_, column) = self.active_cursor_position();
                if let Some(stop) = self.tab_stops.get_mut(column as usize) {
                    *stop = false;
                }
            }
            TabClearType::ClearAllColumns => self.tab_stops.iter_mut().for_each(|stop| *stop = false),
        }
    }

    fn set_keypad_mode(&mut self, application_mode: bool) {
        // Input-layer state; the flag is what the input crate will read.
        self.set_mode_flag(ModeFlags::DECNKM, application_mode);
    }

    fn designate_94_charset(&mut self, gset_number: VTInt, charset: VTID) {
        self.term_output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .designate_94_charset(gset_number.max(0) as usize, charset);
    }

    fn designate_96_charset(&mut self, gset_number: VTInt, charset: VTID) {
        self.term_output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .designate_96_charset(gset_number.max(0) as usize, charset);
    }

    fn locking_shift(&mut self, gset_number: VTInt) {
        self.term_output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .locking_shift(gset_number.max(0) as usize);
    }

    fn locking_shift_right(&mut self, gset_number: VTInt) {
        self.term_output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .locking_shift_right(gset_number.max(0) as usize);
    }

    fn single_shift(&mut self, gset_number: VTInt) {
        self.term_output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .single_shift(gset_number.max(0) as usize);
    }

    fn designate_coding_system(&mut self, coding_system: VTID) {
        // DOCS: only UTF-8 is modelled; anything else leaves the coding system
        // alone rather than silently reinterpreting the byte stream.
        let _ = coding_system;
    }

    fn send_c1_controls(&mut self, _enabled: bool) {}

    fn announce_code_structure(&mut self, _ansi_level: VTInt) {}

    fn tab_set(&mut self, _set_type: TabSetType) {
        let (page_width, _) = self.page_size();
        self.tab_stops.clear();
        self.ensure_tab_stops(page_width);
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
        /// Rectangular rather than stream attribute-change extent (DECSACE).
        const RECTANGULAR_CHANGE_EXTENT = 1 << 23;
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
        dispatch.change_attributes_rectangular_area(1, 1, 2, 2, &VTParameters::default());
        dispatch.reverse_attributes_rectangular_area(1, 1, 2, 2, &VTParameters::default());
        dispatch.copy_rectangular_area(1, 1, 1, 1, 1, 1, 1, 1);
        dispatch.fill_rectangular_area(VTParameter::from(32), 1, 1, 1, 1);
        dispatch.erase_rectangular_area(1, 1, 1, 1);
        dispatch.selective_erase_rectangular_area(1, 1, 1, 1);
        dispatch.select_attribute_change_extent(ChangeExtent::Rectangle);
        dispatch.request_checksum_rectangular_area(0, 1, 1, 1, 1, 1);
        dispatch.set_mode(&ModeParams::DecAwm);
        dispatch.reset_mode(&ModeParams::DecAwm);
        dispatch.request_mode(&ModeParams::DecOm);
        dispatch.device_status_report(StatusType::OperatingStatus, VTParameter::default());
        dispatch.device_status_report(StatusType::MacroSpaceReport, VTParameter::default());
        dispatch.device_status_report(StatusType::MemoryChecksum, VTParameter::default());
        if let Some(mut handler) = dispatch.define_macro(0, MacroDeleteControl::DeleteId, MacroEncoding::Text) {
            handler('\x1b');
        }
        dispatch.invoke_macro(0);
        dispatch.device_attributes();
        dispatch.secondary_device_attributes();
        dispatch.tertiary_device_attributes();
        dispatch.vt52_device_attributes();
        dispatch.request_terminal_parameters(ReportingPermission::Solicited);
        dispatch.set_optional_features(std::collections::HashSet::new());
        dispatch.set_window_title("winterm");
        dispatch.set_current_working_directory("file:///tmp");
        dispatch.set_clipboard("copied");
        dispatch.add_hyperlink("https://example.com", "");
        dispatch.end_hyperlink();
        dispatch.window_manipulation(
            WindowManipulationType::Invalid,
            VTParameter::default(),
            VTParameter::default(),
        );
        dispatch.set_color_table_entry(0, 0x000000);
        dispatch.request_color_table_entry(0);
        dispatch.reset_color_table();
        dispatch.reset_color_table_entry(0);
        dispatch.set_xterm_color_resource(10, 0x00FF0000);
        dispatch.request_xterm_color_resource(10);
        dispatch.reset_xterm_color_resource(10);
        dispatch.assign_color(ColorItem::NormalText, 0, 0);
        dispatch.request_terminal_state_report(ReportFormat::ColorTableReport, VTParameter::from(2));
        drop(dispatch.restore_terminal_state(ReportFormat::ColorTableReport));
        dispatch.cursor_save_state();
        dispatch.cursor_restore_state();
        dispatch.horizontal_tab_set();
        dispatch.forward_tab(1);
        dispatch.backwards_tab(1);
        dispatch.tab_clear(TabClearType::ClearAllColumns);
        dispatch.tab_set(TabSetType::SetEvery8Columns);
        dispatch.set_keypad_mode(true);
        dispatch.designate_94_charset(0, VTID::default());
        dispatch.designate_96_charset(0, VTID::default());
        dispatch.locking_shift(1);
        dispatch.locking_shift_right(1);
        dispatch.single_shift(2);
        dispatch.designate_coding_system(VTID::default());
        dispatch.accept_c1_controls(true);
        dispatch.send_c1_controls(true);
        dispatch.announce_code_structure(2);
        dispatch.insert_character(1);
        dispatch.delete_character(1);
        dispatch.insert_line(1);
        dispatch.delete_line(1);
        dispatch.scroll_up(1);
        dispatch.scroll_down(1);
        dispatch.insert_column(1);
        dispatch.delete_column(1);
        dispatch.page_position_absolute(1);
        dispatch.page_position_relative(1);
        dispatch.page_position_back(1);
        dispatch.next_page(1);
        dispatch.preceding_page(1);
        dispatch.request_displayed_extent();
        dispatch.set_ansi_mode(true);
        dispatch.set_cursor_style(CursorStyle::BlinkingBlock);
        dispatch.enquire_answerback();
        dispatch.warning_bell();
        dispatch.screen_alignment_pattern();
        dispatch.set_kitty_keyboard_protocol(VTParameter::from(1), VTParameter::from(1));
        dispatch.query_kitty_keyboard_protocol();
        dispatch.push_kitty_keyboard_protocol(VTParameter::from(2));
        dispatch.pop_kitty_keyboard_protocol(VTParameter::from(1));
        dispatch.play_sounds(&VTParameters::default());
        drop(dispatch.assign_user_preference_charset(CharsetSize::Size94));
        dispatch.request_user_preference_charset();
        assert_eq!(dispatch.unhandled_count(), 0);
        assert_eq!(dispatch.last_unhandled(), None);

        // The terminal-specific action strings are the family still unported.
        dispatch.do_conemu_action("");
        assert_eq!(dispatch.unhandled_count(), 1);
        assert_eq!(dispatch.last_unhandled(), Some("do_conemu_action"));
    }

    fn page_attributes(dispatch: &mut AdaptDispatch) -> types::TextAttribute {
        dispatch.page_manager_mut().active_page().attributes()
    }

    fn write_row(dispatch: &mut AdaptDispatch, row: i32, text: &str) {
        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let mut buffer = buffer.write().unwrap();
        buffer.cursor_mut().set_position(row, 0);
        buffer.write_string(text);
    }

    #[test]
    fn decrqde_reports_the_visible_extent() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.request_displayed_extent();
        assert_eq!(dispatch.take_responses(), vec!["\x1b[24;80;1;1;1\"w".to_string()]);
    }

    #[test]
    fn ich_and_dch_shift_the_current_line() {
        let mut dispatch = AdaptDispatch::new();
        write_row(&mut dispatch, 0, "abcd");
        dispatch.cursor_position(1, 1);
        dispatch.insert_character(1);

        assert_eq!(cell_text(&mut dispatch, 0, 0), " ");
        assert_eq!(cell_text(&mut dispatch, 0, 1), "a");
        assert_eq!(cell_text(&mut dispatch, 0, 3), "c");

        dispatch.delete_character(1);
        assert_eq!(cell_text(&mut dispatch, 0, 0), "a");
        assert_eq!(cell_text(&mut dispatch, 0, 2), "c");
        assert_eq!(cell_text(&mut dispatch, 0, 3), "d");
        assert_eq!(cell_text(&mut dispatch, 0, 4), "");
    }

    #[test]
    fn il_and_dl_shift_the_scrolling_region() {
        let mut dispatch = AdaptDispatch::new();
        write_row(&mut dispatch, 0, "aaa");
        write_row(&mut dispatch, 1, "bbb");

        dispatch.cursor_position(1, 1);
        dispatch.insert_line(1);
        assert_eq!(cell_text(&mut dispatch, 0, 0), " ");
        assert_eq!(cell_text(&mut dispatch, 1, 0), "a");
        assert_eq!(cell_text(&mut dispatch, 2, 0), "b");

        dispatch.delete_line(1);
        assert_eq!(cell_text(&mut dispatch, 0, 0), "a");
        assert_eq!(cell_text(&mut dispatch, 1, 0), "b");
    }

    #[test]
    fn decic_and_decdc_shift_the_current_row() {
        let mut dispatch = AdaptDispatch::new();
        write_row(&mut dispatch, 0, "abcd");
        dispatch.cursor_position(1, 1);
        dispatch.insert_column(1);

        assert_eq!(cell_text(&mut dispatch, 0, 0), " ");
        assert_eq!(cell_text(&mut dispatch, 0, 1), "a");
        assert_eq!(cell_text(&mut dispatch, 0, 4), "d");

        dispatch.delete_column(1);
        assert_eq!(cell_text(&mut dispatch, 0, 0), "a");
        assert_eq!(cell_text(&mut dispatch, 0, 3), "d");
        assert_eq!(cell_text(&mut dispatch, 0, 4), "");
    }

    #[test]
    fn su_and_sd_move_the_margin_region() {
        let mut dispatch = AdaptDispatch::new();
        write_row(&mut dispatch, 0, "aaa");
        write_row(&mut dispatch, 1, "bbb");
        write_row(&mut dispatch, 2, "ccc");

        dispatch.scroll_up(1);
        assert_eq!(cell_text(&mut dispatch, 0, 0), "b");
        assert_eq!(cell_text(&mut dispatch, 1, 0), "c");
        assert_eq!(cell_text(&mut dispatch, 2, 0), "");
        assert_eq!(cell_text(&mut dispatch, 23, 0), " ");

        dispatch.scroll_down(2);
        assert_eq!(cell_text(&mut dispatch, 0, 0), " ");
        assert_eq!(cell_text(&mut dispatch, 1, 0), " ");
        assert_eq!(cell_text(&mut dispatch, 2, 0), "b");
    }

    #[test]
    fn decanm_queues_a_parser_mode_change() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.reset_mode(&ModeParams::DecAnm);
        assert_eq!(dispatch.take_ansi_mode_change(), Some(false));

        dispatch.set_mode(&ModeParams::DecAnm);
        assert_eq!(dispatch.take_ansi_mode_change(), Some(true));
    }

    #[test]
    fn decrqupss_reports_the_assigned_charset() {
        let mut dispatch = AdaptDispatch::new();
        if let Some(mut handler) = dispatch.assign_user_preference_charset(CharsetSize::Size96) {
            handler('B');
        }
        dispatch.request_user_preference_charset();
        assert_eq!(dispatch.take_responses(), vec!["\x1bP1!uB\x1b\\".to_string()]);
    }

    #[test]
    fn decps_queues_midi_notes() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.play_sounds(&VTParameters::from_values(vec![7, 32, 1, 0]));

        let notes = dispatch.take_sound_requests();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].note, 72);
        assert_eq!(notes[0].velocity, 127);
        assert_eq!(notes[0].duration_micros, 1_000_000);
        assert_eq!(notes[1].note, 71);
        assert_eq!(notes[1].velocity, 0);
    }

    #[test]
    fn kitty_flags_set_push_pop_and_report() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_kitty_keyboard_protocol(VTParameter::from(5), VTParameter::from(1));
        assert_eq!(dispatch.kitty_keyboard_flags(), 5);

        dispatch.push_kitty_keyboard_protocol(VTParameter::from(3));
        assert_eq!(dispatch.kitty_keyboard_flags(), 3);

        dispatch.pop_kitty_keyboard_protocol(VTParameter::from(1));
        assert_eq!(dispatch.kitty_keyboard_flags(), 5);

        dispatch.query_kitty_keyboard_protocol();
        assert_eq!(dispatch.take_responses(), vec!["\x1b[?5u".to_string()]);
    }

    #[test]
    fn decaln_fills_the_page_and_homes_the_cursor() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(5, 5);
        dispatch.screen_alignment_pattern();

        assert_eq!(cell_text(&mut dispatch, 0, 0), "E");
        assert_eq!(cell_text(&mut dispatch, 23, 79), "E");
        assert_eq!(dispatch.active_cursor_position(), (0, 0));
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

    // Ported from `ScreenBufferTests::RectangularAreaOperations`: DECFRA fills
    // a rectangle with a character and the page's active attributes.
    #[test]
    fn decfra_fills_the_rectangle_with_the_requested_character() {
        let mut dispatch = AdaptDispatch::new();
        let buffer_attr = indexed_attribute();
        filled_page(&mut dispatch, 'Z', buffer_attr);

        let mut active = TextAttribute::new();
        active.set_indexed_foreground(4);
        active.set_intense(true);
        dispatch.page_manager_mut().active_page().set_attributes(active);

        // 1-based 3;27 to 6;54 is 0-based rows 2..=5 and columns 26..=53.
        dispatch.fill_rectangular_area(VTParameter::from(42), 3, 27, 6, 54);

        assert_eq!(cell_text(&mut dispatch, 2, 26), "*");
        assert_eq!(cell_text(&mut dispatch, 5, 53), "*");
        assert_eq!(cell_attribute(&mut dispatch, 2, 26), active);

        // Everything outside the rectangle is untouched.
        assert_eq!(cell_text(&mut dispatch, 1, 26), "Z");
        assert_eq!(cell_text(&mut dispatch, 2, 25), "Z");
        assert_eq!(cell_attribute(&mut dispatch, 2, 25), buffer_attr);
    }

    #[test]
    fn decfra_defaults_to_a_space_and_ignores_unprintable_values() {
        let mut dispatch = AdaptDispatch::new();
        filled_page(&mut dispatch, 'Z', indexed_attribute());

        dispatch.fill_rectangular_area(VTParameter::default(), 1, 1, 1, 1);
        assert_eq!(cell_text(&mut dispatch, 0, 0), " ");

        dispatch.fill_rectangular_area(VTParameter::from(1), 1, 2, 1, 2);
        assert_eq!(cell_text(&mut dispatch, 0, 1), "Z");
    }

    // DECERA uses `_GetEraseAttributes`: the active colours survive, the
    // rendition attributes do not.
    #[test]
    fn decera_erases_with_the_active_colours_and_no_rendition() {
        let mut dispatch = AdaptDispatch::new();
        filled_page(&mut dispatch, 'Z', indexed_attribute());
        dispatch.set_graphics_rendition(&VTParameters::from_values(vec![31, 1]));

        dispatch.erase_rectangular_area(3, 27, 6, 54);

        assert_eq!(cell_text(&mut dispatch, 2, 26), " ");
        let erased = cell_attribute(&mut dispatch, 2, 26);
        assert!(!erased.is_intense());
        assert_eq!(erased.get_foreground(), types::TextColor::index16(4));
        assert_eq!(cell_text(&mut dispatch, 1, 26), "Z");
    }

    #[test]
    fn decsera_erases_unprotected_text_and_keeps_attributes() {
        let mut dispatch = AdaptDispatch::new();
        let attributes = indexed_attribute();
        filled_page(&mut dispatch, 'Z', attributes);
        {
            let buffer = dispatch.page_manager_mut().active_page().buffer();
            let mut buffer = buffer.write().unwrap();
            let mut protected = attributes;
            protected.set_protected(true);
            buffer
                .get_row_at_screen_line_mut(2)
                .unwrap()
                .get_cell_mut(26)
                .set_attributes(protected);
        }

        dispatch.selective_erase_rectangular_area(3, 27, 6, 54);

        assert_eq!(cell_text(&mut dispatch, 2, 26), "Z");
        assert_eq!(cell_text(&mut dispatch, 3, 27), " ");
        assert_eq!(cell_attribute(&mut dispatch, 3, 27), attributes);
    }

    // DECCARA changes attributes without touching the text, and combines the
    // options into AND/XOR masks so an unset attribute is cleared.
    #[test]
    fn deccara_changes_attributes_and_leaves_the_text() {
        let mut dispatch = AdaptDispatch::new();
        let attributes = indexed_attribute();
        filled_page(&mut dispatch, 'Z', attributes);

        dispatch.change_attributes_rectangular_area(3, 27, 6, 54, &VTParameters::from_values(vec![7]));

        let changed = cell_attribute(&mut dispatch, 2, 26);
        assert!(changed.is_reverse_video());
        assert_eq!(changed.get_foreground(), attributes.get_foreground());
        assert_eq!(changed.get_background(), attributes.get_background());
        assert_eq!(cell_text(&mut dispatch, 2, 26), "Z");
        assert!(!cell_attribute(&mut dispatch, 1, 26).is_reverse_video());
    }

    #[test]
    fn deccara_clears_the_named_attributes_too() {
        let mut dispatch = AdaptDispatch::new();
        let mut attributes = indexed_attribute();
        attributes.set_intense(true);
        attributes.set_faint(true);
        filled_page(&mut dispatch, 'Z', attributes);

        dispatch.change_attributes_rectangular_area(1, 1, 2, 2, &VTParameters::from_values(vec![22]));

        let changed = cell_attribute(&mut dispatch, 0, 0);
        assert!(!changed.is_intense());
        assert!(!changed.is_faint());
    }

    #[test]
    fn decrara_toggles_the_named_attribute() {
        let mut dispatch = AdaptDispatch::new();
        let mut attributes = indexed_attribute();
        attributes.set_intense(true);
        filled_page(&mut dispatch, 'Z', attributes);

        dispatch.reverse_attributes_rectangular_area(3, 27, 6, 54, &VTParameters::from_values(vec![1]));

        assert!(!cell_attribute(&mut dispatch, 2, 26).is_intense());
        assert_eq!(cell_text(&mut dispatch, 2, 26), "Z");
        assert!(cell_attribute(&mut dispatch, 1, 26).is_intense());
    }

    #[test]
    fn decsace_switches_deccara_between_stream_and_rectangle() {
        let mut dispatch = AdaptDispatch::new();
        filled_page(&mut dispatch, 'Z', indexed_attribute());

        // The default stream extent runs 3;10 to 5;20 in reading order: the top
        // row to the end, the middle rows in full, the bottom row from the left.
        dispatch.change_attributes_rectangular_area(3, 10, 5, 20, &VTParameters::from_values(vec![7]));
        assert!(cell_attribute(&mut dispatch, 2, 9).is_reverse_video());
        assert!(!cell_attribute(&mut dispatch, 2, 8).is_reverse_video());
        assert!(cell_attribute(&mut dispatch, 2, 79).is_reverse_video());
        assert!(cell_attribute(&mut dispatch, 3, 0).is_reverse_video());
        assert!(cell_attribute(&mut dispatch, 4, 19).is_reverse_video());
        assert!(!cell_attribute(&mut dispatch, 4, 20).is_reverse_video());

        let mut dispatch = AdaptDispatch::new();
        filled_page(&mut dispatch, 'Z', indexed_attribute());
        dispatch.select_attribute_change_extent(ChangeExtent::Rectangle);
        dispatch.change_attributes_rectangular_area(3, 10, 5, 20, &VTParameters::from_values(vec![7]));
        assert!(cell_attribute(&mut dispatch, 2, 9).is_reverse_video());
        assert!(cell_attribute(&mut dispatch, 4, 19).is_reverse_video());
        assert!(!cell_attribute(&mut dispatch, 2, 20).is_reverse_video());
        assert!(!cell_attribute(&mut dispatch, 3, 0).is_reverse_video());
    }

    // DECCRA copies a rectangle from one place to another, text and attributes.
    #[test]
    fn deccra_copies_a_rectangle_within_the_page() {
        let mut dispatch = AdaptDispatch::new();
        filled_page(&mut dispatch, 'Z', indexed_attribute());

        let copy_attr = TextAttribute::from_rgb(0, 0xFF0000, Some(0));
        {
            let buffer = dispatch.page_manager_mut().active_page().buffer();
            let mut buffer = buffer.write().unwrap();
            let mut cell = buffer::OutputCell::from_char('*');
            cell.set_attributes(copy_attr);
            buffer.fill_rect(10, 26, 12, 53, &cell);
        }

        // 1-based 11;27 to 13;54 copied to 3;27.
        dispatch.copy_rectangular_area(11, 27, 13, 54, 1, 3, 27, 1);

        assert_eq!(cell_text(&mut dispatch, 2, 26), "*");
        assert_eq!(cell_text(&mut dispatch, 4, 53), "*");
        assert_eq!(cell_attribute(&mut dispatch, 2, 26), copy_attr);
        assert_eq!(cell_text(&mut dispatch, 1, 26), "Z");
        assert_eq!(cell_text(&mut dispatch, 5, 26), "Z");
    }

    #[test]
    fn deccra_reads_overlapping_source_cells_before_overwriting_them() {
        let mut dispatch = AdaptDispatch::new();
        filled_page(&mut dispatch, 'Z', indexed_attribute());
        for (row, ch) in [(0, 'A'), (1, 'B'), (2, 'C')] {
            let buffer = dispatch.page_manager_mut().active_page().buffer();
            let mut buffer = buffer.write().unwrap();
            buffer.get_row_at_screen_line_mut(row).unwrap().write_char(0, ch, false);
        }

        // Move rows 0..=2 down one row, so the destination overlaps the source.
        dispatch.copy_rectangular_area(1, 1, 3, 1, 1, 2, 1, 1);

        assert_eq!(cell_text(&mut dispatch, 0, 0), "A");
        assert_eq!(cell_text(&mut dispatch, 1, 0), "A");
        assert_eq!(cell_text(&mut dispatch, 2, 0), "B");
        assert_eq!(cell_text(&mut dispatch, 3, 0), "C");
        assert_eq!(cell_text(&mut dispatch, 4, 0), "Z");
    }

    // ADR 005: DECCRA moves the destination band's pixels with the cells.
    #[test]
    fn deccra_copies_the_image_band() {
        let mut dispatch = AdaptDispatch::new();
        {
            let buffer = dispatch.page_manager_mut().active_page().buffer();
            let mut buffer = buffer.write().unwrap();
            let mut image = buffer::ImageSlice::new((1, 1));
            let pixels = image.mutable_pixels(2, 4);
            pixels[0] = 0xFFFF_0000;
            pixels[1] = 0xFF00_FF00;
            buffer
                .get_row_at_screen_line_mut(5)
                .unwrap()
                .set_image_slice(Some(image));
        }

        // Source 6;1..4 (row 5, columns 0..4) to 1;1 (row 0).
        dispatch.copy_rectangular_area(6, 1, 6, 4, 1, 1, 1, 1);

        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        let image = buffer.get_row_at_screen_line(0).unwrap().image_slice().unwrap();
        assert_eq!(image.pixels(), &[0xFFFF_0000, 0xFF00_FF00]);
        assert_eq!(image.column_begin(), 2);
    }

    #[test]
    fn deccra_copies_between_pages() {
        let mut dispatch = AdaptDispatch::new();
        filled_page(&mut dispatch, 'Z', indexed_attribute());

        dispatch.set_mode(&ModeParams::AlternateScreenBuffer);
        dispatch.cursor_position(1, 1);
        dispatch.print_string("ALT");
        dispatch.reset_mode(&ModeParams::AlternateScreenBuffer);

        dispatch.copy_rectangular_area(1, 1, 1, 3, 2, 5, 10, 1);

        assert_eq!(cell_text(&mut dispatch, 4, 9), "A");
        assert_eq!(cell_text(&mut dispatch, 4, 10), "L");
        assert_eq!(cell_text(&mut dispatch, 4, 11), "T");
    }

    #[test]
    fn rect_area_zero_extends_to_the_page() {
        let mut dispatch = AdaptDispatch::new();
        filled_page(&mut dispatch, 'Z', indexed_attribute());

        dispatch.erase_rectangular_area(1, 1, 0, 0);

        assert_eq!(cell_text(&mut dispatch, 0, 0), " ");
        assert_eq!(cell_text(&mut dispatch, 23, 79), " ");
    }

    #[test]
    fn decrqcra_reports_zero_until_the_feature_is_enabled() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print('A');

        dispatch.request_checksum_rectangular_area(7, 1, 1, 1, 1, 1);

        assert_eq!(dispatch.take_responses(), vec!["\x1bP7!~0000\x1b\\".to_string()]);
    }

    #[test]
    fn decrqcra_checksums_the_requested_cells() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print('A');
        dispatch.set_optional_features(std::collections::HashSet::from([OptionalFeature::ChecksumReport]));

        dispatch.request_checksum_rectangular_area(1, 1, 1, 1, 1, 1);

        // 'A' (0x41) and the default colour indices 7 and 0, subtracted from zero.
        let expected = 0u16.wrapping_sub(0x41).wrapping_sub(0x70);
        assert_eq!(dispatch.take_responses(), vec![format!("\x1bP1!~{expected:04X}\x1b\\")]);
    }

    // OSC 0/2: xterm caps titles at 1000 UTF-16 code units.
    #[test]
    fn window_title_is_recorded_and_capped() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_window_title("winterm");
        assert_eq!(dispatch.window_title(), Some("winterm"));

        let too_long = "x".repeat(1001);
        dispatch.set_window_title(&too_long);
        assert_eq!(dispatch.window_title(), Some("winterm"));
    }

    #[test]
    fn working_directory_uri_is_recorded() {
        let mut dispatch = AdaptDispatch::new();

        dispatch.set_current_working_directory("file:///tmp/work");

        assert_eq!(dispatch.current_working_directory(), Some("file:///tmp/work"));
    }

    #[test]
    fn clipboard_content_waits_for_the_host() {
        let mut dispatch = AdaptDispatch::new();

        dispatch.set_clipboard("copied");

        assert_eq!(dispatch.take_clipboard_content().as_deref(), Some("copied"));
        assert_eq!(dispatch.take_clipboard_content(), None);
    }

    // OSC 8: the id goes on the page attributes, so it is copied into every
    // cell written until the hyperlink ends, and the URI is resolvable from
    // the buffer.
    #[test]
    fn hyperlinks_set_the_page_id_and_are_resolvable() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.add_hyperlink("https://example.com", "custom");
        let id = page_attributes(&mut dispatch).get_hyperlink_id();
        assert!(id > 0);

        dispatch.print('X');
        assert_eq!(cell_hyperlink_id(&mut dispatch, 0, 0), id);
        {
            let buffer = dispatch.page_manager_mut().active_page().buffer();
            let buffer = buffer.read().unwrap();
            assert_eq!(buffer.get_hyperlink_uri(id), Some("https://example.com"));
        }

        dispatch.end_hyperlink();
        assert_eq!(page_attributes(&mut dispatch).get_hyperlink_id(), 0);
        dispatch.print('Y');
        assert_eq!(cell_hyperlink_id(&mut dispatch, 0, 1), 0);
    }

    #[test]
    fn repeated_hyperlink_starts_reuse_the_id() {
        let mut dispatch = AdaptDispatch::new();

        dispatch.add_hyperlink("https://example.com", "same");
        let first = page_attributes(&mut dispatch).get_hyperlink_id();
        dispatch.end_hyperlink();
        dispatch.add_hyperlink("https://example.com", "same");

        assert_eq!(page_attributes(&mut dispatch).get_hyperlink_id(), first);
    }

    fn define_text_macro(dispatch: &mut AdaptDispatch, id: VTInt, payload: &str) {
        let mut handler = dispatch
            .define_macro(id, MacroDeleteControl::DeleteId, MacroEncoding::Text)
            .expect("macro definition accepted");
        for ch in payload.chars() {
            assert!(handler(ch));
        }
        assert!(!handler('\x1b'));
    }

    #[test]
    fn a_defined_macro_is_queued_for_invocation() {
        let mut dispatch = AdaptDispatch::new();
        define_text_macro(&mut dispatch, 0, "AB");

        dispatch.invoke_macro(0);

        assert_eq!(dispatch.take_macro_input(), vec![b"AB".to_vec()]);
    }

    #[test]
    fn invoking_an_undefined_macro_queues_nothing() {
        let mut dispatch = AdaptDispatch::new();

        dispatch.invoke_macro(3);

        assert!(dispatch.take_macro_input().is_empty());
    }

    #[test]
    fn macro_space_and_checksum_reports_answer_from_the_buffer() {
        let mut dispatch = AdaptDispatch::new();
        define_text_macro(&mut dispatch, 0, "AB");

        dispatch.device_status_report(StatusType::MacroSpaceReport, VTParameter::default());
        dispatch.device_status_report(StatusType::MemoryChecksum, VTParameter::from(7));

        let checksum = 0u16.wrapping_sub(0x41).wrapping_sub(0x42);
        let space = (crate::macro_buffer::MAX_SPACE - 2) / 16;
        assert_eq!(
            dispatch.take_responses(),
            vec![format!("\x1b[{space}*{{"), format!("\x1bP7!~{checksum:04X}\x1b\\"),]
        );
    }

    #[test]
    fn a_hard_reset_clears_macro_definitions() {
        let mut dispatch = AdaptDispatch::new();
        define_text_macro(&mut dispatch, 0, "AB");

        dispatch.hard_reset(false);
        dispatch.invoke_macro(0);

        assert!(dispatch.take_macro_input().is_empty());
        let buffer = dispatch.macro_buffer();
        let buffer = buffer.lock().unwrap();
        assert_eq!(buffer.defined_count(), 0);
    }

    #[test]
    fn window_visibility_and_redraw_requests_wait_for_the_host() {
        let mut dispatch = AdaptDispatch::new();

        dispatch.window_manipulation(
            WindowManipulationType::IconifyWindow,
            VTParameter::default(),
            VTParameter::default(),
        );
        assert_eq!(dispatch.take_window_visibility(), Some(false));
        assert_eq!(dispatch.take_window_visibility(), None);

        dispatch.window_manipulation(
            WindowManipulationType::DeIconifyWindow,
            VTParameter::default(),
            VTParameter::default(),
        );
        assert_eq!(dispatch.take_window_visibility(), Some(true));

        dispatch.window_manipulation(
            WindowManipulationType::RefreshWindow,
            VTParameter::default(),
            VTParameter::default(),
        );
        assert!(dispatch.take_redraw_request());
        assert!(!dispatch.take_redraw_request());
    }

    #[test]
    fn window_resize_records_the_requested_width_and_height() {
        let mut dispatch = AdaptDispatch::new();

        // The second parameter is the width and the first the height.
        dispatch.window_manipulation(
            WindowManipulationType::ResizeWindowInCharacters,
            VTParameter::from(40),
            VTParameter::from(120),
        );

        assert_eq!(dispatch.take_window_resize(), Some((120, 40)));
        assert_eq!(dispatch.take_window_resize(), None);
    }

    #[test]
    fn window_size_reports_use_the_visible_page() {
        let mut dispatch = AdaptDispatch::new();

        dispatch.window_manipulation(
            WindowManipulationType::ReportTextSizeInCharacters,
            VTParameter::default(),
            VTParameter::default(),
        );
        dispatch.window_manipulation(
            WindowManipulationType::ReportCharacterCellSize,
            VTParameter::default(),
            VTParameter::default(),
        );
        dispatch.window_manipulation(
            WindowManipulationType::ReportTextSizeInPixels,
            VTParameter::default(),
            VTParameter::default(),
        );

        assert_eq!(
            dispatch.take_responses(),
            vec![
                "\x1b[8;24;80t".to_string(),
                "\x1b[6;20;10t".to_string(),
                "\x1b[4;480;800t".to_string(),
            ]
        );
    }

    // A sixel payload flows through the DCS handler into the page's row bands,
    // which ADR 005 makes the single home for image content.
    #[test]
    fn a_sixel_payload_paints_into_the_page() {
        let mut dispatch = AdaptDispatch::new();
        let initial_visibility = {
            let buffer = dispatch.page_manager_mut().active_page().buffer();
            let buffer = buffer.read().unwrap();
            buffer.cursor().is_visible()
        };
        let mut handler = dispatch
            .define_sixel_image(0, SixelBackground::Transparent, VTParameter::default())
            .expect("the default conformance level accepts sixel");

        assert!(handler('#'), "the payload continues until ESC");
        {
            let buffer = dispatch.page_manager_mut().active_page().buffer();
            let buffer = buffer.read().unwrap();
            assert!(
                !buffer.cursor().is_visible(),
                "the cursor is hidden while the image draws"
            );
        }

        for ch in "1;2;0;100;0~".chars() {
            assert!(handler(ch), "the payload continues until ESC");
        }
        assert!(!handler('\x1b'), "ESC terminates the payload");

        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        let image = buffer
            .get_row_at_screen_line(0)
            .unwrap()
            .image_slice()
            .expect("the image lands in the first row's band");

        // One sixel column at aspect ratio 2 covers 12 device rows; the band is
        // grown to a whole cell, 10 by 20 pixels.
        assert_eq!(image.cell_size(), (10, 20));
        assert_eq!(image.column_begin(), 0);
        assert_eq!(image.column_end(), 1);

        let pixels = image.pixels();
        for row in 0..12 {
            assert_eq!(pixels[row * 10], 0xFF00_FF00, "row {row} is green");
            assert_eq!(pixels[row * 10 + 1], 0, "row {row} stops after one column");
        }
        assert_eq!(pixels[12 * 10], 0, "the image ends at row 12");

        // The cursor visibility is restored once the image is painted.
        assert_eq!(buffer.cursor().is_visible(), initial_visibility);
        assert_eq!(buffer.cursor().position(), (0, 0));
    }

    // OSC 4 takes a COLORREF (0x00BBGGRR) and reports the 16-bit xterm form.
    #[test]
    fn osc4_sets_queries_and_resets_a_colour() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_color_table_entry(4, 0x00FF0000);

        dispatch.request_color_table_entry(4);
        assert_eq!(
            dispatch.take_responses(),
            vec!["\x1b]4;4;rgb:0000/0000/ffff\x1b\\".to_string()]
        );

        dispatch.reset_color_table_entry(4);
        let table = dispatch.color_table();
        let table = table.lock().unwrap();
        assert_eq!(table.get(4), Some(types::TextColor::rgb(0x00, 0x37, 0xDA)));
    }

    #[test]
    fn osc104_resets_indexed_entries_but_not_aliases() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_color_table_entry(1, 0x000000FF);
        dispatch.set_xterm_color_resource(12, 0x00FF0000);

        dispatch.reset_color_table();

        let table = dispatch.color_table();
        let table = table.lock().unwrap();
        assert_eq!(table.get(1), Some(types::TextColor::rgb(0xC5, 0x0F, 0x1F)));
        assert_eq!(table.get(CURSOR_COLOR), Some(types::TextColor::rgb(0, 0, 255)));
    }

    #[test]
    fn osc10_and_11_move_the_default_aliases() {
        let mut dispatch = AdaptDispatch::new();

        dispatch.set_xterm_color_resource(10, 0x0000FF00);
        dispatch.request_xterm_color_resource(10);

        assert_eq!(
            dispatch.take_responses(),
            vec!["\x1b]10;rgb:0000/ffff/0000\x1b\\".to_string()]
        );
        {
            let table = dispatch.color_table();
            let table = table.lock().unwrap();
            assert_eq!(table.alias_index(ColorAlias::DefaultForeground), DEFAULT_FOREGROUND);
            assert_eq!(table.get(DEFAULT_FOREGROUND), Some(types::TextColor::rgb(0, 255, 0)));
        }

        dispatch.reset_xterm_color_resource(10);
        let table = dispatch.color_table();
        let table = table.lock().unwrap();
        assert_eq!(
            table.alias_index(ColorAlias::DefaultForeground),
            TextColor::DARK_WHITE.get_index() as usize
        );
    }

    #[test]
    fn decac_moves_the_default_and_frame_aliases() {
        let mut dispatch = AdaptDispatch::new();

        dispatch.assign_color(ColorItem::NormalText, 3, 5);
        {
            let table = dispatch.color_table();
            let table = table.lock().unwrap();
            assert_eq!(table.alias_index(ColorAlias::DefaultForeground), 3);
            assert_eq!(table.alias_index(ColorAlias::DefaultBackground), 5);
        }
        assert!(dispatch.take_redraw_request());

        dispatch.assign_color(ColorItem::WindowFrame, 7, 8);
        let table = dispatch.color_table();
        let table = table.lock().unwrap();
        assert_eq!(table.alias_index(ColorAlias::FrameForeground), 7);
        assert_eq!(table.alias_index(ColorAlias::FrameBackground), 8);
    }

    #[test]
    fn decac_ignores_out_of_range_indices() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.assign_color(ColorItem::NormalText, 3, 4);
        dispatch.assign_color(ColorItem::NormalText, -1, 9999);

        let table = dispatch.color_table();
        let table = table.lock().unwrap();
        assert_eq!(table.alias_index(ColorAlias::DefaultForeground), 3);
        assert_eq!(table.alias_index(ColorAlias::DefaultBackground), 4);
    }

    #[test]
    fn decrqtsr_reports_the_colour_table_in_rgb_or_hls() {
        let mut dispatch = AdaptDispatch::new();

        dispatch.request_terminal_state_report(ReportFormat::ColorTableReport, VTParameter::from(2));
        let responses = dispatch.take_responses();
        assert_eq!(responses.len(), 1);
        let response = &responses[0];
        assert!(response.starts_with("\x1bP2$s0;2;"));
        assert!(response.contains("/4;2;0;22;85"));
        assert!(response.ends_with("\x1b\\"));

        dispatch.set_color_table_entry(0, 0x000000FF);
        dispatch.request_terminal_state_report(ReportFormat::ColorTableReport, VTParameter::from(1));
        let responses = dispatch.take_responses();
        assert!(responses[0].contains("0;1;120;50;100"));
    }

    #[test]
    fn decrsts_restores_the_reported_table() {
        let mut source = AdaptDispatch::new();
        source.set_color_table_entry(4, 0x00FF0000);
        source.request_terminal_state_report(ReportFormat::ColorTableReport, VTParameter::from(2));
        let response = source.take_responses().remove(0);

        let mut target = AdaptDispatch::new();
        let mut handler = target
            .restore_terminal_state(ReportFormat::ColorTableReport)
            .expect("colour table restore is supported");
        let body = response.trim_start_matches("\x1bP").trim_end_matches("\x1b\\");
        for ch in body.chars() {
            assert!(handler(ch));
        }
        assert!(!handler('\x1b'));

        let table = target.color_table();
        let table = table.lock().unwrap();
        assert_eq!(table.get(4), Some(types::TextColor::rgb(0, 0, 255)));
        assert!(
            target
                .restore_terminal_state(ReportFormat::TerminalStateReport)
                .is_none()
        );
    }

    #[test]
    fn a_hard_reset_restores_the_default_colour_table() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.set_color_table_entry(0, 0x000000FF);

        dispatch.hard_reset(false);

        let table = dispatch.color_table();
        let table = table.lock().unwrap();
        assert_eq!(table.get(0), Some(types::TextColor::rgb(0x0C, 0x0C, 0x0C)));
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

    // Ported from `_SetColumnMode` (DECCOLM): the mode is inert until XTERM
    // mode 40 allows it; a permitted change resizes the page, clears it, homes
    // the cursor and resets origin mode and the margins.
    #[test]
    fn decolm_only_resizes_once_it_is_allowed() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.print_string("keep");
        dispatch.cursor_position(5, 5);
        dispatch.set_top_bottom_scrolling_margins(2, 20);

        dispatch.set_mode(&ModeParams::DecColm);
        assert_eq!(page_size(&mut dispatch), (80, 24));
        assert!(!dispatch.mode_is_set(ModeFlags::DECCOLM));

        dispatch.set_mode(&ModeParams::EnableDecColmSupport);
        dispatch.set_mode(&ModeParams::DecColm);

        assert_eq!(page_size(&mut dispatch), (132, 24));
        assert_eq!(dispatch.page_manager_mut().visible_page().width(), 132);
        assert!(dispatch.mode_is_set(ModeFlags::DECCOLM));
        assert_eq!(dispatch.active_cursor_position(), (0, 0));
        assert_eq!(dispatch.vertical_margins(), (0, 0));
        assert_eq!(cell_text(&mut dispatch, 0, 0), " ");

        dispatch.reset_mode(&ModeParams::DecColm);
        assert_eq!(page_size(&mut dispatch), (80, 24));
        assert!(!dispatch.mode_is_set(ModeFlags::DECCOLM));
    }

    #[test]
    fn decrqm_reports_unsupported_for_unmodelled_modes() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.request_mode(&ModeParams::GraphemeClusterMode);
        assert_eq!(dispatch.take_responses(), vec!["\x1b[?2027;0$y".to_string()]);
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

    #[test]
    fn forward_tab_moves_to_the_next_eighth_column() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.forward_tab(1);
        assert_eq!(dispatch.active_cursor_position(), (0, 8));
        dispatch.forward_tab(2);
        assert_eq!(dispatch.active_cursor_position(), (0, 24));
    }

    #[test]
    fn backwards_tab_moves_to_the_previous_stop() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(1, 25);
        dispatch.backwards_tab(1);
        assert_eq!(dispatch.active_cursor_position(), (0, 16));
    }

    #[test]
    fn hts_adds_a_stop_and_tbc_clears_it() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(1, 4);
        dispatch.horizontal_tab_set();

        dispatch.cursor_position(1, 1);
        dispatch.forward_tab(1);
        assert_eq!(dispatch.active_cursor_position(), (0, 3));

        dispatch.cursor_position(1, 4);
        dispatch.tab_clear(TabClearType::ClearCurrentColumn);
        dispatch.cursor_position(1, 1);
        dispatch.forward_tab(1);
        assert_eq!(dispatch.active_cursor_position(), (0, 8));
    }

    #[test]
    fn saving_and_restoring_the_cursor_round_trips_position_and_attributes() {
        let mut dispatch = AdaptDispatch::new();
        dispatch.cursor_position(5, 10);
        dispatch.set_graphics_rendition(&VTParameters::from_values(vec![1]));
        dispatch.cursor_save_state();

        dispatch.cursor_position(1, 1);
        dispatch.set_graphics_rendition(&VTParameters::from_values(vec![22]));
        dispatch.cursor_restore_state();

        assert_eq!(dispatch.active_cursor_position(), (4, 9));
        assert!(page_attributes(&mut dispatch).is_intense());
    }

    fn cell_text(dispatch: &mut AdaptDispatch, row: i32, column: i32) -> String {
        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        buffer.get_text_data_at(row, column).unwrap_or_default().to_string()
    }

    fn page_size(dispatch: &mut AdaptDispatch) -> (i32, i32) {
        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        (buffer.screen_width(), buffer.screen_height())
    }

    fn filled_page(dispatch: &mut AdaptDispatch, ch: char, attributes: TextAttribute) {
        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let mut buffer = buffer.write().unwrap();
        let (height, width) = (buffer.screen_height(), buffer.screen_width());
        let mut cell = buffer::OutputCell::from_char(ch);
        cell.set_attributes(attributes);
        buffer.fill_rect(0, 0, height - 1, width - 1, &cell);
    }

    fn cell_attribute(dispatch: &mut AdaptDispatch, row: i32, column: i32) -> TextAttribute {
        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        *buffer.get_cell_data_at(row, column).unwrap().get_attributes()
    }

    fn indexed_attribute() -> TextAttribute {
        let mut attributes = TextAttribute::new();
        attributes.set_indexed_foreground(1);
        attributes.set_indexed_background(2);
        attributes
    }

    fn cell_hyperlink_id(dispatch: &mut AdaptDispatch, row: i32, column: i32) -> u16 {
        let buffer = dispatch.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        buffer
            .get_cell_data_at(row, column)
            .map(|cell| cell.get_attributes().get_hyperlink_id())
            .unwrap_or(0)
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
