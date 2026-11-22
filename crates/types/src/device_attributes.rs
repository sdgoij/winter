//! The DA1 device attributes, as the attribute ids a response carries.
//!
//! Ported from `DeviceAttribute` in `InputStateMachineEngine.hpp` and the
//! `til::enumset` upstream stores them in. A response may carry any id below
//! 64, so the portable form is a bit set rather than an enum: the named ids
//! are the ones upstream documents, and an unknown id is still representable.

/// A set of DA1 device attributes, one bit per attribute id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceAttributes(u64);

impl DeviceAttributes {
    /// Column count is 132.
    pub const COLUMNS_132: u64 = 1;
    /// A printer port is present.
    pub const PRINTER_PORT: u64 = 2;
    /// Sixel graphics.
    pub const SIXEL: u64 = 4;
    /// Selective erase.
    pub const SELECTIVE_ERASE: u64 = 6;
    /// Soft character sets.
    pub const SOFT_CHARACTER_SET: u64 = 7;
    /// User-defined keys.
    pub const USER_DEFINED_KEYS: u64 = 8;
    /// National replacement character sets.
    pub const NATIONAL_REPLACEMENT_CHARACTER_SETS: u64 = 9;
    /// Serbo-Croatian character set.
    pub const SERBO_CROATIAN_CHARACTER_SET: u64 = 12;
    /// 8-bit interface architecture.
    pub const EIGHT_BIT_INTERFACE_ARCHITECTURE: u64 = 14;
    /// Technical character set.
    pub const TECHNICAL_CHARACTER_SET: u64 = 15;
    /// Windowing capability.
    pub const WINDOWING_CAPABILITY: u64 = 18;
    /// Multiple sessions.
    pub const SESSIONS: u64 = 19;
    /// Horizontal scrolling.
    pub const HORIZONTAL_SCROLLING: u64 = 21;
    /// Colour.
    pub const COLOR: u64 = 22;
    /// Greek character set.
    pub const GREEK_CHARACTER_SET: u64 = 23;
    /// Turkish character set.
    pub const TURKISH_CHARACTER_SET: u64 = 24;
    /// Rectangular area operations.
    pub const RECTANGULAR_AREA_OPERATIONS: u64 = 28;
    /// Text macros.
    pub const TEXT_MACROS: u64 = 32;
    /// Latin-2 character set.
    pub const LATIN2_CHARACTER_SET: u64 = 42;
    /// PCTerm.
    pub const PCTERM: u64 = 44;
    /// Soft key mapping.
    pub const SOFT_KEY_MAPPING: u64 = 45;
    /// ASCII terminal emulation.
    pub const ASCII_TERMINAL_EMULATION: u64 = 46;

    /// Creates an empty set.
    pub const fn new() -> Self {
        Self(0)
    }

    /// Creates a set from raw bits.
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// Returns the raw bits.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Adds an attribute id; ids past 63 cannot be represented and are ignored.
    pub fn set(&mut self, attribute: u64) {
        if attribute < u64::BITS as u64 {
            self.0 |= 1 << attribute;
        }
    }

    /// Returns whether an attribute id is present.
    pub const fn contains(self, attribute: u64) -> bool {
        attribute < u64::BITS as u64 && self.0 & (1 << attribute) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_reports_the_ids_it_holds() {
        let mut attributes = DeviceAttributes::new();
        attributes.set(DeviceAttributes::COLOR);
        attributes.set(DeviceAttributes::RECTANGULAR_AREA_OPERATIONS);

        assert!(attributes.contains(DeviceAttributes::COLOR));
        assert!(attributes.contains(DeviceAttributes::RECTANGULAR_AREA_OPERATIONS));
        assert!(!attributes.contains(DeviceAttributes::SIXEL));
    }

    #[test]
    fn an_id_past_the_set_is_ignored() {
        let mut attributes = DeviceAttributes::new();
        attributes.set(64);
        assert_eq!(attributes, DeviceAttributes::new());
        assert!(!attributes.contains(64));
    }
}
