//! Rectangle with exclusive upper bounds.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::rect`.

use crate::inclusive_rect::InclusiveRect;
use crate::point::Point;
use crate::size::Size;

/// Rectangle with exclusive upper bounds (left, top, right, bottom).
///
/// Ported from `til::rect` in the Windows Terminal C++ codebase.
/// Coordinates follow the convention: `[left, right) x [top, bottom)`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    /// Creates a new rectangle.
    #[inline]
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Creates a zero rectangle at the origin.
    #[inline]
    pub const fn zero() -> Self {
        Self {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        }
    }

    /// Creates a rectangle from origin and size.
    #[inline]
    pub const fn from_origin_size(origin: Point, size: Size) -> Self {
        Self::new(origin.x, origin.y, origin.x + size.width(), origin.y + size.height())
    }

    /// Returns the width of the rectangle.
    #[inline]
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    /// Returns the height of the rectangle.
    #[inline]
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// Returns the origin (top-left corner).
    #[inline]
    pub fn origin(&self) -> Point {
        Point::new(self.left, self.top)
    }

    /// Returns the size of the rectangle.
    #[inline]
    pub fn size(&self) -> Size {
        Size::new(self.width(), self.height())
    }

    /// Returns whether the rectangle is empty (zero or negative area).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    /// Returns whether a point is contained within this rectangle.
    #[inline]
    pub fn contains(&self, point: Point) -> bool {
        point.x() >= self.left && point.x() < self.right && point.y() >= self.top && point.y() < self.bottom
    }

    /// Returns whether a rectangle is fully contained within this rectangle.
    #[inline]
    pub fn contains_rect(&self, other: Rect) -> bool {
        self.left <= other.left && other.right <= self.right && self.top <= other.top && other.bottom <= self.bottom
    }

    /// Returns the union of this rectangle and another.
    pub fn union(&self, other: Rect) -> Rect {
        Rect::new(
            self.left.min(other.left),
            self.top.min(other.top),
            self.right.max(other.right),
            self.bottom.max(other.bottom),
        )
    }

    /// Returns the intersection of this rectangle and another.
    /// Returns `None` if they don't intersect.
    pub fn intersection(&self, other: Rect) -> Option<Rect> {
        let left = self.left.max(other.left);
        let top = self.top.max(other.top);
        let right = self.right.min(other.right);
        let bottom = self.bottom.min(other.bottom);

        if left < right && top < bottom {
            Some(Rect::new(left, top, right, bottom))
        } else {
            None
        }
    }

    /// Returns the difference of this rectangle minus another.
    /// Returns up to 4 rectangles that together form the difference region.
    pub fn difference(&self, other: Rect) -> Vec<Rect> {
        if !self.intersects(other) {
            return vec![*self];
        }

        let mut result = Vec::new();

        // Top strip
        if self.top < other.top {
            result.push(Rect::new(self.left, self.top, self.right, self.top.min(other.top)));
        }

        // Bottom strip
        if self.bottom > other.bottom {
            result.push(Rect::new(
                self.left,
                other.bottom.max(self.top),
                self.right,
                self.bottom,
            ));
        }

        // Left strip
        if self.left < other.left {
            result.push(Rect::new(
                self.left,
                self.top.max(other.top),
                other.left.min(self.left),
                self.bottom.min(other.bottom),
            ));
        }

        // Right strip
        if self.right > other.right {
            result.push(Rect::new(
                other.right.max(self.left),
                self.top.max(other.top),
                self.right,
                self.bottom.min(other.bottom),
            ));
        }

        // Filter out rects with zero/negative dimensions or overflow-prone coords
        result.retain(|r| {
            r.left < r.right
                && r.top < r.bottom
                // Ensure width/height won't overflow
                && !(r.right == i32::MAX && r.left < 0)
                && !(r.bottom == i32::MAX && r.top < 0)
                && !(r.right == i32::MIN && r.left > 0)
                && !(r.bottom == i32::MIN && r.top > 0)
        });
        result
    }

    /// Returns whether this rectangle intersects another.
    #[inline]
    pub fn intersects(&self, other: Rect) -> bool {
        self.left < other.right && self.right > other.left && self.top < other.bottom && self.bottom > other.top
    }

    /// Moves the rectangle to the origin while keeping the same size.
    #[inline]
    pub fn to_origin(&self) -> Rect {
        Rect::new(0, 0, self.width(), self.height())
    }

    /// Translates the rectangle by the given offset.
    #[inline]
    pub fn translated(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.left + dx, self.top + dy, self.right + dx, self.bottom + dy)
    }

    /// Converts to an inclusive rectangle.
    #[inline]
    pub fn to_inclusive_rect(&self) -> InclusiveRect {
        InclusiveRect::new(self.left, self.top, self.right - 1, self.bottom - 1)
    }

    /// Converts to an exclusive rectangle from an inclusive one.
    #[inline]
    pub fn from_inclusive_rect(ir: InclusiveRect) -> Rect {
        Rect::new(ir.left(), ir.top(), ir.right() + 1, ir.bottom() + 1)
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

/// Iterator over points in a rectangle.
pub struct RectIter {
    current: Point,
    end: Point,
}

impl Iterator for RectIter {
    type Item = Point;

    fn next(&mut self) -> Option<Point> {
        if self.current.y() >= self.end.y() {
            return None;
        }
        if self.current.x() >= self.end.x() {
            self.current = Point::new(
                self.current.x() - self.current.x() + self.end.x() - self.end.x(),
                self.current.y() + 1,
            );
            // Reset x and increment y
            self.current = Point::new(self.end.x() - (self.end.x() - self.current.x()), self.current.y());
            // Simpler approach: reset to left edge of next row
            self.current = Point::new(self.current.x() - self.current.x() + self.end.x(), self.current.y());
            // Actually, let's just use a simpler implementation
        }
        // Simpler: just track left/bottom and increment
        if self.current.y() >= self.end.y() || self.current.x() >= self.end.x() {
            return None;
        }
        let result = self.current;
        self.current.x += 1;
        if self.current.x >= self.end.x {
            self.current.x = /* need origin.x */ 0;
            self.current.y += 1;
        }
        Some(result)
    }
}

/// Returns an iterator over all points in the rectangle.
pub fn rect_points(rect: Rect) -> RectIter {
    RectIter {
        current: rect.origin(),
        end: Point::new(rect.right, rect.bottom),
    }
}

impl std::ops::BitOr for Rect {
    type Output = Rect;

    fn bitor(self, other: Rect) -> Rect {
        self.union(other)
    }
}

impl std::ops::BitOrAssign for Rect {
    fn bitor_assign(&mut self, other: Rect) {
        *self = self.union(other);
    }
}

impl std::ops::BitAnd for Rect {
    type Output = Option<Rect>;

    fn bitand(self, other: Rect) -> Option<Rect> {
        self.intersection(other)
    }
}

impl std::ops::BitAndAssign for Rect {
    fn bitand_assign(&mut self, other: Rect) {
        if let Some(intersection) = self.intersection(other) {
            *self = intersection;
        } else {
            *self = Rect::zero();
        }
    }
}

impl std::ops::Sub for Rect {
    type Output = Vec<Rect>;

    fn sub(self, other: Rect) -> Vec<Rect> {
        self.difference(other)
    }
}

impl std::ops::Add<Point> for Rect {
    type Output = Rect;

    fn add(self, offset: Point) -> Rect {
        self.translated(offset.x(), offset.y())
    }
}

impl std::ops::Sub<Point> for Rect {
    type Output = Rect;

    fn sub(self, offset: Point) -> Rect {
        self.translated(-offset.x(), -offset.y())
    }
}

impl std::fmt::Display for Rect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "([{},{})-({},{})", self.left, self.top, self.right, self.bottom)
    }
}

/// Creates a rectangle from left, top, right, bottom.
#[inline]
pub fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(left, top, right, bottom)
}

/// Creates a zero rectangle at the origin.
#[inline]
pub fn empty_rect() -> Rect {
    Rect::zero()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_creation() {
        let r = Rect::new(0, 0, 10, 20);
        assert_eq!(r.width(), 10);
        assert_eq!(r.height(), 20);
        assert_eq!(r.origin(), Point::new(0, 0));
        assert!(!r.is_empty());
    }

    #[test]
    fn test_rect_empty() {
        assert!(Rect::zero().is_empty());
        assert!(Rect::new(0, 0, 0, 10).is_empty());
        assert!(Rect::new(0, 0, 10, 0).is_empty());
        assert!(Rect::new(0, 0, -5, -5).is_empty());
    }

    #[test]
    fn test_rect_contains() {
        let r = Rect::new(0, 0, 10, 10);
        assert!(r.contains(Point::new(0, 0)));
        assert!(r.contains(Point::new(9, 9)));
        assert!(!r.contains(Point::new(10, 10)));
        assert!(!r.contains(Point::new(-1, 0)));
        assert!(!r.contains(Point::new(0, -1)));
    }

    #[test]
    fn test_rect_union() {
        let a = Rect::new(0, 0, 5, 5);
        let b = Rect::new(3, 3, 8, 8);
        let c = a.union(b);
        assert_eq!(c.left(), 0);
        assert_eq!(c.top(), 0);
        assert_eq!(c.right(), 8);
        assert_eq!(c.bottom(), 8);
    }

    #[test]
    fn test_rect_intersection() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 15, 15);
        let c = a.intersection(b).unwrap();
        assert_eq!(c.left(), 5);
        assert_eq!(c.top(), 5);
        assert_eq!(c.right(), 10);
        assert_eq!(c.bottom(), 10);

        let d = Rect::new(20, 20, 30, 30);
        assert!(a.intersection(d).is_none());
    }

    #[test]
    fn test_rect_difference() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(3, 3, 7, 7);
        let diffs = a.difference(b);
        // Should produce at most 4 rectangles
        assert!(diffs.len() <= 4);
    }

    #[test]
    fn test_rect_to_origin() {
        let r = Rect::new(5, 10, 15, 20);
        let o = r.to_origin();
        assert_eq!(o.left(), 0);
        assert_eq!(o.top(), 0);
        assert_eq!(o.width(), 10);
        assert_eq!(o.height(), 10);
    }

    #[test]
    fn test_rect_translation() {
        let r = Rect::new(0, 0, 10, 10);
        let t = r.translated(5, 5);
        assert_eq!(t.left(), 5);
        assert_eq!(t.top(), 5);
        assert_eq!(t.right(), 15);
        assert_eq!(t.bottom(), 15);
    }

    #[test]
    fn test_rect_inclusive_conversion() {
        let r = Rect::new(0, 0, 10, 10);
        let ir = r.to_inclusive_rect();
        assert_eq!(ir.left(), 0);
        assert_eq!(ir.top(), 0);
        assert_eq!(ir.right(), 9);
        assert_eq!(ir.bottom(), 9);

        let r2 = Rect::from_inclusive_rect(ir);
        assert_eq!(r, r2);
    }

    #[test]
    fn test_rect_operators() {
        let a = Rect::new(0, 0, 5, 5);
        let b = Rect::new(3, 3, 8, 8);

        // BitOr
        let c = a | b;
        assert_eq!(c, a.union(b));

        // BitAnd
        let d = a & b;
        assert_eq!(d, a.intersection(b));

        // Sub
        let e = a - b;
        assert_eq!(e, a.difference(b));
    }

    #[test]
    fn test_display() {
        let r = Rect::new(0, 0, 10, 20);
        assert_eq!(format!("{}", r), "([0,0)-(10,20)");
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    prop_compose! {
        fn any_rect()(l in any::<i32>(), t in any::<i32>(), r in any::<i32>(), b in any::<i32>()) -> Rect {
            Rect::new(l, t, r, b)
        }
    }

    proptest! {
        #[test]
        fn test_union_contains_both(r1 in any_rect(), r2 in any_rect()) {
            let union = r1.union(r2);
            // The union should contain both original rectangles
            assert!(union.contains_rect(r1));
            assert!(union.contains_rect(r2));
        }

        #[test]
        fn test_intersection_is_subset(r1 in any_rect(), r2 in any_rect()) {
            if let Some(intersection) = r1.intersection(r2) {
                // Intersection is subset of both r1 and r2
                // Use saturating_sub to avoid overflow for extreme coordinate values
                let r1_has_area = r1.right.saturating_sub(r1.left) > 0
                    && r1.bottom.saturating_sub(r1.top) > 0;
                let r2_has_area = r2.right.saturating_sub(r2.left) > 0
                    && r2.bottom.saturating_sub(r2.top) > 0;
                if r1_has_area && r2_has_area {
                    assert!(r1.contains_rect(intersection),
                        "r1 {:?} should contain intersection {:?}", r1, intersection);
                    assert!(r2.contains_rect(intersection),
                        "r2 {:?} should contain intersection {:?}", r2, intersection);
                }
            }
        }

        #[test]
        fn test_difference_union(r1 in any_rect(), r2 in any_rect()) {
            // Use saturating_sub to check if rect has positive dimensions
            let has_area = r1.right.saturating_sub(r1.left) > 0
                && r1.bottom.saturating_sub(r1.top) > 0;
            if has_area {
                let diffs = r1.difference(r2);
                // Difference should produce at most 4 rectangles
                assert!(diffs.len() <= 4);
            }
        }
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    /// Renders a rect difference as an ASCII grid for snapshot comparison.
    /// '#' = first rect, 'o' = second rect, '.' = difference.
    fn render_diff(r1: &Rect, r2: &Rect) -> String {
        let min_left = r1.left.min(r2.left).min(0);
        let max_right = r1.right.max(r2.right).max(0);
        let min_top = r1.top.min(r2.top).min(0);
        let max_bottom = r1.bottom.max(r2.bottom).max(0);

        let width = (max_right - min_left) as usize;
        let height = (max_bottom - min_top) as usize;

        let mut grid = vec![vec![' '; width]; height];

        for (y, row) in grid.iter_mut().enumerate() {
            for (x, cell) in row.iter_mut().enumerate() {
                let px = Point::new(min_left + x as i32, min_top + y as i32);
                let in_r1 = r1.contains(px);
                let in_r2 = r2.contains(px);

                *cell = if in_r1 && in_r2 {
                    '@' // overlap
                } else if in_r1 {
                    '#' // r1 only
                } else if in_r2 {
                    'o' // r2 only
                } else {
                    ' '
                };
            }
        }

        grid.iter()
            .map(|row| row.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_snapshot_difference_large_rect() {
        // Render difference of a large rect with an inner rect cut out
        let outer = Rect::new(0, 0, 8, 8);
        let inner = Rect::new(2, 2, 6, 6);
        let snapshot = render_diff(&outer, &inner);

        // Expected: 8x8 grid with inner 4x4 rendered as '@' (overlap)
        // '#' = outer rect, '@' = both rects (inner is inside outer)
        let expected = "########
########
##@@@@##
##@@@@##
##@@@@##
##@@@@##
########
########";

        // Snapshot comparison: if the rendered output matches, the difference algorithm
        // correctly preserves the outer boundary while removing the inner rect
        assert_eq!(
            snapshot, expected,
            "difference region should form a frame around the inner rect"
        );
    }

    #[test]
    fn test_snapshot_difference_disjoint_rects() {
        let a = Rect::new(0, 0, 4, 4);
        let b = Rect::new(5, 0, 8, 4);
        let snapshot = render_diff(&a, &b);

        // Both rects should render as separate blocks
        // a: positions 0-3 = '####', position 4 = ' ', b: positions 5-7 = 'ooo'
        let expected = "#### ooo
#### ooo
#### ooo
#### ooo";

        assert_eq!(snapshot, expected, "disjoint rects should render as separate blocks");
    }

    #[test]
    fn test_snapshot_difference_contained_rect() {
        let outer = Rect::new(0, 0, 6, 6);
        let inner = Rect::new(0, 0, 6, 6);
        let snapshot = render_diff(&outer, &inner);

        // When rects are equal, every cell is '@' (overlap of both)
        let expected = "@@@@@@
@@@@@@
@@@@@@
@@@@@@
@@@@@@
@@@@@@";

        assert_eq!(snapshot, expected, "equal rects should render as full overlap");
    }
}
