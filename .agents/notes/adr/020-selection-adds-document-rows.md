# ADR 020 — Selection addresses document rows

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/buffer/src/text_buffer.rs`, `crates/winterm/src/view.rs`,
  `crates/winterm/src/grid.rs`; ADR 019

## Context

ADR 019 put the terminal's selection in the application, in **screen** cells.
That made copying from scrollback impossible: `TextBuffer::get_plain_text` read
rows with `get_row_at_screen_line`, so a selection that started in history
copied whatever the live screen had in that row. The view worked around it by
clearing the selection when it scrolled.

Upstream has one coordinate system for this. A selection stores **buffer**
coordinates (`_ConvertToBufferCell` adds `_VisibleStartIndex`), and
`GetPlainText` reads rows with `GetRowByOffset`, which addresses the whole
buffer — scrollback first, then the visible screen. `CopyRequest` carries a
`buffer_coordinates` flag for exactly this case.

This port keeps its history in a separate queue, so the upstream mapping had no
implementation: `get_row_by_buffer_line` subtracts the configured scrollback
height, which is only right when the history is full.

## Decision

Document coordinates are the selection's coordinate system:

- Row 0 is the oldest row still held: history first, then the visible screen.
  `TextBuffer::document_height` and `get_row_by_document_line` expose it.
- A `CopyRequest` with `buffer_coordinates` walks those rows in
  `get_plain_text`, `gen_html` and `gen_rtf`.
- `view::Selection` stores document cells. The window maps a screen row to a
  document row with `history - scroll + screen_row`, and `grid::Screen::top`
  tells the painter which document row is on screen row zero.
- `get_row_by_buffer_line` stays as the fixed-slot mapping for the operations
  that use it; the copy path no longer depends on it.

## Rationale

- **One coordinate system.** Selection, copying and painting all agree, so
  nothing has to convert at the clipboard boundary.
- **The copy path stays the oracle.** History copies use `CopyRequest` and its
  block/line, wide-glyph and wrap rules, not a second extractor.
- **Scrolling is not destructive.** A selection no longer has to be cleared
  when the view moves.

## Consequences

Positive:

- A selection survives scrolling and can span history and screen.
- HTML and RTF copies get the same reach as plain text.

Negative:

- When the scrollback is full and evicts its oldest rows, document indices
  shift under an existing selection. Upstream's buffer coordinates have the
  same property; a selection older than the retained history is not tracked
  across eviction.
- Every copy path that reads rows must go through `request_row`, or document
  mode and screen mode will disagree.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep screen coordinates and clear on scroll | Cannot copy history; the limitation this ADR removes |
| Extract history text in the application | A second copy implementation, against ADR 019 and the one-path rule |
| Fix only `get_plain_text` | HTML and RTF would silently copy the wrong rows |
