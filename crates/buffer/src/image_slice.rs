//! `ImageSlice` — represents a slice of an image for terminal rendering.
//!
//! Used for sixel image rendering and other image-based terminal features.

/// A rectangular slice of an image, used for rendering images in the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImageSlice {
    /// The x coordinate of the top-left corner.
    x: i32,
    /// The y coordinate of the top-left corner.
    y: i32,
    /// Width of the slice.
    width: i32,
    /// Height of the slice.
    height: i32,
    /// The source image width (total width of the full image).
    source_width: i32,
    /// The source image height (total height of the full image).
    source_height: i32,
}

impl ImageSlice {
    /// Creates a new image slice.
    pub const fn new(x: i32, y: i32, width: i32, height: i32, source_width: i32, source_height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            source_width,
            source_height,
        }
    }

    /// Returns the x coordinate of the top-left corner.
    pub const fn x(&self) -> i32 {
        self.x
    }

    /// Returns the y coordinate of the top-left corner.
    pub const fn y(&self) -> i32 {
        self.y
    }

    /// Returns the width.
    pub const fn width(&self) -> i32 {
        self.width
    }

    /// Returns the height.
    pub const fn height(&self) -> i32 {
        self.height
    }

    /// Returns the area (width * height).
    pub const fn area(&self) -> i32 {
        self.width * self.height
    }

    /// Returns the source image width.
    pub const fn source_width(&self) -> i32 {
        self.source_width
    }

    /// Returns the source image height.
    pub const fn source_height(&self) -> i32 {
        self.source_height
    }

    /// Returns whether the slice is valid (non-zero dimensions).
    pub const fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Returns whether the slice is empty.
    pub const fn is_empty(&self) -> bool {
        !self.is_valid()
    }

    /// Returns whether this slice overlaps with another.
    pub fn overlaps(&self, other: &ImageSlice) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    /// Returns a new slice clipped to the source image bounds.
    pub fn clip_to_source(&self) -> ImageSlice {
        let x = self.x.clamp(0, self.source_width - 1);
        let y = self.y.clamp(0, self.source_height - 1);
        let width = (self.source_width - x).max(0);
        let height = (self.source_height - y).max(0);
        ImageSlice::new(x, y, width, height, self.source_width, self.source_height)
    }

    /// Returns a new slice cropped to the given region (relative to this slice).
    pub fn crop(&self, x: i32, y: i32, width: i32, height: i32) -> ImageSlice {
        let new_x = (self.x + x).max(self.x);
        let new_y = (self.y + y).max(self.y);
        let new_width = width.max(0);
        let new_height = height.max(0);
        ImageSlice::new(
            new_x,
            new_y,
            new_width,
            new_height,
            self.source_width,
            self.source_height,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creation() {
        let slice = ImageSlice::new(0, 0, 100, 50, 200, 100);
        assert_eq!(slice.x(), 0);
        assert_eq!(slice.y(), 0);
        assert_eq!(slice.width(), 100);
        assert_eq!(slice.height(), 50);
        assert_eq!(slice.area(), 5000);
        assert!(slice.is_valid());
    }

    #[test]
    fn test_clip_to_source() {
        let slice = ImageSlice::new(50, 50, 100, 100, 200, 200);
        let clipped = slice.clip_to_source();
        assert_eq!(clipped.x(), 50);
        assert_eq!(clipped.y(), 50);
        assert_eq!(clipped.width(), 150);
        assert_eq!(clipped.height(), 150);

        // Outside source bounds
        let slice2 = ImageSlice::new(150, 150, 100, 100, 200, 200);
        let clipped2 = slice2.clip_to_source();
        assert_eq!(clipped2.width(), 50);
        assert_eq!(clipped2.height(), 50);
    }

    #[test]
    fn test_crop() {
        let slice = ImageSlice::new(0, 0, 100, 100, 100, 100);
        let cropped = slice.crop(10, 10, 50, 50);
        assert_eq!(cropped.x(), 10);
        assert_eq!(cropped.y(), 10);
        assert_eq!(cropped.width(), 50);
        assert_eq!(cropped.height(), 50);
    }

    #[test]
    fn test_overlaps() {
        let slice1 = ImageSlice::new(0, 0, 100, 100, 200, 200);
        let slice2 = ImageSlice::new(50, 50, 100, 100, 200, 200);
        assert!(slice1.overlaps(&slice2));

        let slice3 = ImageSlice::new(99, 99, 50, 50, 200, 200);
        assert!(slice1.overlaps(&slice3)); // overlaps by 1 pixel

        let slice4 = ImageSlice::new(101, 101, 50, 50, 200, 200);
        assert!(!slice1.overlaps(&slice4));
    }

    #[test]
    fn test_empty() {
        let slice = ImageSlice::new(0, 0, 0, 0, 0, 0);
        assert!(slice.is_empty());
        assert!(!slice.is_valid());
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    prop_compose! {
        fn any_image_slice()(
            x in 0i32..100i32,
            y in 0i32..100i32,
            width in 1i32..50i32,
            height in 1i32..50i32,
            src_w in 100i32..200i32,
            src_h in 100i32..200i32,
        ) -> ImageSlice {
            ImageSlice::new(x, y, width, height, src_w, src_h)
        }
    }

    proptest! {
        #[test]
        fn test_clip_preserves_within_bounds(slice in any_image_slice()) {
            let clipped = slice.clip_to_source();
            prop_assert!(clipped.x() >= 0);
            prop_assert!(clipped.y() >= 0);
            prop_assert!(clipped.x() + clipped.width() <= clipped.source_width());
            prop_assert!(clipped.y() + clipped.height() <= clipped.source_height());
        }

        #[test]
        fn test_crop_reduces_area(slice in any_image_slice()) {
            let cropped = slice.crop(10, 10, slice.width() / 2, slice.height() / 2);
            prop_assert!(cropped.area() <= slice.area());
        }

        #[test]
        fn test_overlaps_symmetric(slice1 in any_image_slice(), slice2 in any_image_slice()) {
            if slice1.overlaps(&slice2) {
                prop_assert!(slice2.overlaps(&slice1));
            }
        }
    }
}
