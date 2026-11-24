//! The terminal's pixels as an AppKit image.
//!
//! `renderer`'s software backend writes `0xAARRGGBB`, which is BGRA in memory
//! on a little-endian host. An `NSBitmapImageRep` created with `AlphaFirst`
//! has exactly that layout, so a frame is a byte copy: no swizzle, and the same
//! "blit the software backend" shape the WinUI shell uses (ADR 041).
//!
//! One bitmap is reused for the life of a size; only a grid change recreates
//! it. The view draws it into the pane's rect with `respectFlipped`, because
//! the terminal view is flipped and the pixels are top-down.

use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSBitmapFormat, NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace, NSImage};
use objc2_foundation::{NSPoint, NSRect, NSSize};

/// A size's pixel buffer and the image AppKit draws it with.
pub(crate) struct Frame {
    rep: objc2::rc::Retained<NSBitmapImageRep>,
    image: objc2::rc::Retained<NSImage>,
    width: u32,
    height: u32,
}

impl Frame {
    /// Creates a frame of `width` by `height` pixels.
    pub(crate) fn new(width: u32, height: u32, _mtm: MainThreadMarker) -> Option<Self> {
        let (width, height) = (width.max(1), height.max(1));
        let rep = allocate(width, height)?;
        let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(f64::from(width), f64::from(height)));
        image.addRepresentation(&rep);

        Some(Self {
            rep,
            image,
            width,
            height,
        })
    }

    /// Copies `pixels` into the frame, recreating it when the size changed.
    ///
    /// Returns whether a frame is ready to draw.
    pub(crate) fn copy(&mut self, pixels: &[u32], width: u32, height: u32, mtm: MainThreadMarker) -> bool {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) != (self.width, self.height) {
            let Some(replacement) = Self::new(width, height, mtm) else {
                return false;
            };
            *self = replacement;
        }
        if pixels.len() < self.width as usize * self.height as usize {
            return false;
        }

        let destination = self.rep.bitmapData();
        if destination.is_null() {
            return false;
        }

        // SAFETY: `bitmapData` points at the bitmap the rep allocated, which is
        // `width * height * 4` bytes for the layout requested in `allocate`.
        // The slice is borrowed for the copy only, and `pixels` is at least the
        // size checked above.
        unsafe {
            let target = std::slice::from_raw_parts_mut(destination, self.width as usize * self.height as usize * 4);
            for (index, pixel) in pixels
                .iter()
                .take(self.width as usize * self.height as usize)
                .enumerate()
            {
                let bytes = pixel_bytes(*pixel);
                target[index * 4..index * 4 + 4].copy_from_slice(&bytes);
            }
        }

        true
    }

    /// Draws the frame into `rect`.
    pub(crate) fn draw(&self, rect: NSRect) {
        let source = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(f64::from(self.width), f64::from(self.height)),
        );
        // SAFETY: the image is alive for the call, the rect is in the view's
        // coordinate space, and `hints` is allowed to be null.
        unsafe {
            self.image.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
                rect,
                source,
                NSCompositingOperation::SourceOver,
                1.0,
                true,
                None,
            );
        }
    }
}

/// Allocates a rep whose samples are `A`, `R`, `G`, `B` in memory.
fn allocate(width: u32, height: u32) -> Option<objc2::rc::Retained<NSBitmapImageRep>> {
    let width = isize::try_from(width).ok()?;
    let height = isize::try_from(height).ok()?;
    let bytes_per_row = width.checked_mul(4)?;

    // SAFETY: `planes` is null, which asks AppKit to allocate the buffer; the
    // remaining arguments describe a 32-bit ARGB image, and `AlphaFirst` makes
    // that `A`, `R`, `G`, `B` in the big-endian sample order a rep without an
    // endian flag uses.
    unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width,
            height,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            NSBitmapFormat::AlphaFirst,
            bytes_per_row,
            32,
        )
    }
}

/// One `COLORREF` as the bytes an `AlphaFirst` bitmap rep reads.
///
/// The alpha byte is supplied because a `COLORREF` has none, and the samples
/// are big-endian because a rep without an endian flag reads them in channel
/// order: `A`, `R`, `G`, `B`.
fn pixel_bytes(pixel: u32) -> [u8; 4] {
    (pixel | 0xFF00_0000).to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colorref_becomes_an_opaque_argb_sample() {
        assert_eq!(pixel_bytes(0x0012_3456), [0xFF, 0x12, 0x34, 0x56]);
        assert_eq!(pixel_bytes(0x00FF_FFFF), [0xFF, 0xFF, 0xFF, 0xFF]);
    }
}
