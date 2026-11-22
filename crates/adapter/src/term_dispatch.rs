//! `ITermDispatch` — Output state machine callback interface.
//!
//! Ported from `ITermDispatch.hpp` in the Windows Terminal C++ codebase.
//! This is the primary interface for VT sequence dispatching.

use crate::dispatch_types::*;
pub use types::LineRendition;

/// Interface for all output state machine callbacks.
/// When VT sequences are parsed, they are dispatched to methods on this interface.
#[allow(unused_variables)]
pub trait ITermDispatch: std::fmt::Debug {
    /// Reports a callback that reached a dispatcher with no ported handler.
    ///
    /// Upstream's `TermDispatch` is an all-no-op default implementation used by
    /// tests; this is the same layering with a signal attached. A dispatcher that
    /// has not ported a family records the sequence instead of silently
    /// succeeding, so a missing handler shows up in a test rather than in a
    /// terminal drawing the wrong thing. `AdaptDispatch` overrides this to count
    /// the callbacks it cannot yet serve.
    fn unhandled(&mut self, _sequence: &'static str) {}

    // ─── Basic Output ───────────────────────────────────────────────

    /// Called for unknown/unhandled sequences.
    fn unknown_sequence(&mut self) {
        self.unhandled("unknown_sequence");
    }

    /// Prints a single printable character.
    fn print(&mut self, ch: char) {
        self.unhandled("print");
    }

    /// Prints a string of characters.
    fn print_string(&mut self, string: &str) {
        self.unhandled("print_string");
    }

    /// An APC string, with its introducer stripped.
    ///
    /// APC carries no VT state of its own; a protocol that rides it reads the
    /// bytes here. The kitty graphics protocol is one (ADR 054).
    fn apc(&mut self, _apc: &[u8]) {
        self.unhandled("apc");
    }

    // ─── Cursor Movement ────────────────────────────────────────────

    /// CUU — Cursor Up.
    fn cursor_up(&mut self, distance: VTInt) {
        self.unhandled("cursor_up");
    }

    /// CUD — Cursor Down.
    fn cursor_down(&mut self, distance: VTInt) {
        self.unhandled("cursor_down");
    }

    /// CUF — Cursor Forward.
    fn cursor_forward(&mut self, distance: VTInt) {
        self.unhandled("cursor_forward");
    }

    /// CUB — Cursor Backward.
    fn cursor_backward(&mut self, distance: VTInt) {
        self.unhandled("cursor_backward");
    }

    /// CNL — Cursor Next Line.
    fn cursor_next_line(&mut self, distance: VTInt) {
        self.unhandled("cursor_next_line");
    }

    /// CPL — Cursor Previous Line.
    fn cursor_prev_line(&mut self, distance: VTInt) {
        self.unhandled("cursor_prev_line");
    }

    /// HPA — Horizontal Position Absolute.
    fn cursor_horizontal_position_absolute(&mut self, column: VTInt) {
        self.unhandled("cursor_horizontal_position_absolute");
    }

    /// VPA — Vertical Line Position Absolute.
    fn vertical_line_position_absolute(&mut self, line: VTInt) {
        self.unhandled("vertical_line_position_absolute");
    }

    /// HPR — Horizontal Position Relative.
    fn horizontal_position_relative(&mut self, distance: VTInt) {
        self.unhandled("horizontal_position_relative");
    }

    /// VPR — Vertical Position Relative.
    fn vertical_position_relative(&mut self, distance: VTInt) {
        self.unhandled("vertical_position_relative");
    }

    /// CUP — Cursor Position (HVP).
    fn cursor_position(&mut self, line: VTInt, column: VTInt) {
        self.unhandled("cursor_position");
    }

    /// DECSC — Cursor Save State.
    fn cursor_save_state(&mut self) {
        self.unhandled("cursor_save_state");
    }

    /// DECRC — Cursor Restore State.
    fn cursor_restore_state(&mut self) {
        self.unhandled("cursor_restore_state");
    }

    // ─── Insert/Delete ──────────────────────────────────────────────

    /// ICH — Insert Character.
    fn insert_character(&mut self, count: VTInt) {
        self.unhandled("insert_character");
    }

    /// DCH — Delete Character.
    fn delete_character(&mut self, count: VTInt) {
        self.unhandled("delete_character");
    }

    /// SU — Scroll Up.
    fn scroll_up(&mut self, distance: VTInt) {
        self.unhandled("scroll_up");
    }

    /// SD — Scroll Down.
    fn scroll_down(&mut self, distance: VTInt) {
        self.unhandled("scroll_down");
    }

    /// NP — Next Page.
    fn next_page(&mut self, page_count: VTInt) {
        self.unhandled("next_page");
    }

    /// PP — Preceding Page.
    fn preceding_page(&mut self, page_count: VTInt) {
        self.unhandled("preceding_page");
    }

    /// PPA — Page Position Absolute.
    fn page_position_absolute(&mut self, page: VTInt) {
        self.unhandled("page_position_absolute");
    }

    /// PPR — Page Position Relative.
    fn page_position_relative(&mut self, page_count: VTInt) {
        self.unhandled("page_position_relative");
    }

    /// PPB — Page Position Back.
    fn page_position_back(&mut self, page_count: VTInt) {
        self.unhandled("page_position_back");
    }

    /// DECRQDE — Request Displayed Extent.
    fn request_displayed_extent(&mut self) {
        self.unhandled("request_displayed_extent");
    }

    /// IL — Insert Line.
    fn insert_line(&mut self, distance: VTInt) {
        self.unhandled("insert_line");
    }

    /// DL — Delete Line.
    fn delete_line(&mut self, distance: VTInt) {
        self.unhandled("delete_line");
    }

    /// DECIC — Insert Column.
    fn insert_column(&mut self, distance: VTInt) {
        self.unhandled("insert_column");
    }

    /// DECDC — Delete Column.
    fn delete_column(&mut self, distance: VTInt) {
        self.unhandled("delete_column");
    }

    // ─── Modes ──────────────────────────────────────────────────────

    /// DECKPAM — Set Keypad Application Mode.
    fn set_keypad_mode(&mut self, application_mode: bool) {
        self.unhandled("set_keypad_mode");
    }

    /// DECANM — Set ANSI Mode.
    fn set_ansi_mode(&mut self, ansi_mode: bool) {
        self.unhandled("set_ansi_mode");
    }

    /// KKP — Set Kitty Keyboard Protocol.
    fn set_kitty_keyboard_protocol(&mut self, flags: VTParameter, mode: VTParameter) {
        self.unhandled("set_kitty_keyboard_protocol");
    }

    /// KKP — Query Kitty Keyboard Protocol.
    fn query_kitty_keyboard_protocol(&mut self) {
        self.unhandled("query_kitty_keyboard_protocol");
    }

    /// KKP — Push Kitty Keyboard Protocol.
    fn push_kitty_keyboard_protocol(&mut self, flags: VTParameter) {
        self.unhandled("push_kitty_keyboard_protocol");
    }

    /// KKP — Pop Kitty Keyboard Protocol.
    fn pop_kitty_keyboard_protocol(&mut self, count: VTParameter) {
        self.unhandled("pop_kitty_keyboard_protocol");
    }

    /// DECSTBM — Set Top/Bottom Scrolling Margins.
    fn set_top_bottom_scrolling_margins(&mut self, top_margin: VTInt, bottom_margin: VTInt) {
        self.unhandled("set_top_bottom_scrolling_margins");
    }

    /// DECSLRM — Set Left/Right Scrolling Margins.
    fn set_left_right_scrolling_margins(&mut self, left_margin: VTInt, right_margin: VTInt) {
        self.unhandled("set_left_right_scrolling_margins");
    }

    /// ENQ — Enquire/Answerback.
    fn enquire_answerback(&mut self) {
        self.unhandled("enquire_answerback");
    }

    // ─── Control Actions ────────────────────────────────────────────

    /// BEL — Warning Bell.
    fn warning_bell(&mut self) {
        self.unhandled("warning_bell");
    }

    /// CR — Carriage Return.
    fn carriage_return(&mut self) {
        self.unhandled("carriage_return");
    }

    /// IND/NEL/LF/FF/VT — Line Feed.
    fn line_feed(&mut self, line_feed_type: LineFeedType) {
        self.unhandled("line_feed");
    }

    /// RI — Reverse Line Feed.
    fn reverse_line_feed(&mut self) {
        self.unhandled("reverse_line_feed");
    }

    /// DECBI — Back Index.
    fn back_index(&mut self) {
        self.unhandled("back_index");
    }

    /// DECFI — Forward Index.
    fn forward_index(&mut self) {
        self.unhandled("forward_index");
    }

    /// DECSWT — Set Window Title.
    fn set_window_title(&mut self, title: &str) {
        self.unhandled("set_window_title");
    }

    /// OSC 7 — Set Current Working Directory.
    fn set_current_working_directory(&mut self, uri: &str) {
        self.unhandled("set_current_working_directory");
    }

    /// HTS — Horizontal Tab Set.
    fn horizontal_tab_set(&mut self) {
        self.unhandled("horizontal_tab_set");
    }

    /// CHT/HT — Forward Tab.
    fn forward_tab(&mut self, num_tabs: VTInt) {
        self.unhandled("forward_tab");
    }

    /// CBT — Backwards Tab.
    fn backwards_tab(&mut self, num_tabs: VTInt) {
        self.unhandled("backwards_tab");
    }

    /// TBC — Tab Clear.
    fn tab_clear(&mut self, clear_type: TabClearType) {
        self.unhandled("tab_clear");
    }

    /// DECST8C — Tab Set.
    fn tab_set(&mut self, set_type: TabSetType) {
        self.unhandled("tab_set");
    }

    // ─── Colors ─────────────────────────────────────────────────────

    /// OSC — Set Color Table Entry.
    fn set_color_table_entry(&mut self, table_index: usize, color: u32) {
        self.unhandled("set_color_table_entry");
    }

    /// OSC — Request Color Table Entry.
    fn request_color_table_entry(&mut self, table_index: usize) {
        self.unhandled("request_color_table_entry");
    }

    /// OSC — Reset Color Table.
    fn reset_color_table(&mut self) {
        self.unhandled("reset_color_table");
    }

    /// OSC — Reset Color Table Entry.
    fn reset_color_table_entry(&mut self, table_index: usize) {
        self.unhandled("reset_color_table_entry");
    }

    /// OSC — Set Xterm Color Resource.
    fn set_xterm_color_resource(&mut self, resource: usize, color: u32) {
        self.unhandled("set_xterm_color_resource");
    }

    /// OSC — Request Xterm Color Resource.
    fn request_xterm_color_resource(&mut self, resource: usize) {
        self.unhandled("request_xterm_color_resource");
    }

    /// OSC — Reset Xterm Color Resource.
    fn reset_xterm_color_resource(&mut self, resource: usize) {
        self.unhandled("reset_xterm_color_resource");
    }

    /// DECAC — Assign Color.
    fn assign_color(&mut self, item: ColorItem, fg_index: VTInt, bg_index: VTInt) {
        self.unhandled("assign_color");
    }

    // ─── Erase ──────────────────────────────────────────────────────

    /// ED — Erase in Display.
    fn erase_in_display(&mut self, erase_type: EraseType) {
        self.unhandled("erase_in_display");
    }

    /// EL — Erase in Line.
    fn erase_in_line(&mut self, erase_type: EraseType) {
        self.unhandled("erase_in_line");
    }

    /// ECH — Erase Characters.
    fn erase_characters(&mut self, num_chars: VTInt) {
        self.unhandled("erase_characters");
    }

    /// DECSED — Selective Erase in Display.
    fn selective_erase_in_display(&mut self, erase_type: EraseType) {
        self.unhandled("selective_erase_in_display");
    }

    /// DECSEL — Selective Erase in Line.
    fn selective_erase_in_line(&mut self, erase_type: EraseType) {
        self.unhandled("selective_erase_in_line");
    }

    // ─── Rectangular Operations ─────────────────────────────────────

    /// DECCARA — Change Attributes Rectangular Area.
    fn change_attributes_rectangular_area(
        &mut self,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
        attrs: &VTParameters,
    ) {
        self.unhandled("change_attributes_rectangular_area");
    }

    /// DECRARA — Reverse Attributes Rectangular Area.
    fn reverse_attributes_rectangular_area(
        &mut self,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
        attrs: &VTParameters,
    ) {
        self.unhandled("reverse_attributes_rectangular_area");
    }

    /// DECCRA — Copy Rectangular Area.
    // The signature mirrors `ITermDispatch::CopyRectangularArea` in the C++
    // adapter; collapsing the DECCRA operands into a struct would make the port
    // harder to diff against upstream.
    #[allow(clippy::too_many_arguments)]
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
        self.unhandled("copy_rectangular_area");
    }

    /// DECFRA — Fill Rectangular Area.
    fn fill_rectangular_area(&mut self, ch: VTParameter, top: VTInt, left: VTInt, bottom: VTInt, right: VTInt) {
        self.unhandled("fill_rectangular_area");
    }

    /// DECERA — Erase Rectangular Area.
    fn erase_rectangular_area(&mut self, top: VTInt, left: VTInt, bottom: VTInt, right: VTInt) {
        self.unhandled("erase_rectangular_area");
    }

    /// DECSERA — Selective Erase Rectangular Area.
    fn selective_erase_rectangular_area(&mut self, top: VTInt, left: VTInt, bottom: VTInt, right: VTInt) {
        self.unhandled("selective_erase_rectangular_area");
    }

    /// DECSACE — Select Attribute Change Extent.
    fn select_attribute_change_extent(&mut self, change_extent: ChangeExtent) {
        self.unhandled("select_attribute_change_extent");
    }

    /// DECRQCRA — Request Checksum Rectangular Area.
    fn request_checksum_rectangular_area(
        &mut self,
        id: VTInt,
        page: VTInt,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
    ) {
        self.unhandled("request_checksum_rectangular_area");
    }

    // ─── Graphics Rendition ─────────────────────────────────────────

    /// SGR — Set Graphics Rendition.
    fn set_graphics_rendition(&mut self, options: &VTParameters) {
        self.unhandled("set_graphics_rendition");
    }

    /// DECSWL/DECDWL/DECDHL — Set Line Rendition.
    fn set_line_rendition(&mut self, rendition: LineRendition) {
        self.unhandled("set_line_rendition");
    }

    /// DECSCA — Set Character Protection Attribute.
    fn set_character_protection_attribute(&mut self, options: &VTParameters) {
        self.unhandled("set_character_protection_attribute");
    }

    /// XTPUSHSGR — Push Graphics Rendition.
    fn push_graphics_rendition(&mut self, options: &VTParameters) {
        self.unhandled("push_graphics_rendition");
    }

    /// XTPOPSGR — Pop Graphics Rendition.
    fn pop_graphics_rendition(&mut self) {
        self.unhandled("pop_graphics_rendition");
    }

    // ─── Modes (continued) ──────────────────────────────────────────

    /// SM/DECSET — Set Mode.
    fn set_mode(&mut self, param: &ModeParams) {
        self.unhandled("set_mode");
    }

    /// RM/DECSET — Reset Mode.
    fn reset_mode(&mut self, param: &ModeParams) {
        self.unhandled("reset_mode");
    }

    /// DECRQM — Request Mode.
    fn request_mode(&mut self, param: &ModeParams) {
        self.unhandled("request_mode");
    }

    /// DECRQM — Request the state of a mode with no entry in the mode table.
    ///
    /// Upstream's `RequestMode` formats whatever number it was given, with
    /// state `DECRPM_Unsupported`, so a query is always answered. Splitting the
    /// unknown case out keeps `ModeParams` a closed set.
    fn request_unknown_mode(&mut self, mode: VTInt, private: bool) {
        self.unhandled("request_unknown_mode");
    }

    // ─── Device Status ──────────────────────────────────────────────

    /// DSR — Device Status Report.
    fn device_status_report(&mut self, status_type: StatusType, id: VTParameter) {
        self.unhandled("device_status_report");
    }

    /// DA1 — Device Attributes.
    fn device_attributes(&mut self) {
        self.unhandled("device_attributes");
    }

    /// DA2 — Secondary Device Attributes.
    fn secondary_device_attributes(&mut self) {
        self.unhandled("secondary_device_attributes");
    }

    /// DA3 — Tertiary Device Attributes.
    fn tertiary_device_attributes(&mut self) {
        self.unhandled("tertiary_device_attributes");
    }

    /// VT52 Identify.
    fn vt52_device_attributes(&mut self) {
        self.unhandled("vt52_device_attributes");
    }

    /// DECREQTPARM — Request Terminal Parameters.
    fn request_terminal_parameters(&mut self, permission: ReportingPermission) {
        self.unhandled("request_terminal_parameters");
    }

    // ─── Character Sets ─────────────────────────────────────────────

    /// DOCS — Designate Coding System.
    fn designate_coding_system(&mut self, coding_system: VTID) {
        self.unhandled("designate_coding_system");
    }

    /// SCS — Designate 94 Charset.
    fn designate_94_charset(&mut self, gset_number: VTInt, charset: VTID) {
        self.unhandled("designate_94_charset");
    }

    /// SCS — Designate 96 Charset.
    fn designate_96_charset(&mut self, gset_number: VTInt, charset: VTID) {
        self.unhandled("designate_96_charset");
    }

    /// LS0/LS1/LS2/LS3 — Locking Shift.
    fn locking_shift(&mut self, gset_number: VTInt) {
        self.unhandled("locking_shift");
    }

    /// LS1R/LS2R/LS3R — Locking Shift Right.
    fn locking_shift_right(&mut self, gset_number: VTInt) {
        self.unhandled("locking_shift_right");
    }

    /// SS2/SS3 — Single Shift.
    fn single_shift(&mut self, gset_number: VTInt) {
        self.unhandled("single_shift");
    }

    /// DECAC1 — Accept C1 Controls.
    fn accept_c1_controls(&mut self, enabled: bool) {
        self.unhandled("accept_c1_controls");
    }

    /// S8C1T/S7C1T — Send C1 Controls.
    fn send_c1_controls(&mut self, enabled: bool) {
        self.unhandled("send_c1_controls");
    }

    /// ACS — Announce Code Structure.
    fn announce_code_structure(&mut self, ansi_level: VTInt) {
        self.unhandled("announce_code_structure");
    }

    // ─── Resets ─────────────────────────────────────────────────────

    /// DECSTR — Soft Reset.
    fn soft_reset(&mut self) {
        self.unhandled("soft_reset");
    }

    /// RIS — Hard Reset.
    fn hard_reset(&mut self, erase: bool) {
        self.unhandled("hard_reset");
    }

    /// DECALN — Screen Alignment Pattern.
    fn screen_alignment_pattern(&mut self) {
        self.unhandled("screen_alignment_pattern");
    }

    // ─── Cursor ─────────────────────────────────────────────────────

    /// DECSCUSR — Set Cursor Style.
    fn set_cursor_style(&mut self, cursor_style: CursorStyle) {
        self.unhandled("set_cursor_style");
    }

    // ─── Clipboard ──────────────────────────────────────────────────

    /// OSC Set Clipboard.
    fn set_clipboard(&mut self, content: &str) {
        self.unhandled("set_clipboard");
    }

    // ─── Window ─────────────────────────────────────────────────────

    /// Window Manipulation.
    fn window_manipulation(
        &mut self,
        function: WindowManipulationType,
        parameter1: VTParameter,
        parameter2: VTParameter,
    ) {
        self.unhandled("window_manipulation");
    }

    // ─── Hyperlinks ─────────────────────────────────────────────────

    /// Add hyperlink.
    fn add_hyperlink(&mut self, uri: &str, params: &str) {
        self.unhandled("add_hyperlink");
    }

    /// End hyperlink.
    fn end_hyperlink(&mut self) {
        self.unhandled("end_hyperlink");
    }

    // ─── Terminal-Specific Actions ──────────────────────────────────

    /// Do ConEmu action.
    fn do_conemu_action(&mut self, string: &str) {
        self.unhandled("do_conemu_action");
    }

    /// Do iTerm2 action.
    fn do_iterm2_action(&mut self, string: &str) {
        self.unhandled("do_iterm2_action");
    }

    /// Do FinalTerm action.
    fn do_finalterm_action(&mut self, string: &str) {
        self.unhandled("do_finalterm_action");
    }

    /// Do VS Code action.
    fn do_vscode_action(&mut self, string: &str) {
        self.unhandled("do_vscode_action");
    }

    /// Do WT action.
    fn do_wt_action(&mut self, string: &str) {
        self.unhandled("do_wt_action");
    }

    /// Do urxvt action.
    fn do_urxvt_action(&mut self, string: &str) {
        self.unhandled("do_urxvt_action");
    }

    // ─── Sixel ──────────────────────────────────────────────────────

    /// Define Sixel image.
    fn define_sixel_image(
        &mut self,
        macro_parameter: VTInt,
        background_select: SixelBackground,
        background_color: VTParameter,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>> {
        self.unhandled("define_sixel_image");
        None
    }

    // ─── DRCS ───────────────────────────────────────────────────────

    /// Download DRCS.
    // Signature mirrors `ITermDispatch::DownloadDRCS` in the C++ adapter.
    #[allow(clippy::too_many_arguments)]
    fn download_drcs(
        &mut self,
        font_number: VTInt,
        start_char: VTParameter,
        erase_control: DrcsEraseControl,
        cell_matrix: DrcsCellMatrix,
        font_set: DrcsFontSet,
        font_usage: DrcsFontUsage,
        cell_height: VTParameter,
        charset_size: CharsetSize,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>> {
        self.unhandled("download_drcs");
        None
    }

    // ─── User Preference Charset ────────────────────────────────────

    /// DECRQUPSS — Request User Preference Charset.
    fn request_user_preference_charset(&mut self) {
        self.unhandled("request_user_preference_charset");
    }

    /// DECAUPSS — Assign User Preference Charset.
    fn assign_user_preference_charset(
        &mut self,
        charset_size: CharsetSize,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>> {
        self.unhandled("assign_user_preference_charset");
        None
    }

    // ─── Macros ─────────────────────────────────────────────────────

    /// DECDMAC — Define Macro.
    fn define_macro(
        &mut self,
        macro_id: VTInt,
        delete_control: MacroDeleteControl,
        encoding: MacroEncoding,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>> {
        self.unhandled("define_macro");
        None
    }

    /// DECINVM — Invoke Macro.
    fn invoke_macro(&mut self, macro_id: VTInt) {
        self.unhandled("invoke_macro");
    }

    // ─── Terminal State ─────────────────────────────────────────────

    /// DECRQTSR — Request Terminal State Report.
    fn request_terminal_state_report(&mut self, format: ReportFormat, format_option: VTParameter) {
        self.unhandled("request_terminal_state_report");
    }

    /// DECRSTS — Restore Terminal State.
    fn restore_terminal_state(&mut self, format: ReportFormat) -> Option<Box<dyn FnMut(char) -> bool + Send>> {
        self.unhandled("restore_terminal_state");
        None
    }

    /// DECRQSS — Request Setting.
    fn request_setting(&mut self) -> Option<Box<dyn FnMut(char) -> bool + Send>> {
        self.unhandled("request_setting");
        None
    }

    // ─── Presentation State ─────────────────────────────────────────

    /// DECRQPSR — Request Presentation State Report.
    fn request_presentation_state_report(&mut self, format: PresentationReportFormat) {
        self.unhandled("request_presentation_state_report");
    }

    /// DECRSPS — Restore Presentation State.
    fn restore_presentation_state(
        &mut self,
        format: PresentationReportFormat,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>> {
        self.unhandled("restore_presentation_state");
        None
    }

    // ─── Sound ──────────────────────────────────────────────────────

    /// DECPS — Play Sounds.
    fn play_sounds(&mut self, parameters: &VTParameters) {
        self.unhandled("play_sounds");
    }

    // ─── Optional Features ──────────────────────────────────────────

    /// Set optional features.
    fn set_optional_features(&mut self, features: std::collections::HashSet<OptionalFeature>) {
        self.unhandled("set_optional_features");
    }
}

/// Optional features for set_optional_features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionalFeature {
    /// Checksum report enabled.
    ChecksumReport,
    /// Clipboard write enabled.
    ClipboardWrite,
    /// Desktop notifications enabled.
    DesktopNotification,
}
