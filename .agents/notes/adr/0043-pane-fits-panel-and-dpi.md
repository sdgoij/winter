# ADR 043 — The WinUI pane fits the panel's cells and the panel's DPI

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/winui/app.rs`, `crates/winterm/src/winui/surface.rs`,
  `crates/renderer/src/renderer.rs`, `crates/renderer/src/atlas/engine.rs`,
  ADR 006, ADR 040, ADR 041

## Context

The first WinUI shell (ADR 041) fixed the terminal at 80 by 24 cells and built a
DIP-sized image, whatever the window did. A maximised window therefore showed a
small grid, or a stretched one, and a high-DPI monitor got a magnified image
instead of more pixels.

`SwapChainPanel` reports `ActualWidth`/`ActualHeight` in DIPs and a
`CompositionScaleX/Y` (through `SwapChainPanelEvent::Metrics`). It multiplies its
swap chain's content by the composition scale, so a swap chain sized in device
pixels must carry the inverse transform or it is drawn too large.

Separately, `AtlasEngine::ensure_bitmap` copied only the target size and the
viewport into `RenderingPayload`. The software backend falls back to an 8 by 16
cell when the payload's cell size is zero, so it painted a smaller rectangle than
the target and left the rest of the buffer blank.

## Decision

The pane follows the panel.

- The grid is the whole number of cells that fit the panel's size in DIPs,
  recomputed whenever `Metrics` changes.
- The face is rasterised at `FONT_SIZE` times the composition scale, so the
  software buffer holds physical pixels and text is crisp at any scale.
- The swap chain is created and resized to the panel's physical size, and
  `SetMatrixTransform` applies the inverse composition scale. The grid is blitted
  at the origin, one pixel to one pixel; the sub-cell remainder is cleared to the
  terminal background.
- `Renderer::resize(target_size, viewport)` changes a target's pixels and a
  viewport's cells together, because they always describe the same resize.
- `AtlasEngine::ensure_bitmap` copies the whole `Settings` into the payload, so a
  backend reads the cell size, baseline and DPI the engine was given.
- The opening window size is declared once as a constant in DIPs. A declaration
  that changed with the grid would resize the window the user had just dragged.

## Rationale

A cell is the unit of agreement: the pty, the terminal buffer and the viewport
all count cells, so the grid is derived in cells and the pixel target follows.
Rasterising at the composition scale is the only way to get real physical pixels
from a rasteriser whose glyph bitmaps are fixed at load time. Releasing the
Direct2D target before `ResizeBuffers` is required by DXGI, and recreating it
afterwards is the portable way to keep drawing.

## Consequences

- A DPI change rebuilds the face and the renderer. That is rare (moving between
  monitors or changing Windows scaling) and cheap next to a frame.
- `Settings::target_size` and `viewport_cell_count` are `u16`; a panel too large
  for them saturates rather than wrapping.
- The grid is clamped to 1..=1024 cells per axis, far beyond any display, so bad
  metrics cannot ask the terminal for an allocation it cannot meet.
- A backend that ignores the payload's settings now paints the wrong size. The
  payload is the contract; the regression test names it.

## Alternatives considered

- **Paint in DIPs and let Direct2D stretch the bitmap.** Simplest, but text at
  125% or 150% is nearest-neighbour magnified. That is what the panel did before
  this record.
- **Keep the fixed grid and stretch the swap chain to the panel.** That is the
  behaviour this record removes: text grows with the window and the program never
  learns the new size.
- **Resize the swap chain in DIPs and let the composition scale it.** This works
  until the scale is not 1 and then either doubles the transform or leaves a
  letterbox.
- **Create a new swap chain on every resize instead of `ResizeBuffers`.** It
  forces a re-attach to the panel and drops the presented frame; `ResizeBuffers`
  on a composition swap chain is supported and cheaper.
