//! A software backend that rasterises a [`RenderingPayload`] into pixels.
//!
//! Ported from the backend concept in `src/renderer/atlas/`, replacing the D3D
//! swap chain with an in-memory `0xAARRGGBB` buffer. Glyph shapes come from a
//! [`GlyphRasterizer`]; the backend owns the layout, grid lines, cursor,
//! selections and image blitting.

use super::common::{
    FontId, IBackend, RenderingPayload, argb_from_colorref, baseline, cell_colors, cell_rect, glyph_origin,
    grid_line_rects,
};

/// A rasterised glyph's coverage mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphBitmap {
    /// Mask width in pixels.
    pub width: u32,
    /// Mask height in pixels.
    pub height: u32,
    /// Left bearing relative to the pen position.
    pub left: i32,
    /// Top offset relative to the baseline; negative means above it.
    pub top: i32,
    /// Row-major coverage in `0..=255`.
    pub coverage: Vec<u8>,
}

/// Supplies glyph coverage masks to a backend.
pub trait GlyphRasterizer {
    /// Rasterises one glyph of `font` into a coverage mask.
    fn rasterize(&mut self, font: FontId, glyph_index: u16) -> Option<GlyphBitmap>;
}

const DEFAULT_CELL_WIDTH: u32 = 8;
const DEFAULT_CELL_HEIGHT: u32 = 16;

/// Renders the atlas payload into an in-memory pixel buffer.
pub struct SoftwareBackend {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
    rasterizer: Option<Box<dyn GlyphRasterizer>>,
}

impl SoftwareBackend {
    /// Creates a backend without a glyph rasteriser.
    pub fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            pixels: Vec::new(),
            rasterizer: None,
        }
    }

    /// Creates a backend with a glyph rasteriser.
    pub fn with_rasterizer(rasterizer: Box<dyn GlyphRasterizer>) -> Self {
        Self {
            rasterizer: Some(rasterizer),
            ..Self::new()
        }
    }

    /// Returns the rendered pixels, row-major.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Returns the buffer width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the buffer height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns one pixel, or `None` when out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.pixels.get(y as usize * self.width as usize + x as usize).copied()
    }

    fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.pixels = vec![0; width as usize * height as usize];
    }

    fn fill_rect(&mut self, left: i32, top: i32, right: i32, bottom: i32, color: u32) {
        for y in top.max(0)..bottom {
            for x in left.max(0)..right {
                self.put_pixel(x, y, color);
            }
        }
    }

    fn put_pixel(&mut self, x: i32, y: i32, color: u32) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as u32, y as u32);
        if x >= self.width || y >= self.height {
            return;
        }
        let index = y as usize * self.width as usize + x as usize;
        if let Some(pixel) = self.pixels.get_mut(index) {
            *pixel = color;
        }
    }

    fn blend_pixel(&mut self, x: i32, y: i32, color: u32, coverage: u8) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as u32, y as u32);
        if x >= self.width || y >= self.height {
            return;
        }
        let index = y as usize * self.width as usize + x as usize;
        let Some(destination) = self.pixels.get_mut(index) else {
            return;
        };

        let alpha = u32::from(coverage);
        let inverse = 255 - alpha;
        let existing = *destination;
        let blend = |foreground: u8, background: u8| -> u8 {
            ((u32::from(foreground) * alpha + u32::from(background) * inverse) / 255) as u8
        };

        let r = blend((color >> 16) as u8, (existing >> 16) as u8);
        let g = blend((color >> 8) as u8, (existing >> 8) as u8);
        let b = blend(color as u8, existing as u8);
        *destination = 0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
    }

    fn render_backgrounds(&mut self, payload: &RenderingPayload, cell_width: u32, cell_height: u32) {
        let (columns, rows) = payload.settings.viewport_cell_count;
        for row in 0..rows {
            for column in 0..columns {
                let (background, _) = cell_colors(payload, column, row);
                let (left, top, right, bottom) = cell_rect(column, row, cell_width, cell_height);
                self.fill_rect(left, top, right, bottom, background);
            }
        }
    }

    fn render_glyphs(&mut self, payload: &RenderingPayload, cell_width: u32, cell_height: u32) {
        let Some(mut rasterizer) = self.rasterizer.take() else {
            return;
        };

        let baseline = baseline(payload, cell_height);

        for (row_index, row) in payload.rows.iter().enumerate() {
            for (glyph, &column) in row.glyph_indices.iter().zip(&row.glyph_columns) {
                let (_, foreground) = cell_colors(payload, column, row_index as u16);
                let Some(bitmap) = rasterizer.rasterize(FontId(0), *glyph) else {
                    continue;
                };

                let (origin_x, origin_y) = glyph_origin(
                    column,
                    row_index as u16,
                    cell_width,
                    cell_height,
                    baseline,
                    bitmap.left,
                    bitmap.top,
                );

                for y in 0..bitmap.height {
                    for x in 0..bitmap.width {
                        let index = y as usize * bitmap.width as usize + x as usize;
                        if let Some(&coverage) = bitmap.coverage.get(index)
                            && coverage > 0
                        {
                            self.blend_pixel(origin_x + x as i32, origin_y + y as i32, foreground, coverage);
                        }
                    }
                }
            }
        }

        self.rasterizer = Some(rasterizer);
    }

    fn render_grid_lines(&mut self, payload: &RenderingPayload, cell_width: u32, cell_height: u32) {
        for row_index in 0..payload.rows.len() as u16 {
            for rect in grid_line_rects(payload, row_index, cell_width, cell_height) {
                self.fill_rect(rect.left, rect.top, rect.right, rect.bottom, rect.color);
            }
        }
    }

    fn render_images(&mut self, payload: &RenderingPayload, cell_width: u32, cell_height: u32) {
        for (row_index, row) in payload.rows.iter().enumerate() {
            let bitmap = &row.bitmap;
            if !bitmap.active || bitmap.target_width <= 0 {
                continue;
            }
            let (source_width, source_height) = bitmap.source_size;
            if source_width <= 0 || source_height <= 0 {
                continue;
            }

            let left = bitmap.target_offset.max(0) * cell_width as i32;
            let top = row_index as i32 * cell_height as i32;
            let width = bitmap.target_width * cell_width as i32;

            for y in 0..cell_height as i32 {
                let source_y = (y * source_height / cell_height as i32).clamp(0, source_height - 1);
                for x in 0..width {
                    let source_x = (x * source_width / width).clamp(0, source_width - 1);
                    let index = source_y as usize * source_width as usize + source_x as usize;
                    if let Some(&pixel) = bitmap.source.get(index) {
                        self.put_pixel(left + x, top + y, pixel);
                    }
                }
            }
        }
    }

    fn render_cursor(&mut self, payload: &RenderingPayload, cell_width: u32, cell_height: u32) {
        let (left, top, right, bottom) = payload.cursor_rect;
        if right <= left || bottom <= top {
            return;
        }

        let color = argb_from_colorref(payload.settings.cursor.cursor_color);
        let x0 = left * cell_width as i32;
        let y0 = top * cell_height as i32;
        let x1 = right * cell_width as i32;
        let y1 = bottom * cell_height as i32;
        self.fill_rect(x0, y0, x1, y1, color);
    }
}

impl Default for SoftwareBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl IBackend for SoftwareBackend {
    fn release_resources(&mut self) {
        self.pixels = Vec::new();
        self.width = 0;
        self.height = 0;
    }

    fn render(&mut self, payload: &mut RenderingPayload) {
        let (cell_width, cell_height) = payload.settings.font.cell_size;
        let cell_width = if cell_width == 0 {
            DEFAULT_CELL_WIDTH
        } else {
            u32::from(cell_width)
        };
        let cell_height = if cell_height == 0 {
            DEFAULT_CELL_HEIGHT
        } else {
            u32::from(cell_height)
        };

        let (columns, rows) = payload.settings.viewport_cell_count;
        let (target_width, target_height) = payload.settings.target_size;
        let width = if target_width > 0 {
            u32::from(target_width)
        } else {
            u32::from(columns) * cell_width
        };
        let height = if target_height > 0 {
            u32::from(target_height)
        } else {
            u32::from(rows) * cell_height
        };

        self.resize(width, height);
        self.render_backgrounds(payload, cell_width, cell_height);
        self.render_images(payload, cell_width, cell_height);
        self.render_glyphs(payload, cell_width, cell_height);
        self.render_grid_lines(payload, cell_width, cell_height);
        self.render_cursor(payload, cell_width, cell_height);
    }

    fn requires_continuous_redraw(&mut self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::common::CursorSettings;
    use crate::atlas::common::{Bitmap, FontSettings, GridLineRange, MiscellaneousSettings, Settings, ShapedRow};
    use crate::render_frame_info::{GridLineSet, GridLines};

    /// A rasteriser that fills its whole glyph with coverage.
    struct SolidRasterizer;

    impl GlyphRasterizer for SolidRasterizer {
        fn rasterize(&mut self, _font: FontId, _glyph_index: u16) -> Option<GlyphBitmap> {
            Some(GlyphBitmap {
                width: 1,
                height: 1,
                left: 0,
                top: 0,
                coverage: vec![255],
            })
        }
    }

    fn settings() -> Settings {
        Settings {
            font: FontSettings {
                cell_size: (8, 16),
                ..FontSettings::default()
            },
            misc: MiscellaneousSettings {
                background_color: 0x0000_0000,
                foreground_color: 0x00FF_FFFF,
                ..MiscellaneousSettings::default()
            },
            cursor: CursorSettings {
                cursor_color: 0x0000_00FF,
                ..CursorSettings::default()
            },
            viewport_cell_count: (2, 1),
            ..Settings::default()
        }
    }

    fn payload() -> RenderingPayload {
        let mut payload = RenderingPayload {
            settings: settings(),
            ..RenderingPayload::default()
        };

        let background = argb_from_colorref(payload.settings.misc.background_color);
        let foreground = argb_from_colorref(payload.settings.misc.foreground_color);
        payload.color_bitmap_row_stride = 2;
        payload.color_bitmap_depth_stride = 2;
        payload.color_bitmap = vec![background, background, foreground, foreground, 0, 0];

        let mut row = ShapedRow::default();
        row.push_glyphs(
            FontId(0),
            &[super::super::common::ShapedGlyph {
                glyph_index: u16::from(b'A'),
                advance: 1.0,
                offset: (0.0, 0.0),
                color: None,
            }],
            0,
        );
        payload.rows = vec![row];
        payload
    }

    #[test]
    fn background_covers_every_cell() {
        let mut backend = SoftwareBackend::new();
        let mut payload = payload();
        payload.rows.clear();

        backend.render(&mut payload);

        assert_eq!(backend.width(), 16);
        assert_eq!(backend.height(), 16);
        assert_eq!(backend.pixel(0, 0), Some(0xFF00_0000));
        assert_eq!(backend.pixel(15, 15), Some(0xFF00_0000));
    }

    #[test]
    fn a_glyph_paints_the_foreground_colour() {
        let mut backend = SoftwareBackend::with_rasterizer(Box::new(SolidRasterizer));
        let mut payload = payload();

        backend.render(&mut payload);

        // The 16px cell puts the baseline at y = 12.
        assert_eq!(backend.pixel(0, 12), Some(0xFFFF_FFFF));
        assert_eq!(backend.pixel(0, 0), Some(0xFF00_0000));
        assert_eq!(backend.pixel(8, 12), Some(0xFF00_0000));
    }

    #[test]
    fn an_underline_is_drawn_under_the_cell() {
        let mut backend = SoftwareBackend::new();
        let mut payload = payload();
        payload.rows[0].grid_line_ranges.push(GridLineRange {
            lines: GridLineSet::new(),
            from: 0,
            to: 1,
        });
        payload.rows[0].grid_line_ranges[0].lines.insert(GridLines::Underline);

        backend.render(&mut payload);

        assert_eq!(backend.pixel(0, 14), Some(0xFFFF_FFFF));
    }

    #[test]
    fn the_cursor_is_drawn_over_the_cell() {
        let mut backend = SoftwareBackend::new();
        let mut payload = payload();
        payload.rows.clear();
        payload.cursor_rect = (1, 0, 2, 1);

        backend.render(&mut payload);

        assert_eq!(backend.pixel(8, 0), Some(0xFFFF_0000));
    }

    #[test]
    fn an_image_bitmap_is_scaled_into_its_cells() {
        let mut backend = SoftwareBackend::new();
        let mut payload = payload();
        payload.rows.clear();
        payload.rows.push(ShapedRow::default());
        payload.rows[0].bitmap = Bitmap {
            revision: 1,
            source: vec![0xFFFF_0000; 8 * 16],
            source_size: (8, 16),
            target_offset: 0,
            target_width: 1,
            active: true,
        };

        backend.render(&mut payload);

        // One cell wide at 8x16 pixels: the whole first cell is the image.
        assert_eq!(backend.pixel(0, 0), Some(0xFFFF_0000));
        assert_eq!(backend.pixel(7, 15), Some(0xFFFF_0000));
        // The second cell is untouched.
        assert_eq!(backend.pixel(8, 0), Some(0xFF00_0000));
    }

    #[test]
    fn releasing_resources_empties_the_buffer() {
        let mut backend = SoftwareBackend::new();
        backend.release_resources();
        assert_eq!(backend.width(), 0);
        assert!(backend.pixels().is_empty());
    }
}
