//! `OutputStateMachineEngine` — turns parsed sequences into `ITermDispatch` calls.
//!
//! Ported from `OutputStateMachineEngine.cpp`. The identifier arms are upstream's
//! `EscActionCodes`, `CsiActionCodes`, `DcsActionCodes` and `OscActionCodes`; the
//! codes are byte strings rather than a parallel enum, so an arm reads exactly
//! like the C++ case it ports.
//!
//! Not ported yet: the terminal-specific action strings beyond the adapter's
//! existing handlers.

use adapter::term_dispatch::ITermDispatch;
use types::LineRendition;
use types::color_table::color_from_xterm_color;
use types::dispatch_types::{
    ChangeExtent, CharsetSize, ColorItem, CursorStyle, DrcsCellMatrix, DrcsEraseControl, DrcsFontSet, DrcsFontUsage,
    EraseType, LineFeedType, MacroDeleteControl, MacroEncoding, ModeParams, PresentationReportFormat, ReportFormat,
    ReportingPermission, SixelBackground, StatusType, TabClearType, TabSetType, VTID, VTInt, VTParameter, VTParameters,
    WindowManipulationType,
};
use vt::{IStateMachineEngine, StringHandler};

/// The sentinel an OSC colour query uses in place of a colour.
const COLOR_INQUIRY: u32 = 0xfeff_ffff;
/// A colour spec that failed to parse.
const INVALID_COLOR: u32 = 0xffff_ffff;
/// Upstream's cap on the URI taken from OSC 8.
const MAX_URL_LENGTH: usize = 2083;

/// The engine that maps parsed sequences onto an `ITermDispatch`.
pub struct OutputStateMachineEngine<D: ITermDispatch> {
    dispatch: D,
    last_printed_char: char,
    decoder: Utf8Decoder,
}

impl<D: ITermDispatch> OutputStateMachineEngine<D> {
    /// Creates an engine that drives `dispatch`.
    pub fn new(dispatch: D) -> Self {
        Self {
            dispatch,
            last_printed_char: '\0',
            decoder: Utf8Decoder::new(),
        }
    }

    /// Returns the dispatch.
    pub const fn dispatch(&self) -> &D {
        &self.dispatch
    }

    /// Returns the dispatch mutably.
    pub fn dispatch_mut(&mut self) -> &mut D {
        &mut self.dispatch
    }

    fn clear_last_char(&mut self) {
        self.last_printed_char = '\0';
    }

    fn emit_bytes(&mut self, bytes: &[u8]) {
        let mut chars = Vec::new();
        self.decoder.emit(bytes, |ch| chars.push(ch));
        for ch in chars {
            if ch >= ' ' {
                self.last_printed_char = ch;
            }
            self.dispatch.print(ch);
        }
    }

    /// An interrupted UTF-8 prefix has no character to finish, so it becomes
    /// one replacement character, as an invalid sequence would.
    fn flush_decoder(&mut self) {
        if self.decoder.is_empty() {
            return;
        }
        self.decoder.clear();
        self.dispatch.print('\u{FFFD}');
    }
}

impl<D: ITermDispatch> IStateMachineEngine for OutputStateMachineEngine<D> {
    fn unknown_sequence(&mut self) {
        self.flush_decoder();
        self.dispatch.unknown_sequence();
        self.clear_last_char();
    }

    fn action_execute(&mut self, ch: u8) -> bool {
        self.flush_decoder();
        match ch {
            0x05 => self.dispatch.enquire_answerback(),
            0x07 => self.dispatch.warning_bell(),
            0x08 => self.dispatch.cursor_backward(1),
            0x09 => self.dispatch.forward_tab(1),
            0x0D => self.dispatch.carriage_return(),
            0x0A..=0x0C => self.dispatch.line_feed(LineFeedType::DependsOnMode),
            0x0F => self.dispatch.locking_shift(0),
            0x0E => self.dispatch.locking_shift(1),
            // SUB prints the substitute form two glyph; DEL may be translated by
            // a designated 96-character set and is filtered by the buffer if not.
            0x1A => self.dispatch.print('\u{2426}'),
            0x7F => self.dispatch.print('\u{7F}'),
            _ => {}
        }
        self.clear_last_char();
        true
    }

    fn action_execute_from_escape(&mut self, ch: u8) -> bool {
        self.action_execute(ch)
    }

    fn action_print(&mut self, ch: u8) -> bool {
        self.emit_bytes(&[ch]);
        true
    }

    fn action_print_string(&mut self, string: &[u8]) -> bool {
        self.emit_bytes(string);
        true
    }

    fn action_esc_dispatch(&mut self, id: VTID) -> bool {
        self.flush_decoder();
        match id.as_bytes() {
            b"6" => self.dispatch.back_index(),
            b"7" => self.dispatch.cursor_save_state(),
            b"8" => self.dispatch.cursor_restore_state(),
            b"9" => self.dispatch.forward_index(),
            b"=" => self.dispatch.set_keypad_mode(true),
            b">" => self.dispatch.set_keypad_mode(false),
            b"D" => self.dispatch.line_feed(LineFeedType::WithoutReturn),
            b"E" => self.dispatch.line_feed(LineFeedType::WithReturn),
            b"H" => self.dispatch.horizontal_tab_set(),
            b"M" => self.dispatch.reverse_line_feed(),
            b"N" => self.dispatch.single_shift(2),
            b"O" => self.dispatch.single_shift(3),
            b"Z" => self.dispatch.device_attributes(),
            b"\\" => {}
            b"c" => self.dispatch.hard_reset(true),
            b"n" => self.dispatch.locking_shift(2),
            b"o" => self.dispatch.locking_shift(3),
            b"~" => self.dispatch.locking_shift_right(1),
            b"}" => self.dispatch.locking_shift_right(2),
            b"|" => self.dispatch.locking_shift_right(3),
            b" 7" => self.dispatch.accept_c1_controls(true),
            b" F" => self.dispatch.send_c1_controls(false),
            b" G" => self.dispatch.send_c1_controls(true),
            b" L" => self.dispatch.announce_code_structure(1),
            b" M" => self.dispatch.announce_code_structure(2),
            b" N" => self.dispatch.announce_code_structure(3),
            b"#3" => self.dispatch.set_line_rendition(LineRendition::DoubleHeightTop),
            b"#4" => self.dispatch.set_line_rendition(LineRendition::DoubleHeightBottom),
            b"#5" => self.dispatch.set_line_rendition(LineRendition::SingleWidth),
            b"#6" => self.dispatch.set_line_rendition(LineRendition::DoubleWidth),
            b"#8" => self.dispatch.screen_alignment_pattern(),
            _ => {
                let command = id.byte_at(0);
                let parameter = id.subsequence(1);
                match command {
                    b'%' => self.dispatch.designate_coding_system(parameter),
                    b'(' => self.dispatch.designate_94_charset(0, parameter),
                    b')' => self.dispatch.designate_94_charset(1, parameter),
                    b'*' => self.dispatch.designate_94_charset(2, parameter),
                    b'+' => self.dispatch.designate_94_charset(3, parameter),
                    b'-' => self.dispatch.designate_96_charset(1, parameter),
                    b'.' => self.dispatch.designate_96_charset(2, parameter),
                    b'/' => self.dispatch.designate_96_charset(3, parameter),
                    _ => {}
                }
            }
        }
        self.clear_last_char();
        true
    }

    fn action_vt52_esc_dispatch(&mut self, id: VTID, parameters: &VTParameters) -> bool {
        self.flush_decoder();
        match id.as_bytes() {
            b"A" => self.dispatch.cursor_up(1),
            b"B" => self.dispatch.cursor_down(1),
            b"C" => self.dispatch.cursor_forward(1),
            b"D" => self.dispatch.cursor_backward(1),
            b"F" => self.dispatch.designate_94_charset(0, VTID::from_byte(b'C')),
            b"G" => self.dispatch.designate_94_charset(0, VTID::from_byte(b'B')),
            b"H" => self.dispatch.cursor_position(1, 1),
            b"I" => self.dispatch.reverse_line_feed(),
            b"J" => self.dispatch.erase_in_display(EraseType::ToEnd),
            b"K" => self.dispatch.erase_in_line(EraseType::ToEnd),
            b"Y" => self.dispatch.cursor_position(
                parameters.at(0).value_or(0) - i32::from(b' ') + 1,
                parameters.at(1).value_or(0) - i32::from(b' ') + 1,
            ),
            b"Z" => self.dispatch.vt52_device_attributes(),
            b"=" => self.dispatch.set_keypad_mode(true),
            b">" => self.dispatch.set_keypad_mode(false),
            b"<" => self.dispatch.set_ansi_mode(true),
            _ => {}
        }
        self.clear_last_char();
        true
    }

    fn action_csi_dispatch(&mut self, id: VTID, parameters: &VTParameters) -> bool {
        self.flush_decoder();

        // Sub-parameters are only meaningful for SGR and the rectangular
        // attribute operations; a sequence carrying them elsewhere is ignored.
        if parameters.has_sub_params() && !can_accept_sub_params(&id, parameters) {
            return true;
        }

        match id.as_bytes() {
            b"@" => self.dispatch.insert_character(param_or_one(parameters.at(0))),
            b"A" => self.dispatch.cursor_up(param_or_one(parameters.at(0))),
            b"B" => self.dispatch.cursor_down(param_or_one(parameters.at(0))),
            b"C" => self.dispatch.cursor_forward(param_or_one(parameters.at(0))),
            b"D" => self.dispatch.cursor_backward(param_or_one(parameters.at(0))),
            b"E" => self.dispatch.cursor_next_line(param_or_one(parameters.at(0))),
            b"F" => self.dispatch.cursor_prev_line(param_or_one(parameters.at(0))),
            b"G" | b"`" => self
                .dispatch
                .cursor_horizontal_position_absolute(param_or_one(parameters.at(0))),
            b"H" | b"f" => {
                let line = param_or_one(parameters.at(0));
                let column = param_or_one(parameters.at(1));
                self.dispatch.cursor_position(line, column);
            }
            b"I" => self.dispatch.forward_tab(param_or_one(parameters.at(0))),
            b"J" => {
                for parameter in parameters.iter() {
                    self.dispatch.erase_in_display(erase_type(parameter));
                }
            }
            b"?J" => {
                for parameter in parameters.iter() {
                    self.dispatch.selective_erase_in_display(erase_type(parameter));
                }
            }
            b"K" => {
                for parameter in parameters.iter() {
                    self.dispatch.erase_in_line(erase_type(parameter));
                }
            }
            b"?K" => {
                for parameter in parameters.iter() {
                    self.dispatch.selective_erase_in_line(erase_type(parameter));
                }
            }
            b"L" => self.dispatch.insert_line(param_or_one(parameters.at(0))),
            b"M" => self.dispatch.delete_line(param_or_one(parameters.at(0))),
            b"P" => self.dispatch.delete_character(param_or_one(parameters.at(0))),
            b"S" => self.dispatch.scroll_up(param_or_one(parameters.at(0))),
            b"T" => self.dispatch.scroll_down(param_or_one(parameters.at(0))),
            b"U" => self.dispatch.next_page(param_or_one(parameters.at(0))),
            b"V" => self.dispatch.preceding_page(param_or_one(parameters.at(0))),
            b"?W" => {
                for parameter in parameters.iter() {
                    if parameter.value_or(0) == 5 {
                        self.dispatch.tab_set(TabSetType::SetEvery8Columns);
                    }
                }
            }
            b"X" => self.dispatch.erase_characters(param_or_one(parameters.at(0))),
            b"Z" => self.dispatch.backwards_tab(param_or_one(parameters.at(0))),
            b"a" => self
                .dispatch
                .horizontal_position_relative(param_or_one(parameters.at(0))),
            b"b" => {
                if self.last_printed_char != '\0' {
                    let count = param_or_one(parameters.at(0)).max(0) as usize;
                    let repeated = self.last_printed_char.to_string().repeat(count);
                    self.dispatch.print_string(&repeated);
                }
            }
            b"c" => {
                if parameters.at(0).value_or(0) == 0 {
                    self.dispatch.device_attributes();
                }
            }
            b">c" => {
                if parameters.at(0).value_or(0) == 0 {
                    self.dispatch.secondary_device_attributes();
                }
            }
            b"=c" => {
                if parameters.at(0).value_or(0) == 0 {
                    self.dispatch.tertiary_device_attributes();
                }
            }
            b"d" => self
                .dispatch
                .vertical_line_position_absolute(param_or_one(parameters.at(0))),
            b"e" => self.dispatch.vertical_position_relative(param_or_one(parameters.at(0))),
            b"g" => {
                for parameter in parameters.iter() {
                    if let Some(clear_type) = tab_clear_type(parameter.value_or(0)) {
                        self.dispatch.tab_clear(clear_type);
                    }
                }
            }
            b"h" => {
                for parameter in parameters.iter() {
                    if let Some(mode) = mode_param(parameter.value_or(0), false) {
                        self.dispatch.set_mode(&mode);
                    }
                }
            }
            b"?h" => {
                for parameter in parameters.iter() {
                    if let Some(mode) = mode_param(parameter.value_or(0), true) {
                        self.dispatch.set_mode(&mode);
                    }
                }
            }
            b"l" => {
                for parameter in parameters.iter() {
                    if let Some(mode) = mode_param(parameter.value_or(0), false) {
                        self.dispatch.reset_mode(&mode);
                    }
                }
            }
            b"?l" => {
                for parameter in parameters.iter() {
                    if let Some(mode) = mode_param(parameter.value_or(0), true) {
                        self.dispatch.reset_mode(&mode);
                    }
                }
            }
            b"m" => self.dispatch.set_graphics_rendition(parameters),
            b"n" => {
                if let Some(status) = status_type(parameters.at(0).value_or(0), false) {
                    self.dispatch.device_status_report(status, parameters.at(1));
                }
            }
            b"?n" => {
                if let Some(status) = status_type(parameters.at(0).value_or(0), true) {
                    self.dispatch.device_status_report(status, parameters.at(1));
                }
            }
            b"r" => self
                .dispatch
                .set_top_bottom_scrolling_margins(parameters.at(0).value_or(0), parameters.at(1).value_or(0)),
            b"s" => self
                .dispatch
                .set_left_right_scrolling_margins(parameters.at(0).value_or(0), parameters.at(1).value_or(0)),
            b"t" => self.dispatch.window_manipulation(
                window_manipulation_type(parameters.at(0).value_or(0)),
                parameters.at(1),
                parameters.at(2),
            ),
            b"u" => self.dispatch.cursor_restore_state(),
            b"x" => self
                .dispatch
                .request_terminal_parameters(reporting_permission(parameters.at(0).value_or(0))),
            b" P" => self.dispatch.page_position_absolute(param_or_one(parameters.at(0))),
            b" Q" => self.dispatch.page_position_relative(param_or_one(parameters.at(0))),
            b" R" => self.dispatch.page_position_back(param_or_one(parameters.at(0))),
            b" q" => {
                if let Some(style) = cursor_style(parameters.at(0).value_or(0)) {
                    self.dispatch.set_cursor_style(style);
                }
            }
            b"!p" => self.dispatch.soft_reset(),
            b"\"q" => self.dispatch.set_character_protection_attribute(parameters),
            b"\"v" => self.dispatch.request_displayed_extent(),
            b"#p" | b"#q" => self.dispatch.push_graphics_rendition(parameters),
            b"#{" | b"#}" => self.dispatch.pop_graphics_rendition(),
            b"$p" => {
                if let Some(mode) = mode_param(parameters.at(0).value_or(0), false) {
                    self.dispatch.request_mode(&mode);
                }
            }
            b"?$p" => {
                if let Some(mode) = mode_param(parameters.at(0).value_or(0), true) {
                    self.dispatch.request_mode(&mode);
                }
            }
            b"$r" => self.dispatch.change_attributes_rectangular_area(
                param_or_one(parameters.at(0)),
                param_or_one(parameters.at(1)),
                parameters.at(2).value_or(0),
                parameters.at(3).value_or(0),
                &parameters_from(parameters, 4),
            ),
            b"$t" => self.dispatch.reverse_attributes_rectangular_area(
                param_or_one(parameters.at(0)),
                param_or_one(parameters.at(1)),
                parameters.at(2).value_or(0),
                parameters.at(3).value_or(0),
                &parameters_from(parameters, 4),
            ),
            b"$u" => {
                if let Some(format) = report_format(parameters.at(0).value_or(0)) {
                    self.dispatch.request_terminal_state_report(format, parameters.at(1));
                }
            }
            b"$v" => self.dispatch.copy_rectangular_area(
                param_or_one(parameters.at(0)),
                param_or_one(parameters.at(1)),
                parameters.at(2).value_or(0),
                parameters.at(3).value_or(0),
                param_or_one(parameters.at(4)),
                param_or_one(parameters.at(5)),
                param_or_one(parameters.at(6)),
                param_or_one(parameters.at(7)),
            ),
            b"$w" => {
                if let Some(format) = presentation_report_format(parameters.at(0).value_or(0)) {
                    self.dispatch.request_presentation_state_report(format);
                }
            }
            b"$x" => self.dispatch.fill_rectangular_area(
                parameters.at(0),
                param_or_one(parameters.at(1)),
                param_or_one(parameters.at(2)),
                parameters.at(3).value_or(0),
                parameters.at(4).value_or(0),
            ),
            b"$z" => self.dispatch.erase_rectangular_area(
                param_or_one(parameters.at(0)),
                param_or_one(parameters.at(1)),
                parameters.at(2).value_or(0),
                parameters.at(3).value_or(0),
            ),
            b"${" => self.dispatch.selective_erase_rectangular_area(
                param_or_one(parameters.at(0)),
                param_or_one(parameters.at(1)),
                parameters.at(2).value_or(0),
                parameters.at(3).value_or(0),
            ),
            b"&u" => self.dispatch.request_user_preference_charset(),
            b"'}" => self.dispatch.insert_column(param_or_one(parameters.at(0))),
            b"'~" => self.dispatch.delete_column(param_or_one(parameters.at(0))),
            b"*x" => {
                if let Some(extent) = change_extent(parameters.at(0).value_or(0)) {
                    self.dispatch.select_attribute_change_extent(extent);
                }
            }
            b"*y" => self.dispatch.request_checksum_rectangular_area(
                parameters.at(0).value_or(0),
                parameters.at(1).value_or(0),
                param_or_one(parameters.at(2)),
                param_or_one(parameters.at(3)),
                parameters.at(4).value_or(0),
                parameters.at(5).value_or(0),
            ),
            b"*z" => self.dispatch.invoke_macro(parameters.at(0).value_or(0)),
            b",|" => {
                if let Some(item) = color_item(parameters.at(0).value_or(0)) {
                    self.dispatch
                        .assign_color(item, parameters.at(1).value_or(0), parameters.at(2).value_or(0));
                }
            }
            b",~" => self.dispatch.play_sounds(parameters),
            b"=u" => self
                .dispatch
                .set_kitty_keyboard_protocol(parameters.at(0), parameters.at(1)),
            b"?u" => self.dispatch.query_kitty_keyboard_protocol(),
            b">u" => self.dispatch.push_kitty_keyboard_protocol(parameters.at(0)),
            b"<u" => self.dispatch.pop_kitty_keyboard_protocol(parameters.at(0)),
            _ => self.dispatch.unknown_sequence(),
        }

        self.clear_last_char();
        true
    }

    fn action_dcs_dispatch(&mut self, id: VTID, parameters: &VTParameters) -> Option<StringHandler> {
        self.flush_decoder();
        let handler = match id.as_bytes() {
            b"q" => self.dispatch.define_sixel_image(
                param_or_one(parameters.at(0)),
                sixel_background(parameters.at(1).value_or(0)),
                parameters.at(2),
            ),
            b"{" => self.dispatch.download_drcs(
                param_or_one(parameters.at(0)),
                parameters.at(1),
                drcs_erase_control(parameters.at(2).value_or(0)),
                drcs_cell_matrix(parameters.at(3).value_or(0)),
                drcs_font_set(parameters.at(4).value_or(0)),
                drcs_font_usage(parameters.at(5).value_or(0)),
                parameters.at(6),
                charset_size(parameters.at(7).value_or(0)),
            ),
            b"!u" => self
                .dispatch
                .assign_user_preference_charset(charset_size(parameters.at(0).value_or(0))),
            b"!z" => self.dispatch.define_macro(
                parameters.at(0).value_or(0),
                macro_delete_control(parameters.at(1).value_or(0)),
                macro_encoding(parameters.at(2).value_or(0)),
            ),
            b"$p" => report_format(parameters.at(0).value_or(0))
                .and_then(|format| self.dispatch.restore_terminal_state(format)),
            b"$q" => self.dispatch.request_setting(),
            b"$t" => presentation_report_format(parameters.at(0).value_or(0))
                .and_then(|format| self.dispatch.restore_presentation_state(format)),
            _ => {
                self.dispatch.unknown_sequence();
                None
            }
        };

        self.clear_last_char();
        handler.map(|mut handler| -> StringHandler { Box::new(move |byte| handler(byte as char)) })
    }

    fn action_osc_dispatch(&mut self, parameter: i32, string: &[u8]) -> bool {
        self.flush_decoder();
        let text = String::from_utf8_lossy(string);

        match parameter {
            0..=2 | 21 => self.dispatch.set_window_title(&text),
            4 => {
                if let Some(entries) = parse_osc_set_color_table(&text) {
                    for (index, color) in entries {
                        if color == COLOR_INQUIRY {
                            self.dispatch.request_color_table_entry(index);
                        } else {
                            self.dispatch.set_color_table_entry(index, color);
                        }
                    }
                }
            }
            10..=12 | 17 => {
                if let Some(colors) = parse_osc_set_color(&text) {
                    let first_resource = parameter.max(0) as usize;
                    for (resource, color) in (first_resource..).zip(colors) {
                        if color == COLOR_INQUIRY {
                            self.dispatch.request_xterm_color_resource(resource);
                        } else if color != INVALID_COLOR {
                            self.dispatch.set_xterm_color_resource(resource, color);
                        }
                    }
                }
            }
            52 => {
                if let Some(Some(content)) = parse_osc_set_clipboard(&text) {
                    self.dispatch.set_clipboard(&content);
                }
            }
            104 => {
                if text.is_empty() {
                    self.dispatch.reset_color_table();
                } else {
                    for part in text.split(';') {
                        let Ok(index) = part.parse::<usize>() else {
                            break;
                        };
                        self.dispatch.reset_color_table_entry(index);
                    }
                }
            }
            110..=112 | 117 => {
                if text.is_empty() {
                    self.dispatch
                        .reset_xterm_color_resource((parameter - 100).max(0) as usize);
                }
            }
            7 => self.dispatch.set_current_working_directory(&text),
            8 => {
                if let Some((params, uri)) = parse_hyperlink(&text) {
                    if uri.is_empty() {
                        self.dispatch.end_hyperlink();
                    } else {
                        self.dispatch.add_hyperlink(&uri, &params);
                    }
                }
            }
            9 => self.dispatch.do_conemu_action(&text),
            133 => self.dispatch.do_finalterm_action(&text),
            633 => self.dispatch.do_vscode_action(&text),
            777 => self.dispatch.do_urxvt_action(&text),
            1337 => self.dispatch.do_iterm2_action(&text),
            9001 => self.dispatch.do_wt_action(&text),
            _ => self.dispatch.unknown_sequence(),
        }

        self.clear_last_char();
        true
    }

    fn action_ss3_dispatch(&mut self, _ch: u8, _parameters: &VTParameters) -> bool {
        self.flush_decoder();
        self.dispatch.unknown_sequence();
        self.clear_last_char();
        true
    }
}

/// Upstream's `VTParameter` to `VTInt` conversion: an omitted or zero value
/// means one for the distance/count parameters.
fn param_or_one(parameter: VTParameter) -> VTInt {
    let value = parameter.value_or(0);
    if value <= 0 { 1 } else { value }
}

fn parameters_from(parameters: &VTParameters, offset: usize) -> VTParameters {
    let values: Vec<VTInt> = (offset..parameters.len())
        .map(|index| parameters.at(index).value_or(0))
        .collect();
    VTParameters::from_values(values)
}

fn can_accept_sub_params(id: &VTID, parameters: &VTParameters) -> bool {
    match id.as_bytes() {
        b"m" => true,
        b"$r" | b"$t" => !(0..4usize).any(|index| parameters.has_sub_params_for(index)),
        _ => false,
    }
}

fn erase_type(parameter: VTParameter) -> EraseType {
    match parameter.value_or(0) {
        1 => EraseType::FromBeginning,
        2 => EraseType::All,
        3 => EraseType::Scrollback,
        _ => EraseType::ToEnd,
    }
}

fn mode_param(value: VTInt, private: bool) -> Option<ModeParams> {
    use ModeParams::*;
    Some(match (private, value) {
        (false, 4) => Irm,
        (false, 20) => Lnm,
        (true, 1) => DecCkm,
        (true, 2) => DecAnm,
        (true, 3) => DecColm,
        (true, 5) => DecScnm,
        (true, 6) => DecOm,
        (true, 7) => DecAwm,
        (true, 8) => DecArm,
        (true, 12) => Att610StartCursorBlink,
        (true, 25) => DecTcem,
        (true, 40) => EnableDecColmSupport,
        (true, 64) => DecPccm,
        (true, 66) => DecNkm,
        (true, 67) => DecBkm,
        (true, 69) => DecLrmm,
        (true, 80) => DecSdm,
        (true, 117) => DecEcm,
        (true, 1000) => Vt200MouseMode,
        (true, 1002) => ButtonEventMouseMode,
        (true, 1003) => AnyEventMouseMode,
        (true, 1004) => FocusEventMode,
        (true, 1005) => Utf8ExtendedMode,
        (true, 1006) => SgrExtendedMode,
        (true, 1007) => AlternateScroll,
        (true, 1049) => AlternateScreenBuffer,
        (true, 2004) => BracketedPasteMode,
        (true, 2026) => SynchronizedOutput,
        (true, 2027) => GraphemeClusterMode,
        (true, 9001) => Win32InputMode,
        _ => return None,
    })
}

fn status_type(value: VTInt, private: bool) -> Option<StatusType> {
    Some(match (private, value) {
        (false, 5) => StatusType::OperatingStatus,
        (false, 6) => StatusType::CursorPositionReport,
        (true, 6) => StatusType::ExtendedCursorPositionReport,
        (true, 15) => StatusType::PrinterStatus,
        (true, 25) => StatusType::UserDefinedKeys,
        (true, 26) => StatusType::KeyboardStatus,
        (true, 55) => StatusType::LocatorStatus,
        (true, 56) => StatusType::LocatorIdentity,
        (true, 62) => StatusType::MacroSpaceReport,
        (true, 63) => StatusType::MemoryChecksum,
        (true, 75) => StatusType::DataIntegrity,
        (true, 85) => StatusType::MultipleSessionStatus,
        _ => return None,
    })
}

fn tab_clear_type(value: VTInt) -> Option<TabClearType> {
    match value {
        0 => Some(TabClearType::ClearCurrentColumn),
        3 => Some(TabClearType::ClearAllColumns),
        _ => None,
    }
}

fn window_manipulation_type(value: VTInt) -> WindowManipulationType {
    match value {
        1 => WindowManipulationType::DeIconifyWindow,
        2 => WindowManipulationType::IconifyWindow,
        7 => WindowManipulationType::RefreshWindow,
        8 => WindowManipulationType::ResizeWindowInCharacters,
        14 => WindowManipulationType::ReportTextSizeInPixels,
        16 => WindowManipulationType::ReportCharacterCellSize,
        18 => WindowManipulationType::ReportTextSizeInCharacters,
        _ => WindowManipulationType::Invalid,
    }
}

fn cursor_style(value: VTInt) -> Option<CursorStyle> {
    Some(match value {
        0 => CursorStyle::UserDefault,
        1 => CursorStyle::BlinkingBlock,
        2 => CursorStyle::SteadyBlock,
        3 => CursorStyle::BlinkingUnderline,
        4 => CursorStyle::SteadyUnderline,
        5 => CursorStyle::BlinkingBar,
        6 => CursorStyle::SteadyBar,
        _ => return None,
    })
}

fn reporting_permission(value: VTInt) -> ReportingPermission {
    if value == 0 {
        ReportingPermission::Unsolicited
    } else {
        ReportingPermission::Solicited
    }
}

fn change_extent(value: VTInt) -> Option<ChangeExtent> {
    match value {
        0 => Some(ChangeExtent::Default),
        1 => Some(ChangeExtent::Stream),
        2 => Some(ChangeExtent::Rectangle),
        _ => None,
    }
}

fn color_item(value: VTInt) -> Option<ColorItem> {
    match value {
        1 => Some(ColorItem::NormalText),
        2 => Some(ColorItem::WindowFrame),
        _ => None,
    }
}

fn report_format(value: VTInt) -> Option<ReportFormat> {
    match value {
        1 => Some(ReportFormat::TerminalStateReport),
        2 => Some(ReportFormat::ColorTableReport),
        _ => None,
    }
}

fn presentation_report_format(value: VTInt) -> Option<PresentationReportFormat> {
    match value {
        1 => Some(PresentationReportFormat::CursorInformationReport),
        2 => Some(PresentationReportFormat::TabulationStopReport),
        _ => None,
    }
}

fn sixel_background(value: VTInt) -> SixelBackground {
    match value {
        1 => SixelBackground::Transparent,
        2 => SixelBackground::Opaque,
        _ => SixelBackground::Default,
    }
}

fn drcs_erase_control(value: VTInt) -> DrcsEraseControl {
    match value {
        1 => DrcsEraseControl::ReloadedChars,
        2 => DrcsEraseControl::AllRenditions,
        0 => DrcsEraseControl::AllChars,
        other => DrcsEraseControl::Other(other),
    }
}

fn drcs_cell_matrix(value: VTInt) -> DrcsCellMatrix {
    match value {
        1 => DrcsCellMatrix::Invalid,
        2 => DrcsCellMatrix::Size5x10,
        3 => DrcsCellMatrix::Size6x10,
        4 => DrcsCellMatrix::Size7x10,
        0 => DrcsCellMatrix::Default,
        other => DrcsCellMatrix::Other(other),
    }
}

fn drcs_font_set(value: VTInt) -> DrcsFontSet {
    match value {
        1 => DrcsFontSet::Size80x24,
        2 => DrcsFontSet::Size132x24,
        11 => DrcsFontSet::Size80x36,
        12 => DrcsFontSet::Size132x36,
        21 => DrcsFontSet::Size80x48,
        22 => DrcsFontSet::Size132x48,
        0 => DrcsFontSet::Default,
        other => DrcsFontSet::Other(other),
    }
}

fn drcs_font_usage(value: VTInt) -> DrcsFontUsage {
    match value {
        1 => DrcsFontUsage::Text,
        2 => DrcsFontUsage::FullCell,
        0 => DrcsFontUsage::Default,
        other => DrcsFontUsage::Other(other),
    }
}

fn charset_size(value: VTInt) -> CharsetSize {
    match value {
        0 => CharsetSize::Size94,
        1 => CharsetSize::Size96,
        other => CharsetSize::Other(other),
    }
}

fn macro_delete_control(value: VTInt) -> MacroDeleteControl {
    if value == 0 {
        MacroDeleteControl::DeleteId
    } else {
        MacroDeleteControl::DeleteAll
    }
}

fn macro_encoding(value: VTInt) -> MacroEncoding {
    if value == 0 {
        MacroEncoding::Text
    } else {
        MacroEncoding::HexPair
    }
}

/// Upstream's `Utils::SplitString`: an empty string yields no parts, and a
/// trailing delimiter yields a trailing empty part.
fn split_semicolons(string: &str) -> Vec<&str> {
    if string.is_empty() {
        return Vec::new();
    }

    let mut parts = Vec::new();
    let mut current = 0;
    while current < string.len() {
        match string[current..].find(';') {
            None => {
                parts.push(&string[current..]);
                break;
            }
            Some(offset) => {
                let next = current + offset;
                parts.push(&string[current..next]);
                current = next + 1;
                if current >= string.len() {
                    parts.push("");
                }
            }
        }
    }
    parts
}

fn parse_osc_set_color_table(string: &str) -> Option<Vec<(usize, u32)>> {
    let parts = split_semicolons(string);
    if parts.len() < 2 {
        return None;
    }

    let mut entries = Vec::new();
    for pair in parts.as_chunks::<2>().0 {
        let Ok(index) = pair[0].parse::<usize>() else {
            continue;
        };
        if pair[1] == "?" {
            entries.push((index, COLOR_INQUIRY));
        } else if let Some(color) = color_from_xterm_color(pair[1]) {
            entries.push((index, color));
        }
    }

    (!entries.is_empty()).then_some(entries)
}

fn parse_osc_set_color(string: &str) -> Option<Vec<u32>> {
    let parts = split_semicolons(string);
    if parts.is_empty() {
        return None;
    }

    Some(
        parts
            .into_iter()
            .map(|part| {
                if part == "?" {
                    COLOR_INQUIRY
                } else {
                    color_from_xterm_color(part).unwrap_or(INVALID_COLOR)
                }
            })
            .collect(),
    )
}

/// Returns `Some(None)` for a clipboard query, `Some(Some(content))` for a set.
fn parse_osc_set_clipboard(string: &str) -> Option<Option<String>> {
    let position = string.find(';')?;
    let payload = &string[position + 1..];
    if payload == "?" {
        return Some(None);
    }

    let bytes = vt::decode(payload).ok()?;
    Some(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Returns `(params, uri)`; `(";")` closes the hyperlink with an empty URI.
fn parse_hyperlink(string: &str) -> Option<(String, String)> {
    if string == ";" {
        return Some((String::new(), String::new()));
    }

    let position = string.find(';')?;
    let uri: String = string[position + 1..].chars().take(MAX_URL_LENGTH).collect();

    let mut params = String::new();
    for part in string[..position].split(':') {
        if let Some(id_position) = part.find("id=") {
            params = part[id_position + "id=".len()..].to_string();
        }
    }

    Some((params, uri))
}

/// An incremental UTF-8 decoder with an explicit replacement policy.
///
/// The parser hands over bytes; this is the one place they become `char`s.
/// Invalid sequences become U+FFFD, and a sequence interrupted by a control
/// character becomes one replacement character when the engine flushes.
struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    const fn new() -> Self {
        Self { pending: Vec::new() }
    }

    fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    fn clear(&mut self) {
        self.pending.clear();
    }

    fn emit(&mut self, bytes: &[u8], mut print: impl FnMut(char)) {
        for &byte in bytes {
            self.pending.push(byte);

            loop {
                if self.pending.is_empty() {
                    break;
                }

                match std::str::from_utf8(&self.pending) {
                    Ok(text) => {
                        for ch in text.chars() {
                            print(ch);
                        }
                        self.pending.clear();
                        break;
                    }
                    Err(error) => {
                        let valid = error.valid_up_to();
                        if valid > 0 {
                            let text = std::str::from_utf8(&self.pending[..valid]).expect("valid prefix");
                            for ch in text.chars() {
                                print(ch);
                            }
                        }

                        match error.error_len() {
                            Some(length) => {
                                print('\u{FFFD}');
                                self.pending.drain(..valid + length);
                            }
                            None => {
                                // Incomplete sequence; keep the tail waiting.
                                if valid > 0 {
                                    self.pending.drain(..valid);
                                }
                                // A UTF-8 character is at most four bytes, so a
                                // longer tail cannot become valid.
                                if self.pending.len() > 3 {
                                    print('\u{FFFD}');
                                    self.pending.clear();
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use adapter::AdaptDispatch;
    use vt::StateMachine;

    use super::*;

    #[derive(Debug, Default)]
    struct Recorder {
        printed: String,
        bell: usize,
        carriage_returns: usize,
        line_feeds: usize,
        saved: usize,
        restored: usize,
        cursor: (VTInt, VTInt),
        title: String,
        clipboard: Option<String>,
        link: Option<(String, String)>,
        ended_link: usize,
        sgr: Vec<VTInt>,
        charset: Option<(VTInt, Vec<u8>)>,
        assigned: Option<(ColorItem, VTInt, VTInt)>,
        csi_unknown: usize,
    }

    impl ITermDispatch for Recorder {
        fn unknown_sequence(&mut self) {
            self.csi_unknown += 1;
        }

        fn print(&mut self, ch: char) {
            self.printed.push(ch);
        }

        fn print_string(&mut self, string: &str) {
            self.printed.push_str(string);
        }

        fn warning_bell(&mut self) {
            self.bell += 1;
        }

        fn carriage_return(&mut self) {
            self.carriage_returns += 1;
        }

        fn line_feed(&mut self, _line_feed_type: LineFeedType) {
            self.line_feeds += 1;
        }

        fn cursor_save_state(&mut self) {
            self.saved += 1;
        }

        fn cursor_restore_state(&mut self) {
            self.restored += 1;
        }

        fn cursor_position(&mut self, line: VTInt, column: VTInt) {
            self.cursor = (line, column);
        }

        fn set_window_title(&mut self, title: &str) {
            self.title = title.to_string();
        }

        fn set_clipboard(&mut self, content: &str) {
            self.clipboard = Some(content.to_string());
        }

        fn add_hyperlink(&mut self, uri: &str, params: &str) {
            self.link = Some((params.to_string(), uri.to_string()));
        }

        fn end_hyperlink(&mut self) {
            self.ended_link += 1;
        }

        fn set_graphics_rendition(&mut self, options: &VTParameters) {
            self.sgr = options.iter().map(|param| param.value_or(0)).collect();
        }

        fn designate_94_charset(&mut self, gset_number: VTInt, charset: VTID) {
            self.charset = Some((gset_number, charset.as_bytes().to_vec()));
        }

        fn assign_color(&mut self, item: ColorItem, fg_index: VTInt, bg_index: VTInt) {
            self.assigned = Some((item, fg_index, bg_index));
        }
    }

    fn machine() -> StateMachine<OutputStateMachineEngine<Recorder>> {
        StateMachine::new(OutputStateMachineEngine::new(Recorder::default()))
    }

    #[test]
    fn printed_bytes_decoded_once_through_utf8() {
        let mut machine = machine();
        machine.process_string("\u{2588}ok".as_bytes());
        assert_eq!(machine.engine().dispatch().printed, "\u{2588}ok");
    }

    #[test]
    fn c0_controls_reach_the_dispatch() {
        let mut machine = machine();
        machine.process_string(b"\x07\r\n");
        let dispatch = machine.engine().dispatch();
        assert_eq!(dispatch.bell, 1);
        assert_eq!(dispatch.carriage_returns, 1);
        assert_eq!(dispatch.line_feeds, 1);
        assert!(dispatch.printed.is_empty());
    }

    #[test]
    fn csi_cursor_position_reaches_the_dispatch() {
        let mut machine = machine();
        machine.process_string(b"\x1b[3;5H");
        assert_eq!(machine.engine().dispatch().cursor, (3, 5));
    }

    #[test]
    fn csi_sgr_passes_its_parameters() {
        let mut machine = machine();
        machine.process_string(b"\x1b[1;31m");
        assert_eq!(machine.engine().dispatch().sgr, vec![1, 31]);
    }

    #[test]
    fn esc_designates_a_charset_by_its_final_byte() {
        let mut machine = machine();
        machine.process_string(b"\x1b(0");
        let (gset, charset) = machine.engine().dispatch().charset.clone().unwrap();
        assert_eq!(gset, 0);
        assert_eq!(charset, b"0".to_vec());
    }

    #[test]
    fn esc_save_and_restore_reach_the_dispatch() {
        let mut machine = machine();
        machine.process_string(b"\x1b7\x1b8");
        assert_eq!(machine.engine().dispatch().saved, 1);
        assert_eq!(machine.engine().dispatch().restored, 1);
    }

    #[test]
    fn osc_title_reaches_the_dispatch() {
        let mut machine = machine();
        machine.process_string(b"\x1b]2;hello\x07");
        assert_eq!(machine.engine().dispatch().title, "hello");
    }

    #[test]
    fn osc_hyperlink_opens_and_closes() {
        let mut machine = machine();
        machine.process_string(b"\x1b]8;id=x;http://example.com\x1b\\");
        assert_eq!(
            machine.engine().dispatch().link.clone(),
            Some(("x".to_string(), "http://example.com".to_string()))
        );

        machine.process_string(b"\x1b]8;;\x1b\\");
        assert_eq!(machine.engine().dispatch().ended_link, 1);
    }

    #[test]
    fn osc_clipboard_decodes_base64() {
        let mut machine = machine();
        machine.process_string(b"\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(machine.engine().dispatch().clipboard.as_deref(), Some("hello"));
    }

    #[test]
    fn rep_repeats_the_last_printed_character() {
        let mut machine = machine();
        machine.process_string(b"a\x1b[3b");
        assert_eq!(machine.engine().dispatch().printed, "aaaa");
    }

    #[test]
    fn decac_reaches_the_dispatch() {
        let mut machine = machine();
        machine.process_string(b"\x1b[1;5;6,|");
        assert_eq!(
            machine.engine().dispatch().assigned,
            Some((ColorItem::NormalText, 5, 6))
        );
    }

    #[test]
    fn an_unknown_csi_is_reported() {
        let mut machine = machine();
        machine.process_string(b"\x1b[1z");
        assert_eq!(machine.engine().dispatch().csi_unknown, 1);
    }

    // The engine plus the real adapter: a sixel payload reaches the buffer.
    #[test]
    fn a_sixel_payload_flows_from_the_parser_to_the_buffer() {
        let mut machine = StateMachine::new(OutputStateMachineEngine::new(AdaptDispatch::new()));
        machine.process_string(b"\x1bP0;1;0q#1;2;0;100;0~\x1b\\");

        let buffer = machine
            .engine_mut()
            .dispatch_mut()
            .page_manager_mut()
            .active_page()
            .buffer();
        let buffer = buffer.read().unwrap();
        let image = buffer
            .get_row_at_screen_line(0)
            .unwrap()
            .image_slice()
            .expect("the decoded image lands in the first row's band");
        assert_eq!(image.pixels()[0], 0xFF00_FF00);
        assert_eq!(image.cell_size(), (10, 20));
    }

    #[test]
    fn an_incomplete_utf8_sequence_is_replaced_not_dropped() {
        let mut machine = machine();
        machine.process_string(&[0xE2, 0x96]); // interrupted before the last byte
        machine.process_string(b"\x07");
        assert_eq!(machine.engine().dispatch().printed, "\u{FFFD}");
        assert_eq!(machine.engine().dispatch().bell, 1);
    }
}
