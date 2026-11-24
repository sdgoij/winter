# ADR 063 — The WinUI pane paints on the GPU with Direct2D

- Status: Accepted
- Date: 2026-11-23
- Related: `crates/winter/src/winui/gpu.rs`, `winui/surface.rs`,
  `crates/renderer/src/atlas/common.rs`; ADRs 007, 041, 043, 045

## Context

The atlas engine hands a backend a `RenderingPayload`: shaped rows, one colour
bitmap, the cursor rectangle and the cell metrics. `SoftwareBackend` rasterised
it into a CPU buffer and `surface.rs` uploaded that buffer as one Direct2D bitmap
per frame. A frame therefore paid for a full CPU rasterisation and a whole-pane
upload, and the GPU that already hosts the swap chain did nothing.

ADR 007 said the next backend would be `wgpu`, created by the UI layer with a
surface. The WinUI shell settled what that surface is: the pane is a
`SwapChainPanel` whose composition swap chain the shell creates and hands to the
panel binding (ADR 041). `wgpu` cannot adopt an existing `IDXGISwapChain1`, so a
wgpu backend would mean DXGI/D3D12 interop and a second graphics API for a window
that already has a D3D11 device and a Direct2D context.

## Decision

The WinUI shell paints the payload with Direct2D on the pane's own device
context:

- Cell backgrounds, grid lines and the cursor are `FillRectangle` calls with one
  solid brush whose colour changes per cell.
- Text is a DirectWrite `IDWriteFontFace` for the shell's font file, drawn with
  `DrawGlyphRun`, one run per colour change so SGR colours stay on their cells.
  Advances follow the shaped columns, so the grid is reproduced exactly.
- Text antialiasing is grayscale, matching the software backend.
- An image keeps one bitmap per revision and is drawn with `DrawBitmap` between
  the backgrounds and the text.

The frame semantics moved out of `SoftwareBackend` into `atlas::common` — cell
colours, the cell rectangle, the baseline, glyph placement, grid-line rectangles
— so both paths share one definition and the portable parts are tested in the
gate.

`IBackend` does not change. `WINTER_SOFTWARE_RENDERER=1` forces the CPU path.

## Rationale

- **The surface already exists.** The device, the swap chain and the context are
  the pane's; the backend adds a brush and a font face to them.
- **DirectWrite is the platform's own text path.** It is also what upstream's
  `BackendD2D` uses for ordinary text, so the port follows the oracle rather than
  inventing an atlas protocol D2D does not support (see below).
- **No new graphics stack.** `windows` is already here; `windows-numerics` came
  with it as a dependency of `windows`.
- **The frame's cost stops scaling with pixels on the CPU.**

## Consequences

Positive:

- Text, backgrounds, lines and the cursor are drawn by the GPU.
- One draw call per colour run and per cell, with no per-frame CPU blending.

Negative:

- Glyph runs use DirectWrite's shaping of the same font file. The emulator's
  shaped indices come from that font's cmap, so the two agree; a font whose
  engine and DirectWrite disagree would need the software path.
- The backend repaints the whole grid every frame; the payload's dirty rectangle
  and scroll delta are still unused.
- The software backend remains the one the gate exercises.

## Alternatives considered

| Alternative | Why not |
|---|---|
| `ID2D1SpriteBatch` with a glyph atlas | `CreateSpriteBatch` succeeds, but `DrawSpriteBatch` fails on this device context (its HRESULT is not even a D2D code, and the binding drops it), poisoning the context until `EndDraw` reports `D2DERR_WRONG_STATE` |
| `FillOpacityMask` with an A8 atlas | Returns `D2DERR_WRONG_STATE` on an `ID2D1DeviceContext` even with an A8 premultiplied mask, as a headless probe showed; the call is not usable here |
| A `wgpu` backend with a new surface | `wgpu` cannot present into a `SwapChainPanel`'s existing swap chain; DXGI/D3D12 interop is a second stack for the same window |
| Direct3D 11 instancing with an HLSL shader | One draw call for everything, but a shader build step and vertex-input plumbing; Direct2D is already resolving the glyphs |
| Keep the software path and optimise the blit | The CPU rasterisation is the cost, not the upload |
