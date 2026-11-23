# ADR 005 — Images are buffer content

- Status: Accepted
- Date: 2026-11-22
- Related: ADR 001, ADR 002, ADR 003; `crates/buffer/src/image_slice.rs`,
  `row.rs`, `text_buffer.rs`; `crates/adapter/src/sixel_parser.rs`;
  `plan.md` §"Pre-Phase-5 cleanup"

## Context

Sixel (DCS) and DRCS (DECDLD) are the only DCS families still unported. Both
produce raster images, and both upstream handlers (`DefineSixelImage`,
`DownloadDRCS`) return a `StringHandler` that a sixel decoder writes pixels
through.

Upstream stores images in the text buffer, not in a side channel:

- every `ROW` owns an optional `ImageSlice`: a horizontal band of pixels
  covering `columnBegin..columnEnd` cells of that row;
- `ImageSlice` owns the pixel buffer and the cell size, and defines
  `CopyBlock`, `CopyRow`, `CopyCells`, `EraseBlock` and `EraseCells`;
- erase (ED, EL, ECH, DECSED, DECSEL, DECERA, DECSERA), copy (DECCRA), scroll
  and reflow move or erase image pixels alongside cells;
- the renderer draws the band under the text.

The Rust buffer has `ImageSlice` as geometry only (`x`, `y`, `width`,
`height`, `source_*`), `Row` has no image field, and there is no renderer.
Decoding sixel into an adapter-side store would leave the buffer unable to
erase, scroll or copy images, and would give the renderer a second source of
truth for screen state — which `CONTRIBUTING.md` forbids.

## Decision

1. **An image is buffer content.** `Row` gains an optional image band.
   `ImageSlice` gains pixel storage and the cell size, replacing the
   geometry-only fields. The adapter decodes and hands the band to the buffer;
   it never owns pixels.
2. **One band per row, as upstream.** `Row::set_image_slice` replaces the band.
   A decoder drawing across rows creates one band per affected row, clipped to
   `column_begin..column_end`.
3. **Cell operations carry their pixels.** Every operation that clears, moves
   or copies cells must do the same to the overlapping image band: `fill_rect`,
   `copy_row`, the scroll helpers, reflow (`resize`), DECCRA, and each erase
   handler. A cell operation that forgets the band is a bug of the same class
   as the insert-mode shift bugs already fixed.
4. **Reflow neither rescales nor rewraps an image.** `TextBuffer::resize`
   copies a source row's whole band to the first destination row of its logical
   line, as `TextBuffer::Reflow` does with `ImageSlice::CopyRow`; rows the line
   wraps into get no image.
5. **The renderer contract is a band, not a canvas.** A row's band is sampled
   per cell column, scaled to the cell size, and composed under the text. The
   renderer never mutates it.
6. **Image storage lands before the sixel decoder.** `DefineSixelImage` and
   `DownloadDRCS` stay unported until a test can write a band and read it back
   through the buffer. A decoder wired earlier would draw into nothing while
   DA1 advertises sixel support.

## Rationale

- Fidelity: upstream's erase/copy/scroll semantics are written in terms of row
  bands. Porting the operations and the storage together keeps the oracle
  usable instead of forcing a translation.
- One source of truth: the buffer stays the only owner of screen state, so a
  later renderer cannot disagree with the model about what is on screen.
- Testability before rendering: a band on a row is observable without a GPU,
  so the decoder, the erase paths and the copy paths can all be tested now.
- The existing `ImageSlice` type and module give the storage a home without a
  new crate or a dependency-ring change (ADR 001).

## Consequences

Positive:

- DECCRA and the erase family gain image handling once their row helpers move
  bands; there is no second pass to retrofit.
- The sixel and DRCS decoders can be validated by reading rows back, which is
  the only option until Phase 5 exists.

Negative:

- `Row` grows by an `Option`, and every cell-moving path needs a band-aware
  sibling. The risk of a forgotten path is real; each one gets a test.
- `ImageSlice` loses its current geometry-only shape, and its tests are
  rewritten. This is a breaking change inside the buffer crate, made once,
  when the storage lands.
- Reflow truncates images that span wrapped rows, which is what upstream does
  and is visible to users of very wide images.
- Pixel memory must be bounded by the decoder's own limits; the buffer does
  not impose a second cap.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Decode sixel into an adapter-side image store | Buffer operations could not erase, move or copy it; two sources of truth for screen state |
| Keep an id-keyed image table on `TextBuffer` | Neater for a GPU texture atlas, but it diverges from upstream's per-row bands and forces invented sharing and invalidation rules; revisit only if the renderer needs texture sharing |
| Decode and discard until the renderer exists | Placeholder success: `DefineSixelImage` would answer DA1 feature 4 while drawing nothing |
| Store images in the host layer | The host has no image primitives that merge with text, so the adapter would still need a model |
