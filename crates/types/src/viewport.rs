//! `Viewport` — screen dimensions with origin and size.
//!
//! Ported from the Viewport type in the Windows Terminal C++ buffer module.

use utils::point::Point;
use utils::size::Size;

/// Represents the visible screen area with origin and size.
///
/// Ported from the Viewport concept in the Windows Terminal C++ codebase.
/// Combines an origin point (top-left) with a size to define the viewport.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Viewport {
    origin: Point,
    size: Size,
}

impl Viewport {
    /// Creates a viewport with the given size, origin at (0,0).
    pub const fn new(size: Size) -> Self {
        Viewport {
            origin: Point::zero(),
            size,
        }
    }

    /// Creates a viewport with the given origin and size.
    pub const fn from_origin_size(origin: Point, size: Size) -> Self {
        Viewport { origin, size }
    }

    /// Returns the origin (top-left corner).
    pub const fn origin(&self) -> Point {
        self.origin
    }

    /// Returns the size (width and height).
    pub const fn size(&self) -> Size {
        self.size
    }

    /// Returns the width.
    pub fn width(&self) -> i32 {
        self.size.width()
    }

    /// Returns the height.
    pub fn height(&self) -> i32 {
        self.size.height()
    }

    /// Returns the leftmost column.
    pub const fn left(&self) -> i32 {
        self.origin.x
    }

    /// Returns the rightmost column, inclusive.
    pub fn right_inclusive(&self) -> i32 {
        self.origin.x() + self.size.width() - 1
    }

    /// Returns the rightmost column, exclusive.
    pub fn right_exclusive(&self) -> i32 {
        self.origin.x() + self.size.width()
    }

    /// Returns the top row.
    pub const fn top(&self) -> i32 {
        self.origin.y
    }

    /// Returns the bottom row, inclusive.
    pub fn bottom_inclusive(&self) -> i32 {
        self.origin.y() + self.size.height() - 1
    }

    /// Returns the bottom row, exclusive.
    pub fn bottom_exclusive(&self) -> i32 {
        self.origin.y() + self.size.height()
    }

    /// Walks `position` through the viewport by `delta` cells, in reading
    /// order, and reports whether it stayed inside.
    ///
    /// Ported from `Viewport::WalkInBounds`: a step past the right edge wraps
    /// to the next row, and a step past the first cell or the last cell clamps
    /// and fails.
    pub fn walk_in_bounds(&self, position: &mut Point, delta: i32) -> bool {
        let left = i64::from(self.left());
        let top = i64::from(self.top());
        let width = i64::from(self.size.width().max(0));
        let height = i64::from(self.size.height().max(0));
        if width == 0 || height == 0 {
            return false;
        }

        let maximum = width * height - 1;
        let offset = width * (i64::from(position.y) - top) + (i64::from(position.x) - left) + i64::from(delta);
        let clamped = offset.clamp(0, maximum);
        *position = Point::new((clamped % width + left) as i32, (clamped / width + top) as i32);
        offset == clamped
    }

    /// Advances `position` by one cell inside the viewport.
    pub fn increment_in_bounds(&self, position: &mut Point) -> bool {
        self.walk_in_bounds(position, 1)
    }

    /// Moves `position` back by one cell inside the viewport.
    pub fn decrement_in_bounds(&self, position: &mut Point) -> bool {
        self.walk_in_bounds(position, -1)
    }

    /// Walks `position` through a viewport that has one extra column past its
    /// inclusive right, upstream's exclusive bounds.
    ///
    /// A point at `right_exclusive()` is a valid position, so a selection's
    /// exclusive end can be represented.
    pub fn walk_in_exclusive_bounds(&self, position: &mut Point, delta: i32) -> bool {
        let left = i64::from(self.left());
        let top = i64::from(self.top());
        let width = i64::from(self.size.width().max(0) + 1);
        let height = i64::from(self.size.height().max(0));
        if height == 0 {
            return false;
        }

        let maximum = width * height;
        let offset = width * (i64::from(position.y) - top) + (i64::from(position.x) - left) + i64::from(delta);
        let clamped = offset.clamp(0, maximum);
        *position = Point::new((clamped % width + left) as i32, (clamped / width + top) as i32);
        offset == clamped
    }

    /// Advances `position` by one cell in exclusive bounds.
    pub fn increment_in_exclusive_bounds(&self, position: &mut Point) -> bool {
        self.walk_in_exclusive_bounds(position, 1)
    }

    /// Moves `position` back by one cell in exclusive bounds.
    pub fn decrement_in_exclusive_bounds(&self, position: &mut Point) -> bool {
        self.walk_in_exclusive_bounds(position, -1)
    }

    /// Compares two positions in reading order.
    ///
    /// Ported from `Viewport::CompareInBounds`: negative when `first` comes
    /// before `second`, zero when they are the same cell, positive when it
    /// comes after. The magnitude is the distance in cells.
    pub fn compare_in_bounds(&self, first: Point, second: Point) -> i32 {
        let distance = i64::from(first.y - second.y) * i64::from(self.width()) + i64::from(first.x - second.x);
        distance.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    }

    /// Returns the bottom-right corner.
    pub fn bottom_right(&self) -> Point {
        Point::new(
            self.origin.x() + self.size.width() - 1,
            self.origin.y() + self.size.height() - 1,
        )
    }

    /// Clamps a point to be within this viewport.
    pub fn clamp(&self, point: Point) -> Point {
        let clamped_x = point
            .x()
            .max(self.origin.x())
            .min(self.origin.x() + self.size.width() - 1);
        let clamped_y = point
            .y()
            .max(self.origin.y())
            .min(self.origin.y() + self.size.height() - 1);
        Point::new(clamped_x, clamped_y)
    }

    /// Returns whether a point is contained within this viewport.
    pub fn contains(&self, point: Point) -> bool {
        point.x() >= self.origin.x()
            && point.x() < self.origin.x() + self.size.width()
            && point.y() >= self.origin.y()
            && point.y() < self.origin.y() + self.size.height()
    }

    /// Returns the area (width * height).
    pub fn area(&self) -> usize {
        self.size.width().max(0) as usize * self.size.height().max(0) as usize
    }

    /// Returns the intersection of this viewport with another.
    pub fn intersection(&self, other: &Viewport) -> Option<Viewport> {
        let origin_x = self.origin.x().max(other.origin.x());
        let origin_y = self.origin.y().max(other.origin.y());
        let end_x = (self.origin.x() + self.size.width()).min(other.origin.x() + other.size.width());
        let end_y = (self.origin.y() + self.size.height()).min(other.origin.y() + other.size.height());

        if end_x > origin_x && end_y > origin_y {
            Some(Viewport {
                origin: Point::new(origin_x, origin_y),
                size: Size::new(end_x - origin_x, end_y - origin_y),
            })
        } else {
            None
        }
    }

    /// Returns the union of this viewport with another.
    pub fn union(&self, other: &Viewport) -> Viewport {
        let origin_x = self.origin.x().min(other.origin.x());
        let origin_y = self.origin.y().min(other.origin.y());
        let end_x = (self.origin.x() + self.size.width()).max(other.origin.x() + other.size.width());
        let end_y = (self.origin.y() + self.size.height()).max(other.origin.y() + other.size.height());

        Viewport {
            origin: Point::new(origin_x, origin_y),
            size: Size::new((end_x - origin_x).max(0), (end_y - origin_y).max(0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_viewport_creation() {
        let vp = Viewport::new(Size::new(80, 24));
        assert_eq!(vp.origin(), Point::zero());
        assert_eq!(vp.width(), 80);
        assert_eq!(vp.height(), 24);
    }

    #[test]
    fn test_viewport_with_origin() {
        let vp = Viewport::from_origin_size(Point::new(5, 10), Size::new(80, 24));
        assert_eq!(vp.origin(), Point::new(5, 10));
        assert_eq!(vp.bottom_right(), Point::new(84, 33));
    }

    #[test]
    fn test_viewport_clamp() {
        let vp = Viewport::new(Size::new(80, 24));
        // Point within viewport
        assert_eq!(vp.clamp(Point::new(40, 12)), Point::new(40, 12));
        // Point outside (left)
        assert_eq!(vp.clamp(Point::new(-10, 12)), Point::new(0, 12));
        // Point outside (right)
        assert_eq!(vp.clamp(Point::new(100, 12)), Point::new(79, 12));
        // Point outside (top)
        assert_eq!(vp.clamp(Point::new(40, -5)), Point::new(40, 0));
        // Point outside (bottom)
        assert_eq!(vp.clamp(Point::new(40, 50)), Point::new(40, 23));
    }

    #[test]
    fn test_viewport_contains() {
        let vp = Viewport::new(Size::new(80, 24));
        assert!(vp.contains(Point::new(0, 0)));
        assert!(vp.contains(Point::new(79, 23)));
        assert!(!vp.contains(Point::new(80, 0)));
        assert!(!vp.contains(Point::new(0, 24)));
    }

    #[test]
    fn test_viewport_area() {
        let vp = Viewport::new(Size::new(80, 24));
        assert_eq!(vp.area(), 1920);
    }

    #[test]
    fn test_viewport_intersection() {
        let vp1 = Viewport::from_origin_size(Point::new(0, 0), Size::new(80, 24));
        let vp2 = Viewport::from_origin_size(Point::new(40, 12), Size::new(80, 24));
        let inter = vp1.intersection(&vp2).unwrap();
        assert_eq!(inter.origin(), Point::new(40, 12));
        assert_eq!(inter.size(), Size::new(40, 12));
    }

    #[test]
    fn test_viewport_union() {
        let vp1 = Viewport::from_origin_size(Point::new(0, 0), Size::new(40, 12));
        let vp2 = Viewport::from_origin_size(Point::new(40, 12), Size::new(40, 12));
        let union = vp1.union(&vp2);
        assert_eq!(union.origin(), Point::new(0, 0));
        assert_eq!(union.size(), Size::new(80, 24));
    }

    fn edges() -> Viewport {
        Viewport::from_origin_size(Point::new(10, 20), Size::new(10, 10))
    }

    #[test]
    fn increment_in_bounds_walks_rows_and_stops_at_the_last_cell() {
        let view = edges();

        let mut point = Point::new(15, 25);
        assert!(view.increment_in_bounds(&mut point));
        assert_eq!(point, Point::new(16, 25));

        let mut point = Point::new(19, 25);
        assert!(view.increment_in_bounds(&mut point));
        assert_eq!(point, Point::new(10, 26));

        let mut point = Point::new(19, 29);
        assert!(!view.increment_in_bounds(&mut point));
        assert_eq!(point, Point::new(19, 29));
    }

    #[test]
    fn decrement_in_bounds_walks_rows_and_stops_at_the_first_cell() {
        let view = edges();

        let mut point = Point::new(15, 25);
        assert!(view.decrement_in_bounds(&mut point));
        assert_eq!(point, Point::new(14, 25));

        let mut point = Point::new(10, 25);
        assert!(view.decrement_in_bounds(&mut point));
        assert_eq!(point, Point::new(19, 24));

        let mut point = Point::new(10, 20);
        assert!(!view.decrement_in_bounds(&mut point));
        assert_eq!(point, Point::new(10, 20));
    }

    #[test]
    fn walking_by_more_than_one_cell_crosses_rows() {
        let view = Viewport::new(Size::new(10, 5));
        let mut point = Point::new(7, 0);

        assert!(view.walk_in_bounds(&mut point, 5));
        assert_eq!(point, Point::new(2, 1));

        assert!(!view.walk_in_bounds(&mut point, 100));
        assert_eq!(point, Point::new(9, 4));
    }

    #[test]
    fn exclusive_bounds_allow_the_cell_past_the_right_edge() {
        let view = Viewport::new(Size::new(10, 5));

        // Each row has one extra, exclusive column, so the step after the last
        // inclusive cell lands on that column before wrapping.
        let mut point = Point::new(9, 0);
        assert!(view.increment_in_exclusive_bounds(&mut point));
        assert_eq!(point, Point::new(10, 0));

        assert!(view.increment_in_exclusive_bounds(&mut point));
        assert_eq!(point, Point::new(0, 1));

        let mut point = Point::new(0, 0);
        assert!(!view.decrement_in_exclusive_bounds(&mut point));
        assert_eq!(point, Point::new(0, 0));

        let mut point = Point::new(10, 4);
        assert!(view.increment_in_exclusive_bounds(&mut point));
        assert_eq!(point, Point::new(0, 5));
    }

    #[test]
    fn compare_in_bounds_measures_distance_in_cells() {
        let view = Viewport::new(Size::new(10, 5));

        assert_eq!(view.compare_in_bounds(Point::new(3, 2), Point::new(3, 2)), 0);
        assert_eq!(view.compare_in_bounds(Point::new(0, 0), Point::new(1, 1)), -11);
        assert_eq!(view.compare_in_bounds(Point::new(1, 1), Point::new(0, 0)), 11);

        // The last cell of one row is one before the first cell of the next.
        assert_eq!(view.compare_in_bounds(Point::new(9, 0), Point::new(0, 1)), -1);
    }
}
