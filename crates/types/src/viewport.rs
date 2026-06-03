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
        let end_x =
            (self.origin.x() + self.size.width()).min(other.origin.x() + other.size.width());
        let end_y =
            (self.origin.y() + self.size.height()).min(other.origin.y() + other.size.height());

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
        let end_x =
            (self.origin.x() + self.size.width()).max(other.origin.x() + other.size.width());
        let end_y =
            (self.origin.y() + self.size.height()).max(other.origin.y() + other.size.height());

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
}
