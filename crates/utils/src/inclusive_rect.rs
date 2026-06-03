//! Rectangle with inclusive upper bounds.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::inclusive_rect`.

use crate::point::Point;
use crate::rect::Rect;

/// Rectangle with inclusive upper bounds (left, top, right, bottom).
///
/// Ported from `til::inclusive_rect` in the Windows Terminal C++ codebase.
/// Coordinates follow the convention: `[left, right] x [top, bottom]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct InclusiveRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl InclusiveRect {
    /// Creates a new inclusive rectangle.
    #[inline]
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Creates a single-point inclusive rectangle.
    #[inline]
    pub const fn from_point(p: Point) -> Self {
        Self::new(p.x, p.y, p.x, p.y)
    }

    /// Returns the width of the rectangle.
    #[inline]
    pub fn width(&self) -> i32 {
        self.right - self.left + 1
    }

    /// Returns the height of the rectangle.
    #[inline]
    pub fn height(&self) -> i32 {
        self.bottom - self.top + 1
    }

    /// Returns the origin (top-left corner).
    #[inline]
    pub fn origin(&self) -> Point {
        Point::new(self.left, self.top)
    }

    /// Returns the bottom-right point.
    #[inline]
    pub fn bottom_right(&self) -> Point {
        Point::new(self.right, self.bottom)
    }

    /// Returns the size of the rectangle.
    #[inline]
    pub fn size(&self) -> crate::size::Size {
        crate::size::Size::new(self.width(), self.height())
    }

    /// Returns whether the rectangle is empty (zero or negative area).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    /// Returns whether a point is contained within this rectangle.
    #[inline]
    pub fn contains(&self, point: Point) -> bool {
        point.x() >= self.left
            && point.x() <= self.right
            && point.y() >= self.top
            && point.y() <= self.bottom
    }

    /// Returns whether another inclusive rectangle is contained within this one.
    #[inline]
    pub fn contains_rect(&self, other: InclusiveRect) -> bool {
        self.left <= other.left
            && other.right <= self.right
            && self.top <= other.top
            && other.bottom <= self.bottom
    }

    /// Returns whether this rectangle overlaps with another.
    #[inline]
    pub fn overlaps(&self, other: InclusiveRect) -> bool {
        self.left <= other.right
            && self.right >= other.left
            && self.top <= other.bottom
            && self.bottom >= other.top
    }

    /// Returns the union of this rectangle and another.
    pub fn union(&self, other: InclusiveRect) -> InclusiveRect {
        InclusiveRect::new(
            self.left.min(other.left),
            self.top.min(other.top),
            self.right.max(other.right),
            self.bottom.max(other.bottom),
        )
    }

    /// Returns the intersection of this rectangle and another.
    /// Returns `None` if they don't overlap.
    pub fn intersection(&self, other: InclusiveRect) -> Option<InclusiveRect> {
        let left = self.left.max(other.left);
        let top = self.top.max(other.top);
        let right = self.right.min(other.right);
        let bottom = self.bottom.min(other.bottom);

        if left <= right && top <= bottom {
            Some(InclusiveRect::new(left, top, right, bottom))
        } else {
            None
        }
    }

    /// Converts to an exclusive rectangle.
    #[inline]
    pub fn to_exclusive_rect(&self) -> Rect {
        Rect::new(self.left, self.top, self.right + 1, self.bottom + 1)
    }

    /// Returns the left coordinate.
    #[inline]
    pub fn left(&self) -> i32 {
        self.left
    }

    /// Returns the top coordinate.
    #[inline]
    pub fn top(&self) -> i32 {
        self.top
    }

    /// Returns the right coordinate.
    #[inline]
    pub fn right(&self) -> i32 {
        self.right
    }

    /// Returns the bottom coordinate.
    #[inline]
    pub fn bottom(&self) -> i32 {
        self.bottom
    }
}

/// Creates an inclusive rectangle from left, top, right, bottom.
#[inline]
pub fn inclusive_rect(left: i32, top: i32, right: i32, bottom: i32) -> InclusiveRect {
    InclusiveRect::new(left, top, right, bottom)
}

/// Creates an exclusive rectangle from an inclusive one.
#[inline]
pub fn from_exclusive_rect(r: Rect) -> InclusiveRect {
    InclusiveRect::new(r.left(), r.top(), r.right() - 1, r.bottom() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inclusive_rect_creation() {
        let r = InclusiveRect::new(0, 0, 9, 9);
        assert_eq!(r.width(), 10);
        assert_eq!(r.height(), 10);
        assert!(!r.is_empty());
    }

    #[test]
    fn test_inclusive_rect_point() {
        let p = Point::new(5, 10);
        let r = InclusiveRect::from_point(p);
        assert_eq!(r.left(), 5);
        assert_eq!(r.right(), 5);
        assert!(r.contains(p));
        assert!(!r.contains(Point::new(6, 10)));
    }

    #[test]
    fn test_inclusive_rect_contains() {
        let r = InclusiveRect::new(0, 0, 9, 9);
        assert!(r.contains(Point::new(0, 0)));
        assert!(r.contains(Point::new(9, 9)));
        assert!(!r.contains(Point::new(10, 10)));
        assert!(!r.contains(Point::new(-1, -1)));
    }

    #[test]
    fn test_inclusive_rect_union() {
        let a = InclusiveRect::new(0, 0, 4, 4);
        let b = InclusiveRect::new(3, 3, 8, 8);
        let c = a.union(b);
        assert_eq!(c.left(), 0);
        assert_eq!(c.top(), 0);
        assert_eq!(c.right(), 8);
        assert_eq!(c.bottom(), 8);
    }

    #[test]
    fn test_inclusive_rect_intersection() {
        let a = InclusiveRect::new(0, 0, 10, 10);
        let b = InclusiveRect::new(5, 5, 15, 15);
        let c = a.intersection(b).unwrap();
        assert_eq!(c.left(), 5);
        assert_eq!(c.top(), 5);
        assert_eq!(c.right(), 10);
        assert_eq!(c.bottom(), 10);

        let d = InclusiveRect::new(20, 20, 30, 30);
        assert!(a.intersection(d).is_none());
    }

    #[test]
    fn test_inclusive_rect_overlaps() {
        let a = InclusiveRect::new(0, 0, 5, 5);
        let b = InclusiveRect::new(3, 3, 8, 8);
        assert!(a.overlaps(b));

        let c = InclusiveRect::new(6, 6, 10, 10);
        assert!(!a.overlaps(c)); // doesn't overlap (adjacent but not touching)

        let d = InclusiveRect::new(5, 5, 10, 10);
        assert!(a.overlaps(d)); // touches at point (5,5)

        let e = InclusiveRect::new(-1, -1, 3, 3);
        assert!(a.overlaps(e)); // overlaps at (0,0)-(3,3)
    }

    #[test]
    fn test_exclusive_conversion() {
        let ir = InclusiveRect::new(0, 0, 9, 9);
        let r = ir.to_exclusive_rect();
        assert_eq!(r.left(), 0);
        assert_eq!(r.top(), 0);
        assert_eq!(r.right(), 10);
        assert_eq!(r.bottom(), 10);

        let ir2 = from_exclusive_rect(r);
        assert_eq!(ir, ir2);
    }

    #[test]
    fn test_empty() {
        assert!(InclusiveRect::new(5, 5, 4, 4).is_empty());
        assert!(InclusiveRect::new(0, 0, 0, -1).is_empty());
    }
}
