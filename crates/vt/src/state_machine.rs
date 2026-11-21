//! VT state machine — DFA-based escape sequence parser.
//!
//! Ported from the state machine in the Windows Terminal C++ VT module.
//! The parser processes incoming byte sequences through a 19-state DFA
//! to identify CSI, OSC, DCS, APM, SS3, and ESC sequences.

use crate::ascii::*;
use crate::parameters::VtParameters;
use crate::state::VtState;

/// A dispatch action produced by the state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchAction {
    /// Process the given byte as a normal character.
    ProcessChar(u8),
    /// Process the given byte as part of a string.
    ProcessString(u8),
    /// The last character of an escape sequence was received.
    ProcessLastChar,
    /// Inject the given byte.
    InjectByte(u8),
    /// CSI sequence completed.
    CsiComplete,
    /// OSC sequence completed.
    OscComplete,
    /// DCS sequence completed.
    DcsComplete,
    /// APM sequence completed.
    ApmComplete,
    /// SS3 sequence completed.
    Ss3Complete,
    /// ESC sequence completed.
    EscComplete,
}

/// The core VT state machine.
///
/// Processes incoming bytes through a deterministic finite automaton (DFA)
/// with 19 states to parse VT escape sequences.
pub struct StateMachine {
    /// Current state of the machine.
    state: VtState,
    /// Parsed parameters.
    params: VtParameters,
    /// Intermediate characters for CSI sequences.
    intermediate: String,
    /// Accumulated string payload.
    string: String,
    /// Whether the next character is the last of an escape sequence.
    is_last_escape: bool,
}

impl StateMachine {
    /// Creates a new state machine starting in the Ground state.
    pub fn new() -> Self {
        Self {
            state: VtState::Ground,
            params: VtParameters::new(),
            intermediate: String::new(),
            string: String::new(),
            is_last_escape: false,
        }
    }

    /// Returns the current state.
    pub const fn state(&self) -> VtState {
        self.state
    }

    /// Returns a reference to the parsed parameters.
    pub fn params(&self) -> &VtParameters {
        &self.params
    }

    /// Returns a reference to the accumulated intermediate characters.
    pub fn intermediate(&self) -> &str {
        &self.intermediate
    }

    /// Returns a reference to the accumulated string payload.
    pub fn string(&self) -> &str {
        &self.string
    }

    /// Returns whether the current sequence is an ESC sequence.
    pub const fn is_last_escape(&self) -> bool {
        self.is_last_escape
    }

    /// Processes a single byte and optionally produces a dispatch action.
    pub fn process_char(&mut self, c: u8) -> Option<DispatchAction> {
        match self.state {
            VtState::Ground => self.process_ground(c),
            VtState::Prefix => self.process_prefix(c),
            VtState::Escape => self.process_escape(c),
            VtState::EscapeFirst => self.process_escape_first(c),
            VtState::Dpad => self.process_dpad(c),
            VtState::DcsParam => self.process_dcs_param(c),
            VtState::DcsStr => self.process_dcs_str(c),
            VtState::DcsPassthrough => self.process_dcs_passthrough(c),
            VtState::CsiIntermediate => self.process_csi_intermediate(c),
            VtState::CsiParam => self.process_csi_param(c),
            VtState::CsiIgnore => self.process_csi_ignore(c),
            VtState::OscStr => self.process_osc_str(c),
            VtState::OscPassthrough => self.process_osc_passthrough(c),
            VtState::Ss3Ignore => self.process_ss3_ignore(c),
            VtState::Ss3Param => self.process_ss3_param(c),
            VtState::ApmIgnore => self.process_apm_ignore(c),
            VtState::ApmStr => self.process_apm_str(c),
            VtState::ApmPassthrough => self.process_apm_passthrough(c),
            VtState::Error => self.process_error(c),
        }
    }

    /// Resets the machine to the Ground state, clearing all buffers.
    pub fn reset_state(&mut self) {
        self.state = VtState::Ground;
        self.params.clear();
        self.intermediate.clear();
        self.string.clear();
        self.is_last_escape = false;
    }

    // ─── State handlers ─────────────────────────────────────────────

    fn process_ground(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ESC => {
                self.state = VtState::Escape;
                Some(DispatchAction::InjectByte(ESC))
            }
            PAD | HNP | LNP | HPP | LPP | PCE => Some(DispatchAction::InjectByte(c)),
            ST => Some(DispatchAction::ProcessLastChar),
            0x80..=0x9F => {
                self.state = VtState::Prefix;
                Some(DispatchAction::InjectByte(c))
            }
            _ if (0x20..=0x7E).contains(&c) => Some(DispatchAction::ProcessChar(c)),
            _ => Some(DispatchAction::ProcessChar(c)),
        }
    }

    fn process_prefix(&mut self, c: u8) -> Option<DispatchAction> {
        // C1 prefix followed by another prefix byte → DPAD
        if (0x80..=0x9F).contains(&c) {
            self.state = VtState::Dpad;
            Some(DispatchAction::InjectByte(c))
        } else if c == DCS_INTRODUCER {
            self.state = VtState::DcsParam;
            None
        } else {
            // Single prefix byte is treated as part of the stream
            self.state = VtState::Ground;
            Some(DispatchAction::ProcessChar(c))
        }
    }

    fn process_escape(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ESC | PAD | HNP | LNP | HPP | LPP | PCE => {
                self.state = VtState::Ground;
                Some(DispatchAction::InjectByte(c))
            }
            ST => Some(DispatchAction::ProcessLastChar),
            0x80..=0x9F => {
                self.state = VtState::EscapeFirst;
                Some(DispatchAction::InjectByte(c))
            }
            DCS_INTRODUCER => {
                self.state = VtState::DcsParam;
                None
            }
            OSC_INTRODUCER => {
                self.state = VtState::OscStr;
                None
            }
            APC_INTRODUCER => {
                self.state = VtState::ApmStr;
                None
            }
            SS3_INTRODUCER => {
                self.state = VtState::Ss3Param;
                None
            }
            // Two-character escape sequences
            b'[' => {
                self.state = VtState::CsiParam;
                None
            }
            b'P' => {
                self.state = VtState::DcsParam;
                None
            }
            b'X' => {
                self.state = VtState::CsiParam;
                None
            }
            b'^' => {
                self.state = VtState::ApmStr;
                None
            }
            b'\\' => {
                self.state = VtState::Ground;
                Some(DispatchAction::EscComplete)
            }
            b' ' => {
                self.state = VtState::Escape;
                self.is_last_escape = true;
                None
            }
            0x21..=0x7E => {
                self.state = VtState::Ground;
                Some(DispatchAction::EscComplete)
            }
            _ => {
                self.state = VtState::Ground;
                Some(DispatchAction::ProcessChar(c))
            }
        }
    }

    fn process_escape_first(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            0x80..=0x9F => {
                self.state = VtState::Dpad;
                Some(DispatchAction::InjectByte(c))
            }
            _ => {
                self.state = VtState::Ground;
                Some(DispatchAction::ProcessChar(c))
            }
        }
    }

    fn process_dpad(&mut self, c: u8) -> Option<DispatchAction> {
        if (0x80..=0x9F).contains(&c) {
            Some(DispatchAction::InjectByte(c))
        } else if c == ST {
            Some(DispatchAction::ProcessLastChar)
        } else {
            Some(DispatchAction::ProcessChar(c))
        }
    }

    fn process_dcs_param(&mut self, c: u8) -> Option<DispatchAction> {
        if c == ST_ESC {
            self.state = VtState::DcsStr;
            None
        } else if (0x20..=0x7F).contains(&c) {
            self.state = VtState::DcsStr;
            Some(DispatchAction::ProcessChar(c))
        } else {
            Some(DispatchAction::ProcessChar(c))
        }
    }

    fn process_dcs_str(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ST_ESC => {
                self.state = VtState::Ground;
                Some(DispatchAction::ProcessLastChar)
            }
            _ => Some(DispatchAction::ProcessChar(c)),
        }
    }

    fn process_dcs_passthrough(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ST_ESC => {
                self.state = VtState::Ground;
                Some(DispatchAction::ProcessLastChar)
            }
            _ => Some(DispatchAction::ProcessChar(c)),
        }
    }

    fn process_csi_intermediate(&mut self, c: u8) -> Option<DispatchAction> {
        if (0x20..=0x2F).contains(&c) {
            self.intermediate.push(c as char);
            None
        } else if (0x40..=0x7E).contains(&c) {
            self.state = VtState::Ground;
            Some(DispatchAction::CsiComplete)
        } else if c == ST_ESC {
            self.state = VtState::Ground;
            Some(DispatchAction::ProcessLastChar)
        } else {
            self.state = VtState::CsiIgnore;
            Some(DispatchAction::ProcessChar(c))
        }
    }

    fn process_csi_param(&mut self, c: u8) -> Option<DispatchAction> {
        if c == ST_ESC {
            self.state = VtState::CsiIgnore;
            None
        } else if (0x30..=0x3F).contains(&c) {
            None
        } else if (0x30..=0x39).contains(&c) || c == b';' {
            self.state = VtState::CsiParam;
            None
        } else if (0x20..=0x2F).contains(&c) {
            self.intermediate.push(c as char);
            self.state = VtState::CsiIntermediate;
            None
        } else if (0x40..=0x7E).contains(&c) {
            self.state = VtState::Ground;
            Some(DispatchAction::CsiComplete)
        } else {
            self.state = VtState::CsiIgnore;
            Some(DispatchAction::ProcessChar(c))
        }
    }

    fn process_csi_ignore(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ST_ESC => {
                self.state = VtState::Ground;
                Some(DispatchAction::ProcessLastChar)
            }
            _ => Some(DispatchAction::ProcessChar(c)),
        }
    }

    fn process_osc_str(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ST_ESC => {
                self.state = VtState::Ground;
                Some(DispatchAction::OscComplete)
            }
            0x9C => {
                self.state = VtState::Ground;
                Some(DispatchAction::OscComplete)
            }
            _ => Some(DispatchAction::ProcessChar(c)),
        }
    }

    fn process_osc_passthrough(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ST_ESC => {
                self.state = VtState::Ground;
                Some(DispatchAction::ProcessLastChar)
            }
            _ => Some(DispatchAction::ProcessChar(c)),
        }
    }

    fn process_ss3_ignore(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ST_ESC => {
                self.state = VtState::Ground;
                Some(DispatchAction::ProcessLastChar)
            }
            _ => Some(DispatchAction::ProcessChar(c)),
        }
    }

    fn process_ss3_param(&mut self, c: u8) -> Option<DispatchAction> {
        if (0x40..=0x7E).contains(&c) {
            self.state = VtState::Ground;
            Some(DispatchAction::Ss3Complete)
        } else if (0x20..=0x3F).contains(&c) {
            None
        } else if c == ST_ESC {
            self.state = VtState::Ground;
            Some(DispatchAction::ProcessLastChar)
        } else {
            self.state = VtState::Ss3Ignore;
            Some(DispatchAction::ProcessChar(c))
        }
    }

    fn process_apm_ignore(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ST_ESC => {
                self.state = VtState::Ground;
                Some(DispatchAction::ProcessLastChar)
            }
            _ => Some(DispatchAction::ProcessChar(c)),
        }
    }

    fn process_apm_str(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ST_ESC => {
                self.state = VtState::Ground;
                Some(DispatchAction::ApmComplete)
            }
            0x9C => {
                self.state = VtState::Ground;
                Some(DispatchAction::ApmComplete)
            }
            _ => Some(DispatchAction::ProcessChar(c)),
        }
    }

    fn process_apm_passthrough(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ST_ESC => {
                self.state = VtState::Ground;
                Some(DispatchAction::ProcessLastChar)
            }
            _ => Some(DispatchAction::ProcessChar(c)),
        }
    }

    fn process_error(&mut self, c: u8) -> Option<DispatchAction> {
        match c {
            ESC => {
                self.reset_state();
                self.process_char(c)
            }
            _ => {
                // In error state, accumulate until we see ESC
                Some(DispatchAction::ProcessChar(c))
            }
        }
    }
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_state_is_ground() {
        let sm = StateMachine::new();
        assert_eq!(sm.state(), VtState::Ground);
    }

    #[test]
    fn test_process_ascii_char() {
        let mut sm = StateMachine::new();
        let result = sm.process_char(b'A').unwrap();
        assert_eq!(result, DispatchAction::ProcessChar(b'A'));
        assert_eq!(sm.state(), VtState::Ground);
    }

    #[test]
    fn test_process_escape() {
        let mut sm = StateMachine::new();
        let result = sm.process_char(ESC).unwrap();
        assert_eq!(result, DispatchAction::InjectByte(ESC));
        assert_eq!(sm.state(), VtState::Escape);
    }

    #[test]
    fn test_escaped_bracket_becomes_csi() {
        let mut sm = StateMachine::new();
        // ESC [
        sm.process_char(ESC);
        assert_eq!(sm.state(), VtState::Escape);
        let result = sm.process_char(b'[');
        // '[' after ESC goes to CsiParam state
        assert_eq!(sm.state(), VtState::CsiParam);
        // Result may be Some or None depending on implementation
        let _ = result;
    }

    #[test]
    fn test_reset_state() {
        let mut sm = StateMachine::new();
        sm.process_char(ESC);
        assert_eq!(sm.state(), VtState::Escape);
        sm.reset_state();
        assert_eq!(sm.state(), VtState::Ground);
        assert!(sm.params().is_empty());
    }

    #[test]
    fn test_process_osc_sequence() {
        let mut sm = StateMachine::new();
        // ESC ]
        sm.process_char(ESC);
        assert_eq!(sm.state(), VtState::Escape);
        // OSC introducer
        sm.process_char(OSC_INTRODUCER);
        assert_eq!(sm.state(), VtState::OscStr);
        // String content
        sm.process_char(b'0');
        sm.process_char(b':');
        sm.process_char(b'T');
        sm.process_char(b'e');
        sm.process_char(b's');
        sm.process_char(b't');
        // ESC \ terminator
        sm.process_char(ESC);
        let result = sm.process_char(ST_ESC).unwrap();
        assert_eq!(result, DispatchAction::OscComplete);
        assert_eq!(sm.state(), VtState::Ground);
    }

    #[test]
    fn test_process_dcs_sequence() {
        let mut sm = StateMachine::new();
        // ESC P
        sm.process_char(ESC);
        assert_eq!(sm.state(), VtState::Escape);
        let _ = sm.process_char(b'P');
        assert_eq!(sm.state(), VtState::DcsParam);
        // DCS content - first char moves to DcsStr
        sm.process_char(b'?');
        assert_eq!(sm.state(), VtState::DcsStr);
        // More DCS content
        sm.process_char(b'2');
        sm.process_char(b'$');
        // ESC \ terminator
        sm.process_char(ESC);
        let result = sm.process_char(ST_ESC).unwrap();
        assert_eq!(result, DispatchAction::ProcessLastChar);
        assert_eq!(sm.state(), VtState::Ground);
    }

    #[test]
    fn test_csi_sequence_flow() {
        let mut sm = StateMachine::new();
        // Start CSI: ESC [
        sm.process_char(ESC);
        sm.process_char(b'[');
        assert_eq!(sm.state(), VtState::CsiParam);

        // Parameters
        sm.process_char(b'1');
        sm.process_char(b';');
        sm.process_char(b'2');

        // Final byte
        let result = sm.process_char(b'm').unwrap();
        assert_eq!(result, DispatchAction::CsiComplete);
        assert_eq!(sm.state(), VtState::Ground);
    }

    #[test]
    fn test_reset_clears_state() {
        let mut sm = StateMachine::new();
        sm.process_char(ESC);
        sm.process_char(b'[');
        sm.process_char(b'1');
        assert_eq!(sm.state(), VtState::CsiParam);
        // params starts as [0] from construction, never had real params
        // After reset, should be empty

        sm.reset_state();
        assert_eq!(sm.state(), VtState::Ground);
        assert!(sm.params().is_empty());
        assert!(sm.intermediate().is_empty());
        assert!(sm.string().is_empty());
    }

    #[test]
    fn test_single_esc_back_to_ground() {
        let mut sm = StateMachine::new();
        sm.process_char(ESC);
        assert_eq!(sm.state(), VtState::Escape);
        // A single character after ESC completes the sequence
        let result = sm.process_char(b'A').unwrap();
        assert_eq!(result, DispatchAction::EscComplete);
        assert_eq!(sm.state(), VtState::Ground);
    }

    #[test]
    fn test_csi_intermediate_chars() {
        let mut sm = StateMachine::new();
        // ESC [ param intermediate final
        sm.process_char(ESC);
        sm.process_char(b'[');
        // param
        sm.process_char(b'1');
        // intermediate chars (0x20..=0x2F)
        sm.process_char(b'/'); // 0x2F - last intermediate
        assert_eq!(sm.state(), VtState::CsiIntermediate);
        sm.process_char(b' '); // 0x20 - first intermediate
        assert_eq!(sm.state(), VtState::CsiIntermediate);
        // Final dispatch char
        let result = sm.process_char(b'X').unwrap();
        assert_eq!(result, DispatchAction::CsiComplete);
        assert_eq!(sm.state(), VtState::Ground);
        // Intermediate chars: '/' then ' '
        assert_eq!(sm.intermediate(), "/ ");
    }
}
