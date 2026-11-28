//! The Direct2D painter behind a `SwapChainPanel`.
//!
//! The software backend rasterises the pane on the CPU and the surface blits the
//! result; this backend keeps the frame on the GPU. Cell backgrounds, grid lines
//! and the cursor are solid brushes, and text is a DirectWrite glyph run per
//! colour change, drawn with `DrawGlyphRun` on the pane's device context
//! (ADR 063).
//!
//! Every `unsafe` call carries a `SAFETY:` note, as the rest of `winui` does.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use renderer::IBackend;
use renderer::atlas::common::{
    RenderingPayload, argb_from_colorref, baseline, cell_colors, cell_rect, grid_line_rects,
};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, ID2D1SolidColorBrush};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_FACE_TYPE_TRUETYPE, DWRITE_FONT_SIMULATIONS_NONE, DWriteCreateFactory,
    IDWriteFactory, IDWriteFontFace, IDWriteFontFile,
};
use windows::core::{HSTRING, Result};

use super::surface::Surface;

/// Cell metrics used when the payload carries none.
const DEFAULT_CELL_WIDTH: u32 = 8;
const DEFAULT_CELL_HEIGHT: u32 = 16;
/// How many image bitmaps to keep between frames.
const IMAGE_CACHE_LIMIT: usize = 32;

/// The GPU backend: the pane's D3D device, one brush and one font face.
pub struct GpuBackend {
    surface: Rc<RefCell<Option<Surface>>>,
    /// Where this pane's grid sits in the swap chain, in device pixels.
    offset: Rc<Cell<(i32, i32)>>,
    face_path: PathBuf,
    face: Option<IDWriteFontFace>,
    brush: Option<ID2D1SolidColorBrush>,
    images: HashMap<u64, ID2D1Bitmap1>,
}

impl GpuBackend {
    /// Creates a backend that paints into `surface` once the panel has one.
    pub fn new(surface: Rc<RefCell<Option<Surface>>>, offset: Rc<Cell<(i32, i32)>>, face_path: &Path) -> Self {
        Self {
            surface,
            offset,
            face_path: face_path.to_path_buf(),
            face: None,
            brush: None,
            images: HashMap::new(),
        }
    }

    fn ensure_resources(&mut self, surface: &mut Surface) {
        if self.brush.is_none() {
            self.brush = surface.create_solid_brush().ok();
            surface.set_grayscale_text();
        }
        if self.face.is_none() {
            self.face = create_font_face(&self.face_path).ok();
        }
    }

    fn paint(&mut self, surface: &mut Surface, payload: &RenderingPayload) {
        let (cell_width, cell_height) = cell_size(payload);
        let (columns, rows) = payload.settings.viewport_cell_count;
        let (offset_x, offset_y) = self.offset.get();
        let Some(brush) = self.brush.clone() else {
            return;
        };

        // 1. Backgrounds, one fill per cell.
        for row in 0..rows {
            for column in 0..columns {
                let (background, _) = cell_colors(payload, column, row);
                let (left, top, right, bottom) = cell_rect(column, row, cell_width, cell_height);
                surface.set_brush_color(&brush, color_from_argb(background));
                surface.fill_rect(
                    &brush,
                    rect_f(left + offset_x, top + offset_y, right + offset_x, bottom + offset_y),
                );
            }
        }

        // 2. Images, between the backgrounds and the text.
        draw_images(
            surface,
            &mut self.images,
            payload,
            cell_width,
            cell_height,
            (offset_x, offset_y),
        );

        // 3. Text, one run per colour change so SGR colours stay on their cells.
        let Some(face) = self.face.as_ref() else {
            return;
        };
        let em_size = payload.settings.font.font_size;
        let baseline = baseline(payload, cell_height);
        for (row_index, row) in payload.rows.iter().enumerate() {
            let row_index = row_index as u16;
            let origin_y = row_index as f32 * cell_height as f32 + baseline as f32;
            let mut start = 0;
            while start < row.glyph_indices.len() {
                let colour = cell_colors(payload, row.glyph_columns[start], row_index).1;
                let mut end = start + 1;
                while end < row.glyph_indices.len()
                    && cell_colors(payload, row.glyph_columns[end], row_index).1 == colour
                {
                    end += 1;
                }

                let mut advances = Vec::with_capacity(end - start);
                for index in start..end {
                    let column = row.glyph_columns[index];
                    let next = if index + 1 < end {
                        row.glyph_columns[index + 1]
                    } else {
                        column + 1
                    };
                    advances.push(f32::from(next.saturating_sub(column)) * cell_width as f32);
                }

                let origin = windows_numerics::Vector2 {
                    X: f32::from(row.glyph_columns[start]) * cell_width as f32 + offset_x as f32,
                    Y: origin_y + offset_y as f32,
                };
                surface.set_brush_color(&brush, color_from_argb(colour));
                let _ =
                    surface.draw_glyph_run(&brush, origin, face, em_size, &row.glyph_indices[start..end], &advances);
                start = end;
            }
        }

        // 4. Grid lines, then the cursor over the text.
        for row_index in 0..payload.rows.len() as u16 {
            for rect in grid_line_rects(payload, row_index, cell_width, cell_height) {
                surface.set_brush_color(&brush, color_from_argb(rect.color));
                surface.fill_rect(
                    &brush,
                    rect_f(
                        rect.left + offset_x,
                        rect.top + offset_y,
                        rect.right + offset_x,
                        rect.bottom + offset_y,
                    ),
                );
            }
        }

        let (left, top, right, bottom) = payload.cursor_rect;
        if right > left && bottom > top {
            surface.set_brush_color(
                &brush,
                color_from_argb(argb_from_colorref(payload.settings.cursor.cursor_color)),
            );
            surface.fill_rect(
                &brush,
                rect_f(
                    left * cell_width as i32 + offset_x,
                    top * cell_height as i32 + offset_y,
                    right * cell_width as i32 + offset_x,
                    bottom * cell_height as i32 + offset_y,
                ),
            );
        }
    }
}

impl IBackend for GpuBackend {
    fn release_resources(&mut self) {
        self.face = None;
        self.brush = None;
        self.images.clear();
    }

    fn render(&mut self, payload: &mut RenderingPayload) {
        // The handle is cloned so the borrow of the cell does not borrow `self`,
        // which `paint` needs mutably.
        let surface = Rc::clone(&self.surface);
        let mut slot = surface.borrow_mut();
        let Some(surface) = slot.as_mut() else {
            return;
        };

        self.ensure_resources(surface);
        let composite = surface.is_composite();
        if !composite && surface.begin_frame(payload.settings.misc.background_color).is_err() {
            return;
        }
        self.paint(surface, payload);
        // EndDraw pairs with `begin_frame` whatever `paint` decided to draw.
        if !composite {
            let _ = surface.end_frame();
        }
    }

    fn requires_continuous_redraw(&mut self) -> bool {
        false
    }
}

/// Loads the pane's font file as a DirectWrite face.
fn create_font_face(path: &Path) -> Result<IDWriteFontFace> {
    let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
    let path = HSTRING::from(path.to_string_lossy().as_ref());
    // SAFETY: the path is a valid HSTRING for the duration of the call, and no
    // last-write time is requested.
    let file: IDWriteFontFile = unsafe { factory.CreateFontFileReference(&path, None)? };
    // SAFETY: the file is a live font file reference; face zero is the face the
    // shell asked for.
    unsafe {
        factory.CreateFontFace(
            DWRITE_FONT_FACE_TYPE_TRUETYPE,
            &[Some(file)],
            0,
            DWRITE_FONT_SIMULATIONS_NONE,
        )
    }
}

fn cell_size(payload: &RenderingPayload) -> (u32, u32) {
    let (width, height) = payload.settings.font.cell_size;
    (
        if width == 0 {
            DEFAULT_CELL_WIDTH
        } else {
            u32::from(width)
        },
        if height == 0 {
            DEFAULT_CELL_HEIGHT
        } else {
            u32::from(height)
        },
    )
}

fn rect_f(left: i32, top: i32, right: i32, bottom: i32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: left as f32,
        top: top as f32,
        right: right as f32,
        bottom: bottom as f32,
    }
}

fn color_from_argb(argb: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((argb >> 16) & 0xFF) as f32 / 255.0,
        g: ((argb >> 8) & 0xFF) as f32 / 255.0,
        b: (argb & 0xFF) as f32 / 255.0,
        a: ((argb >> 24) & 0xFF) as f32 / 255.0,
    }
}

/// Draws the image rows, keeping one bitmap per revision between frames.
fn draw_images(
    surface: &mut Surface,
    images: &mut HashMap<u64, ID2D1Bitmap1>,
    payload: &RenderingPayload,
    cell_width: u32,
    cell_height: u32,
    offset: (i32, i32),
) {
    let mut used = Vec::new();
    for (row_index, row) in payload.rows.iter().enumerate() {
        let bitmap = &row.bitmap;
        if !bitmap.active || bitmap.target_width <= 0 {
            continue;
        }
        let (source_width, source_height) = bitmap.source_size;
        if source_width <= 0 || source_height <= 0 {
            continue;
        }
        let needed = (source_width as usize) * (source_height as usize);
        if bitmap.source.len() < needed {
            continue;
        }

        used.push(bitmap.revision);
        if let std::collections::hash_map::Entry::Vacant(entry) = images.entry(bitmap.revision) {
            let Ok(created) = surface.create_bitmap(
                source_width as u32,
                source_height as u32,
                &bitmap.source,
                D2D1_ALPHA_MODE_IGNORE,
            ) else {
                continue;
            };
            entry.insert(created);
        }
        let Some(created) = images.get(&bitmap.revision) else {
            continue;
        };

        let left = bitmap.target_offset.max(0) * cell_width as i32 + offset.0;
        let top = row_index as i32 * cell_height as i32 + offset.1;
        let right = left + bitmap.target_width * cell_width as i32;
        let bottom = top + cell_height as i32;
        let _ = surface.draw_bitmap(created, rect_f(left, top, right, bottom));
    }

    // An image that left the viewport gives its bitmap back.
    images.retain(|revision, _| used.contains(revision));
    while images.len() > IMAGE_CACHE_LIMIT {
        let Some(&revision) = images.keys().next() else {
            break;
        };
        images.remove(&revision);
    }
}
