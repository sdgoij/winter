# ADR 014 — Selection is host state; the buffer keeps no selection

- Status: Accepted
- Date: 2026-11-22
- Related: `src/host/selection.cpp`, `src/host/selectionState.cpp`,
  `src/host/selectionInput.cpp`, `crates/buffer/src/selection.rs`;
  ADR 002, ADR 012

## Context

Upstream splits selection across two layers. `SCREEN_INFORMATION`'s host-side
`Selection` owns the interaction state: whether a selection is visible and in
progress, line versus block mode and its alternate flag, the selection flags a
client reads, the anchor and the rectangle, and the cursor it saves while
selecting. `TextBuffer` owns only the coordinate walks and text extraction for a
start/end pair.

This port's Phase 2 invented `buffer::Selection`: an anchor/extent pair in
screen-line coordinates with its own `get_text`. Nothing consumes it — the
adapter, the parser and the renderer never call it — and it cannot cover
scrollback, because its coordinates are screen lines. Porting the host selection
on top would leave two homes for the selection rectangle, which ADR 002
forbids.

## Decision

`host::SelectionState` is the home for the selection anchor, rectangle, flags,
line/block mode and the saved cursor.

`buffer::Selection` and the `TextBuffer` selection methods
(`start_selection`, `extend_selection`, `get_selection_text`,
`clear_selection`, `has_selection`, `get_selection_extent`) are removed. When
the copy path needs text, the buffer gains a rectangle-to-text operation shaped
like upstream's `GetTextRects`/`GetTextSpans`, taking buffer coordinates.

The rest of the family lands by file: `selectionState.cpp` is this state,
`selection.cpp` is the region operations, `selectionInput.cpp` is key and mouse
input. Paint, clipboard and window hit-testing stay with the renderer, the host
queue and the window.

## Rationale

- **One home.** The rectangle exists once, in the layer that also owns the
  flags and the cursor it saves.
- **Buffer coordinates.** Selection spans scrollback; a screen-line pair cannot.
- **The buffer stays storage.** Text extraction is a read over a rectangle, not
  a second selection model.

## Consequences

Positive:

- The host can port the state file without inventing a parallel selection.
- The buffer loses an unused public API rather than carrying it forward.

Negative:

- Code that used `buffer::Selection` must move to the host state; today only the
  buffer's own tests did.
- Text extraction for a copy waits for the buffer's rectangle API, which lands
  with the selection operations.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep `buffer::Selection` and have the host drive it | Two rectangle homes; screen-line coordinates cannot reach scrollback |
| Put the host state in `buffer` | Flags, saved cursor and line/block mode are console interaction, not storage |
| Port only `selection.cpp` and leave the state in the buffer | The operations need the state that would be missing |
