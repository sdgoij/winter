# ADR 019 — The terminal's viewport and selection are application state

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/view.rs`, `crates/winterm/src/grid.rs`;
  ADR 014, ADR 018

## Context

ADR 014 made `host::SelectionState` the home of the console's selection and
removed the buffer's own `Selection`. That decision is about
`SCREEN_INFORMATION`: the console host selects screen cells on behalf of a
client, and the host's buffers are what it operates on.

Upstream has a second selection. `src/cascadia/TerminalCore` keeps
`Terminal::SelectionInfo` (start, end, pivot, block flag) with word, line and
character expansion, and `ControlInteractivity` counts clicks and chooses the
expansion. It selects the terminal's own page buffer and copies through
`TextBuffer::CopyRequest`. This port's application renders the adapter's
`PageManager` buffer, not a `host::ScreenInformation`, so `host::SelectionState`
cannot be reused: its mutating operations take a `ConsoleInformation` and a
`ScreenInformation` the application does not have.

## Decision

`crates/winterm/src/view.rs` owns the terminal-side view state:

- the scroll offset into the buffer's history,
- the selection in screen cells, with character, word, line and block modes,
- the multi-click counter and its timer.

Text extraction goes through the buffer's one copy path, `buffer::CopyRequest`,
not a second implementation.

The console's selection stays in `host` (ADR 014). The two are separate because
upstream keeps them separate: one serves a client reading console cells, the
other serves the user selecting the terminal's screen.

Selection is limited to the visible screen until the copy path can address
history rows; scrolling clears it rather than pointing at different text.

## Rationale

- **A missing dependency, not a preference.** `SelectionState` needs a console
  and a screen; the application has neither.
- **One selection per layer, as upstream.** Reusing the host type for the app
  would put console flags and a saved cursor on a user-facing drag.
- **One copy path.** `CopyRequest` already carries the block/line semantics and
  the wide-glyph widening; the app supplies points and a mode.

## Consequences

Positive:

- The application has a tested, toolkit-free selection and scroll model; a
  native backend reuses it (ADR 018).
- The console host's selection is untouched, so Phase 6's tests keep meaning.

Negative:

- Two selection types exist in the tree. That is upstream's shape, but a future
  reader must not merge them: they select different buffers for different
  owners.
- Selecting across scrollback is deferred until `CopyRequest` addresses history
  rows; the view clears the selection on scroll meanwhile.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Reuse `host::SelectionState` in the app | It requires a `ConsoleInformation` and `ScreenInformation`; the app's buffer is the adapter's page |
| Move the selection state into `buffer` | The buffer is shared by both owners; state there would be a third home |
| Defer selection until the app crate has tabs | The copy path and `view` are independent of tabs, and the window is already usable without it |
