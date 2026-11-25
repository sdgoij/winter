//! A software backend whose last frame its owner can read.
//!
//! `AtlasEngine` owns its `IBackend`, so a shell that presents the software
//! pixels needs a handle to them after `Renderer::paint_frame` returns. This
//! wrapper is that handle: the engine renders into the shared buffer, and the
//! owner reads it through [`SharedBackend::with_pixels`] (ADR 040).
//!
//! The handle is single-threaded on purpose: a window toolkit paints on its UI
//! thread, and the rasteriser it holds is not `Send`.

use std::cell::{Ref, RefCell};
use std::rc::Rc;

use super::common::{IBackend, RenderingPayload};
use super::software::SoftwareBackend;

/// A [`SoftwareBackend`] shared between the atlas engine and its owner.
#[derive(Clone)]
pub struct SharedBackend {
    inner: Rc<RefCell<SoftwareBackend>>,
}

impl SharedBackend {
    /// Wraps `backend` so both the engine and the owner can reach it.
    pub fn new(backend: SoftwareBackend) -> Self {
        Self {
            inner: Rc::new(RefCell::new(backend)),
        }
    }

    /// Runs `read` over the last frame's pixels, row-major `0xAARRGGBB`.
    pub fn with_pixels<T>(&self, read: impl FnOnce(&[u32]) -> T) -> T {
        read(self.inner.borrow().pixels())
    }

    /// Borrows the last frame's pixels, row-major `0xAARRGGBB`.
    ///
    /// A host that copies the frame past the call cannot use [`with_pixels`],
    /// because the borrow ends with the closure; this hands the borrow back
    /// instead (ADR 082).
    pub fn pixels(&self) -> Ref<'_, [u32]> {
        Ref::map(self.inner.borrow(), |backend| backend.pixels())
    }

    /// Returns the last frame's width in pixels.
    pub fn width(&self) -> u32 {
        self.inner.borrow().width()
    }

    /// Returns the last frame's height in pixels.
    pub fn height(&self) -> u32 {
        self.inner.borrow().height()
    }
}

impl IBackend for SharedBackend {
    fn release_resources(&mut self) {
        self.inner.borrow_mut().release_resources();
    }

    fn render(&mut self, payload: &mut RenderingPayload) {
        self.inner.borrow_mut().render(payload);
    }

    fn requires_continuous_redraw(&mut self) -> bool {
        self.inner.borrow_mut().requires_continuous_redraw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::common::{FontSettings, Settings};

    /// A one-cell payload whose background is opaque black.
    fn payload() -> RenderingPayload {
        let settings = Settings {
            target_size: (2, 2),
            viewport_cell_count: (1, 1),
            font: FontSettings {
                cell_size: (2, 2),
                ..FontSettings::default()
            },
            ..Settings::default()
        };

        RenderingPayload {
            settings,
            // Three stacked colour bitmaps: background, foreground, underline.
            color_bitmap: vec![0xFF00_0000; 3],
            color_bitmap_row_stride: 1,
            color_bitmap_depth_stride: 1,
            ..RenderingPayload::default()
        }
    }

    #[test]
    fn the_owner_reads_what_the_engine_rendered() {
        let shared = SharedBackend::new(SoftwareBackend::new());
        let mut backend = shared.clone();

        backend.render(&mut payload());

        assert_eq!(shared.width(), 2);
        assert_eq!(shared.height(), 2);
        shared.with_pixels(|pixels| {
            assert_eq!(pixels.len(), 4);
            assert!(pixels.iter().all(|&pixel| pixel == 0xFF00_0000), "{pixels:?}");
        });
    }
}
