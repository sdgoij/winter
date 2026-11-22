//! `InputStateMachineEngine` — turns input sequences into console records.
//!
//! Ported from `src/terminal/parser/InputStateMachineEngine.cpp`. The engine
//! drives the same DFA as the output side, but reads the VT grammar an
//! application writes *to* a console and produces the `InputRecord`s the host's
//! [`InputBuffer`](crate::input_buffer::InputBuffer) stores: key events for
//! arrows, function keys, SS3 and generic (`~`) sequences, win32-input-mode
//! records verbatim, SGR mouse events, focus changes, and the DA1 answer.
//!
//! `VkKeyScanW` and `MapVirtualKeyW` have no portable counterpart, so the
//! character-to-key rule is the US layout in [`key_for_character`] and a
//! synthesized record carries scan code 0. See ADR 011.
//!
//! The ConPTY startup wait (`WaitUntilDA1`) needs the server's threads; the
//! flag it waits on is here.

use std::time::{Duration, Instant};

use adapter::IInteractDispatch;
use types::dispatch_types::{VTID, VTInt, VTParameter, VTParameters};
use types::input_record::mouse_button;
use types::virtual_key::*;
use types::{ControlKeyState, DeviceAttributes, InputRecord, KeyEventRecord, MouseEventFlags, MouseEventRecord};
use utils::unicode::Utf8Decoder;
use vt::{IStateMachineEngine, StringHandler};

/// The VT modifier bit for Shift, as a modifier parameter carries it.
const VT_SHIFT: VTInt = 1;
/// The VT modifier bit for Alt.
const VT_ALT: VTInt = 2;
/// The VT modifier bit for Ctrl.
const VT_CTRL: VTInt = 4;

/// The SGR mouse bit for Shift.
const SGR_SHIFT: VTInt = 4;
/// The SGR mouse bit for Alt.
const SGR_META: VTInt = 8;
/// The SGR mouse bit for Ctrl.
const SGR_CTRL: VTInt = 16;
/// The SGR mouse bit for a drag.
const SGR_DRAG: VTInt = 32;

/// The interval two clicks must fall within, matching the Windows default that
/// `GetDoubleClickTime` normally reports.
const DEFAULT_DOUBLE_CLICK_TIME: Duration = Duration::from_millis(500);

/// Turns parsed input sequences into console input records.
pub struct InputStateMachineEngine {
    dispatch: Box<dyn IInteractDispatch>,
    received_da1: bool,
    capture_next_cursor_position_report: bool,
    encountered_win32_input_mode_sequence: bool,
    expecting_string_terminator: bool,
    mouse_button_state: u32,
    double_click_time: Duration,
    last_mouse_click_pos: Option<(i16, i16)>,
    last_mouse_click_time: Option<Instant>,
    last_mouse_click_button: Option<VTInt>,
    pending_text: Utf8Decoder,
    scratch: Vec<char>,
}

impl InputStateMachineEngine {
    /// Creates an engine that reports to `dispatch`.
    pub fn new(dispatch: Box<dyn IInteractDispatch>) -> Self {
        Self::with_double_click_time(dispatch, DEFAULT_DOUBLE_CLICK_TIME)
    }

    /// Creates an engine with an explicit double-click interval.
    ///
    /// Upstream asks the system for it; a portable host supplies it.
    pub fn with_double_click_time(dispatch: Box<dyn IInteractDispatch>, double_click_time: Duration) -> Self {
        Self {
            dispatch,
            received_da1: false,
            capture_next_cursor_position_report: false,
            encountered_win32_input_mode_sequence: false,
            expecting_string_terminator: false,
            mouse_button_state: 0,
            double_click_time,
            last_mouse_click_pos: None,
            last_mouse_click_time: None,
            last_mouse_click_button: None,
            pending_text: Utf8Decoder::new(),
            scratch: Vec::new(),
        }
    }

    /// Asks for the next cursor position report to be treated as one.
    ///
    /// `CSI R` doubles as F3; ConPTY marks the window during startup so the
    /// answer to its DSR is not mistaken for a keypress.
    pub const fn capture_next_cursor_position_report(&mut self) {
        self.capture_next_cursor_position_report = true;
    }

    fn flush_text(&mut self) {
        if self.pending_text.is_empty() {
            return;
        }
        self.pending_text.clear();
        self.print_character('\u{FFFD}');
    }

    fn print_character(&self, character: char) {
        let (virtual_key, modifiers) = key_for_character(character);
        self.write_single_key(character, virtual_key, modifiers);
    }

    /// Writes a control character as a key, porting `_DoControlCharacter`.
    fn do_control_character(&mut self, character: char, write_alt: bool) -> bool {
        if character == '\u{3}' && !write_alt {
            // Ctrl+C is handled by the host itself, which is why it is not an
            // ordinary key record.
            let modifiers = ControlKeyState {
                left_ctrl: true,
                ..ControlKeyState::default()
            };
            let down = key_event(true, 1, 'C' as u16, 0, '\u{3}', modifiers);
            let up = key_event(false, 1, 'C' as u16, 0, '\u{3}', modifiers);
            self.dispatch.write_ctrl_key(&down);
            self.dispatch.write_ctrl_key(&up);
            return true;
        }

        if ('\0'..'\u{20}').contains(&character) {
            let mut actual = character;
            let mut write_ctrl = true;

            let generated = match character {
                '\u{8}' => {
                    // Ctrl+Backspace deletes a word; the host receives DEL.
                    actual = '\u{7f}';
                    Some((key_for_character(actual).0, ControlKeyState::default()))
                }
                '\r' => {
                    write_ctrl = false;
                    Some(key_for_character(character))
                }
                '\u{1b}' => {
                    // Escape is the ESC key, not Ctrl+[.
                    write_ctrl = false;
                    Some((VK_ESCAPE, ControlKeyState::default()))
                }
                '\t' => {
                    write_ctrl = false;
                    Some(key_for_character(actual))
                }
                _ => Some(key_for_character(actual)),
            };

            if let Some((virtual_key, mut modifiers)) = generated {
                if write_ctrl {
                    modifiers.left_ctrl = true;
                }
                if write_alt {
                    modifiers.left_alt = true;
                }
                self.write_single_key(actual, virtual_key, modifiers);
            }
        } else if character == '\u{7f}' {
            // Telnet historically sent DEL where a terminal sends backspace;
            // the host follows the terminal.
            let modifiers = ControlKeyState {
                left_alt: write_alt,
                ..ControlKeyState::default()
            };
            self.write_single_key('\u{8}', VK_BACK, modifiers);
        } else {
            self.print_character(character);
        }

        true
    }

    /// Writes a key with its modifier presses, porting `_GenerateWrappedSequence`.
    fn write_single_key(&self, character: char, virtual_key: u16, modifiers: ControlKeyState) {
        let shift = modifiers.shift;
        let ctrl = modifiers.left_ctrl;
        let alt = modifiers.left_alt;
        let mut records = Vec::with_capacity(8);
        let mut current = ControlKeyState::default();

        if shift {
            current.shift = true;
            records.push(key_event(true, 1, VK_SHIFT, 0, '\0', current));
        }
        if alt {
            current.left_alt = true;
            records.push(key_event(true, 1, VK_MENU, 0, '\0', current));
        }
        if ctrl {
            current.left_ctrl = true;
            records.push(key_event(true, 1, VK_CONTROL, 0, '\0', current));
        }

        // The key itself carries the full modifier state, not `current`, so
        // flags such as ENHANCED_KEY reach the record.
        records.push(key_event(true, 1, virtual_key, 0, character, modifiers));
        records.push(key_event(false, 1, virtual_key, 0, character, modifiers));

        if ctrl {
            current.left_ctrl = false;
            records.push(key_event(false, 1, VK_CONTROL, 0, '\0', current));
        }
        if alt {
            current.left_alt = false;
            records.push(key_event(false, 1, VK_MENU, 0, '\0', current));
        }
        if shift {
            current.shift = false;
            records.push(key_event(false, 1, VK_SHIFT, 0, '\0', current));
        }

        self.dispatch.write_input(&records);
    }

    /// Writes a key whose character is implied by the virtual key.
    fn write_single_key_by_vkey(&self, virtual_key: u16, modifiers: ControlKeyState) {
        // Of the keys this engine synthesizes by vkey, only Tab has a
        // character; upstream asks `MapVirtualKey`, which has no portable form.
        let character = if virtual_key == VK_TAB { '\t' } else { '\0' };
        self.write_single_key(character, virtual_key, modifiers);
    }

    fn write_mouse_event(
        &self,
        position: (i16, i16),
        button_state: u32,
        control_keys: ControlKeyState,
        event_flags: MouseEventFlags,
    ) {
        let record = InputRecord::Mouse(MouseEventRecord {
            position,
            button_state,
            control_keys,
            event_flags,
        });
        self.dispatch.write_input(&[record]);
    }

    /// Updates the button state from an SGR encoding, porting
    /// `_UpdateSGRMouseButtonState`.
    fn update_sgr_mouse_button_state(
        &mut self,
        mouse_down: bool,
        encoding: VTInt,
        position: (i16, i16),
    ) -> Option<(u32, MouseEventFlags)> {
        let mut button_state = self.mouse_button_state;
        let mut event_flags = MouseEventFlags::NONE;

        // The first parameter packs the button in its two low and two high
        // bits: `BBDM'MMBB`.
        let button = (encoding & 0x3) | ((encoding & 0xC0) >> 4);

        let mut button_flag = 0;
        match button {
            0 => button_flag = mouse_button::FROM_LEFT_1ST_BUTTON_PRESSED,
            1 => button_flag = mouse_button::FROM_LEFT_2ND_BUTTON_PRESSED,
            2 => button_flag = mouse_button::RIGHTMOST_BUTTON_PRESSED,
            4 => {
                button_state |= mouse_button::SCROLL_DELTA_FORWARD;
                event_flags = event_flags.union(MouseEventFlags::WHEELED);
            }
            5 => {
                button_state |= mouse_button::SCROLL_DELTA_BACKWARD;
                event_flags = event_flags.union(MouseEventFlags::WHEELED);
            }
            6 => {
                button_state |= mouse_button::SCROLL_DELTA_BACKWARD;
                event_flags = event_flags.union(MouseEventFlags::HORIZONTAL_WHEELED);
            }
            7 => {
                button_state |= mouse_button::SCROLL_DELTA_FORWARD;
                event_flags = event_flags.union(MouseEventFlags::HORIZONTAL_WHEELED);
            }
            3 => {}
            _ => return None,
        }

        let now = Instant::now();
        if mouse_down {
            button_state |= button_flag;

            let double_click = self.last_mouse_click_pos == Some(position)
                && self
                    .last_mouse_click_time
                    .is_some_and(|time| now.duration_since(time) < self.double_click_time)
                && self.last_mouse_click_button == Some(button);

            if double_click {
                event_flags = event_flags.union(MouseEventFlags::DOUBLE_CLICK);
                // Clearing the trackers keeps a third click from repeating it.
                self.last_mouse_click_pos = None;
                self.last_mouse_click_time = None;
                self.last_mouse_click_button = None;
            } else if matches!(button, 0..=2) {
                self.last_mouse_click_pos = Some(position);
                self.last_mouse_click_time = Some(now);
                self.last_mouse_click_button = Some(button);
            }
        } else {
            button_state &= !button_flag;
        }

        if encoding & SGR_DRAG != 0 {
            event_flags = event_flags.union(MouseEventFlags::MOVED);
        }

        // Only the low word is button state; the high word holds the wheel
        // delta, and a wheel button's release is never reported.
        self.mouse_button_state = button_state & 0xffff;

        Some((button_state, event_flags))
    }

    fn modifier_state(modifier: VTParameter) -> ControlKeyState {
        // VT modifiers are 1 plus the flag bits, and an omitted parameter is
        // read as 1, so the subtraction is always defined.
        let flags = modifier.value_or(1) - 1;
        ControlKeyState {
            shift: flags & VT_SHIFT != 0,
            left_alt: flags & VT_ALT != 0,
            left_ctrl: flags & VT_CTRL != 0,
            ..ControlKeyState::default()
        }
    }

    fn sgr_mouse_modifier_state(modifier: VTInt) -> ControlKeyState {
        ControlKeyState {
            shift: modifier & SGR_SHIFT != 0,
            left_alt: modifier & SGR_META != 0,
            left_ctrl: modifier & SGR_CTRL != 0,
            ..ControlKeyState::default()
        }
    }

    fn cursor_keys_modifier_state(parameters: &VTParameters, enhanced: bool) -> ControlKeyState {
        let mut modifiers = Self::modifier_state(parameters.at(1));
        if enhanced {
            modifiers.enhanced = true;
        }
        modifiers
    }

    fn generic_keys_modifier_state(parameters: &VTParameters) -> ControlKeyState {
        let mut modifiers = Self::modifier_state(parameters.at(1));
        // Home through Next are in the navigation block; F5 and up are not.
        let identifier = parameters.at(0).value_or(0);
        if (1..=6).contains(&identifier) {
            modifiers.enhanced = true;
        }
        modifiers
    }

    fn generate_win32_key(parameters: &VTParameters) -> InputRecord {
        // `CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`, every field optional.
        let key_down = parameters.at(3).value_or(0) != 0;
        let repeat_count = parameters.at(5).value_or(1).clamp(0, u16::MAX as VTInt) as u16;
        let virtual_key = parameters.at(0).value_or(0).clamp(0, u16::MAX as VTInt) as u16;
        let scan_code = parameters.at(1).value_or(0).clamp(0, u16::MAX as VTInt) as u16;
        // A `char` cannot hold a lone surrogate, so a malformed code becomes
        // NUL rather than a record upstream could build from UTF-16.
        let character = u32::try_from(parameters.at(2).value_or(0))
            .ok()
            .and_then(char::from_u32)
            .unwrap_or('\0');
        let control_keys = ControlKeyState::from_console_flags(parameters.at(4).value_or(0) as u16);

        key_event(key_down, repeat_count, virtual_key, scan_code, character, control_keys)
    }
}

impl IStateMachineEngine for InputStateMachineEngine {
    const IS_INPUT: bool = true;

    fn encountered_win32_input_mode_sequence(&self) -> bool {
        self.encountered_win32_input_mode_sequence
    }

    fn action_execute(&mut self, character: u8) -> bool {
        self.flush_text();
        self.do_control_character(character as char, false)
    }

    fn action_execute_from_escape(&mut self, character: u8) -> bool {
        self.flush_text();
        if self.dispatch.is_vt_input_enabled() {
            return false;
        }
        self.do_control_character(character as char, true)
    }

    fn action_print(&mut self, character: u8) -> bool {
        self.scratch.clear();
        let scratch = &mut self.scratch;
        self.pending_text.emit(&[character], |decoded| scratch.push(decoded));
        for &decoded in &self.scratch {
            self.print_character(decoded);
        }
        true
    }

    fn action_esc_dispatch(&mut self, id: VTID) -> bool {
        self.flush_text();

        // The string terminator of a DCS the engine ignored; returning false
        // asks the parser to flush the buffered string.
        if self.expecting_string_terminator && id.as_bytes() == b"\\" {
            self.expecting_string_terminator = false;
            return false;
        }

        if self.dispatch.is_vt_input_enabled() {
            return false;
        }

        let character = id.as_bytes().last().copied().unwrap_or(0) as char;
        if character == '\u{7f}' {
            self.do_control_character(character, true);
        } else {
            let (virtual_key, mut modifiers) = key_for_character(character);
            // ESC+key is always Alt+key.
            modifiers.left_alt = true;
            self.write_single_key(character, virtual_key, modifiers);
        }

        true
    }

    fn action_vt52_esc_dispatch(&mut self, _id: VTID, _parameters: &VTParameters) -> bool {
        // The input grammar has no VT52 sequences.
        false
    }

    fn action_csi_dispatch(&mut self, id: VTID, parameters: &VTParameters) -> bool {
        self.flush_text();

        let vt_input_enabled = self.dispatch.is_vt_input_enabled();

        if id.as_bytes() == b"<M" || id.as_bytes() == b"<m" {
            if vt_input_enabled {
                return false;
            }

            let mouse_down = id.as_bytes() == b"<M";
            let encoding = parameters.at(0).value_or(0);
            let position = (
                (parameters.at(1).value_or(1) - 1).clamp(0, i16::MAX as VTInt) as i16,
                (parameters.at(2).value_or(1) - 1).clamp(0, i16::MAX as VTInt) as i16,
            );

            if let Some((button_state, event_flags)) =
                self.update_sgr_mouse_button_state(mouse_down, encoding, position)
            {
                let modifiers = Self::sgr_mouse_modifier_state(encoding);
                self.write_mouse_event(position, button_state, modifiers, event_flags);
            }
            return true;
        }

        // `R` is both F3 and the cursor position report. A report is only
        // expected after an explicit request, and win32-input-mode makes an
        // unsolicited `R` more likely to be a report than a key.
        if id.as_bytes() == b"R" {
            if self.capture_next_cursor_position_report {
                self.capture_next_cursor_position_report = false;
                self.dispatch
                    .move_cursor(parameters.at(0).value_or(1), parameters.at(1).value_or(1));
                return true;
            }
            if self.encountered_win32_input_mode_sequence || vt_input_enabled {
                return false;
            }

            self.write_single_key_by_vkey(VK_F3, Self::cursor_keys_modifier_state(parameters, false));
            return true;
        }

        match id.as_bytes() {
            b"A" | b"B" | b"C" | b"D" | b"H" | b"F" | b"P" | b"Q" | b"S" => {
                if vt_input_enabled {
                    return false;
                }
                // F1 through F4 are not in the navigation block; the rest are.
                let enhanced = !matches!(id.as_bytes(), b"P" | b"Q" | b"S");
                let Some(virtual_key) = cursor_key_vkey(id.as_bytes()) else {
                    return false;
                };
                let modifiers = Self::cursor_keys_modifier_state(parameters, enhanced);
                self.write_single_key_by_vkey(virtual_key, modifiers);
                true
            }
            b"~" => {
                if vt_input_enabled {
                    return false;
                }
                let Some(virtual_key) = generic_key_vkey(parameters.at(0).value_or(0)) else {
                    return false;
                };
                let modifiers = Self::generic_keys_modifier_state(parameters);
                self.write_single_key_by_vkey(virtual_key, modifiers);
                true
            }
            b"Z" => {
                if vt_input_enabled {
                    return false;
                }
                let modifiers = ControlKeyState {
                    shift: true,
                    ..ControlKeyState::default()
                };
                self.write_single_key_by_vkey(VK_TAB, modifiers);
                true
            }
            b"I" => {
                self.dispatch.focus_changed(true);
                true
            }
            b"O" => {
                self.dispatch.focus_changed(false);
                true
            }
            b"?c" => {
                if self.received_da1 {
                    return false;
                }

                let mut attributes = DeviceAttributes::new();
                let count = parameters.len();
                if count >= 2 && parameters.at(0).value_or(0) >= 61 {
                    for index in 1..count {
                        let value = parameters.at(index).value_or(0);
                        if (1..64).contains(&value) {
                            attributes.set(value as u64);
                        }
                    }
                }

                self.dispatch.set_device_attributes(attributes);
                self.received_da1 = true;
                true
            }
            b"_" => {
                // Win32 input mode is written as a control key so the host can
                // handle Ctrl+C and Ctrl+Break specially.
                let key = Self::generate_win32_key(parameters);
                self.dispatch.write_ctrl_key(&key);
                self.encountered_win32_input_mode_sequence = true;
                true
            }
            _ => false,
        }
    }

    fn action_dcs_dispatch(&mut self, _id: VTID, _parameters: &VTParameters) -> Option<StringHandler> {
        self.flush_text();
        // Ignoring the string still makes the parser buffer it, so the engine
        // waits for the terminator before returning to normal parsing.
        self.expecting_string_terminator = true;
        None
    }

    fn action_osc_dispatch(&mut self, _parameter: i32, _string: &[u8]) -> bool {
        self.flush_text();
        // OSC on the input stream is only ever a response to a query, so it is
        // passed through regardless of VT input mode.
        false
    }

    fn action_ss3_dispatch(&mut self, character: u8, _parameters: &VTParameters) -> bool {
        self.flush_text();
        if self.dispatch.is_vt_input_enabled() {
            return false;
        }

        if let Some(virtual_key) = ss3_key_vkey(character) {
            self.write_single_key_by_vkey(virtual_key, ControlKeyState::default());
        }

        true
    }
}

fn key_event(
    key_down: bool,
    repeat_count: u16,
    virtual_key: u16,
    scan_code: u16,
    character: char,
    control_keys: ControlKeyState,
) -> InputRecord {
    InputRecord::Key(KeyEventRecord {
        key_down,
        repeat_count,
        virtual_key,
        scan_code,
        character,
        control_keys,
    })
}

fn cursor_key_vkey(id: &[u8]) -> Option<u16> {
    Some(match id {
        b"A" => VK_UP,
        b"B" => VK_DOWN,
        b"C" => VK_RIGHT,
        b"D" => VK_LEFT,
        b"H" => VK_HOME,
        b"F" => VK_END,
        b"P" => VK_F1,
        b"Q" => VK_F2,
        b"S" => VK_F4,
        _ => return None,
    })
}

fn generic_key_vkey(identifier: VTInt) -> Option<u16> {
    Some(match identifier {
        1 => VK_HOME,
        2 => VK_INSERT,
        3 => VK_DELETE,
        4 => VK_END,
        5 => VK_PRIOR,
        6 => VK_NEXT,
        15 => VK_F5,
        17 => VK_F6,
        18 => VK_F7,
        19 => VK_F8,
        20 => VK_F9,
        21 => VK_F10,
        23 => VK_F11,
        24 => VK_F12,
        _ => return None,
    })
}

fn ss3_key_vkey(character: u8) -> Option<u16> {
    Some(match character {
        b'A' => VK_UP,
        b'B' => VK_DOWN,
        b'C' => VK_RIGHT,
        b'D' => VK_LEFT,
        b'H' => VK_HOME,
        b'F' => VK_END,
        b'P' => VK_F1,
        b'Q' => VK_F2,
        b'R' => VK_F3,
        b'S' => VK_F4,
        _ => return None,
    })
}

/// Maps a character to the virtual key and modifiers a US keyboard reports.
///
/// Upstream asks the active layout through `VkKeyScanW`. A portable host has no
/// layout, so this is the fixed default; a character the table does not name
/// keeps its character with virtual key 0 rather than being dropped. See
/// ADR 011.
fn key_for_character(character: char) -> (u16, ControlKeyState) {
    let shifted = ControlKeyState {
        shift: true,
        ..ControlKeyState::default()
    };
    let controlled = ControlKeyState {
        left_ctrl: true,
        ..ControlKeyState::default()
    };

    let mapped = match character {
        '\r' => Some((VK_RETURN, ControlKeyState::default())),
        '\t' => Some((VK_TAB, ControlKeyState::default())),
        '\u{8}' => Some((VK_BACK, ControlKeyState::default())),
        '\u{1b}' => Some((VK_ESCAPE, ControlKeyState::default())),
        // Ctrl+A through Ctrl+Z, which is how the control codes are typed.
        '\u{1}'..='\u{1a}' => Some((u16::from(b'A') + (character as u16 - 1), controlled)),
        ' ' => Some((0x20, ControlKeyState::default())),
        '0'..='9' => Some((0x30 + (character as u16 - u16::from(b'0')), ControlKeyState::default())),
        'a'..='z' => Some((
            u16::from(b'A') + (character as u16 - u16::from(b'a')),
            ControlKeyState::default(),
        )),
        'A'..='Z' => Some((u16::from(b'A') + (character as u16 - u16::from(b'A')), shifted)),
        ';' => Some((0xBA, ControlKeyState::default())),
        ':' => Some((0xBA, shifted)),
        '=' => Some((0xBB, ControlKeyState::default())),
        '+' => Some((0xBB, shifted)),
        ',' => Some((0xBC, ControlKeyState::default())),
        '<' => Some((0xBC, shifted)),
        '-' => Some((0xBD, ControlKeyState::default())),
        '_' => Some((0xBD, shifted)),
        '.' => Some((0xBE, ControlKeyState::default())),
        '>' => Some((0xBE, shifted)),
        '/' => Some((0xBF, ControlKeyState::default())),
        '?' => Some((0xBF, shifted)),
        '`' => Some((0xC0, ControlKeyState::default())),
        '~' => Some((0xC0, shifted)),
        '[' => Some((0xDB, ControlKeyState::default())),
        '{' => Some((0xDB, shifted)),
        '\\' => Some((0xDC, ControlKeyState::default())),
        '|' => Some((0xDC, shifted)),
        ']' => Some((0xDD, ControlKeyState::default())),
        '}' => Some((0xDD, shifted)),
        '\'' => Some((0xDE, ControlKeyState::default())),
        '"' => Some((0xDE, shifted)),
        '!' => Some((0x31, shifted)),
        '@' => Some((0x32, shifted)),
        '#' => Some((0x33, shifted)),
        '$' => Some((0x34, shifted)),
        '%' => Some((0x35, shifted)),
        '^' => Some((0x36, shifted)),
        '&' => Some((0x37, shifted)),
        '*' => Some((0x38, shifted)),
        '(' => Some((0x39, shifted)),
        ')' => Some((0x30, shifted)),
        _ => None,
    };

    mapped.unwrap_or((0, ControlKeyState::default()))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use types::dispatch_types::WindowManipulationType;
    use vt::StateMachine;

    use super::*;

    #[derive(Debug, Default)]
    struct Capture {
        inputs: RefCell<Vec<InputRecord>>,
        ctrl_keys: RefCell<Vec<InputRecord>>,
        cursor: RefCell<Vec<(VTInt, VTInt)>>,
        focus: RefCell<Vec<bool>>,
        attributes: RefCell<Vec<DeviceAttributes>>,
    }

    impl Capture {
        fn keys(&self) -> Vec<KeyEventRecord> {
            self.inputs
                .borrow()
                .iter()
                .filter_map(|record| record.key().copied())
                .collect()
        }

        fn mouse(&self) -> Vec<MouseEventRecord> {
            self.inputs
                .borrow()
                .iter()
                .filter_map(|record| match record {
                    InputRecord::Mouse(record) => Some(*record),
                    _ => None,
                })
                .collect()
        }
    }

    struct Recorder {
        capture: Rc<Capture>,
        vt_input: bool,
    }

    impl std::fmt::Debug for Recorder {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("Recorder")
        }
    }

    impl IInteractDispatch for Recorder {
        fn is_vt_input_enabled(&self) -> bool {
            self.vt_input
        }

        fn write_input(&self, input_events: &[InputRecord]) {
            self.capture.inputs.borrow_mut().extend_from_slice(input_events);
        }

        fn write_ctrl_key(&self, event: &InputRecord) {
            self.capture.ctrl_keys.borrow_mut().push(*event);
        }

        fn write_string(&self, _string: &str) {}

        fn write_string_raw(&self, _string: &str) {}

        fn window_manipulation(&self, _function: WindowManipulationType, _parameter1: VTInt, _parameter2: VTInt) {}

        fn move_cursor(&self, row: VTInt, col: VTInt) {
            self.capture.cursor.borrow_mut().push((row, col));
        }

        fn focus_changed(&self, focused: bool) {
            self.capture.focus.borrow_mut().push(focused);
        }

        fn set_device_attributes(&self, attributes: DeviceAttributes) {
            self.capture.attributes.borrow_mut().push(attributes);
        }
    }

    fn machine(vt_input: bool) -> (Rc<Capture>, StateMachine<InputStateMachineEngine>) {
        let capture = Rc::new(Capture::default());
        let engine = InputStateMachineEngine::new(Box::new(Recorder {
            capture: capture.clone(),
            vt_input,
        }));
        (capture, StateMachine::new(engine))
    }

    #[test]
    fn csi_arrow_up_writes_a_key_down_and_up() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[A");

        let keys = capture.keys();
        assert_eq!(keys.len(), 2);
        assert!(keys[0].key_down);
        assert!(!keys[1].key_down);
        assert_eq!(keys[0].virtual_key, VK_UP);
        assert!(keys[0].control_keys.enhanced);
    }

    #[test]
    fn a_modified_arrow_wraps_the_modifier() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[1;2A");

        let keys = capture.keys();
        assert_eq!(keys.len(), 4);
        assert_eq!(keys[0].virtual_key, VK_SHIFT);
        assert!(keys[0].key_down);
        assert_eq!(keys[1].virtual_key, VK_UP);
        assert!(keys[1].control_keys.shift);
        assert!(!keys[3].key_down);
    }

    #[test]
    fn generic_keys_map_and_set_enhanced() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[3~\x1b[15~");

        let keys = capture.keys();
        assert_eq!(keys[0].virtual_key, VK_DELETE);
        assert!(keys[0].control_keys.enhanced);
        assert_eq!(keys[2].virtual_key, VK_F5);
        assert!(!keys[2].control_keys.enhanced);
    }

    #[test]
    fn an_unknown_generic_key_is_ignored() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[99~");
        assert!(capture.keys().is_empty());
    }

    #[test]
    fn ss3_keys_map() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1bOP");

        let keys = capture.keys();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].virtual_key, VK_F1);
    }

    #[test]
    fn shift_tab_is_a_tab_with_shift() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[Z");

        let keys = capture.keys();
        assert_eq!(keys[1].virtual_key, VK_TAB);
        assert_eq!(keys[1].character, '\t');
        assert!(keys[1].control_keys.shift);
    }

    #[test]
    fn home_and_end_are_enhanced() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[H\x1b[F");

        let keys = capture.keys();
        assert_eq!(keys[0].virtual_key, VK_HOME);
        assert!(keys[0].control_keys.enhanced);
        assert_eq!(keys[2].virtual_key, VK_END);
        assert!(keys[2].control_keys.enhanced);
    }

    #[test]
    fn c0_controls_become_keys() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\t");

        let keys = capture.keys();
        assert_eq!(keys[0].virtual_key, VK_TAB);
        assert_eq!(keys[0].character, '\t');
        assert!(!keys[0].control_keys.left_ctrl);
    }

    #[test]
    fn ctrl_c_uses_the_control_key_path() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x03");

        assert!(capture.keys().is_empty());
        let ctrl_keys = capture.ctrl_keys.borrow();
        assert_eq!(ctrl_keys.len(), 2);
        let key = ctrl_keys[0].key().copied().expect("a key record");
        assert!(key.key_down);
        assert_eq!(key.character, '\u{3}');
        assert!(key.control_keys.left_ctrl);
    }

    #[test]
    fn del_becomes_backspace() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x7f");

        let keys = capture.keys();
        assert_eq!(keys[0].virtual_key, VK_BACK);
        assert_eq!(keys[0].character, '\u{8}');
    }

    #[test]
    fn escape_and_a_character_is_alt() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1bx");

        let keys = capture.keys();
        let key = keys
            .iter()
            .find(|key| key.virtual_key != VK_MENU)
            .expect("the key itself");
        assert_eq!(key.virtual_key, u16::from(b'X'));
        assert_eq!(key.character, 'x');
        assert!(key.control_keys.left_alt);
    }

    #[test]
    fn printable_text_becomes_keys() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"a");

        let keys = capture.keys();
        assert_eq!(keys[0].virtual_key, u16::from(b'A'));
        assert_eq!(keys[0].character, 'a');
    }

    #[test]
    fn utf8_text_is_decoded_once() {
        let (capture, mut machine) = machine(false);
        machine.process_string("é".as_bytes());

        let keys = capture.keys();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].character, 'é');
        assert_eq!(keys[0].virtual_key, 0);
    }

    #[test]
    fn focus_events_are_reported() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[I\x1b[O");
        assert_eq!(*capture.focus.borrow(), [true, false]);
    }

    #[test]
    fn sgr_mouse_press_and_release_track_the_button() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[<0;3;4M");
        machine.process_string(b"\x1b[<0;3;4m");

        let mouse = capture.mouse();
        assert_eq!(mouse[0].position, (2, 3));
        assert_eq!(
            mouse[0].button_state & 0xffff,
            mouse_button::FROM_LEFT_1ST_BUTTON_PRESSED
        );
        assert_eq!(mouse[1].button_state & 0xffff, 0);
    }

    #[test]
    fn sgr_mouse_drag_sets_moved() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[<32;3;4M");

        let mouse = capture.mouse();
        assert!(mouse[0].event_flags.contains(MouseEventFlags::MOVED));
        assert_eq!(mouse[0].button_state, mouse_button::FROM_LEFT_1ST_BUTTON_PRESSED);
    }

    #[test]
    fn a_double_click_sets_the_flag() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[<0;3;4M");
        machine.process_string(b"\x1b[<0;3;4M");

        let mouse = capture.mouse();
        assert!(!mouse[0].event_flags.contains(MouseEventFlags::DOUBLE_CLICK));
        assert!(mouse[1].event_flags.contains(MouseEventFlags::DOUBLE_CLICK));
    }

    #[test]
    fn the_wheel_delta_is_carried_in_the_high_word() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[<64;3;4M\x1b[<65;3;4M");

        let mouse = capture.mouse();
        assert!(mouse[0].event_flags.contains(MouseEventFlags::WHEELED));
        assert_eq!(mouse[0].button_state, mouse_button::SCROLL_DELTA_FORWARD);
        assert_eq!(mouse[1].button_state, mouse_button::SCROLL_DELTA_BACKWARD);
    }

    #[test]
    fn win32_input_mode_is_copied_verbatim() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[65;30;97;1;8;2_");

        let ctrl_keys = capture.ctrl_keys.borrow();
        assert_eq!(ctrl_keys.len(), 1);
        let key = ctrl_keys[0].key().copied().expect("a key record");
        assert_eq!(key.virtual_key, 65);
        assert_eq!(key.scan_code, 30);
        assert_eq!(key.character, 'a');
        assert_eq!(key.repeat_count, 2);
        assert!(key.control_keys.left_ctrl);
        assert!(machine.engine().encountered_win32_input_mode_sequence());
    }

    #[test]
    fn da1_attributes_are_reported_once() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[?61;22c");
        machine.process_string(b"\x1b[?61;22c");

        let attributes = capture.attributes.borrow();
        assert_eq!(attributes.len(), 1);
        assert!(attributes[0].contains(DeviceAttributes::COLOR));
    }

    #[test]
    fn a_captured_cursor_report_moves_the_cursor() {
        let (capture, mut machine) = machine(false);
        machine.engine_mut().capture_next_cursor_position_report();
        machine.process_string(b"\x1b[5;7R");

        assert_eq!(*capture.cursor.borrow(), [(5, 7)]);
        assert!(capture.keys().is_empty());
    }

    #[test]
    fn an_uncaptured_report_is_f3() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[1;5R");

        let keys = capture.keys();
        assert_eq!(keys[1].virtual_key, VK_F3);
        assert!(keys[1].control_keys.left_ctrl);
    }

    #[test]
    fn win32_mode_makes_an_unsolicited_report_a_passthrough() {
        let (capture, mut machine) = machine(false);
        machine.process_string(b"\x1b[65;30;97;1;0;1_");
        machine.process_string(b"\x1b[R");

        assert!(capture.keys().is_empty());
    }

    #[test]
    fn vt_input_mode_passes_keys_through() {
        let (capture, mut machine) = machine(true);
        machine.process_string(b"\x1b[A\x1b[I\x1b[65;30;97;1;0;1_");

        assert!(capture.keys().is_empty());
        assert_eq!(*capture.focus.borrow(), [true]);
        assert_eq!(capture.ctrl_keys.borrow().len(), 1);
    }
}
