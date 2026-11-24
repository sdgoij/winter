# ADR 045 — The WinUI shell draws the active tab

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winui/src/app.rs`, `crates/winter/src/tabs.rs`,
  `crates/winter/src/terminal_render.rs`, ADR 021, ADR 041, ADR 043

## Context

ADR 021 says a tab owns one terminal and the window owns the chords. The egui
shell splits a tab into panes, each with its own widget and paint path. The
WinUI shell has one `SwapChainPanel`: a panel owns a swap chain, and Direct2D
blits one software buffer into it.

A tab whose content is a `TabViewItem` would need its own panel, so switching
tabs would mount and unmount panels, re-attach swap chains and lose the
presented frame. It would also bind a renderer and a surface to each tab while
only one is visible.

## Decision

There is one renderer, one shared backend and one swap chain, and the active
tab's `Control` feeds them.

- A `Tab` is a `Control`, a `View` and a title; `Tabs` (toolkit-free, ADR 018)
  owns the list and the active index.
- Every tab's pty is pumped on each tick, so a background tab keeps parsing
  output and its program can exit; only the active tab is refreshed into the
  renderer and painted.
- The tab strip is a `TabView` above the pane in a two-row `Grid`, so the panel
  stays mounted and switching tabs never rebuilds the surface.
- Tab items carry a stable string key (`tab-<id>`); the close and reorder events
  name that key, and the shell maps it back to the tab.
- The grid is shared: a window resize resizes every tab's terminal, and a new
  tab starts at the active grid.

## Rationale

The visible cost of a background tab is parsing and scrollback, not pixels. One
paint path means a scheme, a DPI change and a face rebuild affect exactly one
renderer, and switching tabs is a `refresh` call rather than a surface rebuild.

## Consequences

- A background tab cannot be painted, captured or measured. Anything that needs
  a tab's pixels (thumbnails, accessibility text) must look at the buffer, not
  the surface.
- `ShapedRow` state belongs to the active tab only; the engine must start each
  frame with empty rows, which it now does (a stale row would draw another tab's
  glyphs over this one's).
- Closing the last tab closes the window, as an exit already did (ADR 036).

## Alternatives considered

- **One `SwapChainPanel` per tab in each `TabViewItem`.** Correct per tab and
  more code: every tab needs a renderer and a surface, and each switch detaches
  and re-attaches a swap chain.
- **A `TabView` with the pane as the selected item's content.** The panel is
  destroyed and recreated on every selection, so the surface is rebuilt for a
  change that only needs a different buffer.
- **Integer keys for tab items.** The toolkit's close event carries an opaque
  `Key`; only string keys can be read back, so the id is formatted into one.
