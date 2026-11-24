# ADR 050 — The WinUI find bar is an overlay over shared find state

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winter/src/find.rs`, `crates/winui/src/app.rs`,
  `crates/winter/src/terminal_render.rs`, `crates/renderer/src/renderer.rs`;
  ADR 019, ADR 046

## Context

The egui pane has searched the buffer since the find bar landed: `find_text`
reports matches in document coordinates, the pane keeps the query and current
match, and it paints the matches itself. The WinUI shell's `Find` chord fell
through its `perform` match, so Ctrl+Shift+F did nothing on Windows.

The state is toolkit-free, and the renderer's `RenderFrameInfo` already carried
`search_highlights` and `search_highlight_focused`; the software engine accepted
them but never painted them.

## Decision

- The state lives in `crate::find::FindBar`, shared by both shells. The egui
  `Pane` and the WinUI `Tab` each own one, so the query survives closing the bar
  and switching tabs keeps each tab's search.
- The WinUI bar is an overlay in the pane's `Grid`, aligned top-right: a
  `TextBox`, previous and next buttons, and the match count. Ctrl+Shift+F opens
  it and requests focus; Enter and Shift+Enter step; Escape closes it.
- While the bar is open, the pane forwards no key or character to the terminal.
  A bound chord still reaches the window, so Ctrl+Shift+F closes it again.
- `FindBar` converts document matches to screen `PointSpan`s (`highlight_spans`).
  `TerminalData` reports them through `search_highlights`, and the renderer
  paints them with the selection brush in one pass after the selection.

## Rationale

One state machine with two renderers is the one-home rule applied to search.
The overlay keeps the terminal running and visible, and Escape gives the only
mode transition the user needs. Highlights belong in the renderer: the shell
has no business writing pixels or duplicating cell metrics.

## Consequences

- The bar covers the top-right cells; the terminal scrolls under it.
- Every match uses the selection brush. The current match is named by the count
  and brought to the middle of the view, not painted a different colour; the
  engine has one selection brush and no second highlight colour.
- While the bar is open the terminal receives no typing even if its field loses
  focus. Escape returns input to the pane.
- A match is computed when the query changes, not when the buffer changes; new
  output does not move the highlights until the query is edited or the bar is
  reopened.

## Alternatives considered

- **A separate find window.** It loses the terminal context and adds focus
  handling for a bar that is one line tall.
- **Paint highlights in the shell.** It repeats the cell metrics and the colour
  table that the renderer already owns.
- **Keep the find state in each shell.** The stepping, wrapping and scroll rules
  would drift the first time one side is changed.
