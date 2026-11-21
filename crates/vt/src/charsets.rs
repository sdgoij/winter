//! VT character set management.
//!
//! Ported from the character set handling in the Windows Terminal C++ VT module.
//! This is the parser-side G0-G3 manager; the 94/96-character translation
//! tables live in `types::charsets`.

/// The available VT character sets.
///
/// VT100 defined four character sets: G0, G1, G2, G3.
/// G0 is the default active set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Charset {
    /// Standard character set (SCS) — ASCII with some VT-specific graphics.
    #[default]
    Standard,
    /// Alternate character set A — DEC special graphics and line drawing.
    AlternateA,
    /// Alternate character set B — DEC special graphics and line drawing (variant).
    AlternateB,
    /// UK character set (DEC).
    Uk,
    /// Norwegian/Norwegian character set.
    Norwegian,
    /// Danish character set.
    Danish,
}

impl Charset {
    /// Returns the name of this charset.
    pub const fn name(self) -> &'static str {
        match self {
            Charset::Standard => "Standard",
            Charset::AlternateA => "AlternateA",
            Charset::AlternateB => "AlternateB",
            Charset::Uk => "UK",
            Charset::Norwegian => "Norwegian",
            Charset::Danish => "Danish",
        }
    }
}

/// Manages the active character sets (G0-G3) for the terminal.
#[derive(Debug, Clone)]
pub struct Charsets {
    /// G0 character set.
    g0: Charset,
    /// G1 character set.
    g1: Charset,
    /// G2 character set (VT220+).
    g2: Charset,
    /// G3 character set (VT220+).
    g3: Charset,
    /// Currently active character set (G0 or G1).
    active: Charset,
    /// Whether G1 is the active set (via SO).
    g1_active: bool,
}

impl Charsets {
    /// Creates a new charset manager with default (Standard) sets.
    pub fn new() -> Self {
        Self {
            g0: Charset::Standard,
            g1: Charset::Standard,
            g2: Charset::Standard,
            g3: Charset::Standard,
            active: Charset::Standard,
            g1_active: false,
        }
    }

    /// Gets the currently active character set.
    pub fn active(&self) -> Charset {
        if self.g1_active { self.g1 } else { self.g0 }
    }

    /// Assigns a charset to G0 (via ESC ( A-Z).
    pub fn assign_g0(&mut self, charset: Charset) {
        self.g0 = charset;
        if !self.g1_active {
            self.active = charset;
        }
    }

    /// Assigns a charset to G1 (via ESC ) A-Z).
    pub fn assign_g1(&mut self, charset: Charset) {
        self.g1 = charset;
        if self.g1_active {
            self.active = charset;
        }
    }

    /// Assigns a charset to G2 (VT220+).
    pub fn assign_g2(&mut self, charset: Charset) {
        self.g2 = charset;
    }

    /// Assigns a charset to G3 (VT220+).
    pub fn assign_g3(&mut self, charset: Charset) {
        self.g3 = charset;
    }

    /// Switches to G0 (SI — Shift In, 0x0F).
    pub fn switch_to_g0(&mut self) {
        self.g1_active = false;
        self.active = self.g0;
    }

    /// Switches to G1 (SO — Shift Out, 0x0E).
    pub fn switch_to_g1(&mut self) {
        self.g1_active = true;
        self.active = self.g1;
    }

    /// Resets all character sets to Standard.
    pub fn reset(&mut self) {
        self.g0 = Charset::Standard;
        self.g1 = Charset::Standard;
        self.g2 = Charset::Standard;
        self.g3 = Charset::Standard;
        self.active = Charset::Standard;
        self.g1_active = false;
    }

    /// Returns whether G1 is currently active.
    pub fn g1_active(&self) -> bool {
        self.g1_active
    }
}

impl Default for Charsets {
    fn default() -> Self {
        Self::new()
    }
}

/// Converts a charset designator character to a Charset variant.
///
/// VT designators:
/// - `A` → UK
/// - `B` → Standard (ASCII)
/// - `0`-`9`, `A`-`Z`, `a`-`z` → Alternate sets
pub fn designator_to_charset(c: char) -> Charset {
    match c {
        'A' => Charset::Uk,
        'B' => Charset::Standard,
        '0' => Charset::AlternateA,
        '1' => Charset::AlternateB,
        _ => Charset::Standard,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_charset() {
        let cs = Charsets::new();
        assert_eq!(cs.active(), Charset::Standard);
        assert!(!cs.g1_active());
    }

    #[test]
    fn test_assign_g0() {
        let mut cs = Charsets::new();
        cs.assign_g0(Charset::AlternateA);
        assert_eq!(cs.active(), Charset::AlternateA);
    }

    #[test]
    fn test_assign_g1() {
        let mut cs = Charsets::new();
        cs.assign_g1(Charset::AlternateA);
        // Not active until SO
        assert_eq!(cs.active(), Charset::Standard);

        cs.switch_to_g1();
        assert_eq!(cs.active(), Charset::AlternateA);
        assert!(cs.g1_active());
    }

    #[test]
    fn test_switch_to_g0() {
        let mut cs = Charsets::new();
        cs.switch_to_g1();
        cs.switch_to_g0();
        assert_eq!(cs.active(), Charset::Standard);
        assert!(!cs.g1_active());
    }

    #[test]
    fn test_switch_to_g1() {
        let mut cs = Charsets::new();
        cs.switch_to_g1();
        assert_eq!(cs.active(), Charset::Standard);
        assert!(cs.g1_active());
    }

    #[test]
    fn test_reset() {
        let mut cs = Charsets::new();
        cs.assign_g0(Charset::AlternateA);
        cs.assign_g1(Charset::AlternateB);
        cs.switch_to_g1();
        cs.reset();
        assert_eq!(cs.active(), Charset::Standard);
        assert!(!cs.g1_active());
    }

    #[test]
    fn test_designator_to_charset() {
        assert_eq!(designator_to_charset('A'), Charset::Uk);
        assert_eq!(designator_to_charset('B'), Charset::Standard);
        assert_eq!(designator_to_charset('0'), Charset::AlternateA);
        assert_eq!(designator_to_charset('1'), Charset::AlternateB);
    }

    #[test]
    fn test_charset_names() {
        assert_eq!(Charset::Standard.name(), "Standard");
        assert_eq!(Charset::AlternateA.name(), "AlternateA");
        assert_eq!(Charset::Uk.name(), "UK");
    }
}
