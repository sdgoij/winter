# ADR 073 — The macOS content path is Metal, with a software fallback

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/winter/src/macos/metal.rs`, `crates/winter/Cargo.toml`,
  `crates/renderer/src/atlas/`; ADRs 007, 041, 063, 072

## Context

ADR 072's first cut rasterised the terminal on the CPU and copied the
software backend's frame into an `NSBitmapImageRep`. That is the same first cut
the WinUI shell shipped (ADR 041); ADR 063 then moved the Windows content path
onto Direct2D: cell backgrounds, images, glyph runs, grid lines and the cursor
became device draws. macOS has the same seam in `renderer::IBackend` and the
same payload, and Metal is available on every Mac the shell supports.

The macOS shell has no interactive WindowServer on the machine where it was
built, so a content path that could only be checked by looking at a live window
could not be verified here. The shell's own `WINTER_SCREENSHOT` path captures
the content view, which cannot see a `CAMetalLayer` drawable.

## Decision

1. **`macos::metal::MetalBackend` implements `renderer::IBackend`.** It owns the
   device, the command queue, three pipeline states and the shared glyph
   rasteriser; the shell's `Content` enum selects it or the software backend.
2. **The frame is two triangles per quad.** Cell backgrounds, grid lines and
   the cursor are solid quads; glyphs are textured quads sampling a one-channel
   atlas; image bands are textured quads sampling a BGRA texture cached by
   revision. Each quad is two explicit triangles, never a strip: a strip's
   bridge triangle would sample across the atlas. Clip-space positions are
   computed on the CPU, so the vertex shaders are pass-throughs and no vertex
   descriptor is needed.
3. **Glyph coverage comes from the path's own rasteriser.** The same
   `GlyphRasterizer` the software backend uses fills a 1024² `R8Unorm` atlas,
   shelf-packed with a one-texel gutter. A glyph that does not fit waits for the
   next frame, which starts a fresh atlas, so a quad already emitted this frame
   never samples a cleared one.
4. **Presentation is a `CAMetalLayer` drawable, with the offscreen readback as
   the screenshot path and the fallback.** The controller adds a `CAMetalLayer`
   above the view's own drawing, sizes it to the grid's whole cells and
   attaches it to the content backend; each frame is encoded into the layer's
   next drawable and presented. The same frame is also drawn into a shared
   `BGRA8Unorm` texture and read back as `0xAARRGGBB` for the existing `Frame`
   blit, because a drawable never appears in `cacheDisplayInRect`: that is how
   `WINTER_SCREENSHOT` captures the shell, and a layer that cannot hand out a
   drawable keeps a visible frame. The readback is also what makes the content
   path testable: the ignored device tests compare pixels, and a real-font
   glyph matches the software backend exactly when its colours are opaque.
   Where antialiased text is blended over a fill, the GPU's round-to-nearest
   differs from the software path's truncation by at most one 8-bit step.
5. **The two overlays that cross the content are layers too.** The find
   backdrop and the bell flash are drawn above the grid, so an opaque content
   layer would hide them. They are `CALayer`s above it, with the same geometry
   `chrome` paints when the software backend is selected; the strip and the
   scrollbar stay outside the content layer's frame and AppKit keeps painting
   them.
6. **The software backend is the fallback.** No Metal device, a shader that
   does not compile, or `WINTER_SOFTWARE=1` selects it, and the shell prints
   why. A host never fails to show a terminal because the GPU path is
   unavailable.
7. **`objc2-metal` and `objc2-quartz-core` are direct dependencies, curated
   like `objc2-app-kit`.** The default feature sets carry whole frameworks;
   only the modules the backend names are enabled. Both crates are already in
   `Cargo.lock` through the toolkit and `wgpu-hal`, so the lock gains no new
   family.

## Rationale

- **It is ADR 063's shape on the platform's own API.** The payload, the draw
  order and the glyph atlas are already shared; the difference is Metal rather
  than Direct2D.
- **The readback stays, as the screenshot path.** It costs a copy per frame,
  but it makes the GPU path verifiable on a machine with no display session
  and keeps one screenshot contract for both backends. The window itself is a
  drawable, so nothing in the content code knows which target it is drawing.
- **The fallback is cheap.** The software path already existed; `Content` is a
  two-variant enum whose methods delegate.

## Consequences

Positive:

- The pane's rasterisation runs on the GPU, matching the WinUI content path.
- The window shows a drawable, so presenting no longer copies the frame into an
  `NSBitmapImageRep` when the GPU path is in use.
- The Metal frame matches the software frame; both backends are covered by
  tests, and a real-font glyph is exact when its colours are opaque.
- macOS keeps one renderer, one `TerminalData` and one screenshot path.

Negative:

- **A frame is drawn twice.** The drawable is what the window shows and the
  offscreen target is the screenshot and the fallback, so the same quads are
  encoded into both and the command buffers are waited on twice. Making the
  readback conditional needs a way to tell the backend a capture is coming, and
  is the next optimisation if the cost shows.
- **The shaders compile at startup.** A few hundred milliseconds on the first
  frame; a precompiled `metallib` is the alternative if that ever matters.
- **A second backend to keep in step.** A change to `RenderingPayload` or the
  pigment rules must be reflected in `metal.rs`; the pixel-equality tests and
  the shared payload are what constrain the drift.
- **A layer is not part of a view's cached display.** The screenshot path hides
  the content and overlay layers and forces the view to paint the readback
  frame for that one draw.
- **Glyph colours and underline shapes follow the software path's model.** An
  explicit glyph colour and a retro shader are still not drawn on either
  backend.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A `CAMetalLayer` as the view's backing layer | The view also paints the strip, and AppKit draws the find field above it; the content is a sublayer sized to the grid, not the view's layer |
| A third-party `metal` crate | Not in the lockfile, so it would need new fetching; `objc2-metal` is already pinned and used by `wgpu-hal` |
| CPU rasterisation into a texture | That is the software path with extra copies |
| Keep the software blit only | Correct but leaves the Windows and macOS content paths at different stages, and the seam already exists |
