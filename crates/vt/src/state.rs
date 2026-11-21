//! VT state machine states.
//!
//! Ported from the state definitions in the Windows Terminal C++ VT module.
//! The VT parser is a DFA with 19 states.

/// The 19 states of the VT state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VtState {
    /// Ground state — normal character processing.
    #[default]
    Ground,
    /// Prefix state — received a C1 prefix (0x90-0x9F).
    Prefix,
    /// Escape state — received ESC (0x1B), awaiting next byte.
    Escape,
    /// Escape first — received ESC followed by a prefix byte.
    EscapeFirst,
    /// DPAD state — parsing a two-byte sequence (DPCP/DPAF).
    Dpad,
    /// DCS param state — parsing DCS parameter bytes.
    DcsParam,
    /// DCS string state — inside a DCS string.
    DcsStr,
    /// DCS passthrough — DCS string content (not accumulating).
    DcsPassthrough,
    /// CSI intermediate — parsing CSI intermediate bytes.
    CsiIntermediate,
    /// CSI param state — parsing CSI parameter bytes.
    CsiParam,
    /// CSI ignore state — inside a CSI ignore sequence.
    CsiIgnore,
    /// OSC string state — inside an OSC string.
    OscStr,
    /// OSC passthrough — OSC content passthrough.
    OscPassthrough,
    /// SS3 ignore state.
    Ss3Ignore,
    /// SS3 param state.
    Ss3Param,
    /// APM ignore state.
    ApmIgnore,
    /// APM string state.
    ApmStr,
    /// APM passthrough state.
    ApmPassthrough,
    /// Error state — unrecoverable error.
    Error,
}

impl VtState {
    /// Returns the display name of this state.
    pub const fn name(self) -> &'static str {
        match self {
            VtState::Ground => "Ground",
            VtState::Prefix => "Prefix",
            VtState::Escape => "Escape",
            VtState::EscapeFirst => "EscapeFirst",
            VtState::Dpad => "Dpad",
            VtState::DcsParam => "DcsParam",
            VtState::DcsStr => "DcsStr",
            VtState::DcsPassthrough => "DcsPassthrough",
            VtState::CsiIntermediate => "CsiIntermediate",
            VtState::CsiParam => "CsiParam",
            VtState::CsiIgnore => "CsiIgnore",
            VtState::OscStr => "OscStr",
            VtState::OscPassthrough => "OscPassthrough",
            VtState::Ss3Ignore => "Ss3Ignore",
            VtState::Ss3Param => "Ss3Param",
            VtState::ApmIgnore => "ApmIgnore",
            VtState::ApmStr => "ApmStr",
            VtState::ApmPassthrough => "ApmPassthrough",
            VtState::Error => "Error",
        }
    }
}

impl std::fmt::Display for VtState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}
