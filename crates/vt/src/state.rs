//! VT state machine states.
//!
//! Ported from `StateMachine::VTStates` in `stateMachine.hpp`. The parser is a
//! deterministic finite automaton over these states.

/// The states of the VT state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VtState {
    /// Ground state — normal character processing.
    #[default]
    Ground,
    /// Escape state — received ESC, awaiting the sequence body.
    Escape,
    /// Escape intermediate — collecting `0x20..=0x2F` after ESC.
    EscapeIntermediate,
    /// CSI entry — received `ESC [`.
    CsiEntry,
    /// CSI intermediate — collecting intermediates after CSI parameters.
    CsiIntermediate,
    /// CSI ignore — the rest of the sequence is discarded.
    CsiIgnore,
    /// CSI parameter — collecting `0x30..=0x3F` parameters.
    CsiParam,
    /// CSI sub-parameter — collecting `0x30..=0x3F` after a `:`.
    CsiSubParam,
    /// OSC parameter — collecting the numeric OSC command.
    OscParam,
    /// OSC string — collecting the OSC payload.
    OscString,
    /// OSC termination — saw ESC, awaiting the `\` of ST.
    OscTermination,
    /// SS3 entry — received `ESC O` for the input engine.
    Ss3Entry,
    /// SS3 parameter.
    Ss3Param,
    /// VT52 parameter — collecting the two bytes of a direct cursor address.
    Vt52Param,
    /// DCS entry — received `ESC P`.
    DcsEntry,
    /// DCS ignore — the rest of the DCS string is discarded.
    DcsIgnore,
    /// DCS intermediate — collecting intermediates before the final byte.
    DcsIntermediate,
    /// DCS parameter — collecting parameters before the final byte.
    DcsParam,
    /// DCS pass-through — handing data-string bytes to the engine's handler.
    DcsPassThrough,
    /// SOS/PM/APC string — ignored until the string terminator.
    SosPmApcString,
    /// APC string — its bytes are delivered to the engine (the kitty
    /// graphics protocol rides here), until ST.
    ApcString,
    /// APC termination — saw ESC, awaiting the `\` of ST.
    ApcTermination,
}

impl VtState {
    /// Returns the display name of this state.
    pub const fn name(self) -> &'static str {
        match self {
            VtState::Ground => "Ground",
            VtState::Escape => "Escape",
            VtState::EscapeIntermediate => "EscapeIntermediate",
            VtState::CsiEntry => "CsiEntry",
            VtState::CsiIntermediate => "CsiIntermediate",
            VtState::CsiIgnore => "CsiIgnore",
            VtState::CsiParam => "CsiParam",
            VtState::CsiSubParam => "CsiSubParam",
            VtState::OscParam => "OscParam",
            VtState::OscString => "OscString",
            VtState::OscTermination => "OscTermination",
            VtState::Ss3Entry => "Ss3Entry",
            VtState::Ss3Param => "Ss3Param",
            VtState::Vt52Param => "Vt52Param",
            VtState::DcsEntry => "DcsEntry",
            VtState::DcsIgnore => "DcsIgnore",
            VtState::DcsIntermediate => "DcsIntermediate",
            VtState::DcsParam => "DcsParam",
            VtState::DcsPassThrough => "DcsPassThrough",
            VtState::SosPmApcString => "SosPmApcString",
            VtState::ApcString => "ApcString",
            VtState::ApcTermination => "ApcTermination",
        }
    }
}

impl std::fmt::Display for VtState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}
