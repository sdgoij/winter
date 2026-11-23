# ADR 021 — A tab owns one terminal; the window owns the chords

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winter/src/tabs.rs`, `crates/winter/src/ui.rs`;
  ADR 018, ADR 019

## Context

The Phase 8 shell owned one session, one `Terminal` and one `View` directly in
`App`. Phase 9 needs more than one, and the plan also names panes. The list of
tabs, the active-index rules and the window chords have no upstream file: they
are the application's own composition of `server` and `term`.

Two things must not be invented per feature: where the active index lives, and
which layer decides that a chord belongs to the window rather than the shell.

## Decision

- `crates/winter/src/tabs.rs` is a toolkit-free `Tabs<T>`: the list, the
  active index, add, close, select and wrap. The active index is kept valid
  when a neighbour closes.
- `Tab` in `ui.rs` owns one session, its `Terminal`, its `View`, its OSC title
  and a fallback title (the program's file stem). One tab is one terminal
  until panes exist; a pane tree will sit inside `Tab`.
- The window owns the clipboard, the modifier sides, the blink state and the
  window title. A `Tab` never owns a clipboard handle.
- Window chords are recognised once, in `command_for`, before a tab sees the
  events: Ctrl+Shift+T (new), Ctrl+Shift+W (close), Ctrl+Tab / Ctrl+Shift+Tab
  (next/previous), Ctrl+Shift+C/V (copy/paste). Everything else is forwarded.
- Every tab keeps draining its pty on each frame, active or not, so a
  background shell cannot block on a full pty buffer.

## Rationale

- **One home for the active index.** A list with its own invariants is testable
  without a pty; `App` only applies commands.
- **A chord is either the window's or the shell's.** Deciding in the event
  splitter is the only way a shell can be sure it never receives Ctrl+Shift+T.
- **One clipboard.** Platform clipboard handles are global; giving each tab its
  own handle would fight over ownership on X11.

## Consequences

Positive:

- Tabs are added, closed and switched with tested list rules, and a background
  tab keeps running.
- Panes have an obvious owner: `Tab` grows a pane tree instead of a second
  terminal in `App`.

Negative:

- Dragging tabs, tab close animations and per-tab clipboard formats are not
  implemented; the strip is buttons.
- The chord map is a small hand-written table rather than the settings model
  Phase 10 will supply; when settings arrive, this table moves.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep one terminal in `App` | No place for a second session, and panes would duplicate the state |
| Put the tab list in `ui.rs` | The index rules would then need a window to test |
| Give each tab its own clipboard | Platform ownership is global; one handle is the safe model |
| Let the shell see every chord | A shell would receive Ctrl+Shift+T and the tab would not open |
