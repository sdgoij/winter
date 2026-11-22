//! VT state machine — the DFA that parses escape sequences.
//!
//! Ported from `stateMachine.cpp`. The machine consumes one byte at a time,
//! collects parameters, sub-parameters, intermediates, and string payloads,
//! and calls the [`IStateMachineEngine`] when a sequence completes.
//!
//! Deliberate divergences from upstream, all ConPTY-specific:
//! `ActionPassThroughString`/`FlushToTerminal` (forwarding a sequence a host
//! does not understand) and injection tracking are absent. A terminal that
//! consumes its own output has nowhere to forward to.

use types::dispatch_types::{VTID, VTParameter, VTParameters};

use crate::ascii;
use crate::i_state_machine_engine::{IStateMachineEngine, StringHandler};
use crate::state::VtState;

/// Values larger than this are clamped, as XTerm and VTE do.
pub const MAX_PARAMETER_VALUE: i32 = 65535;
/// Maximum number of parameters collected for one sequence.
pub const MAX_PARAMETER_COUNT: usize = 32;
/// Maximum number of sub-parameters collected for one parameter.
pub const MAX_SUBPARAMETER_COUNT: u8 = 6;

/// Selectable parser behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserMode {
    /// Interpret `0x80..=0x9F` as C1 introducers.
    AcceptC1,
    /// Parse ANSI sequences (`ESC [` etc.) rather than VT52 sequences.
    Ansi,
}

/// The VT state machine.
pub struct StateMachine<E: IStateMachineEngine> {
    engine: E,
    state: VtState,
    accept_c1: bool,
    ansi: bool,

    identifier: VTID,
    parameters: Vec<VTParameter>,
    parameter_limit_overflowed: bool,
    sub_parameters: Vec<VTParameter>,
    sub_parameter_ranges: Vec<(u8, u8)>,
    sub_parameter_limit_overflowed: bool,
    sub_parameter_counter: u8,

    osc_string: Vec<u8>,
    osc_parameter: i32,

    dcs_string_handler: Option<StringHandler>,
}

impl<E: IStateMachineEngine> StateMachine<E> {
    /// Creates a machine that drives `engine`.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            state: VtState::Ground,
            // The input grammar always accepts C1 so a ConPTY peer can enable
            // S8C1T; the output grammar starts in ANSI mode without C1.
            accept_c1: E::IS_INPUT,
            ansi: true,
            identifier: VTID::new(),
            parameters: Vec::new(),
            parameter_limit_overflowed: false,
            sub_parameters: Vec::new(),
            sub_parameter_ranges: Vec::new(),
            sub_parameter_limit_overflowed: false,
            sub_parameter_counter: 0,
            osc_string: Vec::new(),
            osc_parameter: 0,
            dcs_string_handler: None,
        }
    }

    /// Returns the current state.
    pub const fn state(&self) -> VtState {
        self.state
    }

    /// Returns the engine.
    pub const fn engine(&self) -> &E {
        &self.engine
    }

    /// Returns the engine mutably.
    pub fn engine_mut(&mut self) -> &mut E {
        &mut self.engine
    }

    /// Sets a parser mode.
    pub fn set_parser_mode(&mut self, mode: ParserMode, enabled: bool) {
        match mode {
            ParserMode::AcceptC1 => self.accept_c1 = enabled,
            ParserMode::Ansi => self.ansi = enabled,
        }
    }

    /// Returns a parser mode.
    pub const fn parser_mode(&self, mode: ParserMode) -> bool {
        match mode {
            ParserMode::AcceptC1 => self.accept_c1,
            ParserMode::Ansi => self.ansi,
        }
    }

    /// Processes one byte.
    pub fn process_char(&mut self, ch: u8) {
        let interrupt = ch == ascii::CAN || ch == ascii::SUB;

        if interrupt && !(self.state == VtState::Escape && E::IS_INPUT) {
            self.action_interrupt();
            self.engine.action_execute(ch);
            self.enter_ground();
        } else if (0x80..=0x9F).contains(&ch) {
            // C1 controls are ESC followed by the 7-bit equivalent, and are
            // only recognised when the host asks for them: some code pages
            // use these bytes for characters that are not C1 controls.
            //
            // Unlike upstream, a disabled C1 byte is dispatched normally
            // rather than dropped. This parser sees UTF-8 bytes, and the
            // continuation bytes of many characters fall in this range; the
            // engine decodes them after parsing.
            if self.accept_c1 {
                self.process_char(ascii::ESC);
                self.process_char(ch - 0x40);
            } else {
                self.dispatch(ch);
            }
        } else if ch == ascii::ESC
            && self.state != VtState::OscString
            && self.state != VtState::OscParam
            && self.state != VtState::ApcString
        {
            self.action_interrupt();
            self.enter_escape();
        } else {
            self.dispatch(ch);
        }
    }

    fn dispatch(&mut self, ch: u8) {
        match self.state {
            VtState::Ground => self.event_ground(ch),
            VtState::Escape => self.event_escape(ch),
            VtState::EscapeIntermediate => self.event_escape_intermediate(ch),
            VtState::CsiEntry => self.event_csi_entry(ch),
            VtState::CsiIntermediate => self.event_csi_intermediate(ch),
            VtState::CsiIgnore => self.event_csi_ignore(ch),
            VtState::CsiParam => self.event_csi_param(ch),
            VtState::CsiSubParam => self.event_csi_sub_param(ch),
            VtState::OscParam => self.event_osc_param(ch),
            VtState::OscString => self.event_osc_string(ch),
            VtState::OscTermination => self.event_osc_termination(ch),
            VtState::Ss3Entry => self.event_ss3_entry(ch),
            VtState::Ss3Param => self.event_ss3_param(ch),
            VtState::Vt52Param => self.event_vt52_param(ch),
            VtState::DcsEntry => self.event_dcs_entry(ch),
            VtState::DcsIgnore => {}
            VtState::DcsIntermediate => self.event_dcs_intermediate(ch),
            VtState::DcsParam => self.event_dcs_param(ch),
            VtState::DcsPassThrough => self.event_dcs_pass_through(ch),
            VtState::SosPmApcString => {}
            VtState::ApcString => self.event_apc_string(ch),
            VtState::ApcTermination => self.event_apc_termination(ch),
        }
    }

    /// Processes a slice of bytes.
    pub fn process_string(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.process_char(byte);
        }
    }

    /// Resets the machine to the Ground state.
    pub fn reset_state(&mut self) {
        self.enter_ground();
    }

    /// Returns the parameters collected for the sequence in flight.
    pub fn parameters(&self) -> VTParameters {
        self.gather_parameters()
    }

    // ── Actions ─────────────────────────────────────────────────────

    fn action_param(&mut self, ch: u8) {
        if self.parameter_limit_overflowed {
            return;
        }

        if self.parameters.is_empty() {
            self.push_parameter();
        }

        if ch == b';' {
            if self.parameters.len() >= MAX_PARAMETER_COUNT {
                self.parameter_limit_overflowed = true;
            } else {
                self.push_parameter();
                self.sub_parameter_counter = 0;
                self.sub_parameter_limit_overflowed = false;
            }
        } else {
            let current = self.parameters.last().map_or(0, |param| param.value_or(0));
            let value = accumulate(current, ch);
            if let Some(last) = self.parameters.last_mut() {
                *last = VTParameter::from(value);
            }
        }
    }

    fn push_parameter(&mut self) {
        self.parameters.push(VTParameter::default());
        let start = self.sub_parameters.len() as u8;
        self.sub_parameter_ranges.push((start, start));
    }

    fn action_sub_param(&mut self, ch: u8) {
        if self.sub_parameter_limit_overflowed {
            return;
        }

        if self.parameters.is_empty() {
            self.push_parameter();
        }

        if ch == b':' {
            if self.sub_parameter_counter >= MAX_SUBPARAMETER_COUNT {
                self.sub_parameter_limit_overflowed = true;
            } else {
                self.sub_parameters.push(VTParameter::default());
                if let Some(range) = self.sub_parameter_ranges.last_mut() {
                    range.1 = range.1.saturating_add(1);
                }
                self.sub_parameter_counter += 1;
            }
        } else {
            let current = self.sub_parameters.last().map_or(0, |param| param.value_or(0));
            let value = accumulate(current, ch);
            if let Some(last) = self.sub_parameters.last_mut() {
                *last = VTParameter::from(value);
            }
        }
    }

    fn action_collect(&mut self, ch: u8) {
        self.identifier = self.identifier.with_intermediate(ch);
    }

    fn action_clear(&mut self) {
        self.identifier = VTID::new();
        self.parameters.clear();
        self.parameter_limit_overflowed = false;
        self.sub_parameters.clear();
        self.sub_parameter_ranges.clear();
        self.sub_parameter_counter = 0;
        self.sub_parameter_limit_overflowed = false;
        self.osc_string.clear();
        self.osc_parameter = 0;
        self.dcs_string_handler = None;
    }

    fn action_interrupt(&mut self) {
        // Only DCS strings are interrupted; an OSC string needs a full ST.
        if self.state == VtState::DcsPassThrough
            && let Some(mut handler) = self.dcs_string_handler.take()
        {
            handler(ascii::ESC);
        }
    }

    fn action_osc_dispatch(&mut self) {
        let parameter = self.osc_parameter;
        self.engine.action_osc_dispatch(parameter, &self.osc_string);
    }

    fn action_ss3_dispatch(&mut self, ch: u8) {
        let parameters = self.gather_parameters();
        self.engine.action_ss3_dispatch(ch, &parameters);
    }

    fn action_csi_dispatch(&mut self, ch: u8) {
        let id = self.identifier.finalize_with(ch);
        let parameters = self.gather_parameters();
        self.engine.action_csi_dispatch(id, &parameters);
    }

    fn action_esc_dispatch(&mut self, ch: u8) {
        let id = self.identifier.finalize_with(ch);
        self.engine.action_esc_dispatch(id);
    }

    fn action_vt52_esc_dispatch(&mut self, ch: u8) {
        let id = self.identifier.finalize_with(ch);
        let parameters = self.gather_parameters();
        self.engine.action_vt52_esc_dispatch(id, &parameters);
    }

    fn action_dcs_dispatch(&mut self, ch: u8) {
        let id = self.identifier.finalize_with(ch);
        let parameters = self.gather_parameters();
        match self.engine.action_dcs_dispatch(id, &parameters) {
            Some(handler) => {
                self.dcs_string_handler = Some(handler);
                self.enter_dcs_pass_through();
            }
            None => self.enter_dcs_ignore(),
        }
    }

    fn gather_parameters(&self) -> VTParameters {
        VTParameters::from_parts(
            self.parameters.clone(),
            self.sub_parameters.clone(),
            self.sub_parameter_ranges.clone(),
        )
    }

    // ── State entries ───────────────────────────────────────────────

    fn enter_ground(&mut self) {
        self.state = VtState::Ground;
    }

    fn enter_escape(&mut self) {
        self.state = VtState::Escape;
        self.action_clear();
    }

    fn enter_escape_intermediate(&mut self) {
        self.state = VtState::EscapeIntermediate;
    }

    fn enter_csi_entry(&mut self) {
        self.state = VtState::CsiEntry;
        self.action_clear();
    }

    fn enter_csi_param(&mut self) {
        self.state = VtState::CsiParam;
    }

    fn enter_csi_sub_param(&mut self) {
        self.state = VtState::CsiSubParam;
    }

    fn enter_csi_ignore(&mut self) {
        self.state = VtState::CsiIgnore;
    }

    fn enter_csi_intermediate(&mut self) {
        self.state = VtState::CsiIntermediate;
    }

    fn enter_osc_param(&mut self) {
        self.state = VtState::OscParam;
    }

    fn enter_osc_string(&mut self) {
        self.state = VtState::OscString;
    }

    fn enter_osc_termination(&mut self) {
        self.state = VtState::OscTermination;
    }

    fn enter_ss3_entry(&mut self) {
        self.state = VtState::Ss3Entry;
        self.action_clear();
    }

    fn enter_ss3_param(&mut self) {
        self.state = VtState::Ss3Param;
    }

    fn enter_vt52_param(&mut self) {
        self.state = VtState::Vt52Param;
    }

    fn enter_dcs_entry(&mut self) {
        self.state = VtState::DcsEntry;
        self.action_clear();
    }

    fn enter_dcs_param(&mut self) {
        self.state = VtState::DcsParam;
    }

    fn enter_dcs_ignore(&mut self) {
        self.state = VtState::DcsIgnore;
    }

    fn enter_dcs_intermediate(&mut self) {
        self.state = VtState::DcsIntermediate;
    }

    fn enter_dcs_pass_through(&mut self) {
        self.state = VtState::DcsPassThrough;
    }

    fn enter_sos_pm_apc_string(&mut self) {
        self.state = VtState::SosPmApcString;
        self.engine.unknown_sequence();
    }

    fn enter_apc_string(&mut self) {
        self.state = VtState::ApcString;
        self.engine.action_apc_start();
    }

    fn enter_apc_termination(&mut self) {
        self.state = VtState::ApcTermination;
    }

    // ── Events ──────────────────────────────────────────────────────

    fn event_ground(&mut self, ch: u8) {
        if is_c0(ch) || ch == ascii::DEL {
            self.engine.action_execute(ch);
        } else {
            self.engine.action_print(ch);
        }
    }

    fn event_escape(&mut self, ch: u8) {
        if is_c0(ch) {
            if E::IS_INPUT {
                self.engine.action_execute_from_escape(ch);
                self.enter_ground();
            } else {
                self.engine.action_execute(ch);
            }
        } else if ch == ascii::DEL {
            // Ignore.
        } else if is_intermediate(ch) {
            if E::IS_INPUT {
                self.action_esc_dispatch(ch);
                self.enter_ground();
            } else {
                self.action_collect(ch);
                self.enter_escape_intermediate();
            }
        } else if self.ansi {
            match ch {
                b'[' => self.enter_csi_entry(),
                b']' => self.enter_osc_param(),
                b'O' if E::IS_INPUT => self.enter_ss3_entry(),
                b'P' => self.enter_dcs_entry(),
                b'X' | b'^' => self.enter_sos_pm_apc_string(),
                b'_' => self.enter_apc_string(),
                _ => {
                    self.action_esc_dispatch(ch);
                    self.enter_ground();
                }
            }
        } else if ch == b'Y' {
            self.enter_vt52_param();
        } else {
            self.action_vt52_esc_dispatch(ch);
            self.enter_ground();
        }
    }

    fn event_escape_intermediate(&mut self, ch: u8) {
        if is_c0(ch) {
            self.engine.action_execute(ch);
        } else if is_intermediate(ch) {
            self.action_collect(ch);
        } else if ch == ascii::DEL {
            // Ignore.
        } else if self.ansi {
            self.action_esc_dispatch(ch);
            self.enter_ground();
        } else if ch == b'Y' {
            self.enter_vt52_param();
        } else {
            self.action_vt52_esc_dispatch(ch);
            self.enter_ground();
        }
    }

    fn event_csi_entry(&mut self, ch: u8) {
        if is_c0(ch) {
            self.engine.action_execute(ch);
        } else if ch == ascii::DEL {
            // Ignore.
        } else if is_intermediate(ch) {
            self.action_collect(ch);
            self.enter_csi_intermediate();
        } else if is_digit(ch) || ch == b';' {
            self.action_param(ch);
            self.enter_csi_param();
        } else if ch == b':' {
            self.action_sub_param(ch);
            self.enter_csi_sub_param();
        } else if is_csi_private_marker(ch) {
            self.action_collect(ch);
            self.enter_csi_param();
        } else {
            self.action_csi_dispatch(ch);
            self.enter_ground();
        }
    }

    fn event_csi_intermediate(&mut self, ch: u8) {
        if is_c0(ch) {
            self.engine.action_execute(ch);
        } else if is_intermediate(ch) {
            self.action_collect(ch);
        } else if ch == ascii::DEL {
            // Ignore.
        } else if is_intermediate_invalid(ch) {
            self.enter_csi_ignore();
        } else {
            self.action_csi_dispatch(ch);
            self.enter_ground();
        }
    }

    fn event_csi_ignore(&mut self, ch: u8) {
        if is_c0(ch) {
            self.engine.action_execute(ch);
        } else if ch == ascii::DEL || is_intermediate(ch) || is_intermediate_invalid(ch) {
            // Ignore.
        } else {
            self.enter_ground();
        }
    }

    fn event_csi_param(&mut self, ch: u8) {
        if is_c0(ch) {
            self.engine.action_execute(ch);
        } else if ch == ascii::DEL {
            // Ignore.
        } else if is_digit(ch) || ch == b';' {
            self.action_param(ch);
        } else if ch == b':' {
            self.action_sub_param(ch);
            self.enter_csi_sub_param();
        } else if is_intermediate(ch) {
            self.action_collect(ch);
            self.enter_csi_intermediate();
        } else if is_csi_private_marker(ch) {
            self.enter_csi_ignore();
        } else {
            self.action_csi_dispatch(ch);
            self.enter_ground();
        }
    }

    fn event_csi_sub_param(&mut self, ch: u8) {
        if is_c0(ch) {
            self.engine.action_execute(ch);
        } else if ch == ascii::DEL {
            // Ignore.
        } else if is_digit(ch) || ch == b':' {
            self.action_sub_param(ch);
        } else if ch == b';' {
            self.action_param(ch);
            self.enter_csi_param();
        } else if is_intermediate(ch) {
            self.action_collect(ch);
            self.enter_csi_intermediate();
        } else if is_csi_private_marker(ch) {
            self.enter_csi_ignore();
        } else {
            self.action_csi_dispatch(ch);
            self.enter_ground();
        }
    }

    fn event_osc_param(&mut self, ch: u8) {
        if ch == ascii::BEL {
            self.action_osc_dispatch();
            self.enter_ground();
        } else if ch == ascii::ESC {
            self.enter_osc_termination();
        } else if is_digit(ch) {
            self.osc_parameter = accumulate(self.osc_parameter, ch);
        } else if ch == b';' {
            self.enter_osc_string();
        }
        // Everything else is ignored.
    }

    fn event_osc_string(&mut self, ch: u8) {
        if ch == ascii::BEL {
            self.action_osc_dispatch();
            self.enter_ground();
        } else if ch == ascii::ESC {
            self.enter_osc_termination();
        } else if is_osc_invalid(ch) {
            // Ignore.
        } else {
            self.osc_string.push(ch);
        }
    }

    fn event_osc_termination(&mut self, ch: u8) {
        if ch == b'\\' {
            self.action_osc_dispatch();
            self.enter_ground();
        } else {
            self.enter_escape();
            self.event_escape(ch);
        }
    }

    fn event_apc_string(&mut self, ch: u8) {
        if ch == ascii::ESC {
            self.enter_apc_termination();
        } else if ch == ascii::BEL {
            // APC has no BEL terminator; accepting one costs nothing and
            // keeps a stray control byte from being parsed as content.
            self.engine.action_apc_end();
            self.enter_ground();
        } else {
            self.engine.action_apc(ch);
        }
    }

    fn event_apc_termination(&mut self, ch: u8) {
        if ch == b'\\' {
            self.engine.action_apc_end();
            self.enter_ground();
        } else {
            // The string was abandoned; the ESC starts a fresh sequence.
            self.engine.action_apc_end();
            self.enter_escape();
            self.event_escape(ch);
        }
    }

    fn event_ss3_entry(&mut self, ch: u8) {
        if is_c0(ch) {
            self.engine.action_execute(ch);
        } else if ch == ascii::DEL {
            // Ignore.
        } else if ch == b':' {
            self.enter_csi_ignore();
        } else if is_digit(ch) || ch == b';' {
            self.action_param(ch);
            self.enter_ss3_param();
        } else {
            self.action_ss3_dispatch(ch);
            self.enter_ground();
        }
    }

    fn event_ss3_param(&mut self, ch: u8) {
        if is_c0(ch) {
            self.engine.action_execute(ch);
        } else if ch == ascii::DEL {
            // Ignore.
        } else if is_digit(ch) || ch == b';' {
            self.action_param(ch);
        } else if is_csi_private_marker(ch) || ch == b':' {
            self.enter_csi_ignore();
        } else {
            self.action_ss3_dispatch(ch);
            self.enter_ground();
        }
    }

    fn event_vt52_param(&mut self, ch: u8) {
        if is_c0(ch) {
            self.engine.action_execute(ch);
        } else if ch == ascii::DEL {
            // Ignore.
        } else {
            // VT52 cursor addresses are raw characters, not decimal digits.
            self.parameters.push(VTParameter::from(i32::from(ch)));
            if self.parameters.len() == 2 {
                self.action_vt52_esc_dispatch(b'Y');
                self.enter_ground();
            }
        }
    }

    fn event_dcs_entry(&mut self, ch: u8) {
        if is_c0(ch) || ch == ascii::DEL {
            // Ignore.
        } else if ch == b':' {
            self.enter_dcs_ignore();
        } else if is_digit(ch) || ch == b';' {
            self.action_param(ch);
            self.enter_dcs_param();
        } else if is_intermediate(ch) {
            self.action_collect(ch);
            self.enter_dcs_intermediate();
        } else {
            self.action_dcs_dispatch(ch);
        }
    }

    fn event_dcs_intermediate(&mut self, ch: u8) {
        if is_c0(ch) || ch == ascii::DEL {
            // Ignore.
        } else if is_intermediate(ch) {
            self.action_collect(ch);
        } else if is_intermediate_invalid(ch) {
            self.enter_dcs_ignore();
        } else {
            self.action_dcs_dispatch(ch);
        }
    }

    fn event_dcs_param(&mut self, ch: u8) {
        if is_c0(ch) || ch == ascii::DEL {
            // Ignore.
        } else if is_digit(ch) || ch == b';' {
            self.action_param(ch);
        } else if is_intermediate(ch) {
            self.action_collect(ch);
            self.enter_dcs_intermediate();
        } else if is_csi_private_marker(ch) || ch == b':' {
            self.enter_dcs_ignore();
        } else {
            self.action_dcs_dispatch(ch);
        }
    }

    fn event_dcs_pass_through(&mut self, ch: u8) {
        if is_c0(ch) || is_dcs_pass_through_valid(ch) {
            let keep_going = self.dcs_string_handler.as_mut().is_some_and(|handler| handler(ch));
            if !keep_going {
                self.enter_dcs_ignore();
            }
        }
        // Other bytes are ignored.
    }
}

impl<E: IStateMachineEngine> Default for StateMachine<E>
where
    E: Default,
{
    fn default() -> Self {
        Self::new(E::default())
    }
}

/// Accumulates a decimal digit into `value`, clamping at the maximum.
fn accumulate(value: i32, ch: u8) -> i32 {
    debug_assert!(is_digit(ch));
    value
        .saturating_mul(10)
        .saturating_add(i32::from(ch - b'0'))
        .min(MAX_PARAMETER_VALUE)
}

/// C0 controls, excluding CAN/SUB (interrupts) and ESC.
const fn is_c0(ch: u8) -> bool {
    ch <= 0x17 || ch == 0x19 || (ch >= 0x1C && ch <= 0x1F)
}

/// `0x30..=0x39`.
const fn is_digit(ch: u8) -> bool {
    ch >= b'0' && ch <= b'9'
}

/// `0x20..=0x2F`.
const fn is_intermediate(ch: u8) -> bool {
    ch >= 0x20 && ch <= 0x2F
}

/// A CSI private marker: `<`, `=`, `>`, `?`.
const fn is_csi_private_marker(ch: u8) -> bool {
    ch >= b'<' && ch <= b'?'
}

/// A byte that ends a sequence collecting intermediates.
const fn is_intermediate_invalid(ch: u8) -> bool {
    is_digit(ch) || ch == b':' || ch == b';' || is_csi_private_marker(ch)
}

/// Bytes an OSC string discards.
const fn is_osc_invalid(ch: u8) -> bool {
    ch <= 0x17 || ch == 0x19 || (ch >= 0x1C && ch <= 0x1F)
}

/// Bytes a DCS handler may receive: `0x20..=0x7E`.
const fn is_dcs_pass_through_valid(ch: u8) -> bool {
    ch >= 0x20 && ch < ascii::DEL
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct Recorder {
        printed: Vec<u8>,
        executed: Vec<u8>,
        esc: Vec<VTID>,
        vt52: Vec<(VTID, VTParameters)>,
        csi: Vec<(VTID, VTParameters)>,
        osc: Vec<(i32, Vec<u8>)>,
        dcs: Vec<(VTID, VTParameters)>,
        dcs_data: Arc<Mutex<Vec<u8>>>,
        accept_dcs: bool,
        unknown: usize,
        apc: Vec<u8>,
    }

    impl IStateMachineEngine for Recorder {
        fn unknown_sequence(&mut self) {
            self.unknown += 1;
        }

        fn action_apc_start(&mut self) {
            self.apc.clear();
        }

        fn action_apc(&mut self, ch: u8) {
            self.apc.push(ch);
        }

        fn action_execute(&mut self, ch: u8) -> bool {
            self.executed.push(ch);
            true
        }

        fn action_print(&mut self, ch: u8) -> bool {
            self.printed.push(ch);
            true
        }

        fn action_esc_dispatch(&mut self, id: VTID) -> bool {
            self.esc.push(id);
            true
        }

        fn action_vt52_esc_dispatch(&mut self, id: VTID, parameters: &VTParameters) -> bool {
            self.vt52.push((id, parameters.clone()));
            true
        }

        fn action_csi_dispatch(&mut self, id: VTID, parameters: &VTParameters) -> bool {
            self.csi.push((id, parameters.clone()));
            true
        }

        fn action_osc_dispatch(&mut self, parameter: i32, string: &[u8]) -> bool {
            self.osc.push((parameter, string.to_vec()));
            true
        }

        fn action_dcs_dispatch(&mut self, id: VTID, parameters: &VTParameters) -> Option<StringHandler> {
            self.dcs.push((id, parameters.clone()));
            if !self.accept_dcs {
                return None;
            }
            let data = Arc::clone(&self.dcs_data);
            Some(Box::new(move |ch| {
                data.lock().unwrap().push(ch);
                true
            }))
        }
    }

    fn machine() -> StateMachine<Recorder> {
        StateMachine::new(Recorder::default())
    }

    #[test]
    fn ground_prints_and_executes() {
        let mut machine = machine();
        machine.process_string(b"A\x07B");
        assert_eq!(machine.engine().printed, vec![b'A', b'B']);
        assert_eq!(machine.engine().executed, vec![0x07]);
    }

    #[test]
    fn utf8_bytes_print_individually() {
        let mut machine = machine();
        machine.process_string("\u{2588}".as_bytes());
        assert_eq!(machine.engine().printed, "\u{2588}".as_bytes());
    }

    #[test]
    fn csi_collects_parameters_and_final_byte() {
        let mut machine = machine();
        machine.process_string(b"\x1b[1;2;3m");

        assert_eq!(machine.engine().csi.len(), 1);
        let (id, parameters) = &machine.engine().csi[0];
        assert_eq!(id.as_bytes(), b"m");
        assert_eq!(parameters.len(), 3);
        assert_eq!(parameters.at(0).value(), 1);
        assert_eq!(parameters.at(1).value(), 2);
        assert_eq!(parameters.at(2).value(), 3);
        assert_eq!(machine.state(), VtState::Ground);
    }

    #[test]
    fn empty_csi_parameters_have_effective_size_one() {
        let mut machine = machine();
        machine.process_string(b"\x1b[m");

        let (id, parameters) = &machine.engine().csi[0];
        assert_eq!(id.as_bytes(), b"m");
        assert!(parameters.is_empty());
        assert_eq!(parameters.size(), 1);
        assert_eq!(parameters.at(0).value_or(0), 0);
    }

    #[test]
    fn csi_keeps_a_private_marker() {
        let mut machine = machine();
        machine.process_string(b"\x1b[?25h");

        let (id, parameters) = &machine.engine().csi[0];
        assert_eq!(id.as_bytes(), b"?h");
        assert_eq!(parameters.at(0).value(), 25);
    }

    #[test]
    fn csi_keeps_an_intermediate() {
        let mut machine = machine();
        machine.process_string(b"\x1b[!p");

        let (id, parameters) = &machine.engine().csi[0];
        assert_eq!(id.as_bytes(), b"!p");
        assert!(parameters.is_empty());
    }

    #[test]
    fn csi_collects_sub_parameters() {
        let mut machine = machine();
        machine.process_string(b"\x1b[38:2::1:2:3m");

        let (id, parameters) = &machine.engine().csi[0];
        assert_eq!(id.as_bytes(), b"m");
        assert_eq!(parameters.len(), 1);
        assert_eq!(parameters.at(0).value(), 38);
        assert!(parameters.has_sub_params_for(0));

        let sub_params = parameters.sub_params_for(0).unwrap();
        assert_eq!(sub_params.len(), 5);
        assert_eq!(sub_params.at(0).value(), 2);
        assert!(!sub_params.at(1).has_value());
        assert_eq!(sub_params.at(2).value(), 1);
        assert_eq!(sub_params.at(3).value(), 2);
        assert_eq!(sub_params.at(4).value(), 3);
    }

    #[test]
    fn csi_overflows_parameters_without_panicking() {
        let mut machine = machine();
        let mut input = b"\x1b[".to_vec();
        for _ in 0..40 {
            input.extend_from_slice(b"1;");
        }
        input.push(b'm');
        machine.process_string(&input);

        let (_, parameters) = &machine.engine().csi[0];
        assert!(parameters.len() <= MAX_PARAMETER_COUNT);
    }

    #[test]
    fn escape_keeps_intermediates_before_the_final_byte() {
        let mut machine = machine();
        machine.process_string(b"\x1b(0");
        assert_eq!(machine.engine().esc.len(), 1);
        assert_eq!(machine.engine().esc[0].as_bytes(), b"(0");
        assert_eq!(machine.engine().esc[0].byte_at(0), b'(');
        assert_eq!(machine.engine().esc[0].subsequence(1).as_bytes(), b"0");
    }

    #[test]
    fn escape_executes_a_c0_inside_the_sequence() {
        let mut machine = machine();
        machine.process_string(b"\x1b\x07");
        assert_eq!(machine.engine().executed, vec![0x07]);
        assert_eq!(machine.state(), VtState::Escape);
    }

    #[test]
    fn osc_dispatches_a_bel_terminated_string() {
        let mut machine = machine();
        machine.process_string(b"\x1b]0;title\x07");

        assert_eq!(machine.engine().osc.len(), 1);
        assert_eq!(machine.engine().osc[0], (0, b"title".to_vec()));
        assert_eq!(machine.state(), VtState::Ground);
    }

    #[test]
    fn osc_dispatches_an_st_terminated_string() {
        let mut machine = machine();
        machine.process_string(b"\x1b]8;;http://example.com\x1b\\");

        assert_eq!(machine.engine().osc.len(), 1);
        assert_eq!(machine.engine().osc[0].0, 8);
        assert_eq!(machine.engine().osc[0].1, b";http://example.com".to_vec());
        assert_eq!(machine.state(), VtState::Ground);
    }

    #[test]
    fn dcs_hands_the_data_string_to_the_handler() {
        let mut machine = StateMachine::new(Recorder {
            accept_dcs: true,
            ..Recorder::default()
        });
        machine.process_string(b"\x1bP0;1;0q~-\x1b\\");

        assert_eq!(machine.engine().dcs.len(), 1);
        let (id, parameters) = &machine.engine().dcs[0];
        assert_eq!(id.as_bytes(), b"q");
        assert_eq!(parameters.at(0).value(), 0);
        assert_eq!(parameters.at(1).value(), 1);
        assert_eq!(parameters.at(2).value(), 0);

        // The final byte selects the handler; the data string starts after it,
        // and the ESC that starts ST is part of the data string.
        assert_eq!(*machine.engine().dcs_data.lock().unwrap(), vec![b'~', b'-', ascii::ESC]);
        assert_eq!(machine.state(), VtState::Ground);
    }

    #[test]
    fn an_unsupported_dcs_ignores_its_string() {
        let mut machine = machine();
        machine.process_string(b"\x1bP$qm\x1b\\");

        assert_eq!(machine.engine().dcs.len(), 1);
        assert!(machine.engine().dcs_data.lock().unwrap().is_empty());
        assert_eq!(machine.state(), VtState::Ground);
    }

    #[test]
    fn cancel_interrupts_a_csi() {
        let mut machine = machine();
        machine.process_string(b"\x1b[1\x18m");

        assert!(machine.engine().csi.is_empty());
        assert_eq!(machine.engine().executed, vec![ascii::CAN]);
        assert_eq!(machine.engine().printed, vec![b'm']);
        assert_eq!(machine.state(), VtState::Ground);
    }

    #[test]
    fn c1_controls_are_dispatched_as_utf8_bytes_until_requested() {
        let mut machine = machine();
        machine.process_string(b"\x9b31m");
        // Disabled C1 bytes pass through as data, not as an introducer.
        assert!(machine.engine().csi.is_empty());
        assert_eq!(machine.engine().printed, b"\x9b31m".to_vec());

        machine.set_parser_mode(ParserMode::AcceptC1, true);
        machine.process_string(b"\x9b31m");
        assert_eq!(machine.engine().csi.len(), 1);
        assert_eq!(machine.engine().csi[0].0.as_bytes(), b"m");
        assert_eq!(machine.engine().csi[0].1.at(0).value(), 31);
    }

    #[test]
    fn vt52_mode_dispatches_raw_cursor_parameters() {
        let mut machine = machine();
        machine.set_parser_mode(ParserMode::Ansi, false);

        machine.process_string(b"\x1bA");
        assert_eq!(machine.engine().vt52[0].0.as_bytes(), b"A");

        machine.process_string(b"\x1bY23");
        let (id, parameters) = &machine.engine().vt52[1];
        assert_eq!(id.as_bytes(), b"Y");
        assert_eq!(parameters.at(0).value(), i32::from(b'2'));
        assert_eq!(parameters.at(1).value(), i32::from(b'3'));
    }

    #[test]
    fn sos_pm_apc_strings_are_ignored_until_st() {
        let mut machine = machine();
        machine.process_string(b"\x1bXignored\x1b\\");
        assert_eq!(machine.engine().unknown, 1);
        assert!(machine.engine().printed.is_empty());
        assert_eq!(machine.state(), VtState::Ground);
    }

    #[test]
    fn apc_strings_reach_the_engine() {
        let mut machine = machine();
        machine.process_string(b"\x1b_Ga=T,p=1;QUJD\x1b\\");

        assert_eq!(machine.engine().apc, b"Ga=T,p=1;QUJD".to_vec());
        assert_eq!(machine.engine().unknown, 0, "an APC is not an unknown string");
        assert!(machine.engine().printed.is_empty());
        assert_eq!(machine.state(), VtState::Ground);
    }

    // The parser must consume any byte in any state without panicking.
    #[test]
    fn arbitrary_bytes_never_panic() {
        let mut machine = machine();
        let mut seed = 0x1234_5678u32;
        for _ in 0..20_000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            machine.process_char((seed & 0xFF) as u8);
        }
        let _ = machine.state();
    }
}
