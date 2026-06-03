//! VT dispatch types — identifying the type of incoming VT sequence.
//!
//! Ported from VT dispatch type definitions in the Windows Terminal C++ module.

/// Identifies the type of VT sequence being dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VtId {
    /// Control Sequence Introducer (CSI) — e.g., ESC [ ...
    Csi,
    /// Operating System Command (OSC) — e.g., ESC ] ...
    Osc,
    /// Device Control String (DCS) — e.g., ESC P ...
    Dcs,
    /// Application Message Program (APM) — e.g., ESC ^ ...
    Apm,
    /// Single Shift 3 (SS3) — e.g., ESC O
    Ss3,
    /// Escape (ESC) — standalone ESC or two-byte escape
    Esc,
}

impl VtId {
    /// Returns true if this is a CSI sequence.
    pub const fn is_csi(self) -> bool {
        matches!(self, VtId::Csi)
    }

    /// Returns true if this is an OSC sequence.
    pub const fn is_osc(self) -> bool {
        matches!(self, VtId::Osc)
    }

    /// Returns true if this is a DCS sequence.
    pub const fn is_dcs(self) -> bool {
        matches!(self, VtId::Dcs)
    }

    /// Returns true if this is an APM sequence.
    pub const fn is_apm(self) -> bool {
        matches!(self, VtId::Apm)
    }

    /// Returns true if this is an SS3 sequence.
    pub const fn is_ss3(self) -> bool {
        matches!(self, VtId::Ss3)
    }

    /// Returns true if this is an ESC sequence.
    pub const fn is_esc(self) -> bool {
        matches!(self, VtId::Esc)
    }
}

/// Response bits for Device Attributes (DA1).
///
/// Corresponds to the VT100/VT220 DA1 response:
/// `CSI ? <p1> ; <p2> ; <p3> s`
/// where:
/// - p1: VT model number (e.g., 0 = VT100)
/// - p2: ROM version
/// - p3: options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceAttributesResponse {
    model_number: u32,
    rom_version: u32,
    options: DeviceOptions,
}

impl DeviceAttributesResponse {
    /// Creates a new device attributes response.
    pub const fn new(model_number: u32, rom_version: u32, options: DeviceOptions) -> Self {
        Self {
            model_number,
            rom_version,
            options,
        }
    }

    /// Gets the model number.
    pub const fn model_number(&self) -> u32 {
        self.model_number
    }

    /// Gets the ROM version.
    pub const fn rom_version(&self) -> u32 {
        self.rom_version
    }

    /// Gets the options.
    pub const fn options(&self) -> DeviceOptions {
        self.options
    }
}

/// Bitflags for terminal options in Device Attributes response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceOptions(u32);

impl DeviceOptions {
    /// Terminal has selectable character sets.
    pub const SELECTABLE_GLYPHS: u32 = 1 << 0;
    /// Terminal has vertical lines.
    pub const VERTICAL_LINE_SELECT: u32 = 1 << 1;
}

/// A complete VT sequence for dispatching.
#[derive(Debug, Clone)]
pub struct VtSequence {
    /// The type of the sequence.
    pub id: VtId,
    /// The parsed parameters.
    pub parameters: crate::parameters::VtParameters,
    /// The string payload (for OSC, DCS, APM).
    pub string: String,
}

impl VtSequence {
    /// Creates a new CSI sequence.
    pub fn new_csi(params: crate::parameters::VtParameters) -> Self {
        Self {
            id: VtId::Csi,
            parameters: params,
            string: String::new(),
        }
    }

    /// Creates a new OSC sequence.
    pub fn new_osc(params: crate::parameters::VtParameters, s: String) -> Self {
        Self {
            id: VtId::Osc,
            parameters: params,
            string: s,
        }
    }

    /// Creates a new DCS sequence.
    pub fn new_dcs(params: crate::parameters::VtParameters, s: String) -> Self {
        Self {
            id: VtId::Dcs,
            parameters: params,
            string: s,
        }
    }

    /// Creates a new SS3 sequence.
    pub fn new_ss3() -> Self {
        Self {
            id: VtId::Ss3,
            parameters: crate::parameters::VtParameters::new(),
            string: String::new(),
        }
    }

    /// Creates a new ESC sequence.
    pub fn new_esc() -> Self {
        Self {
            id: VtId::Esc,
            parameters: crate::parameters::VtParameters::new(),
            string: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vt_id_variants() {
        assert!(VtId::Csi.is_csi());
        assert!(VtId::Osc.is_osc());
        assert!(VtId::Dcs.is_dcs());
        assert!(VtId::Apm.is_apm());
        assert!(VtId::Ss3.is_ss3());
        assert!(VtId::Esc.is_esc());

        assert!(!VtId::Csi.is_osc());
        assert!(!VtId::Osc.is_csi());
    }

    #[test]
    fn test_device_attributes_response() {
        let response = DeviceAttributesResponse::new(100, 0, DeviceOptions(0));
        assert_eq!(response.model_number(), 100);
        assert_eq!(response.rom_version(), 0);
    }

    #[test]
    fn test_vt_sequence_csi() {
        let params = crate::parameters::VtParameters::from_string("1;2");
        let seq = VtSequence::new_csi(params);
        assert!(seq.id.is_csi());
    }

    #[test]
    fn test_vt_sequence_osc() {
        let params = crate::parameters::VtParameters::from_string("0");
        let seq = VtSequence::new_osc(params, "Title".to_string());
        assert!(seq.id.is_osc());
        assert_eq!(seq.string, "Title");
    }

    #[test]
    fn test_vt_sequence_ss3() {
        let seq = VtSequence::new_ss3();
        assert!(seq.id.is_ss3());
    }
}
