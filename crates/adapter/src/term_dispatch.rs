//! `ITermDispatch` — Output state machine callback interface.
//!
//! Ported from `ITermDispatch.hpp` in the Windows Terminal C++ codebase.
//! This is the primary interface for VT sequence dispatching.

use crate::dispatch_types::*;
pub use types::LineRendition;

/// Interface for all output state machine callbacks.
/// When VT sequences are parsed, they are dispatched to methods on this interface.
pub trait ITermDispatch: std::fmt::Debug {
    // ─── Basic Output ───────────────────────────────────────────────

    /// Called for unknown/unhandled sequences.
    fn unknown_sequence(&mut self);

    /// Prints a single printable character.
    fn print(&mut self, ch: char);

    /// Prints a string of characters.
    fn print_string(&mut self, string: &str);

    // ─── Cursor Movement ────────────────────────────────────────────

    /// CUU — Cursor Up.
    fn cursor_up(&mut self, distance: VTInt);

    /// CUD — Cursor Down.
    fn cursor_down(&mut self, distance: VTInt);

    /// CUF — Cursor Forward.
    fn cursor_forward(&mut self, distance: VTInt);

    /// CUB — Cursor Backward.
    fn cursor_backward(&mut self, distance: VTInt);

    /// CNL — Cursor Next Line.
    fn cursor_next_line(&mut self, distance: VTInt);

    /// CPL — Cursor Previous Line.
    fn cursor_prev_line(&mut self, distance: VTInt);

    /// HPA — Cursor Horizontal Position Absolute.
    fn cursor_horizontal_position_absolute(&mut self, column: VTInt);

    /// CHA — Cursor Character Absolute (same as HPA).
    fn cursor_character_absolute(&mut self, column: VTInt) {
        self.cursor_horizontal_position_absolute(column);
    }

    /// VPA — Vertical Line Position Absolute.
    fn vertical_line_position_absolute(&mut self, line: VTInt);

    /// HPR — Horizontal Position Relative.
    fn horizontal_position_relative(&mut self, distance: VTInt);

    /// VPR — Vertical Position Relative.
    fn vertical_position_relative(&mut self, distance: VTInt);

    /// CUP — Cursor Position (HVP).
    fn cursor_position(&mut self, line: VTInt, column: VTInt);

    /// DECSC — Cursor Save State.
    fn cursor_save_state(&mut self);

    /// DECRC — Cursor Restore State.
    fn cursor_restore_state(&mut self);

    // ─── Insert/Delete ──────────────────────────────────────────────

    /// ICH — Insert Character.
    fn insert_character(&mut self, count: VTInt);

    /// DCH — Delete Character.
    fn delete_character(&mut self, count: VTInt);

    /// SU — Scroll Up.
    fn scroll_up(&mut self, distance: VTInt);

    /// SD — Scroll Down.
    fn scroll_down(&mut self, distance: VTInt);

    /// NP — Next Page.
    fn next_page(&mut self, page_count: VTInt);

    /// PP — Preceding Page.
    fn preceding_page(&mut self, page_count: VTInt);

    /// PPA — Page Position Absolute.
    fn page_position_absolute(&mut self, page: VTInt);

    /// PPR — Page Position Relative.
    fn page_position_relative(&mut self, page_count: VTInt);

    /// PPB — Page Position Back.
    fn page_position_back(&mut self, page_count: VTInt);

    /// DECRQDE — Request Displayed Extent.
    fn request_displayed_extent(&mut self);

    /// IL — Insert Line.
    fn insert_line(&mut self, distance: VTInt);

    /// DL — Delete Line.
    fn delete_line(&mut self, distance: VTInt);

    /// DECIC — Insert Column.
    fn insert_column(&mut self, distance: VTInt);

    /// DECDC — Delete Column.
    fn delete_column(&mut self, distance: VTInt);

    // ─── Modes ──────────────────────────────────────────────────────

    /// DECKPAM — Set Keypad Application Mode.
    fn set_keypad_mode(&mut self, application_mode: bool);

    /// DECANM — Set ANSI Mode.
    fn set_ansi_mode(&mut self, ansi_mode: bool);

    /// KKP — Set Kitty Keyboard Protocol.
    fn set_kitty_keyboard_protocol(&mut self, flags: VTParameter, mode: VTParameter);

    /// KKP — Query Kitty Keyboard Protocol.
    fn query_kitty_keyboard_protocol(&mut self);

    /// KKP — Push Kitty Keyboard Protocol.
    fn push_kitty_keyboard_protocol(&mut self, flags: VTParameter);

    /// KKP — Pop Kitty Keyboard Protocol.
    fn pop_kitty_keyboard_protocol(&mut self, count: VTParameter);

    /// DECSTBM — Set Top/Bottom Scrolling Margins.
    fn set_top_bottom_scrolling_margins(&mut self, top_margin: VTInt, bottom_margin: VTInt);

    /// DECSLRM — Set Left/Right Scrolling Margins.
    fn set_left_right_scrolling_margins(&mut self, left_margin: VTInt, right_margin: VTInt);

    /// ENQ — Enquire/Answerback.
    fn enquire_answerback(&mut self);

    // ─── Control Actions ────────────────────────────────────────────

    /// BEL — Warning Bell.
    fn warning_bell(&mut self);

    /// CR — Carriage Return.
    fn carriage_return(&mut self);

    /// IND/NEL/LF/FF/VT — Line Feed.
    fn line_feed(&mut self, line_feed_type: LineFeedType);

    /// RI — Reverse Line Feed.
    fn reverse_line_feed(&mut self);

    /// DECBI — Back Index.
    fn back_index(&mut self);

    /// DECFI — Forward Index.
    fn forward_index(&mut self);

    /// DECSWT — Set Window Title.
    fn set_window_title(&mut self, title: &str);

    /// OSC 7 — Set Current Working Directory.
    fn set_current_working_directory(&mut self, uri: &str);

    /// HTS — Horizontal Tab Set.
    fn horizontal_tab_set(&mut self);

    /// CHT/HT — Forward Tab.
    fn forward_tab(&mut self, num_tabs: VTInt);

    /// CBT — Backwards Tab.
    fn backwards_tab(&mut self, num_tabs: VTInt);

    /// TBC — Tab Clear.
    fn tab_clear(&mut self, clear_type: TabClearType);

    /// DECST8C — Tab Set.
    fn tab_set(&mut self, set_type: TabSetType);

    // ─── Colors ─────────────────────────────────────────────────────

    /// OSC — Set Color Table Entry.
    fn set_color_table_entry(&mut self, table_index: usize, color: u32);

    /// OSC — Request Color Table Entry.
    fn request_color_table_entry(&mut self, table_index: usize);

    /// OSC — Reset Color Table.
    fn reset_color_table(&mut self);

    /// OSC — Reset Color Table Entry.
    fn reset_color_table_entry(&mut self, table_index: usize);

    /// OSC — Set Xterm Color Resource.
    fn set_xterm_color_resource(&mut self, resource: usize, color: u32);

    /// OSC — Request Xterm Color Resource.
    fn request_xterm_color_resource(&mut self, resource: usize);

    /// OSC — Reset Xterm Color Resource.
    fn reset_xterm_color_resource(&mut self, resource: usize);

    /// DECAC — Assign Color.
    fn assign_color(&mut self, item: ColorItem, fg_index: VTInt, bg_index: VTInt);

    // ─── Erase ──────────────────────────────────────────────────────

    /// ED — Erase in Display.
    fn erase_in_display(&mut self, erase_type: EraseType);

    /// EL — Erase in Line.
    fn erase_in_line(&mut self, erase_type: EraseType);

    /// ECH — Erase Characters.
    fn erase_characters(&mut self, num_chars: VTInt);

    /// DECSED — Selective Erase in Display.
    fn selective_erase_in_display(&mut self, erase_type: EraseType);

    /// DECSEL — Selective Erase in Line.
    fn selective_erase_in_line(&mut self, erase_type: EraseType);

    // ─── Rectangular Operations ─────────────────────────────────────

    /// DECCARA — Change Attributes Rectangular Area.
    fn change_attributes_rectangular_area(
        &mut self,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
        attrs: &VTParameters,
    );

    /// DECRARA — Reverse Attributes Rectangular Area.
    fn reverse_attributes_rectangular_area(
        &mut self,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
        attrs: &VTParameters,
    );

    /// DECCRA — Copy Rectangular Area.
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
    );

    /// DECFRA — Fill Rectangular Area.
    fn fill_rectangular_area(
        &mut self,
        ch: VTParameter,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
    );

    /// DECERA — Erase Rectangular Area.
    fn erase_rectangular_area(&mut self, top: VTInt, left: VTInt, bottom: VTInt, right: VTInt);

    /// DECSERA — Selective Erase Rectangular Area.
    fn selective_erase_rectangular_area(
        &mut self,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
    );

    /// DECSACE — Select Attribute Change Extent.
    fn select_attribute_change_extent(&mut self, change_extent: ChangeExtent);

    /// DECRQCRA — Request Checksum Rectangular Area.
    fn request_checksum_rectangular_area(
        &mut self,
        id: VTInt,
        page: VTInt,
        top: VTInt,
        left: VTInt,
        bottom: VTInt,
        right: VTInt,
    );

    // ─── Graphics Rendition ─────────────────────────────────────────

    /// SGR — Set Graphics Rendition.
    fn set_graphics_rendition(&mut self, options: &VTParameters);

    /// DECSWL/DECDWL/DECDHL — Set Line Rendition.
    fn set_line_rendition(&mut self, rendition: LineRendition);

    /// DECSCA — Set Character Protection Attribute.
    fn set_character_protection_attribute(&mut self, options: &VTParameters);

    /// XTPUSHSGR — Push Graphics Rendition.
    fn push_graphics_rendition(&mut self, options: &VTParameters);

    /// XTPOPSGR — Pop Graphics Rendition.
    fn pop_graphics_rendition(&mut self);

    // ─── Modes (continued) ──────────────────────────────────────────

    /// SM/DECSET — Set Mode.
    fn set_mode(&mut self, param: &ModeParams);

    /// RM/DECSET — Reset Mode.
    fn reset_mode(&mut self, param: &ModeParams);

    /// DECRQM — Request Mode.
    fn request_mode(&mut self, param: &ModeParams);

    // ─── Device Status ──────────────────────────────────────────────

    /// DSR — Device Status Report.
    fn device_status_report(&mut self, status_type: StatusType, id: VTParameter);

    /// DA1 — Device Attributes.
    fn device_attributes(&mut self);

    /// DA2 — Secondary Device Attributes.
    fn secondary_device_attributes(&mut self);

    /// DA3 — Tertiary Device Attributes.
    fn tertiary_device_attributes(&mut self);

    /// VT52 Identify.
    fn vt52_device_attributes(&mut self);

    /// DECREQTPARM — Request Terminal Parameters.
    fn request_terminal_parameters(&mut self, permission: ReportingPermission);

    // ─── Character Sets ─────────────────────────────────────────────

    /// DOCS — Designate Coding System.
    fn designate_coding_system(&mut self, coding_system: VTID);

    /// SCS — Designate 94 Charset.
    fn designate_94_charset(&mut self, gset_number: VTInt, charset: VTInt);

    /// SCS — Designate 96 Charset.
    fn designate_96_charset(&mut self, gset_number: VTInt, charset: VTInt);

    /// LS0/LS1/LS2/LS3 — Locking Shift.
    fn locking_shift(&mut self, gset_number: VTInt);

    /// LS1R/LS2R/LS3R — Locking Shift Right.
    fn locking_shift_right(&mut self, gset_number: VTInt);

    /// SS2/SS3 — Single Shift.
    fn single_shift(&mut self, gset_number: VTInt);

    /// DECAC1 — Accept C1 Controls.
    fn accept_c1_controls(&mut self, enabled: bool);

    /// S8C1T/S7C1T — Send C1 Controls.
    fn send_c1_controls(&mut self, enabled: bool);

    /// ACS — Announce Code Structure.
    fn announce_code_structure(&mut self, ansi_level: VTInt);

    // ─── Resets ─────────────────────────────────────────────────────

    /// DECSTR — Soft Reset.
    fn soft_reset(&mut self);

    /// RIS — Hard Reset.
    fn hard_reset(&mut self, erase: bool);

    /// DECALN — Screen Alignment Pattern.
    fn screen_alignment_pattern(&mut self);

    // ─── Cursor ─────────────────────────────────────────────────────

    /// DECSCUSR — Set Cursor Style.
    fn set_cursor_style(&mut self, cursor_style: CursorStyle);

    // ─── Clipboard ──────────────────────────────────────────────────

    /// OSC Set Clipboard.
    fn set_clipboard(&mut self, content: &str);

    // ─── Window ─────────────────────────────────────────────────────

    /// Window Manipulation.
    fn window_manipulation(
        &mut self,
        function: WindowManipulationType,
        parameter1: VTParameter,
        parameter2: VTParameter,
    );

    // ─── Hyperlinks ─────────────────────────────────────────────────

    /// Add hyperlink.
    fn add_hyperlink(&mut self, uri: &str, params: &str);

    /// End hyperlink.
    fn end_hyperlink(&mut self);

    // ─── Terminal-Specific Actions ──────────────────────────────────

    /// Do ConEmu action.
    fn do_conemu_action(&mut self, string: &str);

    /// Do iTerm2 action.
    fn do_iterm2_action(&mut self, string: &str);

    /// Do FinalTerm action.
    fn do_finalterm_action(&mut self, string: &str);

    /// Do VS Code action.
    fn do_vscode_action(&mut self, string: &str);

    /// Do WT action.
    fn do_wt_action(&mut self, string: &str);

    // ─── Sixel ──────────────────────────────────────────────────────

    /// Define Sixel image.
    fn define_sixel_image(
        &mut self,
        macro_parameter: VTInt,
        background_select: SixelBackground,
        background_color: VTParameter,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>>;

    // ─── DRCS ───────────────────────────────────────────────────────

    /// Download DRCS.
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
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>>;

    // ─── User Preference Charset ────────────────────────────────────

    /// DECRQUPSS — Request User Preference Charset.
    fn request_user_preference_charset(&mut self);

    /// DECAUPSS — Assign User Preference Charset.
    fn assign_user_preference_charset(
        &mut self,
        charset_size: CharsetSize,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>>;

    // ─── Macros ─────────────────────────────────────────────────────

    /// DECDMAC — Define Macro.
    fn define_macro(
        &mut self,
        macro_id: VTInt,
        delete_control: MacroDeleteControl,
        encoding: MacroEncoding,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>>;

    /// DECINVM — Invoke Macro.
    fn invoke_macro(&mut self, macro_id: VTInt);

    // ─── Terminal State ─────────────────────────────────────────────

    /// DECRQTSR — Request Terminal State Report.
    fn request_terminal_state_report(&mut self, format: ReportFormat);

    /// DECRSTS — Restore Terminal State.
    fn restore_terminal_state(
        &mut self,
        format: ReportFormat,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>>;

    /// DECRQSS — Request Setting.
    fn request_setting(&mut self) -> Option<Box<dyn FnMut(char) -> bool + Send>>;

    // ─── Presentation State ─────────────────────────────────────────

    /// DECRQPSR — Request Presentation State Report.
    fn request_presentation_state_report(&mut self, format: PresentationReportFormat);

    /// DECRSPS — Restore Presentation State.
    fn restore_presentation_state(
        &mut self,
        format: PresentationReportFormat,
    ) -> Option<Box<dyn FnMut(char) -> bool + Send>>;

    // ─── Sound ──────────────────────────────────────────────────────

    /// DECPS — Play Sounds.
    fn play_sounds(&mut self, parameters: &VTParameters);

    // ─── Optional Features ──────────────────────────────────────────

    /// Set optional features.
    fn set_optional_features(&mut self, features: std::collections::HashSet<OptionalFeature>);
}

/// Optional features for set_optional_features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionalFeature {
    /// Checksum report enabled.
    ChecksumReport,
    /// Clipboard write enabled.
    ClipboardWrite,
}
