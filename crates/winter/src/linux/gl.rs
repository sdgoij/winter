//! The OpenGL content backend: cell quads and glyphs drawn on the GPU.
//!
//! The software backend rasterises every pixel on the CPU and the shell blits
//! the result; this backend keeps the rasterisation on the device. Cell
//! backgrounds, images, glyphs, grid lines and the cursor become triangle
//! quads, glyph coverage comes from the same [`GlyphRasterizer`] the software
//! path uses and lands in a one-channel atlas, and images are uploaded per
//! revision.
//!
//! GTK makes the pane's `GdkGLContext` current in `GtkGLArea::render`, which is
//! where the engine's frame is drawn; a frame therefore costs no CPU blit and
//! no texture copy. The software path stays the fallback when this host has no
//! GL, or when `WINTER_SOFTWARE=1` asks for it (ADR 076).
//!
//! `unsafe` is confined to this module by the allow below: loading libGL/libEGL
//! and resolving the context's entry points is FFI, and glow's calls are
//! `unsafe` because they touch driver state. Every call carries a `SAFETY:`
//! note.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};
use std::rc::Rc;

use glow::HasContext;
use libloading::Library;

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

/// The backend handle the engine and the shell share.
#[derive(Clone)]
pub(crate) struct Gl {
    inner: Rc<RefCell<State>>,
}

struct State {
    loader: Loader,
    backend: Option<GlBackend>,
    /// The rasteriser the payload was shaped for, handed to the backend on
    /// creation.
    rasterizer: Option<Box<dyn GlyphRasterizer>>,
    /// A creation failure was reported, so the shell can fall back.
    failed: bool,
    /// The bell's opacity, set by the shell before a frame.
    bell: Option<f32>,
}

impl Gl {
    /// Finds libGL/libEGL, or reports that this host cannot run the GPU path.
    pub(crate) fn new() -> Option<Self> {
        let loader = Loader::new()?;
        Some(Self {
            inner: Rc::new(RefCell::new(State {
                loader,
                backend: None,
                rasterizer: None,
                failed: false,
                bell: None,
            })),
        })
    }

    /// Whether the backend was tried and could not be created.
    pub(crate) fn failed(&self) -> bool {
        self.inner.borrow().failed
    }

    /// Sets the bell flash for the next frame.
    pub(crate) fn set_bell(&self, alpha: Option<f32>) {
        self.inner.borrow_mut().bell = alpha;
    }

    /// Gives the backend the glyph rasteriser the payload was shaped for.
    pub(crate) fn set_rasterizer(&self, rasterizer: Box<dyn GlyphRasterizer>) {
        let mut state = self.inner.borrow_mut();
        if let Some(backend) = state.backend.as_mut() {
            backend.rasterizer = Some(rasterizer);
        } else {
            state.rasterizer = Some(rasterizer);
        }
    }
}

impl IBackend for Gl {
    fn release_resources(&mut self) {
        // The GL objects are owned by the context and released with it; the
        // handle only drops its CPU-side state.
        let mut state = self.inner.borrow_mut();
        state.backend = None;
    }

    fn render(&mut self, payload: &mut RenderingPayload) {
        let mut state = self.inner.borrow_mut();
        if state.backend.is_none() {
            if state.failed {
                return;
            }
            let rasterizer = state.rasterizer.take();
            match GlBackend::new(&state.loader, rasterizer) {
                Some(backend) => state.backend = Some(backend),
                None => {
                    state.failed = true;
                    eprintln!("winter: no OpenGL content path; the software path will be used");
                    return;
                }
            }
        }

        let State { backend, bell, .. } = &mut *state;
        if let Some(backend) = backend.as_mut() {
            backend.render(payload, *bell);
        }
    }

    fn requires_continuous_redraw(&mut self) -> bool {
        false
    }
}

/// The context, its programs and the atlases one frame is drawn with.
struct GlBackend {
    gl: glow::Context,
    solid: Batch,
    texture: Batch,
    atlas: Atlas,
    rasterizer: Option<Box<dyn GlyphRasterizer>>,
    images: HashMap<u64, glow::Texture>,
}

impl GlBackend {
    /// Resolves the current context's entry points and builds the pipelines.
    fn new(loader: &Loader, rasterizer: Option<Box<dyn GlyphRasterizer>>) -> Option<Self> {
        // SAFETY: the caller runs inside the GL area's render callback, so a
        // context is current; glow stores the resolved pointers and only calls
        // them while that context is current.
        let gl = unsafe { glow::Context::from_loader_function(|name| loader.address(name)) };

        let version = gl.version();
        if version.is_embedded {
            if (version.major, version.minor) < (3, 0) {
                eprintln!(
                    "winter: OpenGL ES {}.{} is too old for the GPU path",
                    version.major, version.minor
                );
                return None;
            }
        } else if (version.major, version.minor) < (3, 3) {
            eprintln!(
                "winter: OpenGL {}.{} is too old for the GPU path",
                version.major, version.minor
            );
            return None;
        }

        let solid = Batch::new(
            &gl,
            &solid_vertex_source(version.is_embedded),
            &solid_fragment_source(version.is_embedded),
            &[(0, 2, 0), (1, 4, 2)],
            false,
        )?;
        let texture = Batch::new(
            &gl,
            &texture_vertex_source(version.is_embedded),
            &glyph_fragment_source(version.is_embedded),
            &[(0, 2, 0), (1, 2, 2), (2, 4, 4)],
            true,
        )?;

        let atlas = Atlas::new(&gl)?;
        // The line a live run is checked against: the failures above say why the
        // GPU path was refused, and this one says it is in use.
        eprintln!(
            "winter: {} {}.{} -- the GPU content path",
            if version.is_embedded { "OpenGL ES" } else { "OpenGL" },
            version.major,
            version.minor
        );
        Some(Self {
            gl,
            solid,
            texture,
            atlas,
            rasterizer,
            images: HashMap::new(),
        })
    }

    /// Draws one payload into the framebuffer the GL area bound.
    fn render(&mut self, payload: &mut RenderingPayload, bell: Option<f32>) {
        let (cell_width, cell_height) = cell_size(payload);
        let (columns, rows) = payload.settings.viewport_cell_count;
        let width = target_side(payload.settings.target_size.0, columns, cell_width);
        let height = target_side(payload.settings.target_size.1, rows, cell_height);
        if width == 0 || height == 0 {
            return;
        }

        let data = self.build(payload, cell_width, cell_height, width, height, bell);
        self.atlas.upload(&self.gl);

        let clear = argb_from_colorref(payload.settings.misc.background_color);
        let [red, green, blue, _] = color(clear);
        // SAFETY: the GL area's context is current and its framebuffer is
        // bound; every call below touches only this backend's objects.
        unsafe {
            self.gl.viewport(0, 0, width as i32, height as i32);
            self.gl.clear_color(red, green, blue, 1.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
            self.gl.disable(glow::DEPTH_TEST);
            self.gl.disable(glow::CULL_FACE);
            self.gl.enable(glow::BLEND);
            self.gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        }

        self.solid.draw(&self.gl, &data.solids);
        for image in &data.images {
            let Some(texture) = self.images.get(&image.revision) else {
                continue;
            };
            // SAFETY: the image batch's program samples texture unit zero.
            unsafe {
                self.gl.active_texture(glow::TEXTURE0);
                self.gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
            }
            self.texture.draw(&self.gl, &image.vertices);
        }
        // SAFETY: the glyph atlas lives on texture unit zero for this batch.
        unsafe {
            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas.texture));
        }
        self.texture.draw(&self.gl, &data.glyphs);
        self.solid.draw(&self.gl, &data.overlays);

        // A remaining image texture is not needed by this frame.
        let used: Vec<u64> = data.images.iter().map(|image| image.revision).collect();
        self.images.retain(|revision, texture| {
            if used.contains(revision) {
                return true;
            }
            // SAFETY: the context is current and this texture belongs to it.
            unsafe { self.gl.delete_texture(*texture) };
            false
        });
    }

    /// Turns the payload into the vertex batches this frame draws.
    fn build(
        &mut self,
        payload: &RenderingPayload,
        cell_width: u32,
        cell_height: u32,
        width: u32,
        height: u32,
        bell: Option<f32>,
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

        let images = self.prepare_images(payload, cell_width, cell_height, width, height);

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
        if let Some(alpha) = bell {
            let layer = ((alpha.clamp(0.0, 1.0) * 0.12 * 255.0).round() as u32) << 24;
            push_solid(
                &mut overlays,
                (0, 0, width as i32, height as i32),
                layer | 0x00FF_FFFF,
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

    /// Uploads every image band that is not cached, and returns its quad.
    fn prepare_images(
        &mut self,
        payload: &RenderingPayload,
        cell_width: u32,
        cell_height: u32,
        width: u32,
        height: u32,
    ) -> Vec<ImageDraw> {
        let mut draws = Vec::new();
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

            if !self.images.contains_key(&bitmap.revision) {
                let Some(texture) = image_texture(
                    &self.gl,
                    source_width as u32,
                    source_height as u32,
                    &bitmap.source[..needed],
                ) else {
                    continue;
                };
                self.images.insert(bitmap.revision, texture);
            }

            let left = bitmap.target_offset.max(0) * cell_width as i32;
            let top = row_index as i32 * cell_height as i32;
            let quad = textured_quad(
                (
                    left,
                    top,
                    left + bitmap.target_width * cell_width as i32,
                    top + cell_height as i32,
                ),
                [0.0, 0.0, 1.0, 1.0],
                [1.0; 4],
                width,
                height,
            );
            draws.push(ImageDraw {
                revision: bitmap.revision,
                vertices: quad.to_vec(),
            });
        }
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
        out: &mut Vec<f32>,
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
                let quad = textured_quad(
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
                );
                out.extend_from_slice(&quad);
            }
        }
    }
}

/// One frame's vertex batches.
struct FrameData {
    solids: Vec<f32>,
    images: Vec<ImageDraw>,
    glyphs: Vec<f32>,
    overlays: Vec<f32>,
}

/// One image band and the vertices that draw it.
struct ImageDraw {
    revision: u64,
    vertices: Vec<f32>,
}

/// A program, its vertex array and the buffer its vertices arrive in.
struct Batch {
    program: glow::Program,
    vao: glow::VertexArray,
    buffer: glow::Buffer,
    floats_per_vertex: usize,
}

impl Batch {
    fn new(
        gl: &glow::Context,
        vertex_source: &str,
        fragment_source: &str,
        attributes: &[(u32, i32, usize)],
        textured: bool,
    ) -> Option<Self> {
        // SAFETY: the context is current; every object is created and linked
        // here and dropped with the context.
        unsafe {
            let program = gl.create_program().ok()?;
            let vertex = shader(gl, glow::VERTEX_SHADER, vertex_source)?;
            let fragment = shader(gl, glow::FRAGMENT_SHADER, fragment_source)?;
            gl.attach_shader(program, vertex);
            gl.attach_shader(program, fragment);
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                eprintln!("gl: link: {}", gl.get_program_info_log(program));
                gl.delete_shader(vertex);
                gl.delete_shader(fragment);
                gl.delete_program(program);
                return None;
            }
            gl.detach_shader(program, vertex);
            gl.detach_shader(program, fragment);
            gl.delete_shader(vertex);
            gl.delete_shader(fragment);

            let stride = attributes
                .iter()
                .map(|&(_, size, offset)| offset + size as usize)
                .max()
                .unwrap_or(0);

            let vao = gl.create_vertex_array().ok()?;
            gl.bind_vertex_array(Some(vao));
            let buffer = gl.create_buffer().ok()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            for &(location, size, offset) in attributes {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(
                    location,
                    size,
                    glow::FLOAT,
                    false,
                    (stride * 4) as i32,
                    (offset * 4) as i32,
                );
            }
            gl.bind_vertex_array(None);

            if textured {
                gl.use_program(Some(program));
                let sampler = gl.get_uniform_location(program, "u_texture");
                gl.uniform_1_i32(sampler.as_ref(), 0);
            }

            Some(Self {
                program,
                vao,
                buffer,
                floats_per_vertex: stride,
            })
        }
    }

    fn draw(&self, gl: &glow::Context, vertices: &[f32]) {
        if vertices.is_empty() || self.floats_per_vertex == 0 {
            return;
        }
        let count = vertices.len() / self.floats_per_vertex;
        // SAFETY: the context is current, the buffer holds exactly these
        // vertices, and the program and array are this batch's.
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(vertices), glow::STREAM_DRAW);
            gl.draw_arrays(glow::TRIANGLES, 0, count as i32);
            gl.bind_vertex_array(None);
        }
    }
}

fn shader(gl: &glow::Context, kind: u32, source: &str) -> Option<glow::Shader> {
    // SAFETY: the context is current and `source` is the shader's whole text.
    unsafe {
        let shader = gl.create_shader(kind).ok()?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            eprintln!("gl: compile: {}", gl.get_shader_info_log(shader));
            gl.delete_shader(shader);
            return None;
        }
        Some(shader)
    }
}

/// The glyph atlas and its shelf packer.
struct Atlas {
    texture: glow::Texture,
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
    fn new(gl: &glow::Context) -> Option<Self> {
        // SAFETY: the context is current; the texture is created here and the
        // parameters are the constants glTexParameteri accepts.
        unsafe {
            let texture = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8 as i32,
                ATLAS_SIZE as i32,
                ATLAS_SIZE as i32,
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);

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

    fn upload(&mut self, gl: &glow::Context) {
        if !self.dirty {
            return;
        }
        // SAFETY: the context is current, the texture is the whole atlas, and
        // `coverage` is exactly `ATLAS_SIZE * ATLAS_SIZE` readable bytes.
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                ATLAS_SIZE as i32,
                ATLAS_SIZE as i32,
                glow::RED,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&self.coverage)),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        self.dirty = false;
    }
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
    /// The same entry's four normalized corners.
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

fn image_texture(gl: &glow::Context, width: u32, height: u32, source: &[u32]) -> Option<glow::Texture> {
    let mut bytes = Vec::with_capacity(source.len() * 4);
    for pixel in source {
        bytes.push((pixel >> 16) as u8);
        bytes.push((pixel >> 8) as u8);
        bytes.push(*pixel as u8);
        bytes.push((pixel >> 24) as u8);
    }

    // SAFETY: the context is current; the texture is created here and `bytes`
    // holds four readable bytes per texel.
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            width as i32,
            height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&bytes)),
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(texture)
    }
}

/// The dynamic loader for the context's entry points.
struct Loader {
    _gl: Option<Library>,
    _gles: Option<Library>,
    _egl: Option<Library>,
    glx: Option<unsafe extern "C" fn(*const u8) -> *const c_void>,
    egl: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
}

impl Loader {
    fn new() -> Option<Self> {
        // SAFETY: dlopen of the well-known sonames; the function pointers are
        // only called with NUL-terminated names.
        unsafe {
            let gl = Library::new("libGL.so.1").ok();
            let gles = Library::new("libGLESv2.so.2").ok();
            let egl = Library::new("libEGL.so.1").ok();
            let glx = gl.as_ref().and_then(|library| {
                library
                    .get::<unsafe extern "C" fn(*const u8) -> *const c_void>(b"glXGetProcAddressARB\0")
                    .ok()
                    .map(|symbol| *symbol)
            });
            let egl_function = egl.as_ref().and_then(|library| {
                library
                    .get::<unsafe extern "C" fn(*const c_char) -> *const c_void>(b"eglGetProcAddress\0")
                    .ok()
                    .map(|symbol| *symbol)
            });
            if glx.is_none() && egl_function.is_none() {
                return None;
            }
            Some(Self {
                _gl: gl,
                _gles: gles,
                _egl: egl,
                glx,
                egl: egl_function,
            })
        }
    }

    /// Resolves one entry point for the current context.
    fn address(&self, name: &str) -> *const c_void {
        let Ok(cname) = CString::new(name) else {
            return std::ptr::null();
        };
        // SAFETY: both function pointers come from the loaded libraries and
        // accept a NUL-terminated name; the fallback reads the libraries'
        // symbol tables.
        unsafe {
            if let Some(glx) = self.glx {
                // `c_char` is `i8` on x86_64 and `u8` on aarch64, so naming the
                // target type is the only spelling both accept: `as` is a cast
                // to the same type on one of them, which clippy refuses.
                let address = glx(cname.as_ptr().cast::<u8>());
                if !address.is_null() {
                    return address;
                }
            }
            if let Some(egl) = self.egl {
                let address = egl(cname.as_ptr());
                if !address.is_null() {
                    return address;
                }
            }
            type RawProc = unsafe extern "C" fn();
            let bytes = cname.to_bytes_with_nul();
            if let Some(symbol) = self._gl.as_ref().and_then(|library| library.get::<RawProc>(bytes).ok()) {
                return *symbol as *const c_void;
            }
            if let Some(symbol) = self
                ._gles
                .as_ref()
                .and_then(|library| library.get::<RawProc>(bytes).ok())
            {
                return *symbol as *const c_void;
            }
            std::ptr::null()
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

fn push_solid(out: &mut Vec<f32>, rect: (i32, i32, i32, i32), argb: u32, width: u32, height: u32) {
    let (left, top, right, bottom) = rect;
    if right <= left || bottom <= top {
        return;
    }
    let (x0, x1) = (ndc_x(left as f32, width), ndc_x(right as f32, width));
    let (y0, y1) = (ndc_y(top as f32, height), ndc_y(bottom as f32, height));
    let fill = color(argb);
    let mut vertex = |x: f32, y: f32| {
        out.extend_from_slice(&[x, y]);
        out.extend_from_slice(&fill);
    };
    vertex(x0, y0);
    vertex(x1, y0);
    vertex(x0, y1);
    vertex(x1, y0);
    vertex(x1, y1);
    vertex(x0, y1);
}

#[allow(clippy::too_many_arguments)]
fn textured_quad(rect: (i32, i32, i32, i32), uv: [f32; 4], color: [f32; 4], width: u32, height: u32) -> [f32; 48] {
    let (left, top, right, bottom) = rect;
    let (x0, x1) = (ndc_x(left as f32, width), ndc_x(right as f32, width));
    let (y0, y1) = (ndc_y(top as f32, height), ndc_y(bottom as f32, height));
    let [u0, v0, u1, v1] = uv;
    let mut out = [0.0f32; 48];
    let mut at = 0;
    let mut push = |x: f32, y: f32, u: f32, v: f32| {
        out[at..at + 8].copy_from_slice(&[x, y, u, v, color[0], color[1], color[2], color[3]]);
        at += 8;
    };
    push(x0, y0, u0, v0);
    push(x1, y0, u1, v0);
    push(x0, y1, u0, v1);
    push(x1, y0, u1, v0);
    push(x1, y1, u1, v1);
    push(x0, y1, u0, v1);
    out
}

fn solid_vertex_source(embedded: bool) -> String {
    if embedded {
        "#version 300 es\n\
         precision mediump float;\n\
         layout(location = 0) in vec2 a_position;\n\
         layout(location = 1) in vec4 a_color;\n\
         out vec4 v_color;\n\
         void main() { gl_Position = vec4(a_position, 0.0, 1.0); v_color = a_color; }\n"
            .to_string()
    } else {
        "#version 330 core\n\
         layout(location = 0) in vec2 a_position;\n\
         layout(location = 1) in vec4 a_color;\n\
         out vec4 v_color;\n\
         void main() { gl_Position = vec4(a_position, 0.0, 1.0); v_color = a_color; }\n"
            .to_string()
    }
}

fn solid_fragment_source(embedded: bool) -> String {
    if embedded {
        "#version 300 es\n\
         precision mediump float;\n\
         in vec4 v_color;\n\
         out vec4 frag_color;\n\
         void main() { frag_color = v_color; }\n"
            .to_string()
    } else {
        "#version 330 core\n\
         in vec4 v_color;\n\
         out vec4 frag_color;\n\
         void main() { frag_color = v_color; }\n"
            .to_string()
    }
}

fn texture_vertex_source(embedded: bool) -> String {
    if embedded {
        "#version 300 es\n\
         precision mediump float;\n\
         layout(location = 0) in vec2 a_position;\n\
         layout(location = 1) in vec2 a_uv;\n\
         layout(location = 2) in vec4 a_color;\n\
         out vec2 v_uv;\n\
         out vec4 v_color;\n\
         void main() { gl_Position = vec4(a_position, 0.0, 1.0); v_uv = a_uv; v_color = a_color; }\n"
            .to_string()
    } else {
        "#version 330 core\n\
         layout(location = 0) in vec2 a_position;\n\
         layout(location = 1) in vec2 a_uv;\n\
         layout(location = 2) in vec4 a_color;\n\
         out vec2 v_uv;\n\
         out vec4 v_color;\n\
         void main() { gl_Position = vec4(a_position, 0.0, 1.0); v_uv = a_uv; v_color = a_color; }\n"
            .to_string()
    }
}

fn glyph_fragment_source(embedded: bool) -> String {
    if embedded {
        "#version 300 es\n\
         precision mediump float;\n\
         in vec2 v_uv;\n\
         in vec4 v_color;\n\
         uniform sampler2D u_texture;\n\
         out vec4 frag_color;\n\
         void main() { float coverage = texture(u_texture, v_uv).r; frag_color = vec4(v_color.rgb, v_color.a * coverage); }\n"
            .to_string()
    } else {
        "#version 330 core\n\
         in vec2 v_uv;\n\
         in vec4 v_color;\n\
         uniform sampler2D u_texture;\n\
         out vec4 frag_color;\n\
         void main() { float coverage = texture(u_texture, v_uv).r; frag_color = vec4(v_color.rgb, v_color.a * coverage); }\n"
            .to_string()
    }
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

        assert_eq!(vertices.len(), 36);
        assert_eq!(&vertices[0..2], &[-1.0, 1.0]);
        assert_eq!(&vertices[12..14], &[-1.0, 0.0]);
        assert_eq!(&vertices[24..26], &[0.0, 0.0]);
        assert_eq!(&vertices[2..6], &[1.0, 0.0, 0.0, 1.0]);
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
        assert_eq!(&quad[2..4], &[0.25, 0.5]);
        assert_eq!(&quad[10..12], &[0.75, 0.5]);
        assert_eq!(&quad[18..20], &[0.25, 1.0]);
        assert_eq!(&quad[26..28], &[0.75, 0.5]);
        assert_eq!(&quad[34..36], &[0.75, 1.0]);
        assert_eq!(&quad[42..44], &[0.25, 1.0]);
    }

    #[test]
    fn a_colour_ref_is_argb() {
        assert_eq!(argb_from_colorref(0x0012_3456), 0xFF56_3412);
        assert_eq!(color(0x80FF_0000), [1.0, 0.0, 0.0, 128.0 / 255.0]);
    }
}
