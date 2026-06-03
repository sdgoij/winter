//! ASCII constants used in VT escape sequence parsing.
//!
//! Ported from the ASCII constant definitions in the Windows Terminal C++ VT module.

/// Bell (BEL) — triggers notification.
pub const BEL: u8 = 0x07;

/// Backspace (BS) — moves cursor one column left.
pub const BS: u8 = 0x08;

/// Horizontal Tab (HT) — moves cursor to next tab stop.
pub const HT: u8 = 0x09;

/// Line Feed (LF) — moves cursor one line down.
pub const LF: u8 = 0x0A;

/// Vertical Tab (VT) — moves cursor one line down (legacy).
pub const VT: u8 = 0x0B;

/// Form Feed (FF) — moves cursor one page down (legacy).
pub const FF: u8 = 0x0C;

/// Carriage Return (CR) — moves cursor to column 0 of the current line.
pub const CR: u8 = 0x0D;

/// Escape (ESC) — initiates an escape sequence.
pub const ESC: u8 = 0x1B;

/// Cancel (CAN) — cancels the rest of the current sequence.
pub const CAN: u8 = 0x18;

/// Substitution (SUB) — substitutes invalid character.
pub const SUB: u8 = 0x1A;

/// Escape Indicator (SI) — switches to G0 character set.
pub const SI: u8 = 0x0F;

/// Shift In (SO) — switches to G1 character set.
pub const SO: u8 = 0x0E;

/// Pad Character (PAD).
pub const PAD: u8 = 0x80;

/// High-Negative Part (HNP).
pub const HNP: u8 = 0x81;

/// Low-Negative Part (LNP).
pub const LNP: u8 = 0x82;

/// High-Positive Part (HPP).
pub const HPP: u8 = 0x83;

/// Low-Positive Part (LPP).
pub const LPP: u8 = 0x84;

/// Private Character Entry (PCE).
pub const PCE: u8 = 0x85;

/// Device Control String (DCS) lead.
pub const DCS_LEAD: u8 = b'$';

/// String Terminator (ST) — single byte form.
pub const ST: u8 = 0x9C;

/// String Terminator alternative (ESC \\).
pub const ST_ESC: u8 = b'\\';

/// Introducer for OSC (Operating System Command).
pub const OSC_INTRODUCER: u8 = b']';

/// Introducer for APC (Application Program Command).
pub const APC_INTRODUCER: u8 = b'_';

/// Introducer for DCS (Device Control String).
pub const DCS_INTRODUCER: u8 = b'p';

/// Introducer for SS3 (Single Shift 3).
pub const SS3_INTRODUCER: u8 = b'O';

/// Introducer for APM (Application Message Program).
pub const APM_INTRODUCER: u8 = b'^';

/// First byte of a two-byte C1 sequence.
pub const C1_PREFIX: u8 = 0x90;

/// Last byte of a two-byte C1 sequence.
pub const C1_SUFFIX: u8 = 0x9F;

/// Check if a byte is a printable ASCII character.
#[inline]
pub const fn is_printable(c: u8) -> bool {
    c >= 0x20 && c <= 0x7E
}

/// Check if a byte is an ASCII control character.
#[inline]
pub const fn is_control(c: u8) -> bool {
    c < 0x20 || c == 0x7F
}

/// Check if a byte is a decimal digit.
#[inline]
pub const fn is_digit(c: u8) -> bool {
    c.is_ascii_digit()
}

/// Check if a byte is an intermediate character (0x20..=0x2F).
#[inline]
pub const fn is_intermediate(c: u8) -> bool {
    c >= 0x20 && c <= 0x2F
}

/// Check if a byte is a dispatch character for CSI (0x40..=0x7E).
#[inline]
pub const fn is_csi_dispatch(c: u8) -> bool {
    c >= 0x40 && c <= 0x7E
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_constants() {
        assert_eq!(BEL, 0x07);
        assert_eq!(BS, 0x08);
        assert_eq!(HT, 0x09);
        assert_eq!(LF, 0x0A);
        assert_eq!(CR, 0x0D);
        assert_eq!(ESC, 0x1B);
    }

    #[test]
    fn test_is_printable() {
        assert!(is_printable(b'A'));
        assert!(is_printable(b' '));
        assert!(is_printable(b'~'));
        assert!(!is_printable(0x00));
        assert!(!is_printable(0x1F));
        assert!(!is_printable(0x7F));
    }

    #[test]
    fn test_is_control() {
        assert!(is_control(0x00));
        assert!(is_control(0x1F));
        assert!(is_control(0x7F));
        assert!(!is_control(b'A'));
        assert!(!is_control(b' '));
    }

    #[test]
    fn test_is_digit() {
        assert!(is_digit(b'0'));
        assert!(is_digit(b'9'));
        assert!(!is_digit(b'a'));
        assert!(!is_digit(b' '));
    }

    #[test]
    fn test_is_intermediate() {
        assert!(is_intermediate(b' ')); // 0x20
        assert!(is_intermediate(b'/')); // 0x2F
        assert!(is_intermediate(b'!')); // 0x21, in range
        assert!(!is_intermediate(b'@')); // 0x40, not in range
        assert!(!is_intermediate(b'0')); // 0x30, not in range
    }

    #[test]
    fn test_is_csi_dispatch() {
        assert!(is_csi_dispatch(b'@')); // 0x40
        assert!(is_csi_dispatch(b'~')); // 0x7E
        assert!(is_csi_dispatch(b'{')); // 0x7B, in range 0x40..=0x7E
        assert!(!is_csi_dispatch(b'!')); // 0x21, not in range
    }

    #[test]
    fn test_osc_introducer() {
        assert_eq!(OSC_INTRODUCER, b']');
    }

    #[test]
    fn test_dcs_introducer() {
        assert_eq!(DCS_INTRODUCER, b'p');
    }
}
