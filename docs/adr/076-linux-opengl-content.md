# ADR 076 — The Linux content path is OpenGL, with a software fallback

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/gtk4/src/gl.rs`, `crates/gtk4/Cargo.toml`,
  `crates/renderer/src/atlas/`; ADRs 007, 063, 073, 075

## Context

ADR 075's GTK shell rasterised the terminal on the CPU and copied the software
backend's frame into a Cairo surface every frame. That is the first cut WinUI
shipped (ADR 041) and macOS replaced with Metal (ADR 073); ADR 063 gave Windows
Direct2D. The seam is the same on every platform: `renderer::IBackend`, whose
`render` receives the frame's `RenderingPayload` of cell colours, shaped glyphs,
images and the cursor.

GTK's GPU hook is a `GtkGLArea`: GTK creates the context, makes it current for
the area's `render` signal and composites the area's framebuffer into the
window. GDK deliberately exposes no entry-point resolver, so a backend resolves
its own symbols from libGL/libEGL.

`wgpu` cannot adopt GTK's context or framebuffer; that is the reason ADR 063
rejected it for the WinUI swap chain, and it applies here unchanged.

## Decision

1. **`linux::gl::Gl` implements `renderer::IBackend`.** The shell builds the
   engine with it when this host has GL, and with the software backend
   otherwise; the backend's GL objects are created lazily on the first frame,
   because that is the first moment a context is current.
2. **One `GtkGLArea` above the base drawing area, inset by the pane margins.**
   The GL area's framebuffer is exactly the grid, so its clear covers the cells
   and its viewport needs no inset; the base widget paints the pane's fill, the
   left margin, the scrollbar and its prompt marks, and holds the input
   controllers. The bell flash is a quad in the GL frame, because it crosses the
   grid.
3. **The draw list is the payload's, with the same shared helpers Metal uses.**
   `cell_rect`, `cell_colors`, `glyph_origin`, `grid_line_rects` and `baseline`
   come from `renderer::atlas::common`; backgrounds, grid lines and the cursor
   are solid triangles, glyphs sample a one-channel 1024² atlas filled by the
   same `GlyphRasterizer` the software path uses, and images are RGBA textures
   cached per revision (the `0xAARRGGBB` source is swizzled once on upload).
4. **Two shader variants, desktop and embedded.** A desktop context runs
   `#version 330 core`; a GLES context runs `#version 300 es`. A desktop context
   older than 3.3 or a GLES context older than 3.0 selects the software path.
5. **Presentation is the GL area's framebuffer; nothing is read back.** The
   shell's `WINTER_SCREENSHOT` capture uses `GtkWidgetPaintable`, which includes
   the GL area's composited texture, so the capture path is the same as the
   software one and a frame never pays a full-screen copy.
6. **The software path is the fallback, and it is selectable.** `WINTER_SOFTWARE=1`,
   no libGL/libEGL, a context that fails to realize, a shader that does not
   compile, or a frame whose backend reports a failure rebuilds the renderer with
   the software backend and prints why. A host never fails to show a terminal
   because its GL is missing.
7. **The loader is the module's one unsafe exemption.** `linux::gl` carries a
   module-level `#[allow(unsafe_code)]` for `dlopen`/`dlsym` through
   `libloading`, for `glow`'s calls, and for the vertex/texel slice casts; every
   call has a `SAFETY:` note. The workspace deny stays in force everywhere else.
   `glow`, `libloading` and `bytemuck` are already in `Cargo.lock` through
   `eframe` and `wgpu`, so the lock gains no new family; all three are optional
   and enabled by the `gtk` feature.

## Rationale

- **It is ADR 073's shape on the platform's own API.** The payload, draw order,
  atlas packing and quad construction are the Metal backend's; the difference is
  OpenGL calls over the context GTK already made current.
- **The pane owns no new window or surface.** A GL area is a widget, so the
  window, the tab strip, the find bar and the settings editor stay GTK's, and
  the capture contract does not change.
- **The fallback is cheap.** The software backend already existed and is still
  built; a failed GPU path rebuilds the renderer with it.

## Consequences

Positive:

- The grid's rasterisation runs on the GPU, matching the WinUI and macOS content
  paths; presenting costs no CPU blit.
- The GPU frame matches the software frame in layout, colour and text; both are
  covered by the capture rig, and a drag-select drawn through GL was checked on
  an Xvfb GLES 3.1 context.
- The scale factor is threaded through both content paths. GTK allocates logical
  pixels while the face, the cell and the GL viewport are physical: the shell
  takes the widget's scale factor (the ceiling of a fractional scale, matching
  the GL framebuffer), rasterises the face at that scale, converts the pane's
  logical size to physical cells when it fits the grid, divides the cell back
  down for pointer, wheel and caret positions, and draws the software frame at
  `1/scale`. A 2x or fractional panel therefore shows the whole grid instead of
  a corner of it.
- Linux keeps one renderer, one `TerminalData` and one screenshot path.

Negative:

- **A second backend to keep in step.** A change to `RenderingPayload` or the
  pigment rules must be reflected in `gl.rs` as in `metal.rs`; the shared helpers
  and the capture rig constrain the drift.
- **GL resources live with the context.** A rebuilt renderer drops its backend
  and the objects live until the context is destroyed; there is no per-rebuild
  deletion, which matters only when the font size changes repeatedly.
- **Image uploads swizzle on the CPU.** Bands are rare; a per-revision cache
  keeps the cost to the frame that carries a new image.

## Alternatives considered

| Alternative | Why not |
|---|---|
| `wgpu` backend on a GTK window | It cannot adopt GTK's GL context or framebuffer; ADR 063 rejected the same mismatch for WinUI's swap chain |
| GSK render nodes with one texture per glyph | `GskTextureNode` cannot sample a sub-rectangle of an atlas, and Pango layouts would bypass the renderer's cell metrics |
| CPU rasterisation into a GL texture | That is the software path with extra copies |
| Keep the software blit only | Correct but leaves Linux's content path a generation behind the other hosts, with the seam already in place |
