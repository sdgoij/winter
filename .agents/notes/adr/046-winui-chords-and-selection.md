# ADR 046 — The WinUI shell owns its chords and selects with the pointer

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/winui/app.rs`, `crates/winterm/src/view.rs`,
  `crates/winterm/src/term_control.rs`, `crates/settings/src/keybindings.rs`,
  ADR 019, ADR 023

## Context

The WinUI shell sent every key to the terminal and ignored the pointer. A file
that bound `ctrl+shift+c` to `copy` changed nothing on Windows, and that chord
reached the program as `^C`. There was no way to copy text out at all.

The egui shell already has the pieces: keybindings are settings data with their
own chord syntax (ADR 023), selection lives in document coordinates in the
application (ADR 019), and the buffer owns the copy path.

## Decision

- A key is matched against the settings file's chords before the terminal sees
  it. A bound chord runs its action and is not encoded for the program, so
  `copy`, `paste`, `newTab`, `closeTab`, `nextTab` and `prevTab` work as the file
  says.
- The preview-key handler reports a *bound chord* as handled, so the toolkit
  does not also raise `CharacterReceived` and type the letter. An unbound
  printable key still falls through to the text path, and AltGr text is
  unaffected because no chord is bound to it by default.
- The pane captures the pointer on a press. Without capture the swap chain
  consumes the moves and a drag never reaches the pane.
- A press starts a selection in document coordinates through the existing
  `View`: a double click takes the word, a triple click the row; a drag extends
  it, and a drag past the pane claims the edge row and scrolls one row per step.
- Copy goes through the buffer's copy path and paste through the terminal's
  bracketed-paste filter; `arboard` is the clipboard.
- While an application tracks the mouse and Shift is not held, presses, moves
  and releases are forwarded to it instead of selecting (ADR 019).

## Rationale

One rule for both shells keeps a file's chords meaningful, and routing every
chord through one matcher means a future action is a match arm rather than a
second key path. The clipboard is a host concern, so it stays in the shell.

## Consequences

- A chord the file binds suppresses the character the toolkit would type. An
  unbound Control chord can still type its letter; that predates this record.
- The renderer's selection is only as good as the document coordinates handed to
  it, so a scroll between the press and the drag must be applied to the anchor
  too — the drag path reads the history length after scrolling.
- A flip-model swap chain rotates its buffers, so the Direct2D target is
  re-acquired from buffer 0 every frame. Drawing into the retained target left a
  frame on screen that was never painted.

## Alternatives considered

- **Handle chords inside `Control`.** The terminal does not know the window's
  bindings, and a chord the file unbound must reach the program again.
- **Suppress every Control chord in the preview handler.** That breaks AltGr,
  which Windows reports as Control plus Alt.
- **Select with screen coordinates.** The selection would move under a scroll;
  document coordinates are what the buffer's copy path expects (ADR 020).
