//! Fixed-size enum bitmask backed by u64.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::enumset`.

/// A bitmask enum set backed by a u64.
///
/// Ported from `til::enumset<T>` in the Windows Terminal C++ codebase.
/// Uses position N in the bitmask corresponding to value N of the enum type.
/// Supports up to 64 positions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct EnumSet {
    bits: u64,
}

impl EnumSet {
    /// Creates an empty enum set.
    #[inline]
    pub const fn new() -> Self {
        Self { bits: 0 }
    }

    /// Creates an enum set from raw bits.
    #[inline]
    pub const fn from_bits(bits: u64) -> Self {
        Self { bits }
    }

    /// Returns the underlying bits.
    #[inline]
    pub const fn bits(&self) -> u64 {
        self.bits
    }

    /// Returns whether the bit at the given position is set.
    #[inline]
    pub const fn contains(&self, pos: u64) -> bool {
        (self.bits & (1u64 << pos)) != 0
    }

    /// Returns whether any bit is set.
    #[inline]
    pub const fn is_any(&self) -> bool {
        self.bits != 0
    }

    /// Returns whether all bits are set.
    #[inline]
    pub const fn is_all(&self) -> bool {
        self.bits == u64::MAX
    }

    /// Sets the bit at the given position.
    #[inline]
    pub fn set(&mut self, pos: u64) -> &mut Self {
        self.bits |= 1u64 << pos;
        self
    }

    /// Unsets the bit at the given position.
    #[inline]
    pub fn unset(&mut self, pos: u64) -> &mut Self {
        self.bits &= !(1u64 << pos);
        self
    }

    /// Flips the bit at the given position.
    #[inline]
    pub fn flip(&mut self, pos: u64) -> &mut Self {
        self.bits ^= 1u64 << pos;
        self
    }

    /// Returns whether all of the given positions are set.
    #[inline]
    pub fn all_of(&self, positions: &[u64]) -> bool {
        let mask: u64 = positions.iter().fold(0, |acc, &p| acc | (1u64 << p));
        (self.bits & mask) == mask
    }

    /// Returns whether any of the given positions are set.
    #[inline]
    pub fn any_of(&self, positions: &[u64]) -> bool {
        let mask: u64 = positions.iter().fold(0, |acc, &p| acc | (1u64 << p));
        (self.bits & mask) != 0
    }

    /// Returns the union of this set and another.
    #[inline]
    pub fn union(&self, other: EnumSet) -> EnumSet {
        EnumSet::from_bits(self.bits | other.bits)
    }

    /// Returns the intersection of this set and another.
    #[inline]
    pub fn intersection(&self, other: EnumSet) -> EnumSet {
        EnumSet::from_bits(self.bits & other.bits)
    }

    /// Returns the difference of this set minus another.
    #[inline]
    pub fn difference(&self, other: EnumSet) -> EnumSet {
        EnumSet::from_bits(self.bits & !other.bits)
    }

    /// Returns the symmetric difference of this set and another.
    #[inline]
    pub fn symmetric_difference(&self, other: EnumSet) -> EnumSet {
        EnumSet::from_bits(self.bits ^ other.bits)
    }

    /// Returns whether this set is a subset of another.
    #[inline]
    pub fn is_subset(&self, other: EnumSet) -> bool {
        (self.bits & other.bits) == self.bits
    }

    /// Returns whether this set is a superset of another.
    #[inline]
    pub fn is_superset(&self, other: EnumSet) -> bool {
        other.is_subset(*self)
    }

    /// Returns an iterator over the set bits.
    pub fn iter(&self) -> EnumSetIter {
        EnumSetIter {
            bits: self.bits,
            current: 0,
        }
    }
}

impl std::ops::BitOr for EnumSet {
    type Output = EnumSet;

    fn bitor(self, other: EnumSet) -> EnumSet {
        self.union(other)
    }
}

impl std::ops::BitOrAssign for EnumSet {
    fn bitor_assign(&mut self, other: EnumSet) {
        self.bits |= other.bits;
    }
}

impl std::ops::BitAnd for EnumSet {
    type Output = EnumSet;

    fn bitand(self, other: EnumSet) -> EnumSet {
        self.intersection(other)
    }
}

impl std::ops::BitAndAssign for EnumSet {
    fn bitand_assign(&mut self, other: EnumSet) {
        self.bits &= other.bits;
    }
}

impl std::ops::BitXor for EnumSet {
    type Output = EnumSet;

    fn bitxor(self, other: EnumSet) -> EnumSet {
        self.symmetric_difference(other)
    }
}

impl std::ops::BitXorAssign for EnumSet {
    fn bitxor_assign(&mut self, other: EnumSet) {
        self.bits ^= other.bits;
    }
}

impl std::ops::Not for EnumSet {
    type Output = EnumSet;

    fn not(self) -> EnumSet {
        EnumSet::from_bits(!self.bits)
    }
}

/// Iterator over set bits.
#[derive(Debug)]
pub struct EnumSetIter {
    bits: u64,
    current: u64,
}

impl Iterator for EnumSetIter {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        while self.current < 64 {
            let bit = 1u64 << self.current;
            self.current += 1;
            if (self.bits & bit) != 0 {
                return Some(self.current - 1);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_empty() {
        let set = EnumSet::new();
        assert_eq!(set.bits(), 0);
        assert!(!set.is_any());
        assert!(!set.is_all());
    }

    #[test]
    fn test_set_get() {
        let mut set = EnumSet::new();
        assert!(!set.contains(0));
        assert!(!set.contains(1));

        set.set(0);
        assert!(set.contains(0));
        assert!(!set.contains(1));

        set.set(1);
        assert!(set.contains(0));
        assert!(set.contains(1));
        assert_eq!(set.bits(), 0b11);
    }

    #[test]
    fn test_unset() {
        let mut set = EnumSet::from_bits(0b101);
        set.unset(0);
        assert_eq!(set.bits(), 0b100);
        assert!(!set.contains(0));
        assert!(set.contains(2));
    }

    #[test]
    fn test_flip() {
        let mut set = EnumSet::from_bits(0b101);
        set.flip(1);
        assert_eq!(set.bits(), 0b111);

        set.flip(1);
        assert_eq!(set.bits(), 0b101);
    }

    #[test]
    fn test_union() {
        let a = EnumSet::from_bits(0b011);
        let b = EnumSet::from_bits(0b110);
        let c = a.union(b);
        assert_eq!(c.bits(), 0b111);
    }

    #[test]
    fn test_intersection() {
        let a = EnumSet::from_bits(0b111);
        let b = EnumSet::from_bits(0b110);
        let c = a.intersection(b);
        assert_eq!(c.bits(), 0b110);
    }

    #[test]
    fn test_difference() {
        let a = EnumSet::from_bits(0b111);
        let b = EnumSet::from_bits(0b010);
        let c = a.difference(b);
        assert_eq!(c.bits(), 0b101);
    }

    #[test]
    fn test_symmetric_difference() {
        let a = EnumSet::from_bits(0b101);
        let b = EnumSet::from_bits(0b110);
        let c = a.symmetric_difference(b);
        assert_eq!(c.bits(), 0b011);
    }

    #[test]
    fn test_subset() {
        let a = EnumSet::from_bits(0b011);
        let b = EnumSet::from_bits(0b111);
        assert!(a.is_subset(b));
        assert!(!b.is_subset(a));
        assert!(b.is_superset(a));
        assert!(!a.is_superset(b));
    }

    #[test]
    fn test_all_of_any_of() {
        let set = EnumSet::from_bits(0b101);
        assert!(set.all_of(&[0, 2]));
        assert!(!set.all_of(&[0, 1, 2]));
        assert!(set.any_of(&[0, 1]));
        assert!(!set.any_of(&[4, 5]));
    }

    #[test]
    fn test_not() {
        let set = EnumSet::from_bits(0b0000_0000_0000_0001);
        let not_set = !set;
        assert_eq!(not_set.bits(), 0xFFFF_FFFF_FFFF_FFFE);
    }

    #[test]
    fn test_iter() {
        let set = EnumSet::from_bits(0b10101);
        let bits: Vec<u64> = set.iter().collect();
        assert_eq!(bits, vec![0, 2, 4]);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_union_with_intersection(a in any::<u64>(), b in any::<u64>()) {
            let set_a = EnumSet::from_bits(a);
            let set_b = EnumSet::from_bits(b);
            let union = set_a.union(set_b);
            let intersection = set_a.intersection(set_b);
            assert!(intersection.is_subset(union));
        }

        #[test]
        fn test_intersection_commutative(a in any::<u64>(), b in any::<u64>()) {
            let set_a = EnumSet::from_bits(a);
            let set_b = EnumSet::from_bits(b);
            let ab = set_a.intersection(set_b);
            let ba = set_b.intersection(set_a);
            assert_eq!(ab, ba);
        }

        #[test]
        fn test_union_identity(set in any::<u64>()) {
            let s = EnumSet::from_bits(set);
            let empty = EnumSet::new();
            assert_eq!(s.union(empty), s);
            assert_eq!(empty.union(s), s);
        }
    }
}
