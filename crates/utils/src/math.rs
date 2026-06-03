//! Checked arithmetic operations.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::math` and `til::details`.

/// Error type for checked arithmetic operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ArithmeticError {
    #[error("overflow")]
    Overflow,
    #[error("underflow")]
    Underflow,
    #[error("division by zero")]
    DivisionByZero,
}

/// Checked addition.
#[inline]
pub fn checked_add(a: i32, b: i32) -> Result<i32, ArithmeticError> {
    a.checked_add(b).ok_or(ArithmeticError::Overflow)
}

/// Checked subtraction.
#[inline]
pub fn checked_sub(a: i32, b: i32) -> Result<i32, ArithmeticError> {
    a.checked_sub(b).ok_or(ArithmeticError::Underflow)
}

/// Checked multiplication.
#[inline]
pub fn checked_mul(a: i32, b: i32) -> Result<i32, ArithmeticError> {
    a.checked_mul(b).ok_or(ArithmeticError::Overflow)
}

/// Checked division.
#[inline]
pub fn checked_div(a: i32, b: i32) -> Result<i32, ArithmeticError> {
    a.checked_div(b).ok_or(ArithmeticError::DivisionByZero)
}

/// Checked addition for i64.
#[inline]
pub fn checked_add_64(a: i64, b: i64) -> Result<i64, ArithmeticError> {
    a.checked_add(b).ok_or(ArithmeticError::Overflow)
}

/// Checked subtraction for i64.
#[inline]
pub fn checked_sub_64(a: i64, b: i64) -> Result<i64, ArithmeticError> {
    a.checked_sub(b).ok_or(ArithmeticError::Underflow)
}

/// Checked multiplication for i64.
#[inline]
pub fn checked_mul_64(a: i64, b: i64) -> Result<i64, ArithmeticError> {
    a.checked_mul(b).ok_or(ArithmeticError::Overflow)
}

/// Checked division for i64.
#[inline]
pub fn checked_div_64(a: i64, b: i64) -> Result<i64, ArithmeticError> {
    a.checked_div(b).ok_or(ArithmeticError::DivisionByZero)
}

/// Saturating addition.
#[inline]
pub fn saturating_add(a: i32, b: i32) -> i32 {
    a.saturating_add(b)
}

/// Saturating subtraction.
#[inline]
pub fn saturating_sub(a: i32, b: i32) -> i32 {
    a.saturating_sub(b)
}

/// Saturating multiplication.
#[inline]
pub fn saturating_mul(a: i32, b: i32) -> i32 {
    a.saturating_mul(b)
}

/// Clamps a value between min and max.
#[inline]
pub fn clamp(val: i32, min: i32, max: i32) -> i32 {
    if val < min {
        min
    } else if val > max {
        max
    } else {
        val
    }
}

/// Rounds a floating-point value to the nearest integer.
#[inline]
pub fn round(val: f64) -> i32 {
    val.round() as i32
}

/// Ceiling of a floating-point value.
#[inline]
pub fn ceil(val: f64) -> i32 {
    val.ceil() as i32
}

/// Floor of a floating-point value.
#[inline]
pub fn floor(val: f64) -> i32 {
    val.floor() as i32
}

/// Ceiling division for integers: ceil(a / b).
/// Requires b > 0.
#[inline]
pub fn ceil_div(a: i32, b: i32) -> Result<i32, ArithmeticError> {
    if b == 0 {
        return Err(ArithmeticError::DivisionByZero);
    }
    if b < 0 {
        return ceil_div(a, b).map(|v| -v);
    }
    Ok((a + b - 1) / b)
}

/// Checks if a value can be safely narrowed to a smaller integer type.
/// Returns `Ok(value)` if the narrowing is safe, `Err(value)` otherwise.
pub fn narrow_to<T>(value: i32) -> Result<T, i32>
where
    T: Sized + TryFrom<i32>,
    <T as TryFrom<i32>>::Error: std::fmt::Display,
{
    T::try_from(value).map_err(|_| value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checked_add_ok() {
        assert_eq!(checked_add(1, 2), Ok(3));
    }

    #[test]
    fn test_checked_add_overflow() {
        assert_eq!(checked_add(i32::MAX, 1), Err(ArithmeticError::Overflow));
    }

    #[test]
    fn test_checked_sub_ok() {
        assert_eq!(checked_sub(5, 3), Ok(2));
    }

    #[test]
    fn test_checked_sub_underflow() {
        assert_eq!(checked_sub(i32::MIN, 1), Err(ArithmeticError::Underflow));
    }

    #[test]
    fn test_checked_mul_ok() {
        assert_eq!(checked_mul(3, 4), Ok(12));
    }

    #[test]
    fn test_checked_mul_overflow() {
        assert_eq!(checked_mul(i32::MAX, 2), Err(ArithmeticError::Overflow));
    }

    #[test]
    fn test_checked_div_ok() {
        assert_eq!(checked_div(10, 3), Ok(3));
        assert_eq!(checked_div(10, 2), Ok(5));
    }

    #[test]
    fn test_checked_div_by_zero() {
        assert_eq!(checked_div(10, 0), Err(ArithmeticError::DivisionByZero));
    }

    #[test]
    fn test_saturating_ops() {
        assert_eq!(saturating_add(i32::MAX, 1), i32::MAX);
        assert_eq!(saturating_add(i32::MIN, -1), i32::MIN);
        assert_eq!(saturating_sub(i32::MIN, 1), i32::MIN);
        assert_eq!(saturating_mul(i32::MAX, 2), i32::MAX);
    }

    #[test]
    fn test_clamp() {
        assert_eq!(clamp(5, 0, 10), 5);
        assert_eq!(clamp(-5, 0, 10), 0);
        assert_eq!(clamp(15, 0, 10), 10);
    }

    #[test]
    fn test_rounding() {
        assert_eq!(round(3.5), 4);
        assert_eq!(round(3.4), 3);
        assert_eq!(round(-3.5), -4);
        assert_eq!(ceil(3.1), 4);
        assert_eq!(ceil(3.0), 3);
        assert_eq!(floor(3.9), 3);
        assert_eq!(floor(3.0), 3);
    }

    #[test]
    fn test_ceil_div() {
        assert_eq!(ceil_div(10, 3), Ok(4));
        assert_eq!(ceil_div(9, 3), Ok(3));
        assert_eq!(ceil_div(10, 2), Ok(5));
        assert_eq!(ceil_div(0, 5), Ok(0));
        assert_eq!(ceil_div(10, 0), Err(ArithmeticError::DivisionByZero));
    }

    #[test]
    fn test_narrow_to() {
        assert_eq!(narrow_to::<i16>(100), Ok(100i16));
        assert!(narrow_to::<i16>(i32::MAX).is_err());
    }
}
