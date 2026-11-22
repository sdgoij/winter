//! `IStateMachineEngine` — the grammar the state machine drives.
//!
//! Ported from `IStateMachineEngine.hpp`. The parser owns no terminal
//! vocabulary: every dispatch call hands the engine an identifier plus the
//! parameters collected so far, or a run of bytes to print.
//!
//! Two upstream members are deliberately absent because they exist for
//! ConPTY's pass-through (a host that forwards unknown input sequences):
//! `ActionPassThroughString` and the injection bookkeeping. A terminal that
//! consumes its own output has nowhere to forward a sequence to.

use types::dispatch_types::{VTID, VTParameters};

/// Consumes the bytes of a DCS data string.
///
/// Returns false when the handler can no longer accept input, at which point
/// the parser ignores the rest of the string. The final ESC of `ESC \` is
/// delivered to the handler before the terminator is consumed.
pub type StringHandler = Box<dyn FnMut(u8) -> bool + Send>;

/// The callbacks a state machine drives while parsing.
///
/// Every method returns whether the sequence was handled. The parser uses the
/// result only where upstream does: to decide whether a DCS handler keeps
/// consuming its string.
pub trait IStateMachineEngine {
    /// Whether this engine parses terminal *input* rather than output.
    ///
    /// The input grammar treats `ESC O` as SS3 and maps a control character
    /// after ESC differently; the output grammar dispatches `ESC O` as an ESC
    /// sequence instead. Ported from the `isEngineForInput` constructor flag.
    const IS_INPUT: bool = false;

    /// Called when a SOS/PM/APC string begins.
    ///
    /// SOS and PM strings are ignored, as upstream ignores them. An APC
    /// string is delivered through [`Self::action_apc`] instead, because a
    /// protocol can ride it: the kitty graphics protocol does.
    fn unknown_sequence(&mut self) {}

    /// Called when an APC string (`ESC _`) begins.
    fn action_apc_start(&mut self) {}

    /// One byte of an APC string, before its `ST`.
    fn action_apc(&mut self, _ch: u8) {}

    /// Called when the APC string's `ST` arrives.
    fn action_apc_end(&mut self) {}

    /// Whether a win32-input-mode sequence has been seen.
    ///
    /// Only the input engine can answer true; it exists to let the parser
    /// stop guessing about split `ESC <key>` sequences.
    fn encountered_win32_input_mode_sequence(&self) -> bool {
        false
    }

    /// A C0 control character in the Ground state.
    fn action_execute(&mut self, _ch: u8) -> bool {
        true
    }

    /// A C0 control character while in the Escape state (input engine only).
    fn action_execute_from_escape(&mut self, ch: u8) -> bool {
        self.action_execute(ch)
    }

    /// A printable character.
    fn action_print(&mut self, _ch: u8) -> bool {
        true
    }

    /// A run of printable characters.
    fn action_print_string(&mut self, string: &[u8]) -> bool {
        string.iter().all(|&ch| self.action_print(ch))
    }

    /// A complete ESC sequence.
    fn action_esc_dispatch(&mut self, _id: VTID) -> bool {
        true
    }

    /// A complete VT52 escape sequence.
    fn action_vt52_esc_dispatch(&mut self, _id: VTID, _parameters: &VTParameters) -> bool {
        true
    }

    /// A complete CSI sequence.
    fn action_csi_dispatch(&mut self, _id: VTID, _parameters: &VTParameters) -> bool {
        true
    }

    /// A DCS introducer. Returning `None` makes the parser ignore the string.
    fn action_dcs_dispatch(&mut self, _id: VTID, _parameters: &VTParameters) -> Option<StringHandler> {
        None
    }

    /// A complete OSC sequence.
    ///
    /// The command is the numeric OSC parameter; an empty command reports 0,
    /// as upstream's `_oscParameter` starts at 0.
    fn action_osc_dispatch(&mut self, _parameter: i32, _string: &[u8]) -> bool {
        true
    }

    /// A complete SS3 sequence (input engine only).
    fn action_ss3_dispatch(&mut self, _ch: u8, _parameters: &VTParameters) -> bool {
        true
    }
}
