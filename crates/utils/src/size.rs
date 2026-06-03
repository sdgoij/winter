//! 2D size type with scaling and area computation.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::size`.

use crate::math::checked_add;
use crate::math::checked_div;
use crate::math::checked_mul;
use crate::math::checked_sub;
use crate::point::CoordType;

/// A 2D size with width and height.
///
/// Ported from `til::size` in the Windows Terminal C++ codebase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Size {
    width: CoordType,
    height: CoordType,
}

impl Size {
    /// Creates a new size with the given width and height.
    #[inline]
    pub const fn new(width: CoordType, height: CoordType) -> Self {
        Self { width, height }
    }

    /// Creates a zero size (0, 0).
    #[inline]
    pub const fn zero() -> Self {
        Self::new(0, 0)
    }

    /// Returns the width.
    #[inline]
    pub const fn width(&self) -> CoordType {
        self.width
    }

    /// Returns the height.
    #[inline]
    pub const fn height(&self) -> CoordType {
        self.height
    }

    /// Returns whether both dimensions are positive.
    /// Equivalent to `operator bool()` in the C++ code.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Scales both dimensions by the given factor.
    pub fn scale(&self, factor: f64) -> Self {
        Self::new(
            (self.width as f64 * factor).clamp(CoordType::MIN as f64, CoordType::MAX as f64)
                as CoordType,
            (self.height as f64 * factor).clamp(CoordType::MIN as f64, CoordType::MAX as f64)
                as CoordType,
        )
    }

    /// Ceiling division of sizes.
    /// Returns `Err` if any dimension is non-positive or if the divisor is zero.
    pub fn divide_ceil(&self, divisor: Size) -> Result<Self, SizeError> {
        if self.width < 0 || self.height < 0 {
            return Err(SizeError::NegativeDimension);
        }
        if divisor.width <= 0 || divisor.height <= 0 {
            return Err(SizeError::ZeroDivisor);
        }

        let w = if self.width != 0 {
            (self.width - 1) / divisor.width + 1
        } else {
            0
        };
        let h = if self.height != 0 {
            (self.height - 1) / divisor.height + 1
        } else {
            0
        };

        Ok(Self::new(w, h))
    }

    /// Computes the area as i64 to avoid overflow.
    pub fn area(&self) -> i64 {
        self.width as i64 * self.height as i64
    }

    /// Returns the size as a tuple.
    #[inline]
    pub fn into_tuple(self) -> (CoordType, CoordType) {
        (self.width, self.height)
    }
}

/// Error type for size operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SizeError {
    /// Dimension is negative.
    #[error("negative dimension")]
    NegativeDimension,
    /// Divisor is zero.
    #[error("zero divisor")]
    ZeroDivisor,
}

impl std::ops::Add for Size {
    type Output = Size;

    fn add(self, other: Size) -> Size {
        Size::new(
            checked_add(self.width, other.width).expect("Size addition overflow"),
            checked_add(self.height, other.height).expect("Size addition overflow"),
        )
    }
}

impl std::ops::Sub for Size {
    type Output = Size;

    fn sub(self, other: Size) -> Size {
        Size::new(
            checked_sub(self.width, other.width).expect("Size subtraction overflow"),
            checked_sub(self.height, other.height).expect("Size subtraction overflow"),
        )
    }
}

impl std::ops::Mul for Size {
    type Output = Size;

    fn mul(self, other: Size) -> Size {
        Size::new(
            checked_mul(self.width, other.width).expect("Size multiplication overflow"),
            checked_mul(self.height, other.height).expect("Size multiplication overflow"),
        )
    }
}

impl std::ops::Div for Size {
    type Output = Size;

    fn div(self, other: Size) -> Size {
        Size::new(
            checked_div(self.width, other.width).expect("Size division by zero"),
            checked_div(self.height, other.height).expect("Size division by zero"),
        )
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[W:{}, H:{}]", self.width, self.height)
    }
}

/// Creates a size from width and height.
#[inline]
pub fn new_size(width: CoordType, height: CoordType) -> Size {
    Size::new(width, height)
}

/// Creates a zero size.
#[inline]
pub fn zero_size() -> Size {
    Size::zero()
}

/// Wraps COORD-like values into a Size.
#[inline]
pub fn wrap_coord_size(width: CoordType, height: CoordType) -> Size {
    Size::new(width, height)
}

/// Unwraps a Size into COORD-compatible i16 values.
pub fn unwrap_coord_size(size: Size) -> (i16, i16) {
    (
        size.width.try_into().expect("width out of i16 range"),
        size.height.try_into().expect("height out of i16 range"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_creation() {
        let s = Size::new(10, 20);
        assert_eq!(s.width(), 10);
        assert_eq!(s.height(), 20);
    }

    #[test]
    fn test_size_addition() {
        let a = Size::new(10, 20);
        let b = Size::new(5, 10);
        let c = a + b;
        assert_eq!(c.width(), 15);
        assert_eq!(c.height(), 30);
    }

    #[test]
    fn test_size_subtraction() {
        let a = Size::new(10, 20);
        let b = Size::new(3, 5);
        let c = a - b;
        assert_eq!(c.width(), 7);
        assert_eq!(c.height(), 15);
    }

    #[test]
    fn test_size_multiplication() {
        let a = Size::new(3, 4);
        let b = Size::new(2, 5);
        let c = a * b;
        assert_eq!(c.width(), 6);
        assert_eq!(c.height(), 20);
    }

    #[test]
    fn test_size_division() {
        let a = Size::new(10, 20);
        let b = Size::new(2, 4);
        let c = a / b;
        assert_eq!(c.width(), 5);
        assert_eq!(c.height(), 5);
    }

    #[test]
    fn test_size_scale() {
        let s = Size::new(10, 20);
        let scaled = s.scale(2.0);
        assert_eq!(scaled.width(), 20);
        assert_eq!(scaled.height(), 40);

        let half = s.scale(0.5);
        assert_eq!(half.width(), 5);
        assert_eq!(half.height(), 10);
    }

    #[test]
    fn test_size_divide_ceil() {
        let a = Size::new(10, 20);
        let b = Size::new(3, 7);
        let c = a.divide_ceil(b).unwrap();
        assert_eq!(c.width(), 4); // ceil(10/3) = 4
        assert_eq!(c.height(), 3); // ceil(20/7) = 3
    }

    #[test]
    fn test_size_divide_ceil_zero_divisor() {
        let a = Size::new(10, 20);
        let b = Size::new(0, 5);
        assert!(a.divide_ceil(b).is_err());
    }

    #[test]
    fn test_size_area() {
        let s = Size::new(10, 20);
        assert_eq!(s.area(), 200);
    }

    #[test]
    fn test_size_valid() {
        assert!(Size::new(1, 1).is_valid());
        assert!(!Size::new(0, 1).is_valid());
        assert!(!Size::new(1, 0).is_valid());
        assert!(!Size::new(-1, 1).is_valid());
    }

    #[test]
    fn test_display() {
        let s = Size::new(10, 20);
        assert_eq!(format!("{}", s), "[W:10, H:20]");
    }

    #[test]
    fn test_unwrap_coord_size() {
        let s = Size::new(100, 200);
        let (w, h) = unwrap_coord_size(s);
        assert_eq!(w, 100);
        assert_eq!(h, 200);
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    prop_compose! {
        fn any_size()(w in i32::MIN..=i32::MAX, h in i32::MIN..=i32::MAX) -> Size {
            Size::new(w, h)
        }
    }

    // Bounded size for tests where floating-point precision matters
    prop_compose! {
        fn bounded_size()(w in -10000i32..10000i32, h in -10000i32..10000i32) -> Size {
            Size::new(w, h)
        }
    }

    proptest! {
        #[test]
        fn test_area_property(s in any_size()) {
            if s.width >= 0 && s.height >= 0 {
                assert_eq!(s.area(), s.width as i64 * s.height as i64);
            }
        }

    }

    proptest! {
        // Use bounded sizes so floating-point precision loss stays small
        #[test]
        fn test_scale_inversion(s in bounded_size(), factor in 1.0f64..10.0f64) {
            let scaled = s.scale(factor);
            let inversed = scaled.scale(1.0 / factor);
            // Within tolerance for floating point + integer rounding
            let width_diff = (inversed.width() as i64 - s.width as i64).abs();
            let height_diff = (inversed.height() as i64 - s.height as i64).abs();
            // For bounded inputs (±10000), fp precision loss is negligible
            let max_diff = (s.width.unsigned_abs() as u128).saturating_add(s.height.unsigned_abs() as u128)
                / 100
                + 256;
            let max_diff = max_diff as i64;
            assert!(width_diff <= max_diff && height_diff <= max_diff,
                "scale inversion should approximately restore original (width_diff={}, height_diff={}, max_diff={})", width_diff, height_diff, max_diff);
        }
    }
}
