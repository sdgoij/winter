//! `ImageSlice` — a row's image band and its pixel operations.
//!
//! Ported from `ImageSlice.hpp/cpp` (ADR 005). A band is one cell tall and
//! covers the cell columns `column_begin..column_end`; the buffer is row-major
//! with `pixel_width` pixels per row, and pixels are `0xAARRGGBB` with zero
//! meaning transparent, so erasing a cell zeroes its pixels.
//!
//! The sixel decoder writes through [`ImageSlice::mutable_pixels`]; the buffer
//! erases and copies bands alongside the cells they cover.

use std::sync::atomic::{AtomicU64, Ordering};

/// The source of image revision numbers.
///
/// Upstream uses one process-wide counter so a revision is never reused; zero
/// is reserved as the renderer's "never drawn" sentinel.
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

/// A horizontal band of pixels covering a run of cells in one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSlice {
    revision: u64,
    /// Pixels per cell column and per cell row.
    cell_size: (i32, i32),
    /// Row-major pixels, `0xAARRGGBB`; zero is transparent.
    pixel_buffer: Vec<u32>,
    /// Pixels per row of the buffer.
    pixel_width: i32,
    /// The first covered cell column.
    column_begin: i32,
    /// One past the last covered cell column.
    column_end: i32,
}

impl ImageSlice {
    /// Creates an empty band for cells of `cell_size` pixels.
    pub const fn new(cell_size: (i32, i32)) -> Self {
        Self {
            revision: 0,
            cell_size,
            pixel_buffer: Vec::new(),
            pixel_width: 0,
            column_begin: 0,
            column_end: 0,
        }
    }

    /// Marks the pixels as changed for the renderer.
    pub fn bump_revision(&mut self) {
        self.revision = NEXT_REVISION.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns the revision this band was last marked with.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the pixel size of one cell.
    pub const fn cell_size(&self) -> (i32, i32) {
        self.cell_size
    }

    /// Returns the first covered cell column.
    pub const fn column_begin(&self) -> i32 {
        self.column_begin
    }

    /// Returns one past the last covered cell column.
    pub const fn column_end(&self) -> i32 {
        self.column_end
    }

    /// Returns the pixels per buffer row.
    pub const fn pixel_width(&self) -> i32 {
        self.pixel_width
    }

    /// Returns the pixels, row-major.
    pub fn pixels(&self) -> &[u32] {
        &self.pixel_buffer
    }

    /// Returns whether nothing is left to draw.
    pub const fn is_empty(&self) -> bool {
        self.column_begin >= self.column_end
    }

    /// Returns the pixels for `column_begin..column_end`, growing the band to
    /// cover them if needed.
    ///
    /// Ported from `ImageSlice::MutablePixels`: growing moves the existing
    /// pixels to their new offset instead of dropping them.
    pub fn mutable_pixels(&mut self, column_begin: i32, column_end: i32) -> &mut [u32] {
        let needs_resize =
            self.pixel_buffer.is_empty() || column_begin < self.column_begin || column_end > self.column_end;
        if needs_resize {
            let existing = !self.pixel_buffer.is_empty();
            let old_begin = self.column_begin;
            let old_width = self.pixel_width;

            self.column_begin = if existing {
                self.column_begin.min(column_begin)
            } else {
                column_begin
            };
            self.column_end = if existing {
                self.column_end.max(column_end)
            } else {
                column_end
            };
            self.pixel_width = (self.column_end - self.column_begin) * self.cell_size.0;

            let height = self.cell_size.1.max(0);
            let mut new_buffer = vec![0u32; (self.pixel_width * height).max(0) as usize];
            if existing {
                let offset = (old_begin - self.column_begin) * self.cell_size.0;
                let copy_width = old_width.min(self.pixel_width - offset).max(0);
                for y in 0..height {
                    let src_start = (y * old_width).max(0) as usize;
                    let dst_start = (y * self.pixel_width + offset).max(0) as usize;
                    let count = copy_width as usize;
                    new_buffer[dst_start..dst_start + count]
                        .copy_from_slice(&self.pixel_buffer[src_start..src_start + count]);
                }
            }
            self.pixel_buffer = new_buffer;
        }

        &mut self.pixel_buffer
    }

    /// Zeroes the pixels under `column_begin..column_end`.
    ///
    /// Returns true when the whole band is now empty, which tells the caller to
    /// remove it from its row (ported from `ImageSlice::_eraseCells`).
    pub fn erase_cells(&mut self, column_begin: i32, column_end: i32) -> bool {
        if column_begin <= self.column_begin && column_end >= self.column_end {
            return true;
        }

        let erase_begin = column_begin.max(self.column_begin);
        let erase_end = column_end.min(self.column_end);
        if erase_begin < erase_end {
            let offset = (erase_begin - self.column_begin) * self.cell_size.0;
            let length = (erase_end - erase_begin) * self.cell_size.0;
            for y in 0..self.cell_size.1.max(0) {
                let start = (y * self.pixel_width + offset).max(0) as usize;
                self.pixel_buffer[start..start + length.max(0) as usize].fill(0);
            }
        }
        false
    }

    /// Copies the pixels for `source_begin..source_end` from `source` into
    /// `dest_begin..`.
    ///
    /// Ported from `ImageSlice::_copyCells` without the erase-outside logic,
    /// which the row helper performs first. Returns true when the band is empty
    /// afterwards.
    pub fn copy_cells_from(&mut self, source: &ImageSlice, source_begin: i32, dest_begin: i32, dest_end: i32) -> bool {
        if source.cell_size != self.cell_size {
            return self.is_empty();
        }

        let source_end = source_begin + (dest_end - dest_begin);
        let copy_begin = source_begin.max(source.column_begin);
        let copy_end = source_end.min(source.column_end).max(copy_begin);
        if copy_begin < copy_end {
            let projected = copy_begin + (dest_begin - source_begin);
            let projected_end = copy_end + (dest_begin - source_begin);
            self.mutable_pixels(projected, projected_end);

            let source_offset = (copy_begin - source.column_begin) * source.cell_size.0;
            let dest_offset = (projected - self.column_begin) * self.cell_size.0;
            let width = (copy_end - copy_begin) * source.cell_size.0;
            for y in 0..self.cell_size.1.max(0) {
                let src_start = (y * source.pixel_width + source_offset).max(0) as usize;
                let dst_start = (y * self.pixel_width + dest_offset).max(0) as usize;
                let count = width.max(0) as usize;
                self.pixel_buffer[dst_start..dst_start + count]
                    .copy_from_slice(&source.pixel_buffer[src_start..src_start + count]);
            }
        }
        self.is_empty()
    }

    /// Shifts the pixels at and right of `start` right by `count` cells, which
    /// is what insert mode does to an image.
    pub fn shift_right_cells(&mut self, start: i32, count: i32) {
        if count <= 0 || start < self.column_begin || start >= self.column_end {
            return;
        }
        let moved_columns = self.column_end - start;
        let extend_to = self.column_end + count;
        self.mutable_pixels(self.column_begin, extend_to);

        let start_offset = (start - self.column_begin) * self.cell_size.0;
        let shift = count * self.cell_size.0;
        let width = moved_columns * self.cell_size.0;
        for y in 0..self.cell_size.1.max(0) {
            let base = (y * self.pixel_width).max(0) as usize;
            let range_begin = base + start_offset.max(0) as usize;
            let range_end = (range_begin + width.max(0) as usize).min(base + self.pixel_width.max(0) as usize);
            if range_begin < range_end {
                self.pixel_buffer
                    .copy_within(range_begin..range_end, range_begin + shift.max(0) as usize);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band(cell_size: (i32, i32), begin: i32, end: i32) -> ImageSlice {
        let mut image = ImageSlice::new(cell_size);
        image.mutable_pixels(begin, end);
        image
    }

    #[test]
    fn a_new_band_is_empty() {
        let image = ImageSlice::new((1, 1));
        assert!(image.is_empty());
        assert_eq!(image.pixel_width(), 0);
        assert!(image.pixels().is_empty());
    }

    #[test]
    fn growing_covers_the_requested_columns() {
        let mut image = ImageSlice::new((2, 1));
        image.mutable_pixels(3, 5);

        assert_eq!(image.column_begin(), 3);
        assert_eq!(image.column_end(), 5);
        assert_eq!(image.pixel_width(), 4);
        assert_eq!(image.pixels().len(), 4);
        assert!(!image.is_empty());
    }

    #[test]
    fn growing_keeps_existing_pixels_at_their_offset() {
        let mut image = ImageSlice::new((1, 1));
        image.mutable_pixels(2, 4);
        image.pixel_buffer[0] = 7;

        image.mutable_pixels(0, 4);

        assert_eq!(image.column_begin(), 0);
        assert_eq!(image.pixel_width(), 4);
        assert_eq!(image.pixels()[2], 7);
    }

    #[test]
    fn erasing_a_partial_range_keeps_the_band() {
        let mut image = band((1, 1), 0, 4);
        image.pixel_buffer.copy_from_slice(&[1, 2, 3, 4]);

        assert!(!image.erase_cells(1, 3));
        assert_eq!(image.pixels(), &[1, 0, 0, 4]);
        assert!(!image.is_empty());
    }

    #[test]
    fn erasing_the_whole_range_reports_empty() {
        let mut image = band((1, 1), 2, 4);
        assert!(image.erase_cells(0, 6));
    }

    #[test]
    fn copying_cells_moves_the_pixels() {
        let mut source = band((1, 1), 0, 4);
        source.pixel_buffer.copy_from_slice(&[1, 2, 3, 4]);
        let mut destination = band((1, 1), 0, 4);

        destination.copy_cells_from(&source, 1, 2, 4);

        assert_eq!(destination.pixels(), &[0, 0, 2, 3]);
    }

    #[test]
    fn insert_shifts_the_image_right() {
        let mut image = band((1, 1), 0, 3);
        image.pixel_buffer.copy_from_slice(&[1, 2, 3]);

        image.shift_right_cells(1, 1);
        image.erase_cells(1, 2);

        assert_eq!(image.column_begin(), 0);
        assert_eq!(image.column_end(), 4);
        assert_eq!(image.pixels(), &[1, 0, 2, 3]);
    }

    #[test]
    fn revisions_are_unique() {
        let mut first = ImageSlice::new((1, 1));
        let mut second = ImageSlice::new((1, 1));
        first.bump_revision();
        second.bump_revision();

        assert_ne!(first.revision(), 0);
        assert_ne!(first.revision(), second.revision());
    }
}
