//! The Metal content backend: cell quads and glyphs drawn on the GPU.
//!
//! The software backend rasterises every pixel on the CPU and the shell blits
//! the result; this backend keeps the rasterisation on the device. Cell
//! backgrounds, images, glyphs, grid lines and the cursor become triangle-strip
//! quads, glyph coverage comes from the same [`GlyphRasterizer`] the software
//! path uses and lands in a one-channel atlas, and images are uploaded per
//! revision.
//!
//! A frame is drawn twice: into the layer's drawable, which is what the window
//! shows, and into an offscreen `MTLTexture` that is read back and handed to the
//! existing `Frame` blit. The readback is the screenshot path and the fallback
//! when the layer cannot produce a drawable, so the view and the capture stay
//! unchanged (ADRs 072, 073).
//!
//! Every `unsafe` call carries a `SAFETY:` note, as the rest of `macos` does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSString, NSUInteger};
use objc2_metal::{
    MTLBlendFactor, MTLBuffer, MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat,
    MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState, MTLResourceOptions, MTLSize, MTLStorageMode, MTLStoreAction, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use renderer::atlas::common::{
    RenderingPayload, argb_from_colorref, baseline, cell_colors, cell_rect, glyph_origin, grid_line_rects,
};
use renderer::{FontId, GlyphBitmap, GlyphRasterizer, IBackend};

/// The glyph atlas' edge in pixels.
const ATLAS_SIZE: u32 = 1024;
/// The gutter around an atlas glyph, so a sample cannot reach a neighbour.
const ATLAS_PAD: u32 = 1;
/// Cell metrics used when the payload carries none.
const DEFAULT_CELL_WIDTH: u32 = 8;
const DEFAULT_CELL_HEIGHT: u32 = 16;

/// The shader source, compiled when the backend is created.
///
/// Vertices arrive in clip space, so both vertex functions are a pass-through;
/// the CPU already knows the target's size. `packed_` types give the Rust
/// `#[repr(C)]` structs the same layout.
const SHADERS: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct SolidVertex {
    float2 position;
    float4 tint;
};

struct SolidOut {
    float4 position [[position]];
    float4 tint;
};

vertex SolidOut solid_vertex(uint vid [[vertex_id]],
                             device const SolidVertex* vertices [[buffer(0)]]) {
    SolidVertex input = vertices[vid];
    SolidOut out;
    out.position = float4(input.position, 0.0, 1.0);
    out.tint = input.tint;
    return out;
}

fragment float4 solid_fragment(SolidOut in [[stage_in]]) {
    return in.tint;
}

struct TextureVertex {
    float2 position;
    float2 uv;
    float4 tint;
};

struct TextureOut {
    float4 position [[position]];
    float2 uv;
    float4 tint;
};

vertex TextureOut texture_vertex(uint vid [[vertex_id]],
                                 device const TextureVertex* vertices [[buffer(0)]]) {
    TextureVertex input = vertices[vid];
    TextureOut out;
    out.position = float4(input.position, 0.0, 1.0);
    out.uv = input.uv;
    out.tint = input.tint;
    return out;
}

fragment float4 glyph_fragment(TextureOut in [[stage_in]],
                               texture2d<float> atlas [[texture(0)]]) {
    constexpr sampler nearest(coord::normalized, filter::nearest, address::clamp_to_edge);
    float coverage = atlas.sample(nearest, in.uv).r;
    return float4(in.tint.rgb, in.tint.a * coverage);
}

fragment float4 image_fragment(TextureOut in [[stage_in]],
                               texture2d<float> image [[texture(0)]]) {
    constexpr sampler linear(coord::normalized, filter::linear, address::clamp_to_edge);
    return image.sample(linear, in.uv);
}
"#;

#[repr(C)]
#[derive(Clone, Copy)]
struct SolidVertex {
    position: [f32; 2],
    /// The MSL `float4` starts at offset 16, so the Rust struct carries the gap.
    _padding: [f32; 2],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TextureVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

/// One atlas slot, with the bearings `glyph_origin` needs.
#[derive(Clone, Copy)]
struct AtlasEntry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
}

impl AtlasEntry {
    /// The same entry's eight normalized corners.
    fn uv(self) -> [f32; 4] {
        let scale = 1.0 / ATLAS_SIZE as f32;
        [
            self.x as f32 * scale,
            self.y as f32 * scale,
            (self.x + self.width) as f32 * scale,
            (self.y + self.height) as f32 * scale,
        ]
    }
}

/// A coverage atlas and its shelf packer.
struct Atlas {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    coverage: Vec<u8>,
    entries: HashMap<u16, AtlasEntry>,
    next_x: u32,
    next_y: u32,
    row_height: u32,
    dirty: bool,
    /// Set when a glyph did not fit; the next frame starts a fresh atlas.
    needs_reset: bool,
}

impl Atlas {
    fn new(device: &ProtocolObject<dyn MTLDevice>) -> Option<Self> {
        // SAFETY: the pixel format, dimensions and mipmap flag form a valid
        // two-dimensional texture descriptor.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::R8Unorm,
                ATLAS_SIZE as NSUInteger,
                ATLAS_SIZE as NSUInteger,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead);
        descriptor.setStorageMode(MTLStorageMode::Shared);
        let texture = device.newTextureWithDescriptor(&descriptor)?;
        Some(Self {
            texture,
            coverage: vec![0; (ATLAS_SIZE * ATLAS_SIZE) as usize],
            entries: HashMap::new(),
            next_x: 0,
            next_y: 0,
            row_height: 0,
            dirty: false,
            needs_reset: false,
        })
    }

    /// Stores `bitmap` under `glyph`, or returns the slot it already has.
    fn insert(&mut self, glyph: u16, bitmap: &GlyphBitmap) -> Option<AtlasEntry> {
        if let Some(entry) = self.entries.get(&glyph) {
            return Some(*entry);
        }

        let width = bitmap.width.max(1);
        let height = bitmap.height.max(1);
        let slot_width = width + 2 * ATLAS_PAD;
        let slot_height = height + 2 * ATLAS_PAD;
        if slot_width > ATLAS_SIZE || slot_height > ATLAS_SIZE {
            return None;
        }

        if self.next_x + slot_width > ATLAS_SIZE {
            self.next_x = 0;
            self.next_y += self.row_height;
            self.row_height = 0;
        }
        if self.next_y + slot_height > ATLAS_SIZE {
            // Clearing now would invalidate quads already emitted this frame;
            // the next frame starts over instead.
            self.needs_reset = true;
            return None;
        }

        let x = self.next_x + ATLAS_PAD;
        let y = self.next_y + ATLAS_PAD;
        for row in 0..height {
            let source = (row * width) as usize;
            let Some(row_data) = bitmap.coverage.get(source..source + width as usize) else {
                continue;
            };
            let destination = ((y + row) * ATLAS_SIZE + x) as usize;
            self.coverage[destination..destination + width as usize].copy_from_slice(row_data);
        }

        self.next_x += slot_width;
        self.row_height = self.row_height.max(slot_height);
        self.dirty = true;

        let entry = AtlasEntry {
            x,
            y,
            width,
            height,
            left: bitmap.left,
            top: bitmap.top,
        };
        self.entries.insert(glyph, entry);
        Some(entry)
    }

    /// Starts a frame with a fresh atlas when the last one could not fit a
    /// glyph.
    fn begin_frame(&mut self) {
        if !self.needs_reset {
            return;
        }
        self.coverage.fill(0);
        self.entries.clear();
        self.next_x = 0;
        self.next_y = 0;
        self.row_height = 0;
        self.dirty = true;
        self.needs_reset = false;
    }

    fn upload(&mut self) {
        if !self.dirty {
            return;
        }
        let region = MTLRegion {
            origin: MTLOrigin { x: 0, y: 0, z: 0 },
            size: MTLSize {
                width: ATLAS_SIZE as NSUInteger,
                height: ATLAS_SIZE as NSUInteger,
                depth: 1,
            },
        };
        // SAFETY: the region is the whole texture and `coverage` is exactly
        // `ATLAS_SIZE * ATLAS_SIZE` readable bytes, which is one byte per texel.
        let Some(bytes) = NonNull::new(self.coverage.as_ptr().cast_mut().cast::<c_void>()) else {
            return;
        };
        unsafe {
            self.texture
                .replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, bytes, ATLAS_SIZE as NSUInteger);
        }
        self.dirty = false;
    }
}

/// One image band and the texture that holds it.
struct ImageDraw {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// One frame's quads, ready for either target.
struct FrameData {
    solids: Vec<SolidVertex>,
    images: Vec<ImageDraw>,
    glyphs: Vec<TextureVertex>,
    overlays: Vec<SolidVertex>,
}

/// The device-side state one frame is drawn with.
pub(crate) struct MetalBackend {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    solid: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    glyph: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    image: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    rasterizer: Option<Box<dyn GlyphRasterizer>>,
    atlas: Atlas,
    images: HashMap<u64, Retained<ProtocolObject<dyn MTLTexture>>>,
    target: Option<Retained<ProtocolObject<dyn MTLTexture>>>,
    /// The layer the window presents through, when the controller attached one.
    layer: Option<Retained<CAMetalLayer>>,
    /// Whether the last frame reached the window through a drawable.
    presented: bool,
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

impl MetalBackend {
    /// Creates the device, pipelines and atlas, or returns `None` when the host
    /// has no Metal device or the shaders do not compile.
    fn create() -> Option<Self> {
        let device = match MTLCreateSystemDefaultDevice() {
            Some(device) => device,
            None => {
                eprintln!("metal: no device");
                return None;
            }
        };
        let queue = match device.newCommandQueue() {
            Some(queue) => queue,
            None => {
                eprintln!("metal: no queue");
                return None;
            }
        };
        let source = NSString::from_str(SHADERS);
        let library = match device.newLibraryWithSource_options_error(&source, None) {
            Ok(library) => library,
            Err(error) => {
                eprintln!("metal: shader: {error}");
                return None;
            }
        };
        let solid = match pipeline(&device, &library, "solid_vertex", "solid_fragment", false) {
            Some(pipeline) => pipeline,
            None => {
                eprintln!("metal: solid pipeline");
                return None;
            }
        };
        let glyph = match pipeline(&device, &library, "texture_vertex", "glyph_fragment", true) {
            Some(pipeline) => pipeline,
            None => {
                eprintln!("metal: glyph pipeline");
                return None;
            }
        };
        let image = match pipeline(&device, &library, "texture_vertex", "image_fragment", true) {
            Some(pipeline) => pipeline,
            None => {
                eprintln!("metal: image pipeline");
                return None;
            }
        };
        let atlas = match Atlas::new(&device) {
            Some(atlas) => atlas,
            None => {
                eprintln!("metal: atlas");
                return None;
            }
        };

        Some(Self {
            device,
            queue,
            solid,
            glyph,
            image,
            rasterizer: None,
            atlas,
            images: HashMap::new(),
            target: None,
            layer: None,
            presented: false,
            width: 0,
            height: 0,
            pixels: Vec::new(),
        })
    }

    fn set_rasterizer(&mut self, rasterizer: Box<dyn GlyphRasterizer>) {
        self.rasterizer = Some(rasterizer);
    }

    /// The last frame's pixels, row-major `0xAARRGGBB`.
    fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// One pixel of the last frame, or `None` when out of bounds.
    #[cfg(test)]
    fn pixel(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.pixels.get((y * self.width + x) as usize).copied()
    }

    fn ensure_target(&mut self) -> bool {
        let (width, height) = (self.width, self.height);
        if width == 0 || height == 0 {
            return false;
        }
        if self.target.is_some() {
            return true;
        }

        // SAFETY: the pixel format, dimensions and mipmap flag form a valid
        // two-dimensional texture descriptor.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                width as NSUInteger,
                height as NSUInteger,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        descriptor.setStorageMode(MTLStorageMode::Shared);
        self.target = self.device.newTextureWithDescriptor(&descriptor);
        self.pixels = vec![0; (width * height) as usize];
        self.target.is_some()
    }

    fn render(&mut self, payload: &mut RenderingPayload) {
        let (cell_width, cell_height) = cell_size(payload);
        let (columns, rows) = payload.settings.viewport_cell_count;
        let width = target_side(payload.settings.target_size.0, columns, cell_width);
        let height = target_side(payload.settings.target_size.1, rows, cell_height);
        if width == 0 || height == 0 {
            return;
        }
        if self.width != width || self.height != height {
            self.width = width;
            self.height = height;
            self.target = None;
        }
        if !self.ensure_target() {
            return;
        }

        let data = self.build(payload, cell_width, cell_height, width, height);
        self.atlas.upload();

        // The drawable is what the window shows; the offscreen target stays the
        // screenshot path and the fallback, so both are drawn every frame.
        self.present(payload, &data);
        self.offscreen(payload, &data);
    }

    /// Turns the payload into the quads both targets draw.
    fn build(
        &mut self,
        payload: &RenderingPayload,
        cell_width: u32,
        cell_height: u32,
        width: u32,
        height: u32,
    ) -> FrameData {
        let (columns, rows) = payload.settings.viewport_cell_count;

        let mut solids = Vec::new();
        for row in 0..rows {
            for column in 0..columns {
                let (background, _) = cell_colors(payload, column, row);
                let (left, top, right, bottom) = cell_rect(column, row, cell_width, cell_height);
                push_solid(&mut solids, (left, top, right, bottom), background, width, height);
            }
        }

        let images = self.prepare_images(payload, cell_width, cell_height);

        let mut glyphs = Vec::new();
        if let Some(mut rasterizer) = self.rasterizer.take() {
            self.build_glyphs(
                payload,
                cell_width,
                cell_height,
                width,
                height,
                &mut *rasterizer,
                &mut glyphs,
            );
            self.rasterizer = Some(rasterizer);
        }

        let mut overlays = Vec::new();
        for row_index in 0..payload.rows.len() as u16 {
            for rect in grid_line_rects(payload, row_index, cell_width, cell_height) {
                push_solid(
                    &mut overlays,
                    (rect.left, rect.top, rect.right, rect.bottom),
                    rect.color,
                    width,
                    height,
                );
            }
        }
        let (left, top, right, bottom) = payload.cursor_rect;
        if right > left && bottom > top {
            push_solid(
                &mut overlays,
                (
                    left * cell_width as i32,
                    top * cell_height as i32,
                    right * cell_width as i32,
                    bottom * cell_height as i32,
                ),
                argb_from_colorref(payload.settings.cursor.cursor_color),
                width,
                height,
            );
        }

        FrameData {
            solids,
            images,
            glyphs,
            overlays,
        }
    }

    /// Draws `data` into a drawable and presents it, or hides the layer.
    fn present(&mut self, payload: &RenderingPayload, data: &FrameData) {
        let Some(layer) = self.layer.clone() else {
            self.presented = false;
            return;
        };
        let Some(drawable) = layer.nextDrawable() else {
            // A layer with no drawable (a zero-sized one, say) keeps the
            // readback blit as the visible frame.
            layer.setHidden(true);
            self.presented = false;
            return;
        };

        let texture = drawable.texture();
        if !self.draw_frame(payload, &texture, Some(&drawable), data) {
            layer.setHidden(true);
            self.presented = false;
            return;
        }
        layer.setHidden(false);
        self.presented = true;
    }

    /// Draws `data` into the offscreen target and reads it back.
    fn offscreen(&mut self, payload: &RenderingPayload, data: &FrameData) {
        let Some(target) = self.target.clone() else {
            return;
        };
        if self.draw_frame(payload, &target, None, data) {
            self.pixels = read_texture(&target, self.width, self.height);
        }
    }

    /// Encodes `data` into `target`, presents `drawable` when there is one, and
    /// waits for the GPU.
    fn draw_frame(
        &self,
        payload: &RenderingPayload,
        target: &ProtocolObject<dyn MTLTexture>,
        drawable: Option<&ProtocolObject<dyn CAMetalDrawable>>,
        data: &FrameData,
    ) -> bool {
        let Some(command) = self.queue.commandBuffer() else {
            return false;
        };
        let pass = MTLRenderPassDescriptor::renderPassDescriptor();
        // SAFETY: attachment index zero is the one this pass declares and the
        // only one the pipelines target.
        let attachment = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setTexture(Some(target));
        attachment.setLoadAction(MTLLoadAction::Clear);
        attachment.setStoreAction(MTLStoreAction::Store);
        attachment.setClearColor(clear_color(payload));
        let Some(encoder) = command.renderCommandEncoderWithDescriptor(&pass) else {
            return false;
        };

        // The buffers are unretained by the encoder, so they live until the
        // command buffer has finished.
        let mut keep_alive = Vec::new();
        if !data.solids.is_empty() {
            encoder.setRenderPipelineState(&self.solid);
            draw_solid(&encoder, &self.device, &data.solids, &mut keep_alive);
        }
        if !data.images.is_empty() {
            encoder.setRenderPipelineState(&self.image);
            for image in &data.images {
                let quad = textured_quad(
                    (image.left, image.top, image.right, image.bottom),
                    [0.0, 0.0, 1.0, 1.0],
                    [1.0, 1.0, 1.0, 1.0],
                    self.width,
                    self.height,
                );
                draw_textured(&encoder, &self.device, &quad, &image.texture, &mut keep_alive);
            }
        }
        if !data.glyphs.is_empty() {
            encoder.setRenderPipelineState(&self.glyph);
            draw_textured(
                &encoder,
                &self.device,
                &data.glyphs,
                &self.atlas.texture,
                &mut keep_alive,
            );
        }
        if !data.overlays.is_empty() {
            encoder.setRenderPipelineState(&self.solid);
            draw_solid(&encoder, &self.device, &data.overlays, &mut keep_alive);
        }

        encoder.endEncoding();
        if let Some(drawable) = drawable {
            let drawable: &ProtocolObject<dyn MTLDrawable> = drawable.as_ref();
            command.presentDrawable(drawable);
        }
        command.commit();
        command.waitUntilCompleted();
        drop(keep_alive);
        true
    }

    /// Uploads every image band that is not cached, and evicts the rest.
    fn prepare_images(&mut self, payload: &RenderingPayload, cell_width: u32, cell_height: u32) -> Vec<ImageDraw> {
        let mut draws = Vec::new();
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
            let needed = source_width as usize * source_height as usize;
            if bitmap.source.len() < needed {
                continue;
            }

            used.push(bitmap.revision);
            if !self.images.contains_key(&bitmap.revision) {
                let Some(texture) = image_texture(
                    &self.device,
                    source_width as u32,
                    source_height as u32,
                    &bitmap.source[..needed],
                ) else {
                    continue;
                };
                self.images.insert(bitmap.revision, texture);
            }
            let Some(texture) = self.images.get(&bitmap.revision) else {
                continue;
            };

            let left = bitmap.target_offset.max(0) * cell_width as i32;
            let top = row_index as i32 * cell_height as i32;
            draws.push(ImageDraw {
                texture: texture.clone(),
                left,
                top,
                right: left + bitmap.target_width * cell_width as i32,
                bottom: top + cell_height as i32,
            });
        }

        self.images.retain(|revision, _| used.contains(revision));
        draws
    }

    #[allow(clippy::too_many_arguments)]
    fn build_glyphs(
        &mut self,
        payload: &RenderingPayload,
        cell_width: u32,
        cell_height: u32,
        width: u32,
        height: u32,
        rasterizer: &mut dyn GlyphRasterizer,
        out: &mut Vec<TextureVertex>,
    ) {
        let baseline = baseline(payload, cell_height);
        self.atlas.begin_frame();

        for (row_index, row) in payload.rows.iter().enumerate() {
            for (glyph, &column) in row.glyph_indices.iter().zip(&row.glyph_columns) {
                let (_, foreground) = cell_colors(payload, column, row_index as u16);
                let entry = if let Some(entry) = self.atlas.entries.get(glyph).copied() {
                    entry
                } else {
                    let Some(bitmap) = rasterizer.rasterize(FontId(0), *glyph) else {
                        continue;
                    };
                    let Some(entry) = self.atlas.insert(*glyph, &bitmap) else {
                        continue;
                    };
                    entry
                };

                let (origin_x, origin_y) = glyph_origin(
                    column,
                    row_index as u16,
                    cell_width,
                    cell_height,
                    baseline,
                    entry.left,
                    entry.top,
                );
                out.extend_from_slice(&textured_quad(
                    (
                        origin_x,
                        origin_y,
                        origin_x + entry.width as i32,
                        origin_y + entry.height as i32,
                    ),
                    entry.uv(),
                    color(foreground),
                    width,
                    height,
                ));
            }
        }
    }

    fn release_resources(&mut self) {
        self.target = None;
        self.images.clear();
        self.pixels.clear();
        self.presented = false;
        self.width = 0;
        self.height = 0;
    }
}

/// The backend handle the engine and the shell share.
#[derive(Clone)]
pub(crate) struct Metal {
    inner: Rc<RefCell<MetalBackend>>,
}

impl Metal {
    /// Creates the backend, or returns `None` when this host cannot run it.
    pub(crate) fn new() -> Option<Self> {
        MetalBackend::create().map(|inner| Self {
            inner: Rc::new(RefCell::new(inner)),
        })
    }

    /// Gives the backend the glyph rasteriser the frame payload was shaped for.
    pub(crate) fn set_rasterizer(&self, rasterizer: Box<dyn GlyphRasterizer>) {
        self.inner.borrow_mut().set_rasterizer(rasterizer);
    }

    /// Gives the backend the layer the window presents through.
    ///
    /// The layer's geometry belongs to the controller; this wires the device and
    /// the pixel format, so a backend rebuilt for a new scale can re-attach the
    /// same layer.
    pub(crate) fn set_layer(&self, layer: Retained<CAMetalLayer>) {
        let mut inner = self.inner.borrow_mut();
        layer.setDevice(Some(&*inner.device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        layer.setFramebufferOnly(true);
        layer.setOpaque(true);
        // The view's coordinate space is flipped, so the layer's top edge is
        // the drawable's first row.
        layer.setGeometryFlipped(true);
        inner.layer = Some(layer);
    }

    /// Whether a layer is attached, which a rebuilt backend loses.
    pub(crate) fn layer_attached(&self) -> bool {
        self.inner.borrow().layer.is_some()
    }

    /// Whether the last frame reached the window through a drawable.
    pub(crate) fn presented(&self) -> bool {
        self.inner.borrow().presented
    }

    /// Runs `read` over the last frame's pixels, row-major `0xAARRGGBB`.
    pub(crate) fn with_pixels<T>(&self, read: impl FnOnce(&[u32]) -> T) -> T {
        read(self.inner.borrow().pixels())
    }

    pub(crate) fn width(&self) -> u32 {
        self.inner.borrow().width
    }

    pub(crate) fn height(&self) -> u32 {
        self.inner.borrow().height
    }
}

impl IBackend for Metal {
    fn release_resources(&mut self) {
        self.inner.borrow_mut().release_resources();
    }

    fn render(&mut self, payload: &mut RenderingPayload) {
        self.inner.borrow_mut().render(payload);
    }

    fn requires_continuous_redraw(&mut self) -> bool {
        false
    }
}

fn pipeline(
    device: &ProtocolObject<dyn MTLDevice>,
    library: &ProtocolObject<dyn MTLLibrary>,
    vertex: &str,
    fragment: &str,
    blend: bool,
) -> Option<Retained<ProtocolObject<dyn MTLRenderPipelineState>>> {
    let descriptor = MTLRenderPipelineDescriptor::new();
    let vertex_function = match library.newFunctionWithName(&NSString::from_str(vertex)) {
        Some(function) => function,
        None => {
            eprintln!("metal: missing function {vertex}");
            return None;
        }
    };
    let fragment_function = match library.newFunctionWithName(&NSString::from_str(fragment)) {
        Some(function) => function,
        None => {
            eprintln!("metal: missing function {fragment}");
            return None;
        }
    };
    descriptor.setVertexFunction(Some(&*vertex_function));
    descriptor.setFragmentFunction(Some(&*fragment_function));
    descriptor.setRasterSampleCount(1);

    // SAFETY: attachment index zero is the only one this descriptor declares.
    let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
    attachment.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
    if blend {
        attachment.setBlendingEnabled(true);
        attachment.setSourceRGBBlendFactor(MTLBlendFactor::SourceAlpha);
        attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
        attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
    }

    match device.newRenderPipelineStateWithDescriptor_error(&descriptor) {
        Ok(pipeline) => Some(pipeline),
        Err(error) => {
            eprintln!("metal: pipeline {vertex:?}/{fragment:?}: {error}");
            None
        }
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

fn target_side(configured: u16, cells: u16, cell: u32) -> u32 {
    if configured > 0 {
        u32::from(configured)
    } else {
        u32::from(cells) * cell
    }
}

fn clear_color(payload: &RenderingPayload) -> MTLClearColor {
    let color = payload.settings.misc.background_color;
    MTLClearColor {
        red: f64::from((color & 0xFF) as u8) / 255.0,
        green: f64::from(((color >> 8) & 0xFF) as u8) / 255.0,
        blue: f64::from(((color >> 16) & 0xFF) as u8) / 255.0,
        alpha: 1.0,
    }
}

fn color(argb: u32) -> [f32; 4] {
    [
        f32::from((argb >> 16) as u8) / 255.0,
        f32::from((argb >> 8) as u8) / 255.0,
        f32::from(argb as u8) / 255.0,
        f32::from((argb >> 24) as u8) / 255.0,
    ]
}

fn ndc_x(x: f32, width: u32) -> f32 {
    x / width as f32 * 2.0 - 1.0
}

fn ndc_y(y: f32, height: u32) -> f32 {
    1.0 - y / height as f32 * 2.0
}

fn push_solid(out: &mut Vec<SolidVertex>, rect: (i32, i32, i32, i32), argb: u32, width: u32, height: u32) {
    let (left, top, right, bottom) = rect;
    if right <= left || bottom <= top {
        return;
    }
    let (x0, x1) = (ndc_x(left as f32, width), ndc_x(right as f32, width));
    let (y0, y1) = (ndc_y(top as f32, height), ndc_y(bottom as f32, height));
    let fill = color(argb);
    out.extend_from_slice(&[
        solid_vertex(x0, y0, fill),
        solid_vertex(x1, y0, fill),
        solid_vertex(x0, y1, fill),
        solid_vertex(x1, y0, fill),
        solid_vertex(x1, y1, fill),
        solid_vertex(x0, y1, fill),
    ]);
}

fn solid_vertex(x: f32, y: f32, color: [f32; 4]) -> SolidVertex {
    SolidVertex {
        position: [x, y],
        _padding: [0.0; 2],
        color,
    }
}

fn textured_quad(
    rect: (i32, i32, i32, i32),
    uv: [f32; 4],
    color: [f32; 4],
    width: u32,
    height: u32,
) -> [TextureVertex; 6] {
    let (left, top, right, bottom) = rect;
    let (x0, x1) = (ndc_x(left as f32, width), ndc_x(right as f32, width));
    let (y0, y1) = (ndc_y(top as f32, height), ndc_y(bottom as f32, height));
    let [u0, v0, u1, v1] = uv;
    let vertex = |position: [f32; 2], uv: [f32; 2]| TextureVertex { position, uv, color };
    let top_left = vertex([x0, y0], [u0, v0]);
    let top_right = vertex([x1, y0], [u1, v0]);
    let bottom_left = vertex([x0, y1], [u0, v1]);
    let bottom_right = vertex([x1, y1], [u1, v1]);
    [top_left, top_right, bottom_left, top_right, bottom_right, bottom_left]
}

fn make_buffer<T>(
    device: &ProtocolObject<dyn MTLDevice>,
    data: &[T],
) -> Option<Retained<ProtocolObject<dyn MTLBuffer>>> {
    if data.is_empty() {
        return None;
    }
    let length = std::mem::size_of_val(data);
    let pointer = NonNull::new(data.as_ptr().cast_mut().cast::<c_void>())?;
    // SAFETY: `pointer` names `length` readable bytes, and the new buffer copies
    // them before returning.
    unsafe {
        device.newBufferWithBytes_length_options(pointer, length as NSUInteger, MTLResourceOptions::StorageModeShared)
    }
}

fn draw_solid(
    encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    device: &ProtocolObject<dyn MTLDevice>,
    vertices: &[SolidVertex],
    keep_alive: &mut Vec<Retained<ProtocolObject<dyn MTLBuffer>>>,
) {
    let Some(buffer) = make_buffer(device, vertices) else {
        return;
    };
    // SAFETY: binding zero is the vertex shader's slot, the buffer holds these
    // vertices, and the draw covers every one of them as two triangles per
    // quad.
    unsafe {
        encoder.setVertexBuffer_offset_atIndex(Some(&*buffer), 0, 0);
        encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, vertices.len() as NSUInteger);
    }
    keep_alive.push(buffer);
}

fn draw_textured(
    encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    device: &ProtocolObject<dyn MTLDevice>,
    vertices: &[TextureVertex],
    texture: &ProtocolObject<dyn MTLTexture>,
    keep_alive: &mut Vec<Retained<ProtocolObject<dyn MTLBuffer>>>,
) {
    let Some(buffer) = make_buffer(device, vertices) else {
        return;
    };
    // SAFETY: binding zero is the vertex shader's slot, texture zero is the
    // fragment shader's, the buffer holds these vertices, and the draw covers
    // all of them as two triangles per quad.
    unsafe {
        encoder.setVertexBuffer_offset_atIndex(Some(&*buffer), 0, 0);
        encoder.setFragmentTexture_atIndex(Some(texture), 0);
        encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, vertices.len() as NSUInteger);
    }
    keep_alive.push(buffer);
}

fn image_texture(
    device: &ProtocolObject<dyn MTLDevice>,
    width: u32,
    height: u32,
    source: &[u32],
) -> Option<Retained<ProtocolObject<dyn MTLTexture>>> {
    // SAFETY: the pixel format, dimensions and mipmap flag form a valid
    // two-dimensional texture descriptor.
    let descriptor = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::BGRA8Unorm,
            width as NSUInteger,
            height as NSUInteger,
            false,
        )
    };
    descriptor.setUsage(MTLTextureUsage::ShaderRead);
    descriptor.setStorageMode(MTLStorageMode::Shared);
    let texture = device.newTextureWithDescriptor(&descriptor)?;

    // The buffer's images are `0xAARRGGBB`; `to_le_bytes` is `B, G, R, A`,
    // which is what a BGRA8 texel expects.
    let mut bytes = Vec::with_capacity(source.len() * 4);
    for pixel in source {
        bytes.extend_from_slice(&pixel.to_le_bytes());
    }
    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width: width as NSUInteger,
            height: height as NSUInteger,
            depth: 1,
        },
    };
    // SAFETY: the region is the whole texture and `bytes` holds four readable
    // bytes per texel.
    let bytes = NonNull::new(bytes.as_ptr().cast_mut().cast::<c_void>())?;
    unsafe {
        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, bytes, (width * 4) as NSUInteger);
    }
    Some(texture)
}

/// Reads a BGRA8 texture back as row-major `0xAARRGGBB`.
fn read_texture(target: &ProtocolObject<dyn MTLTexture>, width: u32, height: u32) -> Vec<u32> {
    let mut bytes = vec![0u8; (width * height * 4) as usize];
    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width: width as NSUInteger,
            height: height as NSUInteger,
            depth: 1,
        },
    };
    // SAFETY: the region is the whole target, and `bytes` holds a row of
    // `width * 4` bytes for each of the target's `height` rows.
    let Some(pointer) = NonNull::new(bytes.as_mut_ptr().cast::<c_void>()) else {
        return Vec::new();
    };
    unsafe {
        target.getBytes_bytesPerRow_fromRegion_mipmapLevel(pointer, (width * 4) as NSUInteger, region, 0);
    }
    // A BGRA8Unorm texel is `B, G, R, A` in memory, which is exactly
    // `0xAARRGGBB` read little-endian.
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| u32::from_le_bytes(*pixel))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_space_puts_the_origin_top_left() {
        assert_eq!(ndc_x(0.0, 100), -1.0);
        assert_eq!(ndc_x(100.0, 100), 1.0);
        assert_eq!(ndc_y(0.0, 50), 1.0);
        assert_eq!(ndc_y(50.0, 50), -1.0);
    }

    #[test]
    fn a_quad_is_six_vertices_two_triangles() {
        let mut vertices = Vec::new();
        push_solid(&mut vertices, (0, 0, 8, 16), 0xFFFF_0000, 16, 32);

        assert_eq!(vertices.len(), 6);
        assert_eq!(vertices[0].position, [-1.0, 1.0]);
        assert_eq!(vertices[2].position, [-1.0, 0.0]);
        assert_eq!(vertices[4].position, [0.0, 0.0]);
        assert_eq!(vertices[0].color, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_quad_that_is_empty_or_upside_down_draws_nothing() {
        let mut vertices = Vec::new();
        push_solid(&mut vertices, (4, 0, 4, 16), 0xFFFF_FFFF, 8, 16);
        push_solid(&mut vertices, (0, 16, 8, 0), 0xFFFF_FFFF, 8, 16);
        assert!(vertices.is_empty());
    }

    #[test]
    fn a_textured_quad_carries_its_uv_corners() {
        let quad = textured_quad((0, 0, 4, 4), [0.25, 0.5, 0.75, 1.0], [1.0; 4], 8, 8);
        assert_eq!(quad[0].uv, [0.25, 0.5]);
        assert_eq!(quad[1].uv, [0.75, 0.5]);
        assert_eq!(quad[2].uv, [0.25, 1.0]);
        assert_eq!(quad[3].uv, [0.75, 0.5]);
        assert_eq!(quad[4].uv, [0.75, 1.0]);
        assert_eq!(quad[5].uv, [0.25, 1.0]);
    }

    #[test]
    #[ignore = "needs a Metal device"]
    fn a_layer_without_a_drawable_keeps_the_readback_frame() {
        let Some(mut backend) = MetalBackend::create() else {
            eprintln!("metal: no backend; skipping");
            return;
        };
        // A zero-sized layer cannot hand out a drawable, which is the case the
        // readback blit has to cover.
        let layer = CAMetalLayer::new();
        layer.setDevice(Some(&*backend.device));
        backend.layer = Some(layer);

        let mut payload = payload();
        payload.rows.clear();
        payload.cursor_rect = (1, 0, 2, 1);

        backend.render(&mut payload);

        assert!(!backend.presented);
        assert_eq!(backend.pixel(0, 0), Some(0xFF00_0000));
        assert_eq!(backend.pixel(8, 0), Some(0xFFFF_0000));
    }

    #[test]
    #[ignore = "needs a Metal device and a drawable"]
    fn a_drawable_presents_the_frame() {
        let Some(mut backend) = MetalBackend::create() else {
            eprintln!("metal: no backend; skipping");
            return;
        };
        let layer = CAMetalLayer::new();
        layer.setDevice(Some(&*backend.device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        layer.setFramebufferOnly(true);
        layer.setDrawableSize(objc2_foundation::NSSize::new(16.0, 16.0));
        backend.layer = Some(layer);

        let mut payload = payload();

        backend.render(&mut payload);

        if !backend.presented {
            eprintln!("metal: this host hands out no drawable; skipping");
            return;
        }
        assert!(backend.presented);
        // The offscreen path still ran, so the screenshot has a frame too.
        assert_eq!(backend.pixel(0, 0), Some(0xFF00_0000));
    }

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

    /// A rasteriser whose 2x2 glyph has a checkered coverage mask.
    struct PatternRasterizer;

    impl GlyphRasterizer for PatternRasterizer {
        fn rasterize(&mut self, _font: FontId, _glyph_index: u16) -> Option<GlyphBitmap> {
            Some(GlyphBitmap {
                width: 2,
                height: 2,
                left: 0,
                top: 0,
                coverage: vec![255, 0, 0, 255],
            })
        }
    }

    /// A rasteriser whose 4x4 glyph is half coverage.
    struct HalfRasterizer;

    impl GlyphRasterizer for HalfRasterizer {
        fn rasterize(&mut self, _font: FontId, _glyph_index: u16) -> Option<GlyphBitmap> {
            Some(GlyphBitmap {
                width: 4,
                height: 4,
                left: 0,
                top: 0,
                coverage: vec![128; 16],
            })
        }
    }

    fn payload() -> RenderingPayload {
        let settings = renderer::Settings {
            font: renderer::FontSettings {
                cell_size: (8, 16),
                ..renderer::FontSettings::default()
            },
            misc: renderer::MiscellaneousSettings {
                background_color: 0x0000_0000,
                foreground_color: 0x00FF_FFFF,
                ..renderer::MiscellaneousSettings::default()
            },
            cursor: renderer::CursorSettings {
                cursor_color: 0x0000_00FF,
                ..renderer::CursorSettings::default()
            },
            viewport_cell_count: (2, 1),
            ..renderer::Settings::default()
        };
        let mut payload = RenderingPayload {
            settings,
            ..RenderingPayload::default()
        };

        let background = argb_from_colorref(payload.settings.misc.background_color);
        let foreground = argb_from_colorref(payload.settings.misc.foreground_color);
        payload.color_bitmap_row_stride = 2;
        payload.color_bitmap_depth_stride = 2;
        payload.color_bitmap = vec![background, background, foreground, foreground, 0, 0];

        let mut row = renderer::ShapedRow::default();
        row.push_glyphs(
            FontId(0),
            &[renderer::ShapedGlyph {
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
    #[ignore = "needs a Metal device"]
    fn a_metal_frame_has_the_backgrounds_and_the_cursor() {
        let Some(mut backend) = MetalBackend::create() else {
            eprintln!("metal: no backend; skipping");
            return;
        };
        let mut payload = payload();
        payload.rows.clear();
        payload.cursor_rect = (1, 0, 2, 1);

        backend.render(&mut payload);

        assert_eq!((backend.width, backend.height), (16, 16));
        assert_eq!(backend.pixel(0, 0), Some(0xFF00_0000));
        assert_eq!(backend.pixel(7, 15), Some(0xFF00_0000));
        assert_eq!(backend.pixel(8, 0), Some(0xFFFF_0000));
    }

    #[test]
    #[ignore = "needs a Metal device"]
    fn a_metal_glyph_paints_the_foreground_colour() {
        let Some(mut backend) = MetalBackend::create() else {
            eprintln!("metal: no backend; skipping");
            return;
        };
        backend.rasterizer = Some(Box::new(SolidRasterizer));
        let mut payload = payload();

        backend.render(&mut payload);

        // The 16px cell puts the baseline at y = 12, where the 1x1 glyph sits.
        assert_eq!(backend.pixel(0, 12), Some(0xFFFF_FFFF));
        assert_eq!(backend.pixel(0, 0), Some(0xFF00_0000));
        assert_eq!(backend.pixel(8, 12), Some(0xFF00_0000));
    }

    #[test]
    #[ignore = "needs a Metal device"]
    fn a_metal_glyph_keeps_its_coverage_mask() {
        let Some(mut backend) = MetalBackend::create() else {
            eprintln!("metal: no backend; skipping");
            return;
        };
        backend.rasterizer = Some(Box::new(PatternRasterizer));
        let mut payload = payload();

        backend.render(&mut payload);

        // The 2x2 mask starts at the baseline: white, black / black, white.
        assert_eq!(backend.pixel(0, 12), Some(0xFFFF_FFFF));
        assert_eq!(backend.pixel(1, 12), Some(0xFF00_0000));
        assert_eq!(backend.pixel(0, 13), Some(0xFF00_0000));
        assert_eq!(backend.pixel(1, 13), Some(0xFFFF_FFFF));
    }

    #[test]
    #[ignore = "needs a Metal device"]
    fn half_coverage_is_drawn_once() {
        let Some(mut backend) = MetalBackend::create() else {
            eprintln!("metal: no backend; skipping");
            return;
        };
        backend.rasterizer = Some(Box::new(HalfRasterizer));
        let mut payload = payload();

        backend.render(&mut payload);

        // One source-over draw at 50% coverage over black is 0x80.
        assert_eq!(backend.pixel(0, 12), Some(0xFF80_8080));
        assert_eq!(backend.pixel(3, 15), Some(0xFF80_8080));
    }

    /// Builds a payload with real `M`s at three columns, and the rasteriser
    /// that shaped it.
    fn shaped_payload(face: &renderer::FontFace) -> (RenderingPayload, renderer::FontDueRasterizer) {
        let size = 32.0;
        let rasterizer = renderer::FontDueRasterizer::from_face(face, size).expect("a face");
        let (ascent, descent) = rasterizer.vertical_metrics();
        let cell_width = (rasterizer.advance_width('M').ceil() as u16).max(1);
        let cell_height = ((ascent + descent).ceil() as u16).max(1);
        let baseline = ((f32::from(cell_height) - descent).round() as u16).clamp(1, cell_height - 1);

        let mut payload = payload();
        payload.settings.font.cell_size = (cell_width, cell_height);
        payload.settings.font.baseline = baseline;
        payload.settings.viewport_cell_count = (3, 1);
        payload.settings.target_size = (cell_width * 3, cell_height);

        let background = argb_from_colorref(payload.settings.misc.background_color);
        let foreground = argb_from_colorref(payload.settings.misc.foreground_color);
        payload.color_bitmap_row_stride = 3;
        payload.color_bitmap_depth_stride = 3;
        payload.color_bitmap = vec![background; 3]
            .into_iter()
            .chain(vec![foreground; 3])
            .chain(vec![0; 3])
            .collect();

        let glyph = rasterizer.glyph_index('M');
        let mut row = renderer::ShapedRow::default();
        for column in 0..3 {
            row.push_glyphs(
                FontId(0),
                &[renderer::ShapedGlyph {
                    glyph_index: glyph,
                    advance: f32::from(cell_width),
                    offset: (0.0, 0.0),
                    color: None,
                }],
                column,
            );
        }
        payload.rows = vec![row];
        (payload, rasterizer)
    }

    #[test]
    #[ignore = "needs a Metal device"]
    fn a_metal_glyph_matches_the_software_backend() {
        let Some(face) = super::super::find_face() else {
            eprintln!("metal: no face; skipping");
            return;
        };
        let (payload, rasterizer) = shaped_payload(&face);

        let mut software = renderer::SoftwareBackend::with_rasterizer(Box::new(
            renderer::FontDueRasterizer::from_face(&face, 32.0).expect("a face"),
        ));
        let mut software_payload = payload.clone();
        software.render(&mut software_payload);

        let Some(mut metal) = MetalBackend::create() else {
            eprintln!("metal: no backend; skipping");
            return;
        };
        metal.rasterizer = Some(Box::new(rasterizer));
        let mut metal_payload = payload.clone();
        metal.render(&mut metal_payload);

        assert_eq!((metal.width, metal.height), (software.width(), software.height()));
        let mut mismatches = 0;
        let mut first = None;
        for y in 0..metal.height {
            for x in 0..metal.width {
                let metal_pixel = metal.pixel(x, y);
                let software_pixel = software.pixel(x, y);
                if metal_pixel == software_pixel {
                    continue;
                }
                mismatches += 1;
                if first.is_none() {
                    first = Some(format!(
                        "({x},{y}) metal {:08x} software {:08x}",
                        metal_pixel.unwrap_or(0),
                        software_pixel.unwrap_or(0)
                    ));
                }
            }
        }
        assert_eq!(mismatches, 0, "first mismatch: {first:?}");
    }
}
