//! The icon a tab wears: its program's, or a terminal's.
//!
//! Windows extracts the icon from the executable and macOS asks Launch
//! Services. Linux has neither, so the desktop's icon theme is the source: a
//! theme icon named after the program wins, and `utilities-terminal` is the
//! fallback (ADR 070's rule, on this platform).
//!
//! The theme draws an icon through GSK, not Cairo, so it is rendered once per
//! program and pixel size and cached as a Cairo surface; the strip is painted
//! every frame and must not resolve an icon per frame.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use gtk4 as gtk;

use gtk::prelude::*;
use gtk::{cairo, gdk, gio};

/// The size a tab's icon is drawn at, matching the other shells.
pub const SIZE: f64 = 16.0;

/// The space between an icon and its title.
pub const GAP: i32 = 6;

/// A rendered icon, or nothing when the theme has none for the program.
type Rendered = Option<Rc<cairo::ImageSurface>>;

/// The icons already resolved, keyed by program and rendered pixel size.
#[derive(Default)]
pub struct Icons {
    cache: RefCell<HashMap<String, HashMap<i32, Rendered>>>,
}

impl Icons {
    pub fn new() -> Self {
        Self::default()
    }

    /// `program`'s icon at `pixels` physical pixels, or `None` when the theme
    /// has nothing to draw for it.
    ///
    /// `native` is the widget that owns the GSK renderer the icon is drawn
    /// through; the strip passes itself.
    pub fn for_program(&self, native: &impl IsA<gtk::Widget>, program: &str, pixels: i32) -> Rendered {
        if let Some(found) = self.cache.borrow().get(program).and_then(|sizes| sizes.get(&pixels)) {
            return found.clone();
        }

        let icon = render(native, program, pixels);
        // A failure may be a renderer that is not ready yet, so only a success
        // is remembered.
        if icon.is_some() {
            self.cache
                .borrow_mut()
                .entry(program.to_string())
                .or_default()
                .insert(pixels, icon.clone());
        }
        icon
    }
}

/// Draws `program`'s theme icon at `pixels` physical pixels into a surface.
fn render(native: &impl IsA<gtk::Widget>, program: &str, pixels: i32) -> Rendered {
    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    let name = if theme.has_icon(icon_name(program)) {
        icon_name(program)
    } else {
        "utilities-terminal"
    };
    if !theme.has_icon(name) {
        return None;
    }

    let icon = gio::ThemedIcon::new(name);
    let paintable = theme.lookup_by_gicon(
        &icon,
        pixels,
        1,
        gtk::TextDirection::None,
        gtk::IconLookupFlags::empty(),
    );

    let size = f64::from(pixels);
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, size, size);
    let node = snapshot.to_node()?;
    let renderer = native.native()?.renderer()?;
    let texture = renderer.render_texture(&node, None);

    let downloader = gdk::TextureDownloader::new(&texture);
    let (bytes, source_stride) = downloader.download_bytes();
    let width = texture.width() as usize;
    let height = texture.height() as usize;
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width as i32, height as i32).ok()?;
    let destination_stride = surface.stride() as usize;
    {
        let mut destination = surface.data().ok()?;
        let source: &[u8] = bytes.as_ref();
        if !copy_pixels(
            source,
            source_stride,
            width,
            height,
            downloader.format(),
            &mut destination,
            destination_stride,
        ) {
            return None;
        }
    }

    Some(Rc::new(surface))
}

/// The icon name a program is asked for: its file name without a directory.
fn icon_name(program: &str) -> &str {
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
}

/// Copies a GSK texture into a Cairo `ARGB32` surface.
///
/// Cairo stores a pixel as premultiplied `0xAARRGGBB` in native byte order, so
/// a little-endian machine wants B, G, R, A. GDK names its formats by the order
/// the bytes have in memory instead, and a straight format must be
/// premultiplied on the way in.
fn copy_pixels(
    source: &[u8],
    source_stride: usize,
    width: usize,
    height: usize,
    format: gdk::MemoryFormat,
    destination: &mut [u8],
    destination_stride: usize,
) -> bool {
    let (red, green, blue, alpha, premultiplied) = match format {
        gdk::MemoryFormat::R8g8b8a8Premultiplied => (0, 1, 2, 3, true),
        gdk::MemoryFormat::R8g8b8a8 => (0, 1, 2, 3, false),
        gdk::MemoryFormat::B8g8r8a8Premultiplied => (2, 1, 0, 3, true),
        gdk::MemoryFormat::B8g8r8a8 => (2, 1, 0, 3, false),
        gdk::MemoryFormat::A8r8g8b8Premultiplied => (1, 2, 3, 0, true),
        gdk::MemoryFormat::A8r8g8b8 => (1, 2, 3, 0, false),
        gdk::MemoryFormat::A8b8g8r8 => (3, 2, 1, 0, false),
        _ => return false,
    };

    let Some(row) = width.checked_mul(4) else {
        return false;
    };
    if width == 0 || height == 0 || source_stride < row || destination_stride < row {
        return false;
    }
    let Some(source_len) = source_stride.checked_mul(height) else {
        return false;
    };
    let Some(destination_len) = destination_stride.checked_mul(height) else {
        return false;
    };
    if source.len() < source_len || destination.len() < destination_len {
        return false;
    }

    for line in 0..height {
        let start = line * source_stride;
        let source = &source[start..start + row];
        let start = line * destination_stride;
        let destination = &mut destination[start..start + row];
        let (source, _) = source.as_chunks::<4>();
        let (destination, _) = destination.as_chunks_mut::<4>();
        for (source, destination) in source.iter().zip(destination.iter_mut()) {
            let alpha_value = source[alpha];
            let channel = |value: u8| {
                if premultiplied {
                    value
                } else {
                    ((u16::from(value) * u16::from(alpha_value) + 127) / 255) as u8
                }
            };
            destination[0] = channel(source[blue]);
            destination[1] = channel(source[green]);
            destination[2] = channel(source[red]);
            destination[3] = alpha_value;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdk::MemoryFormat;

    #[test]
    fn a_program_path_is_reduced_to_its_icon_name() {
        assert_eq!(icon_name("/usr/bin/bash"), "bash");
        assert_eq!(icon_name("pwsh"), "pwsh");
    }

    #[test]
    fn an_rgba_icon_is_swizzled_to_cairos_argb32() {
        let source = [10, 20, 30, 40];
        let mut destination = [0u8; 4];
        assert!(copy_pixels(
            &source,
            4,
            1,
            1,
            MemoryFormat::R8g8b8a8Premultiplied,
            &mut destination,
            4
        ));
        assert_eq!(destination, [30, 20, 10, 40]);
    }

    #[test]
    fn a_bgra_icon_is_already_in_cairos_order() {
        let source = [30, 20, 10, 40];
        let mut destination = [0u8; 4];
        assert!(copy_pixels(
            &source,
            4,
            1,
            1,
            MemoryFormat::B8g8r8a8Premultiplied,
            &mut destination,
            4
        ));
        assert_eq!(destination, [30, 20, 10, 40]);
    }

    #[test]
    fn a_straight_icon_is_premultiplied_on_the_way_in() {
        let source = [200, 100, 50, 128];
        let mut destination = [0u8; 4];
        assert!(copy_pixels(
            &source,
            4,
            1,
            1,
            MemoryFormat::R8g8b8a8,
            &mut destination,
            4
        ));
        assert_eq!(destination, [25, 50, 100, 128]);
    }

    #[test]
    fn an_alpha_first_icon_is_read_in_its_memory_order() {
        // A, R, G, B.
        let source = [40, 10, 20, 30];
        let mut destination = [0u8; 4];
        assert!(copy_pixels(
            &source,
            4,
            1,
            1,
            MemoryFormat::A8r8g8b8Premultiplied,
            &mut destination,
            4
        ));
        assert_eq!(destination, [30, 20, 10, 40]);
    }

    #[test]
    fn a_row_is_read_with_its_stride_and_a_short_buffer_is_refused() {
        // Two pixels in a row of four: the padding is skipped.
        let source = [1, 2, 3, 4, 5, 6, 7, 8, 200, 200, 200, 200];
        let mut destination = [0u8; 8];
        assert!(copy_pixels(
            &source,
            8,
            2,
            1,
            MemoryFormat::R8g8b8a8Premultiplied,
            &mut destination,
            8
        ));
        assert_eq!(destination, [3, 2, 1, 4, 7, 6, 5, 8]);
        assert!(!copy_pixels(
            &source,
            8,
            2,
            1,
            MemoryFormat::R8g8b8a8Premultiplied,
            &mut destination,
            4
        ));
    }
}
