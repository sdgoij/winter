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
2. **The frame is triangle strips.** Cell backgrounds, grid lines and the cursor
   are solid quads; glyphs are textured quads sampling a one-channel atlas;
   image bands are textured quads sampling a BGRA texture cached by revision.
   Clip-space positions are computed on the CPU, so the vertex shaders are
   pass-throughs and no vertex descriptor is needed.
3. **Glyph coverage comes from the path's own rasteriser.** The same
   `GlyphRasterizer` the software backend uses fills a 1024² `R8Unorm` atlas,
   shelf-packed with a one-texel gutter. A glyph that does not fit waits for the
   next frame, which starts a fresh atlas, so a quad already emitted this frame
   never samples a cleared one.
4. **Presentation is an offscreen texture and a readback.** The content is
   rendered into a shared `BGRA8Unorm` texture, read back as `0xAARRGGBB` and
   handed to the existing `Frame` blit. This keeps the view, the screenshot
   path and the Apple appearance code unchanged, and it is what makes the
   content path testable: the ignored device tests compare pixels, and a
   real-font glyph matches the software backend exactly when its colours are
   opaque. Where antialiased text is blended over a fill, the GPU's
   round-to-nearest differs from the software path's truncation by at most one
   8-bit step. A `CAMetalLayer` is the next, presentation-only step.
5. **The software backend is the fallback.** No Metal device, a shader that
   does not compile, or `WINTER_SOFTWARE=1` selects it, and the shell prints
   why. A host never fails to show a terminal because the GPU path is
   unavailable.
6. **`objc2-metal` is a direct dependency, curated like `objc2-app-kit`.** The
   default feature set carries the whole MTL4, ray-tracing and machine-learning
   surface; only the modules the backend names are enabled. The crate is
   already in `Cargo.lock` through `wgpu-hal`, so the lock gains no new family.

## Rationale

- **It is ADR 063's shape on the platform's own API.** The payload, the draw
  order and the glyph atlas are already shared; the difference is Metal rather
  than Direct2D.
- **The readback is a milestone, not a mistake.** It costs a copy per frame,
  but it makes the GPU path verifiable on a machine with no display session and
  keeps one screenshot contract for both backends. Presentation can move to a
  drawable without changing any of the content code.
- **The fallback is cheap.** The software path already existed; `Content` is a
  two-variant enum whose methods delegate.

## Consequences

Positive:

- The pane's rasterisation runs on the GPU, matching the WinUI content path.
- The Metal frame matches the software frame; both backends are covered by
  tests, and a real-font glyph is exact when its colours are opaque.
- macOS keeps one renderer, one `TerminalData` and one screenshot path.

Negative:

- **A frame is read back to the CPU.** The offscreen texture is copied every
  frame so the existing blit can draw it; a `CAMetalLayer` drawable is the fix,
  and the readback stays only for screenshots then.
- **The shaders compile at startup.** A few hundred milliseconds on the first
  frame; a precompiled `metallib` is the alternative if that ever matters.
- **A second backend to keep in step.** A change to `RenderingPayload` or the
  pigment rules must be reflected in `metal.rs`; the pixel-equality tests and
  the shared payload are what constrain the drift.
- **Glyph colours and underline shapes follow the software path's model.** An
  explicit glyph colour and a retro shader are still not drawn on either
  backend.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A `CAMetalLayer` first | The only live check here cannot capture a drawable, so the content path would ship unverified |
| A third-party `metal` crate | Not in the lockfile, so it would need new fetching; `objc2-metal` is already pinned and used by `wgpu-hal` |
| CPU rasterisation into a texture | That is the software path with extra copies |
| Keep the software blit only | Correct but leaves the Windows and macOS content paths at different stages, and the seam already exists |
