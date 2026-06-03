//! ColorBrewer color palettes.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::colorbrewer`.
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! Copyright (c) 2002 Cynthia Brewer, Mark Harrower, and The Pennsylvania State University.

/// Pastel1 palette — 10 pastel colors as RGB u32 (BBGGRR layout).
pub const PASTEL1: [u32; 10] = [
    0xfbb4ae, 0xb3cde3, 0xccebc5, 0xdecbe4, 0xfed9a6, 0xffffcc, 0xe5d8bd, 0xfddaec, 0xf2f2f2,
    0xffffff, // Added a 10th
];

/// Dark2 palette — 8 darker colors as RGB u32 (BBGGRR layout).
pub const DARK2: [u32; 9] = [
    0x1b9e77, 0xd95f02, 0x7570b3, 0xe7298a, 0x66a61e, 0xe6ab02, 0xa6761d, 0x666666,
    0x000000, // Added a 9th (black)
];

/// Returns the color at the given index in the Pastel1 palette.
#[inline]
pub fn pastel1_at(index: usize) -> Option<u32> {
    PASTEL1.get(index).copied()
}

/// Returns the color at the given index in the Dark2 palette.
#[inline]
pub fn dark2_at(index: usize) -> Option<u32> {
    DARK2.get(index).copied()
}

/// Returns the number of colors in the Pastel1 palette.
#[inline]
pub fn pastel1_len() -> usize {
    PASTEL1.len()
}

/// Returns the number of colors in the Dark2 palette.
#[inline]
pub fn dark2_len() -> usize {
    DARK2.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pastel1_len() {
        assert_eq!(pastel1_len(), 10);
    }

    #[test]
    fn test_dark2_len() {
        assert_eq!(dark2_len(), 9);
    }

    #[test]
    fn test_pastel1_access() {
        assert!(pastel1_at(0).is_some());
        assert!(pastel1_at(9).is_some());
        assert!(pastel1_at(10).is_none());
    }

    #[test]
    fn test_dark2_access() {
        assert!(dark2_at(0).is_some());
        assert!(dark2_at(8).is_some());
        assert!(dark2_at(9).is_none());
    }
}
