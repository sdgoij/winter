//! 2D point type with checked arithmetic.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::point`.

use crate::math::checked_add;
use crate::math::checked_div;
use crate::math::checked_mul;
use crate::math::checked_sub;
use crate::size::Size;
use std::ops::{Add, Mul, Sub};

/// Coordinate type used throughout the terminal codebase.
/// Matches the C++ `CoordType = int32_t`.
pub type CoordType = i32;

/// Minimum valid coordinate value.
pub const COORD_TYPE_MIN: CoordType = i32::MIN;

/// Maximum valid coordinate value.
pub const COORD_TYPE_MAX: CoordType = i32::MAX;

/// A 2D point with x and y coordinates.
///
/// Ported from `til::point` in the Windows Terminal C++ codebase.
/// Supports checked arithmetic operations that panic on overflow in debug mode
/// and wrap in release mode (matching C++ behavior with `gsl::narrowing_error`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Point {
    pub x: CoordType,
    pub y: CoordType,
}

impl Point {
    /// Creates a new point with the given coordinates.
    #[inline]
    pub const fn new(x: CoordType, y: CoordType) -> Self {
        Self { x, y }
    }

    /// Creates a zero point (0, 0).
    #[inline]
    pub const fn zero() -> Self {
        Self { x: 0, y: 0 }
    }

    /// Creates a point from a size.
    #[inline]
    pub const fn from_size(size: Size) -> Self {
        Self::new(size.width(), size.height())
    }

    /// Returns whether both coordinates are non-negative.
    /// Equivalent to `operator bool()` in the C++ code.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.x >= 0 && self.y >= 0
    }

    /// Returns the x coordinate.
    #[inline]
    pub fn x(&self) -> CoordType {
        self.x
    }

    /// Returns the y coordinate.
    #[inline]
    pub fn y(&self) -> CoordType {
        self.y
    }

    /// Returns the point as a tuple.
    #[inline]
    pub fn into_tuple(self) -> (CoordType, CoordType) {
        (self.x, self.y)
    }

    /// Returns a point translated by the given offset.
    #[inline]
    pub fn translated(&self, dx: CoordType, dy: CoordType) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }

    /// Checked addition of two points.
    /// Returns `None` if any coordinate overflows.
    #[inline]
    pub fn checked_add(&self, other: Point) -> Option<Point> {
        Some(Point::new(
            checked_add(self.x, other.x).ok()?,
            checked_add(self.y, other.y).ok()?,
        ))
    }

    /// Checked subtraction of two points.
    /// Returns `None` if any coordinate overflows.
    #[inline]
    pub fn checked_sub(&self, other: Point) -> Option<Point> {
        Some(Point::new(
            checked_sub(self.x, other.x).ok()?,
            checked_sub(self.y, other.y).ok()?,
        ))
    }

    /// Checked multiplication of two points.
    /// Returns `None` if any coordinate overflows.
    #[inline]
    pub fn checked_mul(&self, other: Point) -> Option<Point> {
        let x = checked_mul(self.x, other.x).ok()?;
        let y = checked_mul(self.y, other.y).ok()?;
        Some(Point::new(x, y))
    }

    /// Checked division of two points (element-wise).
    /// Returns `None` if any coordinate overflows or if division by zero occurs.
    #[inline]
    pub fn checked_div(&self, other: Point) -> Option<Point> {
        let x = checked_div(self.x, other.x).ok()?;
        let y = checked_div(self.y, other.y).ok()?;
        Some(Point::new(x, y))
    }

    /// Adds another point to this point, wrapping on overflow.
    #[inline]
    pub fn add_wrapping(&self, other: Point) -> Point {
        Self::new(self.x.wrapping_add(other.x), self.y.wrapping_add(other.y))
    }

    /// Subtracts another point from this point, wrapping on overflow.
    #[inline]
    pub fn sub_wrapping(&self, other: Point) -> Point {
        Self::new(self.x.wrapping_sub(other.x), self.y.wrapping_sub(other.y))
    }

    /// Multiplies two points element-wise, wrapping on overflow.
    #[inline]
    pub fn mul_wrapping(&self, other: Point) -> Point {
        Self::new(self.x.wrapping_mul(other.x), self.y.wrapping_mul(other.y))
    }
}

impl std::ops::Add for Point {
    type Output = Point;

    fn add(self, other: Point) -> Point {
        self.checked_add(other).expect("Point addition overflow")
    }
}

impl std::ops::AddAssign for Point {
    fn add_assign(&mut self, other: Point) {
        *self = self.add(other);
    }
}

impl std::ops::Sub for Point {
    type Output = Point;

    fn sub(self, other: Point) -> Point {
        self.checked_sub(other).expect("Point subtraction overflow")
    }
}

impl std::ops::SubAssign for Point {
    fn sub_assign(&mut self, other: Point) {
        *self = self.sub(other);
    }
}

impl std::ops::Mul for Point {
    type Output = Point;

    fn mul(self, other: Point) -> Point {
        self.checked_mul(other).expect("Point multiplication overflow")
    }
}

impl std::ops::MulAssign for Point {
    fn mul_assign(&mut self, other: Point) {
        *self = self.mul(other);
    }
}

impl std::ops::Div for Point {
    type Output = Point;

    fn div(self, other: Point) -> Point {
        self.checked_div(other).expect("Point division overflow or zero")
    }
}

impl std::ops::Div<CoordType> for Point {
    type Output = Point;

    fn div(self, scale: CoordType) -> Point {
        Self::new(
            checked_div(self.x, scale).expect("Point division by zero"),
            checked_div(self.y, scale).expect("Point division by zero"),
        )
    }
}

impl std::ops::Mul<CoordType> for Point {
    type Output = Point;

    fn mul(self, scale: CoordType) -> Point {
        Self::new(
            checked_mul(self.x, scale).expect("Point scaling overflow"),
            checked_mul(self.y, scale).expect("Point scaling overflow"),
        )
    }
}

impl PartialOrd for Point {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Point {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Lexicographic order by (y, x) — matches C++ implementation
        match self.y.cmp(&other.y) {
            std::cmp::Ordering::Equal => self.x.cmp(&other.x),
            other => other,
        }
    }
}

impl std::fmt::Display for Point {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(X:{}, Y:{})", self.x, self.y)
    }
}

/// Wraps COORD-like values into a Point.
#[inline]
pub fn wrap_coord(x: CoordType, y: CoordType) -> Point {
    Point::new(x, y)
}

/// Unwraps a Point into COORD-compatible i16 values.
/// Panics if values don't fit in i16 range (-32768 to 32767).
pub fn unwrap_coord(point: Point) -> (i16, i16) {
    (
        point.x.try_into().expect("x coordinate out of i16 range"),
        point.y.try_into().expect("y coordinate out of i16 range"),
    )
}

/// Unwraps a Point into COORD-compatible i16 values with clamping.
pub fn unwrap_coord_clamped(point: Point) -> (i16, i16) {
    const MIN: i16 = -32768;
    const MAX: i16 = 32767;
    let x = if point.x < MIN as i32 {
        MIN
    } else if point.x > MAX as i32 {
        MAX
    } else {
        point.x as i16
    };
    let y = if point.y < MIN as i32 {
        MIN
    } else if point.y > MAX as i32 {
        MAX
    } else {
        point.y as i16
    };
    (x, y)
}

/// Creates a point at the origin.
#[inline]
pub fn origin() -> Point {
    Point::zero()
}

/// Creates a point from x and y.
#[inline]
pub fn new_point(x: CoordType, y: CoordType) -> Point {
    Point::new(x, y)
}

/// An inclusive span between two points, used for selections and search hits.
///
/// Ported from `til::point_span`. The coordinates are inclusive on both ends;
/// [`PointSpan::iterate_rows_exclusive`] provides the right-exclusive form some
/// callers want.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct PointSpan {
    /// The first point in the span.
    pub start: Point,
    /// The last point in the span.
    pub end: Point,
}

impl PointSpan {
    /// Creates a span from `start` to `end`, inclusive.
    pub const fn new(start: Point, end: Point) -> Self {
        Self { start, end }
    }

    /// Calls `func(row, left, right)` for every row in the span, with inclusive
    /// columns clamped to `0..width`.
    pub fn iterate_rows(&self, width: CoordType, mut func: impl FnMut(CoordType, CoordType, CoordType)) {
        if width <= 0 {
            return;
        }

        let last_column = width - 1;
        let left = self.start.x().clamp(0, last_column);
        let top = self.start.y();
        let right = self.end.x().clamp(0, last_column);
        let bottom = self.end.y();

        for row in top..=bottom {
            let row_left = if row != top { 0 } else { left };
            let row_right = if row != bottom { last_column } else { right };
            func(row, row_left, row_right);
        }
    }

    /// Calls `func(row, left, right)` for every row, with an exclusive right
    /// bound that may equal `width`.
    ///
    /// A start at the right-exclusive edge moves to the next row's left edge; an
    /// end at column zero moves to the previous row's right edge.
    pub fn iterate_rows_exclusive(&self, width: CoordType, mut func: impl FnMut(CoordType, CoordType, CoordType)) {
        if width <= 0 {
            return;
        }

        let mut left = self.start.x().clamp(0, width);
        let mut top = self.start.y();
        let mut right = self.end.x().clamp(0, width);
        let mut bottom = self.end.y();

        if left == width {
            top += 1;
            left = 0;
        }
        if right == 0 {
            bottom -= 1;
            right = width;
        }

        for row in top..=bottom {
            let row_left = if row != top { 0 } else { left };
            let row_right = if row != bottom { width } else { right };
            func(row, row_left, row_right);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_creation() {
        let p = Point::new(10, 20);
        assert_eq!(p.x, 10);
        assert_eq!(p.y, 20);

        let zero = Point::zero();
        assert_eq!(zero.x, 0);
        assert_eq!(zero.y, 0);
    }

    #[test]
    fn test_point_addition() {
        let a = Point::new(1, 2);
        let b = Point::new(3, 4);
        let c = a + b;
        assert_eq!(c.x, 4);
        assert_eq!(c.y, 6);
    }

    #[test]
    fn test_point_subtraction() {
        let a = Point::new(5, 10);
        let b = Point::new(2, 3);
        let c = a - b;
        assert_eq!(c.x, 3);
        assert_eq!(c.y, 7);
    }

    #[test]
    fn test_point_multiplication() {
        let a = Point::new(3, 4);
        let b = Point::new(2, 5);
        let c = a * b;
        assert_eq!(c.x, 6);
        assert_eq!(c.y, 20);
    }

    #[test]
    fn test_point_division() {
        let a = Point::new(10, 20);
        let b = Point::new(2, 4);
        let c = a / b;
        assert_eq!(c.x, 5);
        assert_eq!(c.y, 5);
    }

    #[test]
    fn test_point_scalar_multiply() {
        let a = Point::new(3, 5);
        let c = a * 2;
        assert_eq!(c.x, 6);
        assert_eq!(c.y, 10);
    }

    #[test]
    fn test_point_scalar_divide() {
        let a = Point::new(10, 14);
        let c = a / 2;
        assert_eq!(c.x, 5);
        assert_eq!(c.y, 7);
    }

    #[test]
    fn test_point_valid() {
        assert!(Point::new(0, 0).is_valid());
        assert!(Point::new(1, 1).is_valid());
        assert!(Point::new(1, 5).is_valid());
        assert!(!Point::new(-1, -1).is_valid());
        assert!(!Point::new(-1, 0).is_valid());
        assert!(!Point::new(0, -1).is_valid());
        assert!(!Point::new(5, -1).is_valid());
    }

    #[test]
    fn test_point_comparison() {
        let a = Point::new(1, 2);
        let b = Point::new(1, 3);
        let c = Point::new(2, 3);

        assert!(a < b);
        assert!(b < c);
        assert!(a < c);
        assert_eq!(a, a);
    }

    #[test]
    fn test_checked_add_overflow() {
        let a = Point::new(i32::MAX, 5);
        assert!(a.checked_add(Point::new(1, 0)).is_none());
    }

    #[test]
    fn test_wrapping_add() {
        let a = Point::new(i32::MAX, 5);
        let c = a.add_wrapping(Point::new(1, 0));
        assert_eq!(c.x, i32::MIN);
        assert_eq!(c.y, 5);
    }

    #[test]
    fn test_unwrap_coord() {
        let p = Point::new(100, 200);
        let (x, y) = unwrap_coord(p);
        assert_eq!(x, 100);
        assert_eq!(y, 200);
    }

    #[test]
    fn test_unwrap_coord_clamped() {
        let p = Point::new(i32::MAX, i32::MIN);
        let (x, y) = unwrap_coord_clamped(p);
        assert_eq!(x, 32767);
        assert_eq!(y, -32768);
    }

    #[test]
    fn test_display() {
        let p = Point::new(5, 10);
        assert_eq!(format!("{}", p), "(X:5, Y:10)");
    }

    #[test]
    fn point_span_iterates_each_row() {
        let span = PointSpan::new(Point::new(8, 0), Point::new(4, 2));
        let mut rows = Vec::new();
        span.iterate_rows(16, |row, left, right| rows.push((row, left, right)));

        assert_eq!(rows, vec![(0, 8, 15), (1, 0, 15), (2, 0, 4)]);
    }

    #[test]
    fn point_span_clamps_columns_to_the_width() {
        let span = PointSpan::new(Point::new(-5, 0), Point::new(100, 0));
        let mut rows = Vec::new();
        span.iterate_rows(16, |row, left, right| rows.push((row, left, right)));

        assert_eq!(rows, vec![(0, 0, 15)]);
    }

    #[test]
    fn point_span_with_zero_width_emits_nothing() {
        let span = PointSpan::new(Point::new(0, 0), Point::new(4, 2));
        let mut count = 0;
        span.iterate_rows(0, |_, _, _| count += 1);
        assert_eq!(count, 0);
    }

    #[test]
    fn point_span_exclusive_moves_edge_points_to_neighbouring_rows() {
        let span = PointSpan::new(Point::new(16, 0), Point::new(0, 2));
        let mut rows = Vec::new();
        span.iterate_rows_exclusive(16, |row, left, right| rows.push((row, left, right)));

        assert_eq!(rows, vec![(1, 0, 16)]);
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    prop_compose! {
        fn any_point()(x in any::<i32>(), y in any::<i32>()) -> Point {
            Point::new(x, y)
        }
    }

    proptest! {
        #[test]
        fn test_add_sub_cancel(a in any_point(), b in any_point()) {
            // Use checked arithmetic to avoid overflow panics
            if let Some(sum) = a.checked_add(b)
                && let Some(result) = sum.checked_sub(b) {
                    assert_eq!(result, a);
                }
        }

        #[test]
        fn test_add_identity(a in any_point()) {
            let zero = Point::zero();
            let result = a + zero;
            assert_eq!(result, a);
        }

        #[test]
        fn test_valid_coords(a in any_point()) {
            let is_valid = a.x >= 0 && a.y >= 0;
            assert_eq!(a.is_valid(), is_valid);
        }

        #[test]
        fn test_mul_scalar(a in any_point(), s in 1i32..100i32) {
            if let Some(result) = a.checked_mul(Point::new(s, s)) {
                assert_eq!(result.x, a.x * s);
                assert_eq!(result.y, a.y * s);
            }
        }
    }
}
