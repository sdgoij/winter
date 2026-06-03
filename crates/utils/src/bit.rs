//! Bit manipulation utilities.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::bit`.

/// Number of set bits (population count).
#[inline]
pub fn popcount<T>(x: T) -> u32
where
    T: UnsignedInteger,
{
    x.count_ones()
}

/// Number of zero bits.
#[inline]
pub fn count_zeros<T>(x: T) -> u32
where
    T: UnsignedInteger,
{
    x.count_zeros()
}

/// Number of leading zero bits.
#[inline]
pub fn count_leading_zeros<T>(x: T) -> u32
where
    T: UnsignedInteger,
{
    x.leading_zeros()
}

/// Number of trailing zero bits.
#[inline]
pub fn count_trailing_zeros<T>(x: T) -> u32
where
    T: UnsignedInteger,
{
    x.trailing_zeros()
}

/// Reverses the bits.
#[inline]
pub fn reverse_bits<T>(x: T) -> T
where
    T: UnsignedInteger,
{
    x.reverse_bits()
}

/// Swaps bytes (for endianness conversion).
#[inline]
pub fn byte_swap<T>(x: T) -> T
where
    T: UnsignedInteger,
{
    x.swap_bytes()
}

/// Checks if a value is a power of two.
#[inline]
pub fn is_power_of_two<T>(x: T) -> bool
where
    T: UnsignedInteger + std::ops::BitAnd<Output = T> + std::ops::Sub<Output = T> + PartialEq,
{
    x != T::zero() && (x & (x - T::one())) == T::zero()
}

/// Rounds a value up to the next power of two.
/// Returns the value itself if it's already a power of two.
#[inline]
pub fn next_power_of_two<T>(x: T) -> T
where
    T: UnsignedInteger + std::ops::Shl<u32, Output = T> + PartialEq,
{
    if x == T::zero() {
        return T::one();
    }
    if is_power_of_two(x) {
        return x;
    }
    T::one() << (T::BITS - x.leading_zeros())
}

/// Rounds a value down to the previous power of two.
#[inline]
pub fn floor_power_of_two<T>(x: T) -> T
where
    T: UnsignedInteger + std::ops::Shl<u32, Output = T> + PartialEq,
{
    if x == T::zero() {
        return T::zero();
    }
    T::one() << (T::BITS - <u32 as UnsignedInteger>::one() - x.leading_zeros())
}

/// Trait for unsigned integer types.
pub trait UnsignedInteger:
    std::ops::BitAnd<Output = Self>
    + std::ops::Sub<Output = Self>
    + std::ops::Shl<u32, Output = Self>
    + std::iter::Sum<Self>
    + Copy
    + Clone
    + Default
    + PartialEq
{
    const BITS: u32;

    fn count_ones(self) -> u32;
    fn count_zeros(self) -> u32;
    fn leading_zeros(self) -> u32;
    fn trailing_zeros(self) -> u32;
    fn reverse_bits(self) -> Self;
    fn swap_bytes(self) -> Self;
    fn one() -> Self;
    fn zero() -> Self;
    fn from_u64(v: u64) -> Self;
}

impl UnsignedInteger for u8 {
    const BITS: u32 = 8;
    fn count_ones(self) -> u32 {
        u8::count_ones(self)
    }
    fn count_zeros(self) -> u32 {
        u8::count_zeros(self)
    }
    fn leading_zeros(self) -> u32 {
        u8::leading_zeros(self)
    }
    fn trailing_zeros(self) -> u32 {
        u8::trailing_zeros(self)
    }
    fn reverse_bits(self) -> Self {
        u8::reverse_bits(self)
    }
    fn swap_bytes(self) -> Self {
        u8::swap_bytes(self)
    }
    fn one() -> Self {
        1
    }
    fn zero() -> Self {
        0
    }
    fn from_u64(v: u64) -> Self {
        v as Self
    }
}

impl UnsignedInteger for u16 {
    const BITS: u32 = 16;
    fn count_ones(self) -> u32 {
        u16::count_ones(self)
    }
    fn count_zeros(self) -> u32 {
        u16::count_zeros(self)
    }
    fn leading_zeros(self) -> u32 {
        u16::leading_zeros(self)
    }
    fn trailing_zeros(self) -> u32 {
        u16::trailing_zeros(self)
    }
    fn reverse_bits(self) -> Self {
        u16::reverse_bits(self)
    }
    fn swap_bytes(self) -> Self {
        u16::swap_bytes(self)
    }
    fn one() -> Self {
        1
    }
    fn zero() -> Self {
        0
    }
    fn from_u64(v: u64) -> Self {
        v as Self
    }
}

impl UnsignedInteger for u32 {
    const BITS: u32 = 32;
    fn count_ones(self) -> u32 {
        u32::count_ones(self)
    }
    fn count_zeros(self) -> u32 {
        u32::count_zeros(self)
    }
    fn leading_zeros(self) -> u32 {
        u32::leading_zeros(self)
    }
    fn trailing_zeros(self) -> u32 {
        u32::trailing_zeros(self)
    }
    fn reverse_bits(self) -> Self {
        u32::reverse_bits(self)
    }
    fn swap_bytes(self) -> Self {
        u32::swap_bytes(self)
    }
    fn one() -> Self {
        1
    }
    fn zero() -> Self {
        0
    }
    fn from_u64(v: u64) -> Self {
        v as Self
    }
}

impl UnsignedInteger for u64 {
    const BITS: u32 = 64;
    fn count_ones(self) -> u32 {
        u64::count_ones(self)
    }
    fn count_zeros(self) -> u32 {
        u64::count_zeros(self)
    }
    fn leading_zeros(self) -> u32 {
        u64::leading_zeros(self)
    }
    fn trailing_zeros(self) -> u32 {
        u64::trailing_zeros(self)
    }
    fn reverse_bits(self) -> Self {
        u64::reverse_bits(self)
    }
    fn swap_bytes(self) -> Self {
        u64::swap_bytes(self)
    }
    fn one() -> Self {
        1
    }
    fn zero() -> Self {
        0
    }
    fn from_u64(v: u64) -> Self {
        v
    }
}

/// As unsigned version — performs sign-extension-free conversion.
#[inline]
pub fn as_unsigned<T>(v: T) -> <T as SignedInteger>::Unsigned
where
    T: SignedInteger,
{
    // SAFETY: We cast through u64 which preserves the bit pattern
    // The to_bits() returns the raw u64 representation
    <T as SignedInteger>::Unsigned::from_u64(T::to_bits(v))
}

/// Trait for signed integer types.
pub trait SignedInteger {
    type Unsigned: UnsignedInteger;

    fn to_bits(self) -> u64;
}

impl SignedInteger for i8 {
    type Unsigned = u8;
    fn to_bits(self) -> u64 {
        self as u8 as u64
    }
}

impl SignedInteger for i16 {
    type Unsigned = u16;
    fn to_bits(self) -> u64 {
        self as u16 as u64
    }
}

impl SignedInteger for i32 {
    type Unsigned = u32;
    fn to_bits(self) -> u64 {
        self as u32 as u64
    }
}

impl SignedInteger for i64 {
    type Unsigned = u64;
    fn to_bits(self) -> u64 {
        self as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_popcount() {
        assert_eq!(popcount(0u8), 0);
        assert_eq!(popcount(1u8), 1);
        assert_eq!(popcount(0xFFu8), 8);
        assert_eq!(popcount(0b1010_1010u8), 4);
    }

    #[test]
    fn test_count_zeros() {
        assert_eq!(count_zeros(0u8), 8);
        assert_eq!(count_zeros(1u8), 7);
        assert_eq!(count_zeros(0xFFu8), 0);
    }

    #[test]
    fn test_reverse_bits() {
        let x: u8 = 0b1010_0001;
        let reversed = reverse_bits(x);
        assert_eq!(reversed, 0b1000_0101); // Rust stdlib reverse_bits result
        assert_eq!(reverse_bits(reverse_bits(x)), x);
    }

    #[test]
    fn test_reverse_bits_various() {
        assert_eq!(reverse_bits(0u8), 0u8);
        assert_eq!(reverse_bits(1u8), 0b1000_0000u8);
        assert_eq!(reverse_bits(0xFFu8), 0xFFu8);
        assert_eq!(reverse_bits(0b1100_0000u8), 0b0000_0011u8);
    }

    #[test]
    fn test_byte_swap() {
        let x: u16 = 0x1234;
        assert_eq!(byte_swap(x), 0x3412);
    }

    #[test]
    fn test_is_power_of_two() {
        assert!(is_power_of_two(1u8));
        assert!(is_power_of_two(2u8));
        assert!(is_power_of_two(4u8));
        assert!(is_power_of_two(8u8));
        assert!(!is_power_of_two(0u8));
        assert!(!is_power_of_two(3u8));
        assert!(!is_power_of_two(6u8));
    }

    #[test]
    fn test_next_power_of_two() {
        assert_eq!(next_power_of_two(0u8), 1);
        assert_eq!(next_power_of_two(1u8), 1);
        assert_eq!(next_power_of_two(2u8), 2);
        assert_eq!(next_power_of_two(3u8), 4);
        assert_eq!(next_power_of_two(8u8), 8);
        assert_eq!(next_power_of_two(9u8), 16);
    }

    #[test]
    fn test_floor_power_of_two() {
        assert_eq!(floor_power_of_two(0u8), 0);
        assert_eq!(floor_power_of_two(1u8), 1);
        assert_eq!(floor_power_of_two(3u8), 2);
        assert_eq!(floor_power_of_two(8u8), 8);
        assert_eq!(floor_power_of_two(9u8), 8);
    }

    #[test]
    fn test_as_unsigned() {
        assert_eq!(as_unsigned(-1i8), 255u8);
        assert_eq!(as_unsigned(1i8), 1u8);
        assert_eq!(as_unsigned(0i32), 0u32);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_reverse_bits_double(x in any::<u32>()) {
            assert_eq!(reverse_bits(reverse_bits(x)), x);
        }

        #[test]
        fn test_popcount_matches_std(x in any::<u32>()) {
            assert_eq!(popcount(x), x.count_ones());
        }

        #[test]
        fn test_is_power_of_two_property(x in 1u32..=u32::MAX) {
            if is_power_of_two(x) {
                assert!(x.trailing_zeros() > 0 || x == 1);
            }
        }
    }
}
